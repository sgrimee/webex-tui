//! Controlled fake-only transport tests: no runtime startup, sockets or sleeps.
use super::*;
use crate::{
    bridge::{
        session::{SessionFailure, SessionLifecycle, SessionPhase, SubscriptionId},
        write_dispatch::UiWriteBackend,
    },
    frontend::{
        continuity::{ContinuityBoundary, DaemonInstanceId, LifecycleControl, UiLifecycleFrame},
        data::{ComposedBody, MessageId, MessageSnapshot, MessageStatus, RoomId},
        editor_submission::{
            DraftRevision, DraftSnapshot, DraftStamp, DraftTarget, EditorSubmission,
            QueueAdmission, SubmissionError,
        },
        editor_write::EditorWriteStatus,
        envelopes::{
            FrontendError, FrontendIntent, Operation, ReadOutcome, RequestSequence, WriteOutcome,
        },
        events::{ChangeScope, ResyncReason},
        lifecycle::{LifecycleProjection, ProjectionEffects},
        requests::RoomViewRequests,
    },
};
use std::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};
use tokio::sync::oneshot;
use webex_bridge::{client::ClientError, dto};

type Reply = Result<dto::Message, ClientError>;

struct Received {
    room: String,
    anchor: Option<String>,
    body: dto::MessageBodyInput,
    edit: bool,
}

struct Control {
    /// When set, session issue never completes: models a lifecycle future
    /// dropped after its first phase publication.
    issue_pending: std::sync::atomic::AtomicBool,
    calls: AtomicUsize,
    polls: AtomicUsize,
    future_drops: AtomicUsize,
    received: Mutex<Option<Received>>,
    reply: Mutex<Option<oneshot::Receiver<Reply>>>,
}

struct Writer(Arc<Control>);
struct WriteFuture {
    control: Arc<Control>,
    reply: oneshot::Receiver<Reply>,
}
impl Future for WriteFuture {
    type Output = Reply;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Reply> {
        self.control.polls.fetch_add(1, Ordering::SeqCst);
        match Pin::new(&mut self.reply).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(reply)) => Poll::Ready(reply),
            Poll::Ready(Err(_)) => Poll::Ready(Err(ClientError::Transport(
                std::io::Error::other("controlled completion lost").into(),
            ))),
        }
    }
}
impl Drop for WriteFuture {
    fn drop(&mut self) {
        self.control.future_drops.fetch_add(1, Ordering::SeqCst);
    }
}
impl Writer {
    fn write(
        &mut self,
        room: &str,
        anchor: Option<&str>,
        body: &dto::MessageBodyInput,
        edit: bool,
    ) -> WriteFuture {
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        *self.0.received.lock().unwrap() = Some(Received {
            room: room.into(),
            anchor: anchor.map(str::to_owned),
            body: dto::MessageBodyInput {
                kind: body.kind.clone(),
                content: body.content.clone(),
            },
            edit,
        });
        WriteFuture {
            control: Arc::clone(&self.0),
            reply: self.0.reply.lock().unwrap().take().expect("no replay"),
        }
    }
}
impl UiWriteBackend for Writer {
    fn send(
        &mut self,
        room: &str,
        parent: Option<&str>,
        body: &dto::MessageBodyInput,
    ) -> impl Future<Output = Reply> + Send {
        self.write(room, parent, body, false)
    }
    fn edit(
        &mut self,
        message: &str,
        room: &str,
        body: &dto::MessageBodyInput,
    ) -> impl Future<Output = Reply> + Send {
        self.write(room, Some(message), body, true)
    }
    async fn delete(&mut self, _: &str) -> Result<(), ClientError> {
        panic!("editor-only seam")
    }
    async fn leave(&mut self, _: &str) -> Result<(), ClientError> {
        panic!("editor-only seam")
    }
}

struct ClockState {
    now: Instant,
    expire_after_reads: Option<usize>,
}
#[derive(Clone)]
struct Clock(Arc<Mutex<ClockState>>);
impl Clock {
    fn advance(&self, seconds: u64) {
        self.0.lock().unwrap().now += Duration::from_secs(seconds);
    }
}
impl SessionClock for Clock {
    fn now(&self) -> Instant {
        let mut state = self.0.lock().unwrap();
        if let Some(reads) = state.expire_after_reads {
            if reads == 0 {
                state.now += Duration::from_secs(100);
                state.expire_after_reads = None;
            } else {
                state.expire_after_reads = Some(reads - 1);
            }
        }
        state.now
    }
}
struct Backend(Arc<Control>);
impl SessionBackend for Backend {
    type Client = Arc<Control>;
    type Stream = ();
    type Writer = Writer;
    type SnapshotReader<'a> = ();
    async fn issue(&mut self, _: &str) -> Result<Self::Client, SessionFailure> {
        if self.0.issue_pending.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        Ok(Arc::clone(&self.0))
    }
    async fn subscribe(&mut self, _: &Self::Client) -> Result<(), SessionFailure> {
        Ok(())
    }
    async fn continuity(&mut self, _: &Self::Client) -> Result<ContinuityBoundary, SessionFailure> {
        Ok(marker())
    }
    fn lease_ttl(_: &Self::Client) -> u64 {
        100
    }
    fn writer(client: &Self::Client) -> Writer {
        Writer(Arc::clone(client))
    }
    fn snapshot_reader(_: &Self::Client) {}
}
#[derive(Default)]
struct Host {
    snapshots: usize,
}
impl PresentationHost for Host {
    type Snapshot = ();
    fn session_phase(&mut self, _: SessionPhase) {}
    fn lifecycle(&mut self, _: &LifecycleProjection, _: ProjectionEffects) {}
    fn install_snapshot(&mut self, _: ()) {
        self.snapshots += 1;
    }
}

