//! Unconnected UI session/lease/SSE-control owner with injectable IO and clock.
//!
//! The real backend owns UiClient (and its private bearer token) and EventStream.
//! Nothing starts automatically. The caller supplies a paired secret for each
//! explicit connect or new-session renewal; this owner never stores it, pairs
//! or accesses Keychain. In-place renewal (`renew_in_place_if_due`) needs no
//! secret: it extends the same token's lease and keeps the stream open.
//! No message write is dispatched/retried here. Snapshot data stays host-owned.

use super::{
    events::project_continuity,
    projection::valid_id,
    read_dispatch::{dispatch_read_observed, ReadDispatchError, UiReadBackend},
    write_dispatch::{BridgeUiWriteBackend, DispatchError, UiWriteBackend, WriteDispatcher},
};
use crate::frontend::{
    continuity::{
        ContinuityBoundary, DaemonInstanceId, LifecycleControl, SnapshotRelation, UiLifecycleFrame,
    },
    envelopes::{FrontendIntent, FrontendResult, RequestEnvelope, ResultEnvelope},
    events::{ChangeScope, LifecycleEvent, ResyncReason},
};
use std::{
    collections::VecDeque,
    future::Future,
    time::{Duration, Instant},
};
use webex_bridge::{
    client::{events::EventStream, ui::UiClient, BridgeClient, ClientError},
    dto,
};

const MAX_PENDING: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionFailure {
    /// Exactly `401 auth_required` on session creation: the secret is not
    /// (or no longer) paired. The only answer that may lead to pairing.
    NotPaired,
    AuthRequired,
    Forbidden,
    Transport,
    InvalidResponse,
    InvalidLease,
    InvalidContinuity,
    NotReady,
    Expired,
    StaleSnapshot,
    StreamEnded,
    CounterExhausted,
}

/// Session-level authorization rejection on an ordinary read or write: 401
/// `auth_required`, or 403 `forbidden_scope|peer|session`. Resource-level
/// 403s (e.g. `write_not_allowed`) and other errors do not end the session.
pub(super) fn session_auth_failure(error: &ClientError) -> Option<SessionFailure> {
    match error {
        ClientError::Api { status, code, .. } => match (status.as_u16(), code.as_str()) {
            (401, _) => Some(SessionFailure::AuthRequired),
            (403, "forbidden_scope" | "forbidden_peer" | "forbidden_session") => {
                Some(SessionFailure::Forbidden)
            }
            _ => None,
        },
        _ => None,
    }
}

/// Session-creation failure. Only exactly `401 auth_required` means "not
/// paired"; every other answer (other 401 codes, 400, 403, 503, transport)
/// keeps its ordinary classification and never leads to pairing.
pub(super) fn issue_failure(error: ClientError) -> SessionFailure {
    match &error {
        ClientError::Api { status, code, .. }
            if status.as_u16() == 401 && code == "auth_required" =>
        {
            SessionFailure::NotPaired
        }
        _ => io_failure(error),
    }
}

pub(super) fn io_failure(error: ClientError) -> SessionFailure {
    match error {
        ClientError::Transport(_) => SessionFailure::Transport,
        ClientError::InvalidResponse(_) => SessionFailure::InvalidResponse,
        ClientError::Api { status, .. } if status.as_u16() == 401 => SessionFailure::AuthRequired,
        ClientError::Api { status, .. } if status.as_u16() == 403 => SessionFailure::Forbidden,
        ClientError::Api { .. } => SessionFailure::InvalidResponse,
    }
}

/// Why an in-place UI session renewal did not extend the lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RenewFailure {
    /// 401 `pairing_removed`: the daemon removed this client's pairing and
    /// already ended the session. Re-pairing is a user decision.
    PairingRemoved,
    /// 409 `session_cap_reached`: the 12 h absolute lifetime is reached. The
    /// token stays valid until its current expiry; create a new session.
    CapReached,
    /// 404: an older daemon without the renew route. The token stays valid;
    /// create a new session (the pre-renew behaviour).
    Unsupported,
    /// The session itself was refused (401 `auth_required` or any other 401,
    /// 403 `forbidden_*`): it is over.
    Session(SessionFailure),
    /// Transport, 5xx/503 or any other response: nothing proves the session
    /// is invalid, so the still-valid lease is kept and renewal retried.
    Transient,
}

