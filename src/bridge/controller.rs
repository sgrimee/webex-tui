//! Host/controller interface over the session and the neutral view projection,
//! owned by the runtime's command worker.
//!
//! Calls are explicit: no task/poll loop, startup, pairing, terminal or App
//! mutation lives here. The view host receives neutral
//! refresh hints, never a client/token/secret, and owns its selected-room/cache UI.

use super::{
    events::project_ui_event,
    presentation::InstallId,
    read_dispatch::{ReadCredits, ReadTerminal},
    session::{
        FrameAction, LeaseAction, RenewStep, SessionBackend, SessionClock, SessionFailure,
        SessionLifecycle, SessionPhase, SessionReadError, SessionWriteError, SnapshotTicket,
        SubscriptionId, WriteAdmissionStamp,
    },
};
use crate::frontend::{
    continuity::UiLifecycleFrame,
    envelopes::{FrontendIntent, FrontendResult, RequestEnvelope, ResultEnvelope},
    events::{ConnectionState, LifecycleEvent},
    lifecycle::{LifecycleProjection, ProjectionEffects},
};
use webex_bridge::client::events::BridgeEvent;

/// Infallible installation of an already prepared/validated local view snapshot.
/// All IO/fallible conversion must finish before commit_snapshot is called.
/// Callbacks must not dispatch writes or synchronously re-enter this controller.
pub(crate) trait PresentationHost {
    type Snapshot;
    fn session_phase(&mut self, phase: SessionPhase);
    fn lifecycle(&mut self, projection: &LifecycleProjection, effects: ProjectionEffects);
    fn install_snapshot(&mut self, snapshot: Self::Snapshot);
    /// Whether the presentation owner has confirmed installing the latest
    /// snapshot. Write admission stays revoked until it has. Hosts that install
    /// synchronously inside `install_snapshot` are acknowledged immediately.
    fn snapshot_acknowledged(&self) -> bool {
        true
    }
    /// Generation of the latest installed snapshot, if the host tracks one.
    /// The worker captures it when a read is dequeued.
    fn current_install(&self) -> Option<InstallId> {
        None
    }
    /// Exactly one terminal per admitted read, in production order with the
    /// other presentation events, tagged with the install generation captured
    /// at dequeue. Required: a host must never drop reads.
    fn deliver_read(&mut self, terminal: ReadTerminal, install: Option<InstallId>);
    /// A host whose consumer drains terminals later returns the shared budget
    /// it releases on drain. `None` means terminals are consumed on delivery.
    fn read_credits(&self) -> Option<ReadCredits> {
        None
    }
}

pub(crate) struct HostController<B: SessionBackend, C: SessionClock, H: PresentationHost> {
    session: SessionLifecycle<B, C>,
    projection: LifecycleProjection,
    host: H,
}

impl<B: SessionBackend, C: SessionClock, H: PresentationHost> HostController<B, C, H> {
    pub(crate) fn new(session: SessionLifecycle<B, C>, host: H) -> Self {
        Self {
            session,
            projection: LifecycleProjection::default(),
            host,
        }
    }

    /// The host can request a typed write result but never sees the capability.
    pub(crate) async fn dispatch_write(
        &mut self,
        intent: FrontendIntent,
    ) -> Result<FrontendResult, SessionWriteError> {
        // Compare phases like check_lease, so any transition observed by the
        // call (not only active-session removal) is published without lag.
        let previous = self.session.phase();
        let had_active = self.session.subscription().is_some();
        let result = self.session.dispatch_write(intent).await;
        // phase() is lease-aware, so a silent expiry already reads Expired
        // before the call: also publish when the active session was removed.
        if self.session.phase() != previous || (had_active && self.session.subscription().is_none())
        {
            self.phase_changed();
        }
        result
    }