type Worker = CommandWorker<Backend, Clock, Host>;
struct Fixture {
    endpoint: CommandMailbox,
    worker: Worker,
    control: Arc<Control>,
    reply: Option<oneshot::Sender<Reply>>,
    clock: Clock,
}
fn fixture() -> Fixture {
    let (tx, rx) = oneshot::channel();
    let control = Arc::new(Control {
        issue_pending: std::sync::atomic::AtomicBool::new(false),
        calls: AtomicUsize::new(0),
        polls: AtomicUsize::new(0),
        future_drops: AtomicUsize::new(0),
        received: Mutex::new(None),
        reply: Mutex::new(Some(rx)),
    });
    let clock = Clock(Arc::new(Mutex::new(ClockState {
        now: Instant::now(),
        expire_after_reads: None,
    })));
    let (endpoint, worker) = CommandWorker::new(
        SessionLifecycle::new(Backend(Arc::clone(&control)), clock.clone()),
        Host::default(),
    );
    Fixture {
        endpoint,
        worker,
        control,
        reply: Some(tx),
        clock,
    }
}
fn marker() -> ContinuityBoundary {
    ContinuityBoundary {
        instance_id: DaemonInstanceId::new("synthetic-instance"),
        watermark: 0,
    }
}
fn lag() -> UiLifecycleFrame {
    UiLifecycleFrame::Control(LifecycleControl::ResyncRequired {
        boundary: marker(),
        scope: ChangeScope::WholeMirror,
        reason: ResyncReason::EventLag,
    })
}
async fn install(worker: &mut Worker) {
    let ticket = worker.begin_snapshot().await.unwrap();
    worker.commit_snapshot(ticket, ()).unwrap();
}
async fn active(f: &mut Fixture) -> SubscriptionId {
    let sub = f.worker.connect("fake-paired-secret").await.unwrap();
    f.worker
        .frame(
            sub,
            UiLifecycleFrame::Control(LifecycleControl::Ready(marker())),
        )
        .unwrap();
    install(&mut f.worker).await;
    sub
}
fn draft(target: DraftTarget, body: ComposedBody) -> DraftSnapshot {
    DraftSnapshot {
        stamp: DraftStamp {
            target,
            revision: DraftRevision::default(),
        },
        body,
    }
}
fn targets() -> [DraftTarget; 3] {
    [
        DraftTarget::Send {
            room_id: RoomId::new("room/a?opaque"),
        },
        DraftTarget::Reply {
            room_id: RoomId::new("room/a?opaque"),
            parent_id: MessageId::new("parent/a?opaque"),
        },
        DraftTarget::Edit {
            room_id: RoomId::new("room/a?opaque"),
            message_id: MessageId::new("message/a?opaque"),
        },
    ]
}
fn plain() -> DraftSnapshot {
    draft(
        targets()[0].clone(),
        ComposedBody::PlainText("private body".into()),
    )
}
fn copy(request: &RequestEnvelope) -> RequestEnvelope {
    RequestEnvelope {
        context: request.context.clone(),
        intent: request.intent.clone(),
    }
}
fn request() -> RequestEnvelope {
    EditorSubmission::default().prepare(&plain()).unwrap()
}
fn admitted(f: &Fixture, ledger: &mut EditorSubmission, draft: &DraftSnapshot) -> RequestEnvelope {
    let request = ledger.prepare(draft).unwrap();
    assert_eq!(
        f.endpoint.try_submit(copy(&request)),
        CommandAdmission::Accepted
    );
    ledger
        .admit(&request.context, QueueAdmission::Accepted)
        .unwrap();
    request
}
fn message(target: &DraftTarget) -> dto::Message {
    let (room, id, parent) = match target {
        DraftTarget::Send { room_id } => (room_id, "new/message?opaque", None),
        DraftTarget::Reply { room_id, parent_id } => (
            room_id,
            "new/message?opaque",
            Some(parent_id.as_str().into()),
        ),
        DraftTarget::Edit {
            room_id,
            message_id,
        } => (room_id, message_id.as_str(), None),
    };
    dto::Message {
        id: id.into(),
        room_id: room.as_str().into(),
        revision: 3,
        parent_id: parent,
        sender_id: None,
        created_at: None,
        updated_at: None,
        status: "active".into(),
        markdown: None,
        body: Some(dto::MessageBody {
            kind: "plainText".into(),
            content: "private canonical".into(),
        }),
        attachment_count: 0,
    }
}
fn send_reply(f: &mut Fixture, reply: Reply) {
    assert!(f.reply.take().unwrap().send(reply).is_ok());
}
fn error() -> FrontendError {
    FrontendError {
        code: "safe".into(),
        message: "safe".into(),
    }
}
fn snapshot() -> MessageSnapshot {
    MessageSnapshot {
        id: MessageId::new("new"),
        room_id: RoomId::new("room/a?opaque"),
        parent_id: None,
        revision: Some(1),
        sender_id: None,
        sender_email: None,
        created_at: None,
        updated_at: None,
        status: MessageStatus::Active,
        body: None,
        attachment_count: None,
    }
}
fn poll_once<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}
fn count(control: &Control) -> usize {
    control.calls.load(Ordering::SeqCst)
}
fn assert_result(
    terminal: &SubmissionTerminal,
    request: &RequestEnvelope,
    result: &FrontendResult,
) {
    let SubmissionTerminal::Result(envelope) = terminal else {
        panic!("expected exact terminal")
    };
    assert_eq!(envelope.context, request.context);
    assert!(envelope.result == *result);
}
fn assert_unstarted(terminal: SubmissionTerminal, request: &RequestEnvelope) {
    assert!(
        matches!(terminal, SubmissionTerminal::NotDispatched { context, reason: NotDispatchedReason::UnstartedShutdown } if context == request.context)
    );
}