/// Classify a bridge renewal error. Only a session-level refusal ends the
/// session; a transient failure never kills a still-valid lease.
pub(super) fn renew_failure(error: ClientError) -> RenewFailure {
    if error.is_pairing_removed() {
        RenewFailure::PairingRemoved
    } else if error.is_session_cap_reached() {
        RenewFailure::CapReached
    } else if let Some(failure) = session_auth_failure(&error) {
        RenewFailure::Session(failure)
    } else if matches!(&error, ClientError::Api { status, .. } if status.as_u16() == 404) {
        RenewFailure::Unsupported
    } else {
        RenewFailure::Transient
    }
}

/// Result of one due in-place renewal attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RenewStep {
    /// No active lease is due for renewal.
    NotDue,
    /// Same token and subscription; the lease deadline was extended.
    Renewed,
    /// The lease is still valid but cannot be extended in place (12 h cap,
    /// an older daemon without renew, or a zero lease reply): create a new
    /// session now.
    NewSession,
    /// Transient failure: the still-valid lease is untouched; retry later.
    Retry,
    /// The daemon removed this client's pairing; the session is Failed.
    PairingRemoved,
    /// The renewal failed; the session is Failed and the caller's reconnect
    /// path (with its backoff) applies.
    Failed(SessionFailure),
}

pub(crate) trait SessionBackend: Send {
    type Client: Send + Sync;
    type Stream: Send;
    /// Private mutation capability created from an active client. It is retained
    /// by the lifecycle and is never returned through this trait's public API.
    type Writer: UiWriteBackend;
    /// Owned, non-cloneable read-only facade created for one snapshot ticket
    /// or one active read and dropped right after it. Implementations must
    /// not expose a cloneable write client, bearer token or event/mutation
    /// method. (Owned rather than borrowed so generic futures holding it are
    /// provably Send; see rustc#100013.)
    type SnapshotReader: UiReadBackend;
    fn issue(
        &mut self,
        secret: &str,
    ) -> impl Future<Output = Result<Self::Client, SessionFailure>> + Send;
    fn subscribe(
        &mut self,
        client: &Self::Client,
    ) -> impl Future<Output = Result<Self::Stream, SessionFailure>> + Send;
    fn continuity(
        &mut self,
        client: &Self::Client,
    ) -> impl Future<Output = Result<ContinuityBoundary, SessionFailure>> + Send;
    /// Extend the client's lease in place (same token, open stream kept)
    /// and return the seconds until the new expiry. No secret is involved.
    fn renew(
        &mut self,
        client: &Self::Client,
    ) -> impl Future<Output = Result<u64, RenewFailure>> + Send;
    /// End the session on the daemon (revoke this token). Best effort: the
    /// result is ignored and an older daemon without the route is fine.
    fn logout(&mut self, _client: &Self::Client) -> impl Future<Output = ()> + Send {
        async {}
    }
    fn lease_ttl(client: &Self::Client) -> u64;
    fn writer(client: &Self::Client) -> Self::Writer;
    fn snapshot_reader(client: &Self::Client) -> Self::SnapshotReader;
}

/// Non-cloneable, read-only UI capability held only while preparing a fenced
/// snapshot. It deliberately exposes neither the bearer nor mutation/event
/// methods from [`UiClient`].
pub(crate) struct UiSnapshotReader {
    client: UiClient,
}

impl UiSnapshotReader {
    pub(crate) async fn me(&self) -> Result<dto::Person, ClientError> {
        self.client.me().await
    }

    pub(crate) async fn rooms(&self) -> Result<Vec<dto::Room>, ClientError> {
        self.client.rooms().await
    }

    pub(crate) async fn room(&self, room_id: &str) -> Result<dto::Room, ClientError> {
        self.client.room(room_id).await
    }

    pub(crate) async fn team(&self, team_id: &str) -> Result<dto::Team, ClientError> {
        self.client.team(team_id).await
    }

    pub(crate) async fn room_messages_page(
        &self,
        room_id: &str,
        before: Option<&str>,
        max: Option<u32>,
    ) -> Result<dto::MessagePage, ClientError> {
        self.client.room_messages_page(room_id, before, max).await
    }

    pub(crate) async fn thread_children(
        &self,
        parent_id: &str,
        before: Option<&str>,
        max: Option<u32>,
    ) -> Result<dto::ThreadChildren, ClientError> {
        self.client.thread_children(parent_id, before, max).await
    }

