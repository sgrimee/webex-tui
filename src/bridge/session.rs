//! Unconnected UI session/lease/SSE-control owner with injectable IO and clock.
//!
//! The real backend owns UiClient (and its private bearer token) and EventStream.
//! Nothing starts automatically. The caller supplies a paired secret for each
//! explicit connect/renew; this owner never stores it, pairs or accesses Keychain.
//! No message write is dispatched/retried here. Snapshot data stays host-owned.

use super::{events::project_continuity, projection::valid_id};
use crate::frontend::{
    continuity::{
        ContinuityBoundary, DaemonInstanceId, LifecycleControl, SnapshotRelation, UiLifecycleFrame,
    },
    events::{ChangeScope, LifecycleEvent, ResyncReason},
};
use std::{
    collections::VecDeque,
    future::Future,
    time::{Duration, Instant},
};
use webex_bridge::client::{events::EventStream, ui::UiClient, BridgeClient, ClientError};

const MAX_PENDING: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionFailure {
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

fn io_failure(error: ClientError) -> SessionFailure {
    match error {
        ClientError::Transport(_) => SessionFailure::Transport,
        ClientError::InvalidResponse(_) => SessionFailure::InvalidResponse,
        ClientError::Api { status, .. } if status.as_u16() == 401 => SessionFailure::AuthRequired,
        ClientError::Api { status, .. } if status.as_u16() == 403 => SessionFailure::Forbidden,
        ClientError::Api { .. } => SessionFailure::InvalidResponse,
    }
}

pub(crate) trait SessionBackend: Send {
    type Client: Send + Sync;
    type Stream: Send;
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
    fn lease_ttl(client: &Self::Client) -> u64;
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
    async fn issue(&mut self, secret: &str) -> Result<UiClient, SessionFailure> {
        self.bridge
            .create_ui_session(secret)
            .await
            .map_err(io_failure)
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
    fn lease_ttl(client: &UiClient) -> u64 {
        client.lease_ttl()
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

struct Active<C, S> {
    client: C,
    stream: S,
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
    active: Option<Active<B::Client, B::Stream>>,
    last_subscription: u64,
    last_instance: Option<DaemonInstanceId>,
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

    pub(crate) fn subscription(&self) -> Option<SubscriptionId> {
        self.active.as_ref().map(|a| a.subscription)
    }
    pub(crate) fn lease_deadline(&self) -> Option<Instant> {
        self.active.as_ref().map(|a| a.lease_deadline)
    }
    pub(crate) fn renew_at(&self) -> Option<Instant> {
        self.active.as_ref().map(|a| a.renew_at)
    }

    /// Snapshot/read access only; the controller still owns operation dispatch.
    pub(crate) fn read_client(&self) -> Option<&B::Client> {
        if matches!(
            self.phase(),
            SessionPhase::SnapshotRequired | SessionPhase::Active
        ) {
            self.active.as_ref().map(|a| &a.client)
        } else {
            None
        }
    }

    /// The host may enable user-write controls only after snapshot reconciliation.
    /// This returns a typed client, never a raw bearer token or an automatic write.
    pub(crate) fn active_client(&self) -> Option<&B::Client> {
        if self.phase() == SessionPhase::Active {
            self.read_client()
        } else {
            None
        }
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

    /// Explicit initial connect or reconnect. Old handles are dropped first;
    /// failure/cancellation cannot leave an old token available for new work.
    /// The host must validate daemon/API compatibility before supplying a secret.
    pub(crate) async fn connect(&mut self, secret: &str) -> Result<SubscriptionId, SessionFailure> {
        self.active = None;
        self.phase = SessionPhase::Connecting;
        let next = match self.last_subscription.checked_add(1) {
            Some(next) => next,
            None => return Err(self.fail(SessionFailure::CounterExhausted)),
        };
        // Reserve before IO so a cancelled attempt can never reuse its epoch.
        self.last_subscription = next;
        let started = self.clock.now();
        let client = match self.backend.issue(secret).await {
            Ok(client) => client,
            Err(error) => return Err(self.fail(error)),
        };
        let ttl = B::lease_ttl(&client);
        if ttl == 0 {
            return Err(self.fail(SessionFailure::InvalidLease));
        }
        let lease = Duration::from_secs(ttl);
        let Some(lease_deadline) = started.checked_add(lease) else {
            return Err(self.fail(SessionFailure::InvalidLease));
        };
        let Some(renew_at) = started.checked_add(lease - lease / 5) else {
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
        let subscription = SubscriptionId(next);
        self.active = Some(Active {
            client,
            stream,
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

    /// Caller-scheduled lease tick; there is no background timer/retry loop here.
    pub(crate) fn check_lease(&mut self) -> LeaseAction {
        let Some(active) = &self.active else {
            return LeaseAction::Reconnect;
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

    pub(crate) async fn renew_if_due(&mut self, secret: &str) -> Result<bool, SessionFailure> {
        if self.check_lease() == LeaseAction::None {
            return Ok(false);
        }
        self.connect(secret).await.map(|_| true)
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