#[tokio::test]
async fn exact_send_reply_edit_fields_context_and_formats_roundtrip() {
    for target in targets() {
        for body in [
            ComposedBody::PlainText("**literal**".into()),
            ComposedBody::Markdown("**markdown**".into()),
        ] {
            let mut f = fixture();
            active(&mut f).await;
            let draft = draft(target.clone(), body.clone());
            let mut ledger = EditorSubmission::default();
            let request = admitted(&f, &mut ledger, &draft);
            assert!(matches!(ledger.prepare(&draft), Err(SubmissionError::Busy)));
            send_reply(&mut f, Ok(message(&target)));
            assert!(f.worker.handle_one().await);
            let received = f.control.received.lock().unwrap().take().unwrap();
            assert_eq!(received.room, "room/a?opaque");
            let (anchor, edit) = match &target {
                DraftTarget::Send { .. } => (None, false),
                DraftTarget::Reply { parent_id, .. } => (Some(parent_id.as_str()), false),
                DraftTarget::Edit { message_id, .. } => (Some(message_id.as_str()), true),
            };
            assert_eq!(received.anchor.as_deref(), anchor);
            assert_eq!(received.edit, edit);
            match &body {
                ComposedBody::PlainText(text) => {
                    assert_eq!(received.body.kind, "plainText");
                    assert_eq!(&received.body.content, text);
                }
                ComposedBody::Markdown(text) => {
                    assert_eq!(received.body.kind, "markdown");
                    assert_eq!(&received.body.content, text);
                }
            }
            let terminal = f.endpoint.drain().unwrap();
            let SubmissionTerminal::Result(envelope) = &terminal else {
                panic!("expected result")
            };
            assert_eq!(envelope.context, request.context);
            assert_eq!(envelope.result.operation(), request.context.operation);
            let effects = ledger.reduce(terminal, Some(&draft.stamp));
            assert_eq!(effects.status, Some(EditorWriteStatus::Ready));
            assert!(effects.clear_current == Some(draft.stamp));
            assert!(effects.canonical.is_some());
            assert_eq!(count(&f.control), 1);
            assert!(!f.worker.handle_one().await);
        }
    }
}

#[tokio::test]
async fn slot_capacity_one_spans_queued_inflight_and_undrained_terminal() {
    let mut f = fixture();
    active(&mut f).await;
    assert_eq!(f.endpoint.try_submit(request()), CommandAdmission::Accepted);
    assert_eq!(f.endpoint.try_submit(request()), CommandAdmission::Full); // queued
    assert!(f.endpoint.drain().is_none());
    let mut handle = Box::pin(f.worker.handle_one());
    assert!(poll_once(handle.as_mut()).is_pending());
    assert_eq!(count(&f.control), 1);
    assert_eq!(f.endpoint.try_submit(request()), CommandAdmission::Full); // inflight, queue empty
    assert!(f.endpoint.drain().is_none());
    for _ in 0..3 {
        assert!(poll_once(handle.as_mut()).is_pending());
    }
    assert_eq!(count(&f.control), 1); // same future retained, never recreated
    assert!(f
        .reply
        .take()
        .unwrap()
        .send(Ok(message(&targets()[0])))
        .is_ok());
    assert!(handle.await);
    assert_eq!(f.endpoint.try_submit(request()), CommandAdmission::Full); // undrained
    assert!(f.endpoint.drain().is_some());
    assert!(f.endpoint.drain().is_none());
    assert!(!f.worker.handle_one().await);
    assert_eq!(count(&f.control), 1);
    assert_eq!(f.endpoint.try_submit(request()), CommandAdmission::Accepted);
    f.worker.shutdown();
    assert!(f.endpoint.drain().is_some()); // new work not replay
}

#[tokio::test]
async fn closed_full_not_ready_reports_preserve_ledger_draft_and_sequence() {
    for refusal in [
        CommandAdmission::Full,
        CommandAdmission::Closed,
        CommandAdmission::NotReady,
    ] {
        let mut f = fixture();
        if refusal != CommandAdmission::NotReady {
            active(&mut f).await;
        }
        if refusal == CommandAdmission::Full {
            assert_eq!(f.endpoint.try_submit(request()), CommandAdmission::Accepted);
        }
        if refusal == CommandAdmission::Closed {
            f.worker.shutdown();
        }
        let original = plain();
        let mut ledger = EditorSubmission::default();
        let first = ledger.prepare(&original).unwrap();
        assert_eq!(f.endpoint.try_submit(copy(&first)), refusal);
        // Mailbox refusals ARE ledger reports: no lossy mapping step.
        let expected = match refusal {
            CommandAdmission::Full => SubmissionError::QueueFull,
            CommandAdmission::Closed => SubmissionError::QueueClosed,
            _ => SubmissionError::QueueNotReady,
        };
        assert_eq!(ledger.admit(&first.context, refusal), Err(expected));
        assert_eq!(ledger.status(), EditorWriteStatus::Unavailable);
        assert!(ledger.pending_context().is_none());
        assert!(ledger.review().is_none());
        let next = ledger.prepare(&original).unwrap();
        assert_ne!(next.context.id, first.context.id);
        assert!(first.intent == next.intent);
        assert!(original == plain());
        assert_eq!(count(&f.control), 0);
    }
}