    pub(crate) async fn message_read(
        &self,
        message_id: &str,
    ) -> Result<dto::MessageRead, ClientError> {
        self.client.message_read(message_id).await
    }

    pub(crate) async fn person_read(
        &self,
        person_id: &str,
    ) -> Result<dto::PersonRead, ClientError> {
        self.client.person_read(person_id).await
    }
}

pub(crate) struct BridgeSessionBackend {
    bridge: BridgeClient,
}

impl BridgeSessionBackend {
    pub(crate) fn new(bridge: BridgeClient) -> Self {
        Self { bridge }
    }
}

impl SessionBackend for BridgeSessionBackend {
    type Client = UiClient;
    type Stream = EventStream;
    type Writer = BridgeUiWriteBackend;
    type SnapshotReader = UiSnapshotReader;
    async fn issue(&mut self, secret: &str) -> Result<UiClient, SessionFailure> {
        self.bridge
            .create_ui_session(secret)
            .await
            .map_err(issue_failure)
    }
    async fn subscribe(&mut self, client: &UiClient) -> Result<EventStream, SessionFailure> {
        client.events().await.map_err(io_failure)
    }
    async fn continuity(
        &mut self,
        client: &UiClient,
    ) -> Result<ContinuityBoundary, SessionFailure> {
        project_continuity(client.continuity().await.map_err(io_failure)?)
            .map_err(|_| SessionFailure::InvalidContinuity)
    }
    async fn renew(&mut self, client: &UiClient) -> Result<u64, RenewFailure> {
        client.renew().await.map_err(renew_failure)
    }
    async fn logout(&mut self, client: &UiClient) {
        // 204, 401 (already gone) and 404/405 (older daemon) are all fine.
        if let Err(error) = client.logout().await {
            log_logout_refusal(&error);
        }
    }
    fn lease_ttl(client: &UiClient) -> u64 {
        client.lease_ttl()
    }
    fn writer(client: &UiClient) -> BridgeUiWriteBackend {
        BridgeUiWriteBackend::from_client(client)
    }
    fn snapshot_reader(client: &UiClient) -> UiSnapshotReader {
        UiSnapshotReader {
            client: client.clone(),
        }
    }
}

/// A 403 on logout (another process's token, an agent bearer) leaves the
/// token alive until expiry: worth a metadata-only debug line.
fn log_logout_refusal(error: &ClientError) -> bool {
    match error {
        ClientError::Api { status, code, .. } if status.as_u16() == 403 => {
            log::debug!("bridge logout refused: {} {code}", status.as_u16());
            true
        }
        _ => false,
    }
}

pub(crate) trait SessionClock: Send + Sync {
    fn now(&self) -> Instant;
}