    /// Active-only read; the host receives a correlated typed result and never
    /// the read facade. A lease expiry observed here is published like a write.
    pub(crate) async fn dispatch_read(
        &mut self,
        request: RequestEnvelope,
    ) -> Result<ResultEnvelope, SessionReadError> {
        let previous = self.session.phase();
        let had_active = self.session.subscription().is_some();
        let result = self.session.dispatch_read(request).await;
        // phase() is lease-aware, so a silent expiry already reads Expired
        // before the call: also publish when the active session was removed.
        if self.session.phase() != previous || (had_active && self.session.subscription().is_none())
        {
            self.phase_changed();
        }
        result
    }

    pub(super) fn write_admission_stamp(&self) -> WriteAdmissionStamp {
        self.session.write_admission_stamp()
    }

    pub(crate) fn phase(&self) -> SessionPhase {
        self.session.phase()
    }
    pub(crate) fn subscription(&self) -> Option<SubscriptionId> {
        self.session.subscription()
    }
    #[cfg(test)]
    pub(crate) fn projection(&self) -> &LifecycleProjection {
        &self.projection
    }
    pub(crate) fn host(&self) -> &H {
        &self.host
    }
    /// Worker-only: deliver read terminals through the same ordered sink.
    pub(super) fn host_mut(&mut self) -> &mut H {
        &mut self.host
    }

    fn phase_changed(&mut self) {
        let phase = self.session.phase();
        self.host.session_phase(phase);
        // Local session failure is not evidence of upstream OAuth failure.
        let state = match phase {
            SessionPhase::Connecting => ConnectionState::Connecting,
            SessionPhase::AwaitingReady | SessionPhase::SnapshotRequired | SessionPhase::Active => {
                ConnectionState::Connected
            }
            _ => ConnectionState::Disconnected,
        };
        self.projection.local_daemon = state;
        self.host
            .lifecycle(&self.projection, ProjectionEffects::default());
    }

    fn apply_event(&mut self, event: LifecycleEvent) {
        let effects = self.projection.apply(event);
        self.host.lifecycle(&self.projection, effects);
    }

    fn apply_frame(&mut self, frame: UiLifecycleFrame) {
        // Controls have already been handled by SessionLifecycle. Unknown frames
        // retain their stream marker there but never leak content to the host.
        if let UiLifecycleFrame::Change { event, .. } = frame {
            self.apply_event(event);
        }
    }

    fn consume_action(&mut self, action: FrameAction) {
        match action {
            FrameAction::NeedSnapshot { scope, reason } => {
                self.phase_changed();
                self.apply_event(LifecycleEvent::ResyncRequired { scope, reason });
            }
            FrameAction::Apply(frame) => self.apply_frame(frame),
            FrameAction::Expired | FrameAction::ReconnectRequired => self.phase_changed(),
            FrameAction::StaleSubscription | FrameAction::Buffered | FrameAction::Covered => {}
        }
    }

    /// Explicit secret use only. Caller validates service/API and obtains its
    /// own paired secret before this call; neither controller nor host stores it.
    pub(crate) async fn connect(
        &mut self,
        paired_secret: &str,
    ) -> Result<SubscriptionId, SessionFailure> {
        // Publish before the first await so a dropped future cannot leave the
        // presentation host showing a stale active/disconnected phase.
        let attempt = self.session.begin_connect();
        self.phase_changed();
        let result = match attempt {
            Ok(attempt) => self.session.complete_connect(attempt, paired_secret).await,
            Err(error) => Err(error),
        };
        self.phase_changed();
        result
    }

    pub(crate) async fn renew_if_due(
        &mut self,
        paired_secret: &str,
    ) -> Result<bool, SessionFailure> {
        // Only an active lease that is due may enter Connecting here. Expired,
        // explicitly disconnected and failed states require an explicit caller
        // reconnect decision rather than a leftover renewal tick.
        let attempt = self.session.begin_renew_if_due();
        let Some(attempt) = (match attempt {
            Ok(attempt) => attempt,
            Err(error) => {
                self.phase_changed();
                return Err(error);
            }
        }) else {
            return Ok(false);
        };
        self.phase_changed();
        let result = self
            .session
            .complete_connect(attempt, paired_secret)
            .await
            .map(|_| true);
        self.phase_changed();
        result
    }