#[tokio::test]
async fn invalid_bounds_anchors_operations_and_retargeted_context_refuse_zero_io() {
    let mut f = fixture();
    active(&mut f).await;
    let mut invalid = Vec::new();
    for body in ["".into(), "nul\0text".into(), "é".repeat(3720)] {
        let mut r = request();
        if let FrontendIntent::SendMessage { body: field, .. } = &mut r.intent {
            *field = ComposedBody::Markdown(body);
        }
        invalid.push(r);
    }
    for id in ["".into(), "bad\nanchor".into(), "x".repeat(1025)] {
        let mut r = request();
        if let FrontendIntent::SendMessage { parent_id, .. } = &mut r.intent {
            *parent_id = Some(MessageId::new(id));
        }
        invalid.push(r);
    }
    let mut r = request();
    r.context.operation = Operation::EditMessage;
    invalid.push(r);
    let mut views = RoomViewRequests::default();
    views.select_room(Some("room/a?opaque".into())).unwrap();
    let stamp = views.start_read().unwrap().unwrap();
    let mut r = RequestSequence::default()
        .begin(request().intent, Some(stamp))
        .unwrap();
    if let FrontendIntent::SendMessage { room_id, .. } = &mut r.intent {
        *room_id = RoomId::new("room/b");
    }
    invalid.push(r);
    let mut r = request();
    r.intent = FrontendIntent::DeleteMessage {
        room_id: RoomId::new("r"),
        message_id: MessageId::new("m"),
    };
    r.context.operation = Operation::DeleteMessage;
    invalid.push(r);
    for r in invalid {
        assert_eq!(f.endpoint.try_submit(r), CommandAdmission::InvalidRequest);
    }
    assert!(!f.worker.handle_one().await);
    assert_eq!(count(&f.control), 0);
    let mut r = request();
    if let FrontendIntent::SendMessage { body, .. } = &mut r.intent {
        *body = ComposedBody::PlainText("é".repeat(3719) + "x");
    }
    assert_eq!(f.endpoint.try_submit(r), CommandAdmission::Accepted); // exact 7439 bytes
    f.worker.shutdown();
}

#[tokio::test]
async fn full_room_view_context_is_retained_even_when_focus_changes() {
    let mut f = fixture();
    active(&mut f).await;
    let mut views = RoomViewRequests::default();
    views.select_room(Some("room/a?opaque".into())).unwrap();
    let stamp = views.start_read().unwrap().unwrap();
    let r = RequestSequence::default()
        .begin(request().intent, Some(stamp))
        .unwrap();
    assert_eq!(f.endpoint.try_submit(copy(&r)), CommandAdmission::Accepted);
    views.select_room(Some("room/b".into())).unwrap();
    send_reply(&mut f, Ok(message(&targets()[0])));
    f.worker.handle_one().await;
    let SubmissionTerminal::Result(result) = f.endpoint.drain().unwrap() else {
        panic!("result")
    };
    assert_eq!(result.context, r.context);
    assert!(!result.is_current_room_view(&views));
    assert_eq!(count(&f.control), 1);
}

#[tokio::test]
async fn stale_snapshot_generation_refuses_even_after_same_subscription_ready_again() {
    let mut f = fixture();
    let sub = active(&mut f).await;
    let r = request();
    assert_eq!(f.endpoint.try_submit(copy(&r)), CommandAdmission::Accepted);
    let old = f.worker.controller.write_admission_stamp();
    f.worker.controller.frame(sub, lag()).unwrap();
    install(&mut f.worker).await;
    assert_eq!(f.worker.controller.subscription(), Some(sub));
    assert_eq!(f.worker.controller.phase(), SessionPhase::Active);
    assert!(old != f.worker.controller.write_admission_stamp());
    assert!(f.worker.handle_one().await);
    assert!(
        matches!(f.endpoint.drain(), Some(SubmissionTerminal::NotDispatched { context, reason: NotDispatchedReason::NotReady }) if context == r.context)
    );
    assert_eq!(count(&f.control), 0);
    assert!(!f.worker.handle_one().await);
}

#[tokio::test]
async fn stale_subscription_and_actual_unpublished_readiness_refuse_without_waiting() {
    for state in 0..5 {
        let mut f = fixture();
        let sub = active(&mut f).await;
        let r = request();
        assert_eq!(f.endpoint.try_submit(copy(&r)), CommandAdmission::Accepted);
        match state {
            0 => {
                f.worker.controller.frame(sub, lag()).unwrap();
            }
            1 => f.clock.advance(100), // published readiness remains Active, actual expired
            2 => {
                f.worker
                    .controller
                    .stream_failed(sub, SessionFailure::Transport);
            }
            3 => f.worker.controller.disconnect(),
            _ => {
                active(&mut f).await;
            } // ready replacement must not send old command
        }
        assert!(f.worker.handle_one().await);
        assert_eq!(count(&f.control), 0);
        let terminal = f.endpoint.drain().unwrap();
        assert!(
            matches!(terminal, SubmissionTerminal::NotDispatched { context, .. } if context == r.context)
        );
        assert!(!f.worker.handle_one().await); // no delayed reconnect queue
    }
}

#[tokio::test]
async fn lease_expires_between_dequeue_fence_and_gate_error_proves_zero_io() {
    let mut f = fixture();
    active(&mut f).await;
    let mut ledger = EditorSubmission::default();
    let draft = plain();
    let r = admitted(&f, &mut ledger, &draft);
    // Actual admission stamp reads a still-valid clock once. Gate's check_lease
    // then reads the controlled expiry and returns the proven pre-I/O error.
    f.clock.0.lock().unwrap().expire_after_reads = Some(1);
    assert!(f.worker.handle_one().await);
    let terminal = f.endpoint.drain().unwrap();
    assert!(
        matches!(&terminal, SubmissionTerminal::NotDispatched { context, reason: NotDispatchedReason::Expired } if context == &r.context)
    );
    assert_eq!(count(&f.control), 0);
    let effects = ledger.reduce(terminal, Some(&draft.stamp));
    assert_eq!(effects.status, Some(EditorWriteStatus::Unavailable));
    assert!(effects.clear_current.is_none());
    assert!(ledger.review().unwrap().draft == draft);
}