pub(crate) struct SystemClock;
impl SessionClock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SubscriptionId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionPhase {
    Disconnected,
    Connecting,
    AwaitingReady,
    SnapshotRequired,
    Active,
    Expired,
    Failed(SessionFailure),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LeaseAction {
    None,
    Renew,
    Reconnect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FrameAction {
    StaleSubscription,
    NeedSnapshot {
        scope: ChangeScope,
        reason: ResyncReason,
    },
    ReconnectRequired,
    Buffered,
    Apply(UiLifecycleFrame),
    Covered,
    Expired,
}

#[derive(Clone, Debug)]
pub(crate) struct SnapshotTicket {
    subscription: SubscriptionId,
    generation: u64,
    boundary: ContinuityBoundary,
}

pub(crate) struct ConnectAttempt {
    subscription: SubscriptionId,
    started: Instant,
}

struct Active<C, S, W: UiWriteBackend> {
    client: C,
    stream: S,
    // Retained privately until the active session is replaced or dropped.
    _writer: WriteDispatcher<W>,
    subscription: SubscriptionId,
    lease_deadline: Instant,
    renew_at: Instant,
    observed: Option<ContinuityBoundary>,
    snapshot: Option<ContinuityBoundary>,
    snapshot_generation: u64,
    pending: VecDeque<UiLifecycleFrame>,
}

pub(crate) struct SessionLifecycle<B: SessionBackend, C: SessionClock> {
    backend: B,
    clock: C,
    phase: SessionPhase,
    active: Option<Active<B::Client, B::Stream, B::Writer>>,
    last_subscription: u64,
    last_instance: Option<DaemonInstanceId>,
}

/// Private admission fence, not a client/capability or a reconnect promise.
/// Snapshot generations are subscription-local and readiness is lease-aware.
/// Deliberately not `PartialEq`: compare identities with `same_admission`,
/// which ignores the lease deadline (an in-place renewal moves it).
#[derive(Clone, Copy)]
pub(super) struct WriteAdmissionStamp {
    subscription: Option<SubscriptionId>,
    snapshot_generation: Option<u64>,
    /// Lets a published hint expire on its own after a silent lease expiry,
    /// without the worker having to observe the clock first.
    lease_deadline: Option<Instant>,
    ready: bool,
}

impl WriteAdmissionStamp {
    pub(super) fn ready(self) -> bool {
        self.ready
    }

    /// Same admission identity: subscription, snapshot generation and
    /// readiness. The lease deadline is not part of it: an in-place renewal
    /// extends the same session, so a write queued just before it must still
    /// be dispatched. Lease validity is checked separately at dequeue (the
    /// actual stamp's readiness is lease-aware and `dispatch_write` checks
    /// the lease again).
    pub(super) fn same_admission(self, other: Self) -> bool {
        self.subscription == other.subscription
            && self.snapshot_generation == other.snapshot_generation
            && self.ready == other.ready
    }

    /// Ready and not past its lease deadline at `now` (same clock domain as
    /// the session clock; production uses `Instant::now()` for both).
    pub(super) fn admissible_at(self, now: Instant) -> bool {
        self.ready && self.lease_deadline.is_some_and(|deadline| now < deadline)
    }

    /// Same identity, never admissible. Used for synchronous revocation when a
    /// non-Active phase is published; only the actual stamp can restore Active.
    pub(super) fn revoked(self) -> Self {
        Self {
            ready: false,
            ..self
        }
    }
}

/// Every error here is proven before any read I/O. Once a read is attempted,
/// its failure is a typed `ReadOutcome::Rejected` inside the result envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionReadError {
    NotReady,
    Expired,
    Failed,
    Dispatch(ReadDispatchError),
}

/// Every error here is proven before backend mutation I/O. Once a backend write
/// is attempted its failure is carried inside FrontendResult, never this enum.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionWriteError {
    NotReady,
    Expired,
    Failed,
    Dispatch(DispatchError),
}

impl<B: SessionBackend, C: SessionClock> SessionLifecycle<B, C> {
    pub(crate) fn new(backend: B, clock: C) -> Self {
        Self {
            backend,
            clock,
            phase: SessionPhase::Disconnected,
            active: None,
            last_subscription: 0,
            last_instance: None,
        }
    }

    pub(crate) fn phase(&self) -> SessionPhase {
        if self
            .active
            .as_ref()
            .is_some_and(|a| self.clock.now() >= a.lease_deadline)
        {
            SessionPhase::Expired
        } else {
            self.phase
        }
    }

    pub(super) fn write_admission_stamp(&self) -> WriteAdmissionStamp {
        WriteAdmissionStamp {
            subscription: self.active.as_ref().map(|a| a.subscription),
            snapshot_generation: self.active.as_ref().map(|a| a.snapshot_generation),
            lease_deadline: self.active.as_ref().map(|a| a.lease_deadline),
            ready: self.phase() == SessionPhase::Active,
        }
    }

    pub(crate) async fn dispatch_write(
        &mut self,
        intent: FrontendIntent,
    ) -> Result<FrontendResult, SessionWriteError> {
        self.check_lease();
        match self.phase() {
            SessionPhase::Active => {}
            SessionPhase::Expired => return Err(SessionWriteError::Expired),
            SessionPhase::Failed(_) => return Err(SessionWriteError::Failed),
            _ => return Err(SessionWriteError::NotReady),
        }
        let (result, failure) = self
            .active
            .as_mut()
            .ok_or(SessionWriteError::NotReady)?
            ._writer
            .dispatch_observed(intent)
            .await
            .map_err(SessionWriteError::Dispatch)?;
        // A rejected token ends this session like a stream failure. The write
        // still settles with its typed outcome and is never replayed.
        if let Some(failure) = failure {
            self.fail(failure);
        }
        Ok(result)
    }