    /// In-place renewal of a due lease: no secret, same subscription, no
    /// phase change on success. A refusal that ends the session publishes the
    /// new phase; the cap leaves the session untouched for the caller's
    /// new-session fallback.
    pub(crate) async fn renew_in_place_if_due(&mut self) -> RenewStep {
        let step = self.session.renew_in_place_if_due().await;
        if matches!(step, RenewStep::PairingRemoved | RenewStep::Failed(_)) {
            self.phase_changed();
        }
        step
    }

    /// Caller-scheduled lease check, not a background timer/reconnect command.
    pub(crate) fn check_lease(&mut self) -> LeaseAction {
        let previous = self.session.phase();
        let action = self.session.check_lease();
        if self.session.phase() != previous || action == LeaseAction::Reconnect {
            self.phase_changed();
        }
        action
    }

    pub(crate) fn disconnect(&mut self) {
        self.session.disconnect();
        self.phase_changed();
    }

    pub(crate) async fn logout(&mut self) {
        self.session.logout().await;
        self.phase_changed();
    }

    /// Borrow a non-cloneable, backend-defined read facade only for the exact
    /// snapshot ticket currently being prepared. The raw `UiClient` and its
    /// write-capable bearer never cross this controller boundary.
    pub(crate) fn snapshot_reader(
        &self,
        ticket: &SnapshotTicket,
    ) -> Result<B::SnapshotReader, SessionFailure> {
        self.session.snapshot_reader(ticket)
    }
    pub(crate) fn stream_mut(&mut self) -> Option<&mut B::Stream> {
        self.check_lease();
        self.session.stream_mut()
    }

    /// Decode the real bridge frame only for the current subscription. A stale
    /// malformed frame must not cause the replacement to fail or re-pair.
    pub(crate) fn bridge_event(
        &mut self,
        subscription: SubscriptionId,
        event: &BridgeEvent,
    ) -> Result<FrameAction, SessionFailure> {
        if self.session.subscription() != Some(subscription) {
            return Ok(FrameAction::StaleSubscription);
        }
        let frame = match project_ui_event(event) {
            Ok(frame) => frame,
            Err(_) => {
                let action = self
                    .session
                    .stream_failed(subscription, SessionFailure::InvalidResponse);
                self.consume_action(action);
                return Err(SessionFailure::InvalidResponse);
            }
        };
        self.frame(subscription, frame)
    }

    pub(crate) fn frame(
        &mut self,
        subscription: SubscriptionId,
        frame: UiLifecycleFrame,
    ) -> Result<FrameAction, SessionFailure> {
        let result = self.session.observe(subscription, frame);
        match &result {
            Ok(action) => self.consume_action(action.clone()),
            Err(_) => self.phase_changed(),
        }
        result
    }

    pub(crate) fn stream_failed(
        &mut self,
        subscription: SubscriptionId,
        failure: SessionFailure,
    ) -> FrameAction {
        let action = self.session.stream_failed(subscription, failure);
        self.consume_action(action.clone());
        action
    }

    pub(crate) async fn begin_snapshot(&mut self) -> Result<SnapshotTicket, SessionFailure> {
        let result = self.session.begin_snapshot().await;
        if result.is_err() {
            self.phase_changed();
        }
        result
    }

    /// Never install stale/failed snapshot data. Pending post-boundary metadata
    /// is applied only after host installation; no selected-room changes are
    /// inferred by the controller and no write outcome is filtered here.
    pub(crate) fn commit_snapshot(
        &mut self,
        ticket: SnapshotTicket,
        snapshot: H::Snapshot,
    ) -> Result<usize, SessionFailure> {
        let pending = match self.session.finish_snapshot(ticket) {
            Ok(pending) => pending,
            Err(error) => {
                self.phase_changed();
                return Err(error);
            }
        };
        self.host.install_snapshot(snapshot);
        self.phase_changed();
        let count = pending.len();
        for frame in pending {
            self.apply_frame(frame);
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests;