#[tokio::test]
async fn lifecycle_lease_hint_during_retained_future_cannot_replace_known_success() {
    let mut f = fixture();
    active(&mut f).await;
    let r = request();
    assert_eq!(f.endpoint.try_submit(copy(&r)), CommandAdmission::Accepted);
    let mut handle = Box::pin(f.worker.handle_one());
    assert!(poll_once(handle.as_mut()).is_pending());
    f.clock.advance(100); // only an injected tick, not concurrent controller access
    assert!(poll_once(handle.as_mut()).is_pending());
    assert_eq!(count(&f.control), 1);
    assert!(f
        .reply
        .take()
        .unwrap()
        .send(Ok(message(&targets()[0])))
        .is_ok());
    assert!(handle.await);
    assert_eq!(f.worker.controller.phase(), SessionPhase::Expired);
    f.worker.shutdown();
    let SubmissionTerminal::Result(envelope) = f.endpoint.drain().unwrap() else {
        panic!("success stays success after attempt even when lease expires")
    };
    assert_eq!(envelope.context, r.context);
    assert!(matches!(
        envelope.result,
        FrontendResult::SendMessage(WriteOutcome::Success(_))
    ));
}

#[test]
fn every_proven_session_error_maps_to_exact_not_dispatched() {
    for (error, reason) in [
        (SessionWriteError::NotReady, NotDispatchedReason::NotReady),
        (SessionWriteError::Expired, NotDispatchedReason::Expired),
        (
            SessionWriteError::Failed,
            NotDispatchedReason::SessionFailed,
        ),
        (
            SessionWriteError::Dispatch(DispatchError::WrongIntent),
            NotDispatchedReason::WrongIntent,
        ),
    ] {
        let r = request();
        let context = r.context.clone();
        assert!(
            matches!(complete(r, Err(error)), SubmissionTerminal::NotDispatched { context: actual, reason: actual_reason } if actual == context && actual_reason == reason)
        );
    }
}

#[test]
fn post_attempt_operation_resource_or_request_operation_mismatch_is_uncertain() {
    let mut wrong_room = snapshot();
    wrong_room.room_id = RoomId::new("other");
    let mut wrong_parent = snapshot();
    wrong_parent.parent_id = Some(MessageId::new("other"));
    let mut wrong_id = snapshot();
    wrong_id.id = MessageId::new("");
    for result in [
        FrontendResult::EditMessage(WriteOutcome::Rejected(error())),
        FrontendResult::WhoAmI(ReadOutcome::Rejected(error())),
        FrontendResult::SendMessage(WriteOutcome::Success(wrong_room)),
        FrontendResult::SendMessage(WriteOutcome::Success(wrong_parent)),
        FrontendResult::SendMessage(WriteOutcome::Success(wrong_id)),
    ] {
        let r = request();
        let context = r.context.clone();
        assert!(
            matches!(complete(r, Ok(result)), SubmissionTerminal::Uncertain { context: actual, reason: UncertainReason::CompletionMismatch } if actual == context)
        );
    }
    let mut r = request();
    r.context.operation = Operation::EditMessage;
    let context = r.context.clone();
    assert!(
        matches!(complete(r, Ok(FrontendResult::SendMessage(WriteOutcome::Rejected(error())))), SubmissionTerminal::Uncertain { context: actual, .. } if actual == context)
    );
    let draft = draft(targets()[2].clone(), ComposedBody::PlainText("body".into()));
    let r = EditorSubmission::default().prepare(&draft).unwrap();
    assert!(matches!(
        complete(
            r,
            Ok(FrontendResult::EditMessage(WriteOutcome::Success(
                snapshot()
            )))
        ),
        SubmissionTerminal::Uncertain { .. }
    ));
}

#[tokio::test]
async fn backend_rejection_unknown_transport_loss_and_invalid_success_are_retained_no_replay() {
    for status in 0..5 {
        let mut f = fixture();
        active(&mut f).await;
        let mut ledger = EditorSubmission::default();
        let draft = plain();
        let r = admitted(&f, &mut ledger, &draft);
        let expected = if status == 0 {
            EditorWriteStatus::Rejected
        } else {
            EditorWriteStatus::DeliveryUnknown
        };
        match status {
            0 => send_reply(
                &mut f,
                Err(ClientError::Api {
                    status: reqwest::StatusCode::FORBIDDEN,
                    code: "forbidden".into(),
                    message: "synthetic backend detail".into(),
                }),
            ),
            1 => send_reply(
                &mut f,
                Err(ClientError::Api {
                    status: reqwest::StatusCode::ACCEPTED,
                    code: "delivery_unknown".into(),
                    message: "synthetic backend detail".into(),
                }),
            ),
            2 => send_reply(
                &mut f,
                Err(ClientError::Transport(
                    std::io::Error::other("synthetic backend detail").into(),
                )),
            ),
            3 => {
                drop(f.reply.take());
            }
            _ => {
                let mut m = message(&draft.stamp.target);
                m.room_id = "wrong".into();
                send_reply(&mut f, Ok(m));
            }
        }
        f.worker.handle_one().await;
        f.worker.shutdown(); // retain exact known rejection/unknown, not fabricated interruption
        let terminal = f.endpoint.drain().unwrap();
        assert!(
            matches!(&terminal, SubmissionTerminal::Result(envelope) if envelope.context == r.context)
        );
        let effects = ledger.reduce(terminal, Some(&draft.stamp));
        assert_eq!(effects.status, Some(expected));
        assert!(effects.clear_current.is_none());
        assert!(ledger.review().unwrap().draft == draft);
        assert!(matches!(
            ledger.prepare(&draft),
            Err(SubmissionError::ReviewNeeded)
        ));
        assert_eq!(count(&f.control), 1);
        assert!(!f.worker.handle_one().await);
    }
}