    /// Ordinary page/history/point read for an Active session only. During
    /// SnapshotRequired the ticket-fenced snapshot reader is the sole path, so
    /// reads cannot race ahead of snapshot installation. No retry or replay.
    pub(crate) async fn dispatch_read(
        &mut self,
        request: RequestEnvelope,
    ) -> Result<ResultEnvelope, SessionReadError> {
        self.check_lease();
        match self.phase() {
            SessionPhase::Active => {}
            SessionPhase::Expired => return Err(SessionReadError::Expired),
            SessionPhase::Failed(_) => return Err(SessionReadError::Failed),
            _ => return Err(SessionReadError::NotReady),
        }
        let (envelope, failure) = {
            let active = self.active.as_ref().ok_or(SessionReadError::NotReady)?;
            let reader = B::snapshot_reader(&active.client);
            dispatch_read_observed(&reader, request)
                .await
                .map_err(SessionReadError::Dispatch)?
        };
        // A rejected token ends this session exactly like a stream failure;
        // the typed rejection is still returned to its requester.
        if let Some(failure) = failure {
            self.fail(failure);
        }
        Ok(envelope)
    }

    pub(crate) fn subscription(&self) -> Option<SubscriptionId> {
        self.active.as_ref().map(|a| a.subscription)
    }

    #[cfg(test)]
    pub(crate) fn backend_for_tests(&self) -> &B {
        &self.backend
    }
    #[cfg(test)]
    pub(crate) fn lease_deadline(&self) -> Option<Instant> {
        self.active.as_ref().map(|a| a.lease_deadline)
    }
    #[cfg(test)]
    pub(crate) fn renew_at(&self) -> Option<Instant> {
        self.active.as_ref().map(|a| a.renew_at)
    }

    /// Borrow a non-cloneable read facade only while preparing this exact
    /// snapshot. The ticket fence prevents an old/replaced subscription from
    /// retaining a write-capable `UiClient` beyond reconciliation.
    pub(crate) fn snapshot_reader(
        &self,
        ticket: &SnapshotTicket,
    ) -> Result<B::SnapshotReader, SessionFailure> {
        let active = self.active.as_ref().ok_or(SessionFailure::NotReady)?;
        if self.phase() != SessionPhase::SnapshotRequired
            || ticket.subscription != active.subscription
            || ticket.generation != active.snapshot_generation
        {
            return Err(SessionFailure::StaleSnapshot);
        }
        Ok(B::snapshot_reader(&active.client))
    }

    /// A future host may poll this owned stream and feed projected frames back
    /// with its subscription ID. No stream polling/task loop is started here.
    pub(crate) fn stream_mut(&mut self) -> Option<&mut B::Stream> {
        self.check_lease();
        self.active.as_mut().map(|a| &mut a.stream)
    }

    /// EOF/parser/transport failures are also subscription-fenced. The host
    /// decides when to reconnect; this never replays a read or user write.
    pub(crate) fn stream_failed(
        &mut self,
        subscription: SubscriptionId,
        failure: SessionFailure,
    ) -> FrameAction {
        if self.subscription() != Some(subscription) {
            return FrameAction::StaleSubscription;
        }
        self.fail(failure);
        FrameAction::ReconnectRequired
    }

    fn fail(&mut self, failure: SessionFailure) -> SessionFailure {
        self.active = None;
        self.phase = SessionPhase::Failed(failure);
        failure
    }

    pub(crate) fn disconnect(&mut self) {
        self.active = None;
        self.phase = SessionPhase::Disconnected;
    }

    /// Disconnect and ask the daemon to revoke the token. The session is
    /// dropped before the call, so cancelling it leaves nothing usable.
    pub(crate) async fn logout(&mut self) {
        let active = self.active.take();
        self.phase = SessionPhase::Disconnected;
        if let Some(active) = active {
            self.backend.logout(&active.client).await;
        }
    }

    /// Start an explicit initial-connect or reconnect attempt. Old handles are
    /// dropped before any IO so cancellation cannot leave their token available
    /// for new work. Controllers call this synchronously before awaiting to
    /// publish `Connecting` to their presentation host.
    pub(crate) fn begin_connect(&mut self) -> Result<ConnectAttempt, SessionFailure> {
        self.active = None;
        self.phase = SessionPhase::Connecting;
        let next = match self.last_subscription.checked_add(1) {
            Some(next) => next,
            None => return Err(self.fail(SessionFailure::CounterExhausted)),
        };
        // Reserve before IO so a cancelled attempt can never reuse its epoch.
        self.last_subscription = next;
        Ok(ConnectAttempt {
            subscription: SubscriptionId(next),
            started: self.clock.now(),
        })
    }

    /// Complete a previously announced explicit connection attempt.
    pub(crate) async fn complete_connect(
        &mut self,
        attempt: ConnectAttempt,
        secret: &str,
    ) -> Result<SubscriptionId, SessionFailure> {
        if self.phase != SessionPhase::Connecting
            || self.active.is_some()
            || self.last_subscription != attempt.subscription.0
        {
            return Err(SessionFailure::NotReady);
        }
        let client = match self.backend.issue(secret).await {
            Ok(client) => client,
            Err(error) => return Err(self.fail(error)),
        };
        let ttl = B::lease_ttl(&client);
        if ttl == 0 {
            return Err(self.fail(SessionFailure::InvalidLease));
        }
        let lease = Duration::from_secs(ttl);
        let Some(lease_deadline) = attempt.started.checked_add(lease) else {
            return Err(self.fail(SessionFailure::InvalidLease));
        };
        let Some(renew_at) = attempt.started.checked_add(lease - lease / 5) else {
            return Err(self.fail(SessionFailure::InvalidLease));
        };
        if self.clock.now() >= lease_deadline {
            return Err(self.fail(SessionFailure::Expired));
        }
        let stream = match self.backend.subscribe(&client).await {
            Ok(stream) => stream,
            Err(error) => return Err(self.fail(error)),
        };
        if self.clock.now() >= lease_deadline {
            return Err(self.fail(SessionFailure::Expired));
        }
        let subscription = attempt.subscription;
        let writer = B::writer(&client);
        self.active = Some(Active {
            client,
            stream,
            _writer: WriteDispatcher::new(writer),
            subscription,
            lease_deadline,
            renew_at,
            observed: None,
            snapshot: None,
            snapshot_generation: 0,
            pending: VecDeque::new(),
        });
        self.phase = SessionPhase::AwaitingReady;
        Ok(subscription)
    }

    /// Explicit initial connect or reconnect. The controller-facing path uses
    /// `begin_connect` first so presentation can observe `Connecting` before
    /// this future reaches its first cancellation point.
    pub(crate) async fn connect(&mut self, secret: &str) -> Result<SubscriptionId, SessionFailure> {
        let attempt = self.begin_connect()?;
        self.complete_connect(attempt, secret).await
    }

    /// Caller-scheduled lease tick; there is no background timer/retry loop here.
    pub(crate) fn check_lease(&mut self) -> LeaseAction {
        let Some(active) = &self.active else {
            return if self.phase == SessionPhase::Expired {
                LeaseAction::Reconnect
            } else {
                LeaseAction::None
            };
        };
        let now = self.clock.now();
        if now >= active.lease_deadline {
            self.active = None;
            self.phase = SessionPhase::Expired;
            LeaseAction::Reconnect
        } else if now >= active.renew_at {
            LeaseAction::Renew
        } else {
            LeaseAction::None
        }
    }

    /// Start an eligible renewal. A periodic tick never reconnects an explicit
    /// disconnect, a failed attempt, or any no-active state; callers must use
    /// the explicit `connect` policy for those transitions.
    pub(crate) fn begin_renew_if_due(&mut self) -> Result<Option<ConnectAttempt>, SessionFailure> {
        if self.check_lease() == LeaseAction::Renew {
            self.begin_connect().map(Some)
        } else {
            Ok(None)
        }
    }

    /// Renew a due lease in place: same token, subscription, stream and
    /// snapshot; only the lease deadline and renewal point move. The phase
    /// stays Active throughout, nothing is mutated before the reply, so a
    /// cancelled call leaves the old (still valid) lease untouched. The new
    /// lease is the daemon's reply measured from the request start, never
    /// past what it granted (it also never exceeds the 12 h cap).
    pub(crate) async fn renew_in_place_if_due(&mut self) -> RenewStep {
        if self.check_lease() != LeaseAction::Renew {
            return RenewStep::NotDue;
        }
        let Some(active) = &self.active else {
            return RenewStep::NotDue;
        };
        let started = self.clock.now();
        let reply = self.backend.renew(&active.client).await;
        let ttl = match reply {
            Ok(ttl) => ttl,
            Err(RenewFailure::CapReached | RenewFailure::Unsupported) => {
                return RenewStep::NewSession
            }
            Err(RenewFailure::Transient) => return RenewStep::Retry,
            Err(RenewFailure::PairingRemoved) => {
                self.fail(SessionFailure::AuthRequired);
                return RenewStep::PairingRemoved;
            }
            Err(RenewFailure::Session(failure)) => {
                self.fail(failure);
                return RenewStep::Failed(failure);
            }
        };
        let lease = Duration::from_secs(ttl);
        let (Some(lease_deadline), Some(renew_at)) = (
            started.checked_add(lease),
            started.checked_add(lease - lease / 5),
        ) else {
            return RenewStep::NewSession;
        };
        if ttl == 0 || self.clock.now() >= lease_deadline {
            return RenewStep::NewSession;
        }
        let Some(active) = &mut self.active else {
            return RenewStep::NotDue;
        };
        active.lease_deadline = lease_deadline;
        active.renew_at = renew_at;
        RenewStep::Renewed
    }