#[tokio::test]
async fn completed_success_and_rejection_survive_backpressure_worker_drop_and_shutdown() {
    for rejected in [false, true] {
        let mut f = fixture();
        active(&mut f).await;
        let r = request();
        assert_eq!(f.endpoint.try_submit(copy(&r)), CommandAdmission::Accepted);
        if rejected {
            send_reply(
                &mut f,
                Err(ClientError::Api {
                    status: reqwest::StatusCode::FORBIDDEN,
                    code: "forbidden".into(),
                    message: "detail".into(),
                }),
            );
        } else {
            send_reply(&mut f, Ok(message(&targets()[0])));
        }
        let mut handle = Box::pin(f.worker.handle_one());
        assert!(matches!(poll_once(handle.as_mut()), Poll::Ready(true)));
        drop(handle); // caller discards its already-ready waiter, exact terminal survives
        assert_eq!(f.endpoint.try_submit(request()), CommandAdmission::Full);
        assert!(!f.worker.handle_one().await);
        f.clock.advance(100);
        f.worker.check_lease();
        f.worker.shutdown();
        f.worker.shutdown();
        drop(f.worker);
        let SubmissionTerminal::Result(result) = f.endpoint.drain().unwrap() else {
            panic!("retain known terminal, no fabrication")
        };
        assert_eq!(result.context, r.context);
        if rejected {
            assert!(matches!(
                result.result,
                FrontendResult::SendMessage(WriteOutcome::Rejected(_))
            ));
        } else {
            assert!(matches!(
                result.result,
                FrontendResult::SendMessage(WriteOutcome::Success(_))
            ));
        }
        assert!(f.endpoint.drain().is_none());
        assert_eq!(count(&f.control), 1);
        assert_eq!(f.endpoint.try_submit(request()), CommandAdmission::Closed);
    }
}

#[tokio::test]
async fn dropping_ui_endpoint_does_not_cancel_pending_write_or_erase_known_terminal() {
    let mut f = fixture();
    active(&mut f).await;
    assert_eq!(f.endpoint.try_submit(request()), CommandAdmission::Accepted);
    let shared = Arc::clone(&f.worker.shared); // private test observer, not a public capability
    let mut handle = Box::pin(f.worker.handle_one());
    assert!(poll_once(handle.as_mut()).is_pending());
    drop(f.endpoint);
    assert_eq!(f.control.future_drops.load(Ordering::SeqCst), 0);
    assert!(f
        .reply
        .take()
        .unwrap()
        .send(Ok(message(&targets()[0])))
        .is_ok());
    assert!(handle.await);
    assert_eq!(count(&f.control), 1);
    f.worker.shutdown();
    drop(f.worker);
    assert!(matches!(
        &lock(&shared).slot,
        Slot::Terminal(SubmissionTerminal::Result(_))
    ));
}

#[tokio::test]
async fn explicit_pending_future_interruption_is_uncertain_and_shutdown_never_replays() {
    let mut f = fixture();
    active(&mut f).await;
    let r = request();
    assert_eq!(f.endpoint.try_submit(copy(&r)), CommandAdmission::Accepted);
    let mut handle = Box::pin(f.worker.handle_one());
    assert!(poll_once(handle.as_mut()).is_pending());
    assert_eq!(count(&f.control), 1);
    drop(handle); // explicit owner shutdown/deadline; normal select must retain it
    assert_eq!(f.control.future_drops.load(Ordering::SeqCst), 1);
    f.worker.shutdown();
    drop(f.worker);
    assert!(
        matches!(f.endpoint.drain(), Some(SubmissionTerminal::Uncertain { context, reason: UncertainReason::AttemptInterrupted }) if context == r.context)
    );
    assert!(f.endpoint.drain().is_none());
    assert_eq!(count(&f.control), 1);
    assert!(f
        .reply
        .take()
        .unwrap()
        .send(Ok(message(&targets()[0])))
        .is_err());
    // A new mailbox owns no old work or recovery queue.
    let mut fresh = fixture();
    active(&mut fresh).await;
    assert!(!fresh.worker.handle_one().await);
    assert_eq!(count(&fresh.control), 0);
}

#[tokio::test]
async fn shutdown_and_worker_loss_before_attempt_prove_unstarted() {
    for drop_worker in [false, true] {
        let mut f = fixture();
        active(&mut f).await;
        let r = request();
        assert_eq!(f.endpoint.try_submit(copy(&r)), CommandAdmission::Accepted);
        // Constructing but never polling a future does not dequeue/attempt.
        drop(Box::pin(f.worker.handle_one()));
        if !drop_worker {
            f.worker.shutdown();
            assert!(!f.worker.handle_one().await);
        }
        drop(f.worker);
        assert_unstarted(f.endpoint.drain().unwrap(), &r);
        assert_eq!(count(&f.control), 0);
        assert_eq!(f.endpoint.try_submit(request()), CommandAdmission::Closed);
    }
}