    #[cfg(test)]
    pub(crate) async fn renew_if_due(&mut self, secret: &str) -> Result<bool, SessionFailure> {
        let Some(attempt) = self.begin_renew_if_due()? else {
            return Ok(false);
        };
        self.complete_connect(attempt, secret).await.map(|_| true)
    }

    fn valid_lease(&mut self) -> Result<(), SessionFailure> {
        if self.phase() == SessionPhase::Expired {
            self.check_lease();
            Err(SessionFailure::Expired)
        } else if self.active.is_none() {
            Err(SessionFailure::NotReady)
        } else {
            Ok(())
        }
    }

    fn invalidate_snapshot(&mut self) -> Result<(), SessionFailure> {
        let Some(active) = &mut self.active else {
            return Err(SessionFailure::NotReady);
        };
        let Some(next) = active.snapshot_generation.checked_add(1) else {
            return Err(self.fail(SessionFailure::CounterExhausted));
        };
        active.snapshot_generation = next;
        active.snapshot = None;
        active.pending.clear();
        self.phase = SessionPhase::SnapshotRequired;
        Ok(())
    }

    /// After Ready, get the protected subscribe-first boundary; snapshot data
    /// reads are performed separately by the host using read_client().
    pub(crate) async fn begin_snapshot(&mut self) -> Result<SnapshotTicket, SessionFailure> {
        self.valid_lease()?;
        if self.phase != SessionPhase::SnapshotRequired {
            return Err(SessionFailure::NotReady);
        }
        let active = self.active.as_ref().unwrap();
        let boundary = match self.backend.continuity(&active.client).await {
            Ok(boundary) => boundary,
            Err(error) => return Err(self.fail(error)),
        };
        self.valid_lease()?;
        let active = self.active.as_ref().unwrap();
        let observed = active.observed.as_ref().ok_or(SessionFailure::NotReady)?;
        if valid_id(boundary.instance_id.as_str()).is_err()
            || boundary.instance_id != observed.instance_id
            || boundary.watermark < observed.watermark
        {
            return Err(self.fail(SessionFailure::InvalidContinuity));
        }
        Ok(SnapshotTicket {
            subscription: active.subscription,
            generation: active.snapshot_generation,
            boundary,
        })
    }

    /// Commit only the current ticket. Buffered metadata newer than the captured
    /// boundary is returned to the host, not dropped or executed as actions.
    pub(crate) fn finish_snapshot(
        &mut self,
        ticket: SnapshotTicket,
    ) -> Result<Vec<UiLifecycleFrame>, SessionFailure> {
        self.valid_lease()?;
        let active = self.active.as_mut().unwrap();
        if self.phase != SessionPhase::SnapshotRequired
            || ticket.subscription != active.subscription
            || ticket.generation != active.snapshot_generation
        {
            return Err(SessionFailure::StaleSnapshot);
        }
        if active
            .observed
            .as_ref()
            .is_none_or(|b| b.instance_id != ticket.boundary.instance_id)
        {
            return Err(self.fail(SessionFailure::InvalidContinuity));
        }
        let pending = active
            .pending
            .drain(..)
            .filter(|frame| {
                frame_boundary(frame)
                    .is_none_or(|b| b.relative_to(&ticket.boundary) == SnapshotRelation::After)
            })
            .collect();
        active.snapshot = Some(ticket.boundary);
        self.phase = SessionPhase::Active;
        Ok(pending)
    }

    /// Explicit controls are handled before snapshot comparisons. Late frames
    /// from a replaced stream cannot expire or invalidate the replacement.
    pub(crate) fn observe(
        &mut self,
        subscription: SubscriptionId,
        frame: UiLifecycleFrame,
    ) -> Result<FrameAction, SessionFailure> {
        if self.subscription() != Some(subscription) {
            return Ok(FrameAction::StaleSubscription);
        }
        if matches!(
            &frame,
            UiLifecycleFrame::Control(LifecycleControl::SessionExpired)
        ) {
            self.active = None;
            self.phase = SessionPhase::Expired;
            return Ok(FrameAction::Expired);
        }
        self.valid_lease()?;
        if let UiLifecycleFrame::Control(LifecycleControl::Ready(boundary)) = &frame {
            if self.phase != SessionPhase::AwaitingReady {
                return Err(self.fail(SessionFailure::InvalidContinuity));
            }
            if valid_id(boundary.instance_id.as_str()).is_err() {
                return Err(self.fail(SessionFailure::InvalidContinuity));
            }
            let reason = if self
                .last_instance
                .as_ref()
                .is_some_and(|id| id != &boundary.instance_id)
            {
                ResyncReason::DaemonRestart
            } else {
                ResyncReason::StreamReconnect
            };
            self.last_instance = Some(boundary.instance_id.clone());
            self.active.as_mut().unwrap().observed = Some(boundary.clone());
            self.invalidate_snapshot()?;
            return Ok(FrameAction::NeedSnapshot {
                scope: ChangeScope::WholeMirror,
                reason,
            });
        }
        if self.phase == SessionPhase::AwaitingReady {
            return Err(self.fail(SessionFailure::InvalidContinuity));
        }
        if let Some(boundary) = frame_boundary(&frame) {
            if valid_id(boundary.instance_id.as_str()).is_err() {
                return Err(self.fail(SessionFailure::InvalidContinuity));
            }
            let active = self.active.as_mut().unwrap();
            let observed = active.observed.as_mut().unwrap();
            if observed.instance_id != boundary.instance_id {
                self.disconnect();
                return Ok(FrameAction::ReconnectRequired);
            }
            observed.watermark = observed.watermark.max(boundary.watermark);
        }
        if let UiLifecycleFrame::Control(LifecycleControl::ResyncRequired { reason, .. }) = frame {
            self.invalidate_snapshot()?;
            // Stream continuity requires a whole loaded-state snapshot. Room
            // resync events remain ordinary, room-scoped lifecycle changes.
            return Ok(FrameAction::NeedSnapshot {
                scope: ChangeScope::WholeMirror,
                reason,
            });
        }
        // Only shared connection metadata may be unsequenced. Unknown events
        // without a marker carry no payload and remain harmless/ignorable.
        if matches!(&frame, UiLifecycleFrame::Change { boundary:None, event } if !matches!(event,LifecycleEvent::Connection { .. }))
        {
            return Err(self.fail(SessionFailure::InvalidContinuity));
        }
        let active = self.active.as_mut().unwrap();
        if self.phase == SessionPhase::SnapshotRequired {
            if active.pending.len() == MAX_PENDING {
                self.invalidate_snapshot()?;
                return Ok(FrameAction::NeedSnapshot {
                    scope: ChangeScope::WholeMirror,
                    reason: ResyncReason::EventLag,
                });
            }
            active.pending.push_back(frame);
            Ok(FrameAction::Buffered)
        } else if frame_boundary(&frame)
            .zip(active.snapshot.as_ref())
            .is_some_and(|(b, s)| b.relative_to(s) == SnapshotRelation::Covered)
        {
            Ok(FrameAction::Covered)
        } else {
            Ok(FrameAction::Apply(frame))
        }
    }
}

fn frame_boundary(frame: &UiLifecycleFrame) -> Option<&ContinuityBoundary> {
    match frame {
        UiLifecycleFrame::Control(LifecycleControl::Ready(boundary))
        | UiLifecycleFrame::Control(LifecycleControl::ResyncRequired { boundary, .. }) => {
            Some(boundary)
        }
        UiLifecycleFrame::Change { boundary, .. } | UiLifecycleFrame::Unrecognized { boundary } => {
            boundary.as_ref()
        }
        UiLifecycleFrame::Control(LifecycleControl::SessionExpired) => None,
    }
}

#[cfg(test)]
mod tests;