#[tokio::test]
async fn reservation_drop_proofs_and_terminal_overwrite_fence() {
    let mut f = fixture();
    active(&mut f).await;
    let r = request();
    assert_eq!(f.endpoint.try_submit(copy(&r)), CommandAdmission::Accepted);
    let (_, reservation) = reserve(&f.worker.shared).unwrap();
    drop(reservation);
    assert_unstarted(f.endpoint.drain().unwrap(), &r);
    assert_eq!(count(&f.control), 0);
    assert_eq!(f.endpoint.try_submit(copy(&r)), CommandAdmission::Accepted);
    let (_, mut reservation) = reserve(&f.worker.shared).unwrap();
    reservation.attempted = true;
    let result = FrontendResult::SendMessage(WriteOutcome::Success(snapshot()));
    reservation.finish(complete(copy(&r), Ok(result.clone())));
    reservation.finished = false; // even a later guard failure cannot overwrite a terminal
    drop(reservation);
    f.worker.shutdown();
    assert_result(&f.endpoint.drain().unwrap(), &r, &result);
}

#[tokio::test]
async fn worker_never_borrows_presentation_draft_lock_and_newer_text_survives() {
    let mut f = fixture();
    active(&mut f).await;
    let current = Arc::new(Mutex::new(plain()));
    let mut ledger = EditorSubmission::default();
    let original = current.lock().unwrap().clone();
    admitted(&f, &mut ledger, &original);
    let mut handle = Box::pin(f.worker.handle_one());
    assert!(poll_once(handle.as_mut()).is_pending());
    {
        let mut draft = current.try_lock().expect("worker has no App/draft mutex");
        draft.stamp.revision = draft.stamp.revision.next().unwrap();
        draft.stamp.target = DraftTarget::Send {
            room_id: RoomId::new("room/b"),
        };
        draft.body = ComposedBody::Markdown("new private text".into());
    }
    assert!(f
        .reply
        .take()
        .unwrap()
        .send(Ok(message(&original.stamp.target)))
        .is_ok());
    handle.await;
    let current = current.lock().unwrap();
    let effects = ledger.reduce(f.endpoint.drain().unwrap(), Some(&current.stamp));
    assert_eq!(effects.status, Some(EditorWriteStatus::Ready));
    assert!(effects.clear_current.is_none());
    assert_eq!(effects.canonical.unwrap().room_id.as_str(), "room/a?opaque");
    assert!(current.body == ComposedBody::Markdown("new private text".into()));
}

#[test]
fn endpoints_worker_and_concrete_handle_future_are_send_without_exposing_capabilities() {
    fn assert_send<T: Send>(_: &T) {}
    let mut f = fixture();
    assert_send(&f.endpoint);
    assert_send(&f.worker);
    let handle = f.worker.handle_one();
    assert_send(&handle);
    drop(handle);
    // Worker-owned lifecycle futures stay spawnable by a later runtime.
    let connect = f.worker.connect("fake-paired-secret");
    assert_send(&connect);
    drop(connect);
    let renew = f.worker.renew_if_due("fake-paired-secret");
    assert_send(&renew);
    drop(renew);
    let snapshot = f.worker.begin_snapshot();
    assert_send(&snapshot);
    drop(snapshot);
}

fn published_ready(worker: &Worker) -> bool {
    lock(&worker.shared).published.ready()
}

#[tokio::test]
async fn invalid_owned_copy_refusal_releases_ledger_slot_without_io() {
    let mut f = fixture();
    active(&mut f).await;
    let original = plain();
    let mut ledger = EditorSubmission::default();
    let first = ledger.prepare(&original).unwrap();
    // Models validator drift between ledger and mailbox: the owned copy is
    // refused before ownership, so the ledger must release its prepared slot.
    let mut owned = copy(&first);
    if let FrontendIntent::SendMessage { body, .. } = &mut owned.intent {
        *body = ComposedBody::PlainText(String::new());
    }
    let refusal = f.endpoint.try_submit(owned);
    assert_eq!(refusal, CommandAdmission::InvalidRequest);
    assert_eq!(
        ledger.admit(&first.context, refusal),
        Err(SubmissionError::QueueInvalidRequest)
    );
    assert!(ledger.pending_context().is_none());
    assert!(ledger.review().is_none());
    assert_eq!(ledger.status(), EditorWriteStatus::Unavailable);
    let next = ledger.prepare(&original).unwrap();
    assert_ne!(next.context.id, first.context.id);
    assert_eq!(
        f.endpoint.try_submit(copy(&next)),
        CommandAdmission::Accepted
    );
    assert!(f.endpoint.drain().is_none());
    assert_eq!(count(&f.control), 0);
    f.worker.shutdown();
}

#[tokio::test]
async fn worker_lifecycle_methods_republish_actual_admission_after_each_mutation() {
    let mut f = fixture();
    assert!(!published_ready(&f.worker));
    assert_eq!(f.endpoint.try_submit(request()), CommandAdmission::NotReady);

    let sub = f.worker.connect("fake-paired-secret").await.unwrap();
    assert_eq!(f.worker.phase(), SessionPhase::AwaitingReady);
    assert!(!published_ready(&f.worker));
    f.worker
        .frame(
            sub,
            UiLifecycleFrame::Control(LifecycleControl::Ready(marker())),
        )
        .unwrap();
    assert_eq!(f.worker.phase(), SessionPhase::SnapshotRequired);
    assert!(!published_ready(&f.worker));
    install(&mut f.worker).await; // no manual publication anywhere
    assert!(published_ready(&f.worker));
    let first = lock(&f.worker.shared).published;

    // Lag resync: revoked, then a new generation is published on reinstall.
    f.worker.frame(sub, lag()).unwrap();
    assert!(!published_ready(&f.worker));
    install(&mut f.worker).await;
    assert!(published_ready(&f.worker));
    assert!(lock(&f.worker.shared).published != first);

    // A renewal tick that is not due must not revoke admission spuriously.
    assert!(!f.worker.renew_if_due("fake-paired-secret").await.unwrap());
    assert!(published_ready(&f.worker));
    assert_eq!(f.worker.check_lease(), LeaseAction::None);
    assert!(published_ready(&f.worker));
    // A due renewal replaces the session: not ready until its own snapshot.
    f.clock.advance(85);
    assert!(f.worker.renew_if_due("fake-paired-secret").await.unwrap());
    assert_eq!(f.worker.phase(), SessionPhase::AwaitingReady);
    assert!(!published_ready(&f.worker));

    for step in 0..4 {
        let sub = active(&mut f).await;
        assert!(published_ready(&f.worker));
        match step {
            0 => {
                f.worker.stream_failed(sub, SessionFailure::Transport);
            }
            1 => f.worker.disconnect(),
            2 => {
                f.clock.advance(100);
                assert_eq!(f.worker.check_lease(), LeaseAction::Reconnect);
            }
            _ => {
                f.clock.advance(100);
                assert!(f.worker.stream_mut().is_none());
            }
        }
        assert!(!published_ready(&f.worker), "step {step}");
        assert_eq!(f.endpoint.try_submit(request()), CommandAdmission::NotReady);
    }
    assert_eq!(count(&f.control), 0);
}

#[tokio::test]
async fn host_phase_hook_revokes_without_worker_publication_and_never_restores() {
    // Direct controller calls bypass the worker's post-call publication, as a
    // lifecycle future dropped after its first phase publication would.
    let mut f = fixture();
    active(&mut f).await;
    assert!(published_ready(&f.worker));
    f.worker.controller.disconnect();
    assert!(!published_ready(&f.worker));

    let mut f = fixture();
    let sub2 = active(&mut f).await;
    // Connecting is published before the first await of connect.
    f.worker
        .controller
        .connect("fake-paired-secret")
        .await
        .unwrap();
    assert!(!published_ready(&f.worker));
    let replacement = f.worker.subscription().unwrap();
    assert_ne!(replacement, sub2);
    f.worker
        .controller
        .frame(
            replacement,
            UiLifecycleFrame::Control(LifecycleControl::Ready(marker())),
        )
        .unwrap();
    let ticket = f.worker.controller.begin_snapshot().await.unwrap();
    f.worker.controller.commit_snapshot(ticket, ()).unwrap();
    assert_eq!(f.worker.phase(), SessionPhase::Active);
    // Active is never restored by the host hook; only an actual worker stamp.
    assert!(!published_ready(&f.worker));
    assert_eq!(f.endpoint.try_submit(request()), CommandAdmission::NotReady);
    assert_eq!(f.worker.check_lease(), LeaseAction::None);
    assert!(published_ready(&f.worker));
    assert_eq!(f.worker.host().snapshots, 2);
}

#[tokio::test]
async fn published_hint_expires_with_its_lease_without_a_worker_call() {
    let mut f = fixture();
    active(&mut f).await;
    let now = SessionClock::now(&f.clock);
    assert!(published_ready(&f.worker));
    // Silent expiry: no worker/controller call observes the clock.
    f.clock.advance(100);
    let later = SessionClock::now(&f.clock);
    assert!(published_ready(&f.worker), "hint itself is unchanged");
    assert_eq!(
        f.endpoint.try_submit_at(request(), later),
        CommandAdmission::NotReady
    );
    // Same hint is still admissible at an earlier clock reading.
    assert_eq!(
        f.endpoint.try_submit_at(request(), now),
        CommandAdmission::Accepted
    );
    // Accepted only transfers ownership: dequeue proves zero I/O on expiry.
    assert!(f.worker.handle_one().await);
    assert!(matches!(
        f.endpoint.drain(),
        Some(SubmissionTerminal::NotDispatched { .. })
    ));
    assert_eq!(count(&f.control), 0);
}

#[tokio::test]
async fn dropped_pending_connect_and_due_renewal_leave_admission_revoked() {
    for renewal in [false, true] {
        let mut f = fixture();
        active(&mut f).await;
        assert!(published_ready(&f.worker));
        if renewal {
            f.clock.advance(85); // past the 80% renewal point, before expiry
        }
        f.control.issue_pending.store(true, Ordering::SeqCst);
        {
            let mut lifecycle: Pin<Box<dyn Future<Output = bool> + '_>> = if renewal {
                Box::pin(async { f.worker.renew_if_due("fake-paired-secret").await.is_ok() })
            } else {
                Box::pin(async { f.worker.connect("fake-paired-secret").await.is_ok() })
            };
            assert!(poll_once(lifecycle.as_mut()).is_pending());
        } // dropped mid-await: the worker's post-call publication never runs
        assert_eq!(
            f.worker.phase(),
            SessionPhase::Connecting,
            "renewal={renewal}"
        );
        assert!(!published_ready(&f.worker), "renewal={renewal}");
        assert_eq!(
            f.endpoint
                .try_submit_at(request(), SessionClock::now(&f.clock)),
            CommandAdmission::NotReady
        );
        assert!(!f.worker.handle_one().await);
        // A later explicit connect + snapshot install restores admission.
        f.control.issue_pending.store(false, Ordering::SeqCst);
        active(&mut f).await;
        assert!(published_ready(&f.worker));
        assert_eq!(
            f.endpoint
                .try_submit_at(request(), SessionClock::now(&f.clock)),
            CommandAdmission::Accepted
        );
        f.worker.shutdown();
        assert_eq!(count(&f.control), 0);
    }
}
