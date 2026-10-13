//! Dormant single-task bridge runtime loop. Not started from `main` or App.
//!
//! The runtime solely owns the `CommandWorker` (and therefore the session
//! controller, SSE stream and capabilities). It drives, in one task:
//! - snapshot preparation whenever the session needs one (bounded by
//!   `snapshot_deadline`), through an injected `SnapshotSource` that only sees
//!   the ticket-fenced read facade;
//! - queued writes (`handle_one`, `WRITE_DEADLINE`) and reads (`handle_read`,
//!   `READ_DEADLINE`), one at a time, never retried;
//! - SSE frames from the current subscription;
//! - lease ticks: due in-place renewal (no secret), and new-session renewal
//!   or reconnection with bounded exponential backoff, each re-borrowing the
//!   TUI-owned pairing secret from its store for exactly one call
//!   (`connect_deadline`) and dropping it immediately;
//! - App wakeups (submission, snapshot ack, shutdown) via `RuntimeControl`.
//!
//! Renewal (at 80% of the lease) is in place: `POST /v1/ui/sessions/renew`
//! keeps the same token, subscription, SSE stream and installed snapshot and
//! needs no secret. Only when the daemon refuses with the 12 h cap (or a
//! zero lease, or an older daemon's 404) does the loop fall back to a new
//! session: secret re-borrow, new subscription, Ready, snapshot and
//! install/ack. A removed pairing (or session creation's exact `401
//! auth_required`) exits `PairingRequired`; a session-level
//! refusal (401, 403 `forbidden_*`) fails the session and the reconnect path
//! applies; transient failures (transport, 5xx) keep the valid lease and
//! retry with backoff. Re-snapshots still follow reconnects, lag and
//! long writes/snapshots.
//!
//! Writes and snapshot preparation borrow the worker, so SSE is not polled
//! meanwhile; the session buffers or signals lag and the loop resyncs. Exit,
//! shutdown and drop settle every admitted read and write exactly once through
//! the existing worker/mailbox guarantees. No logging; no secret is retained.

use super::{
    commands::CommandWorker,
    controller::PresentationHost,
    pairing::{SecretStoreFailure, UiSecretStore},
    session::{
        io_failure, LeaseAction, RenewStep, SessionBackend, SessionClock, SessionFailure,
        SessionPhase, SubscriptionId,
    },
};
use std::{
    future::Future,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{sync::Notify, time::Instant};
use webex_bridge::client::events::{BridgeEvent, EventStream};

/// Cancel-safe source of SSE events for the current subscription: dropping a
/// pending `next_event` future must not lose parser state or a parsed event.
/// `None` is a clean end of stream.
pub(crate) trait EventSource: Send {
    fn next_event(
        &mut self,
    ) -> impl Future<Output = Option<Result<BridgeEvent, SessionFailure>>> + Send;
}

/// Real bridge SSE: `next_event` is a cancel-safe poll over the parser, whose
/// state lives in the stream (bridge `0693534`).
impl EventSource for EventStream {
    async fn next_event(&mut self) -> Option<Result<BridgeEvent, SessionFailure>> {
        EventStream::next_event(self)
            .await
            .map(|event| event.map_err(io_failure))
    }
}

/// Builds a presentation snapshot from the ticket-fenced read facade only.
pub(crate) trait SnapshotSource<B: SessionBackend>: Send {
    type Snapshot;
    fn build(
        &mut self,
        reader: &B::SnapshotReader,
    ) -> impl Future<Output = Result<Self::Snapshot, SessionFailure>> + Send;
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RuntimeConfig {
    /// Lease check / renewal tick.
    pub(crate) tick: Duration,
    /// Bound for one connect or renewal (issue + subscribe).
    pub(crate) connect_deadline: Duration,
    /// Bound for one snapshot preparation (boundary + reads + commit).
    pub(crate) snapshot_deadline: Duration,
    /// First reconnect is immediate; then min · 2^(n-1), capped at max.
    pub(crate) backoff_min: Duration,
    pub(crate) backoff_max: Duration,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            tick: Duration::from_secs(1),
            connect_deadline: Duration::from_secs(15),
            snapshot_deadline: Duration::from_secs(30),
            backoff_min: Duration::from_secs(1),
            backoff_max: Duration::from_secs(30),
        }
    }
}

/// Why the loop ended. In every case admitted work has been settled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeExit {
    Shutdown,
    /// The daemon refused the pairing secret itself (session creation's
    /// `401 auth_required`, or renewal's `pairing_removed`); explicit
    /// re-pairing is a user decision outside this loop.
    PairingRequired(SessionFailure),
    /// The TUI secret store denied access or no longer holds the secret.
    /// Transient store unavailability is retried with backoff instead.
    SecretUnavailable,
    /// The stored secret is malformed. Permanent and never overwritten: the
    /// user removes the Keychain item and re-pairs.
    SecretCorrupt,
}

/// App-side control: wake after submit/ack, or request shutdown. Cloneable,
/// capability-free and synchronous.
#[derive(Clone)]
pub(crate) struct RuntimeControl {
    wake: Arc<Notify>,
    stop: Arc<AtomicBool>,
    stop_notify: Arc<Notify>,
}

impl RuntimeControl {
    /// A control not attached to any runtime, for glue tests that drive the
    /// presentation pump by hand.
    #[cfg(test)]
    pub(crate) fn detached() -> Self {
        Self {
            wake: Arc::new(Notify::new()),
            stop: Arc::new(AtomicBool::new(false)),
            stop_notify: Arc::new(Notify::new()),
        }
    }

    pub(crate) fn wake(&self) {
        self.wake.notify_one();
    }
    /// Request shutdown. An in-flight connect, renewal or snapshot is
    /// abandoned promptly (each is fenced); an attempted write is never
    /// cancelled, so quitting may wait up to `WRITE_DEADLINE` (30 s) for it.
    pub(crate) fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.wake.notify_one();
        self.stop_notify.notify_one();
    }
}

/// Resolves once shutdown has been requested.
async fn stopped(stop: Arc<AtomicBool>, notify: Arc<Notify>) {
    loop {
        let notified = notify.notified();
        if stop.load(Ordering::SeqCst) {
            return;
        }
        notified.await;
    }
}

enum Woke {
    App,
    Event(SubscriptionId, Option<Result<BridgeEvent, SessionFailure>>),
    Tick,
    /// Nothing ready while queued work is waiting: resume the work.
    Idle,
}

/// Queued writes/reads handled back to back before SSE, App wakeups and
/// ticks are polled again, so a full read queue cannot starve them.
const MAX_BURST: usize = 4;

pub(crate) struct Runtime<B, C, H, K, P>
where
    B: SessionBackend,
    C: SessionClock,
    H: PresentationHost,
{
    worker: CommandWorker<B, C, H>,
    secrets: K,
    snapshots: P,
    config: RuntimeConfig,
    wake: Arc<Notify>,
    stop: Arc<AtomicBool>,
    stop_notify: Arc<Notify>,
    attempts: u32,
    reconnect_at: Option<Instant>,
    /// Persistent lease-tick schedule. Checked at the top of every loop
    /// iteration, so steady SSE traffic or queued work cannot starve renewal.
    next_tick: Instant,
    /// When the session last became Active. Reconnect backoff resets only
    /// after staying Active for `backoff_max`, so a session that fails right
    /// after becoming Active (crash loop, rejected stream) keeps backing off.
    active_since: Option<Instant>,
    /// Renewal secret-load retries after transient store unavailability are
    /// paced by the reconnect backoff, not every tick (REVIEW-13 LOW-4).
    renew_attempts: u32,
    renew_retry_at: Option<Instant>,
    /// Subscription whose in-place renewal hit the 12 h cap: renew it by a
    /// new session only.
    new_session_for: Option<SubscriptionId>,
    burst: usize,
}

impl<B, C, H, K, P> Runtime<B, C, H, K, P>
where
    B: SessionBackend,
    B::Stream: EventSource,
    C: SessionClock,
    H: PresentationHost,
    K: UiSecretStore,
    P: SnapshotSource<B, Snapshot = H::Snapshot>,
{
    /// `worker` is typically already connected by the pairing bootstrap; a
    /// disconnected worker is connected on the first loop iteration.
    pub(crate) fn new(
        worker: CommandWorker<B, C, H>,
        secrets: K,
        snapshots: P,
        config: RuntimeConfig,
    ) -> (Self, RuntimeControl) {
        let wake = Arc::new(Notify::new());
        let stop = Arc::new(AtomicBool::new(false));
        let stop_notify = Arc::new(Notify::new());
        let control = RuntimeControl {
            wake: Arc::clone(&wake),
            stop: Arc::clone(&stop),
            stop_notify: Arc::clone(&stop_notify),
        };
        (
            Self {
                worker,
                secrets,
                snapshots,
                config,
                wake,
                stop,
                stop_notify,
                attempts: 0,
                reconnect_at: None,
                next_tick: Instant::now(),
                active_since: None,
                renew_attempts: 0,
                renew_retry_at: None,
                new_session_for: None,
                burst: 0,
            },
            control,
        )
    }

    /// Run until shutdown or a terminal pairing/secret failure, then settle
    /// all admitted work and disconnect. Dropping this future instead is also
    /// safe: the worker's guards settle in-flight and queued work.
    pub(crate) async fn run(mut self) -> RuntimeExit {
        let exit = self.run_inner().await;
        self.worker.shutdown();
        self.worker.disconnect();
        exit
    }

    async fn run_inner(&mut self) -> RuntimeExit {
        loop {
            if self.stop.load(Ordering::SeqCst) {
                return RuntimeExit::Shutdown;
            }
            let now = Instant::now();
            if now >= self.next_tick {
                self.next_tick = now + self.config.tick;
                if let Some(exit) = self.on_tick().await {
                    return exit;
                }
            }
            match self.worker.phase() {
                SessionPhase::SnapshotRequired => {
                    self.prepare_snapshot().await;
                    continue;
                }
                SessionPhase::Active => {
                    self.reconnect_at = None;
                    let since = *self.active_since.get_or_insert(now);
                    if now.saturating_duration_since(since) >= self.config.backoff_max {
                        self.attempts = 0;
                    }
                }
                SessionPhase::AwaitingReady => {}
                SessionPhase::Connecting
                | SessionPhase::Disconnected
                | SessionPhase::Expired
                | SessionPhase::Failed(_) => {
                    self.active_since = None;
                    if let Some(exit) = self.reconnect_if_due().await {
                        return exit;
                    }
                }
            }
            // Writes are never held for a later session: the worker settles
            // them immediately if the session is not ready. Reads are
            // idempotent and are held until the session is Active (they hold
            // read credits meanwhile and are settled on shutdown), so the App
            // never spins on NotReady right after an install.
            let active = self.worker.phase() == SessionPhase::Active;
            let write = self.worker.has_queued_write();
            let read = active && self.worker.has_queued_read();
            if (write || read) && self.burst < MAX_BURST {
                self.burst += 1;
                if write {
                    self.worker.handle_one().await;
                } else {
                    self.worker.handle_read().await;
                }
                continue;
            }
            self.burst = 0;
            let woke = self.wait(write || read).await;
            match woke {
                Woke::App => self.worker.refresh_admission(),
                Woke::Event(subscription, event) => self.on_event(subscription, event),
                // The tick itself runs at the top of the next iteration.
                Woke::Tick | Woke::Idle => {}
            }
        }
    }

    /// Wait for an App wakeup, an SSE event or the next tick. With
    /// `poll_only` (queued work waiting) this only polls and returns `Idle`
    /// when nothing is ready.
    async fn wait(&mut self, poll_only: bool) -> Woke {
        let wake = Arc::clone(&self.wake);
        let deadline = match self.reconnect_at {
            Some(at) => at.min(self.next_tick),
            None => self.next_tick,
        };
        let subscription = self.worker.subscription();
        let worker = &mut self.worker;
        tokio::select! {
            biased;
            () = wake.notified() => Woke::App,
            event = async {
                match (subscription, worker.stream_mut()) {
                    (Some(subscription), Some(stream)) => (subscription, stream.next_event().await),
                    _ => std::future::pending().await,
                }
            } => Woke::Event(event.0, event.1),
            () = tokio::time::sleep_until(deadline) => Woke::Tick,
            () = std::future::ready(()), if poll_only => Woke::Idle,
        }
    }

    fn on_event(
        &mut self,
        subscription: SubscriptionId,
        event: Option<Result<BridgeEvent, SessionFailure>>,
    ) {
        match event {
            // Projection/continuity failures are handled by the controller.
            Some(Ok(event)) => {
                let _ = self.worker.bridge_event(subscription, &event);
            }
            Some(Err(failure)) => {
                self.worker.stream_failed(subscription, failure);
            }
            None => {
                self.worker
                    .stream_failed(subscription, SessionFailure::StreamEnded);
            }
        }
    }

    async fn on_tick(&mut self) -> Option<RuntimeExit> {
        if self.worker.check_lease() != LeaseAction::Renew {
            self.renew_attempts = 0;
            self.renew_retry_at = None;
            return None; // expiry is handled by the reconnect path
        }
        let now = Instant::now();
        if self.renew_retry_at.is_some_and(|at| now < at) {
            return None;
        }
        // 1. In place: same token and SSE stream, no secret, no snapshot.
        // Skipped once the daemon reported the 12 h cap for this
        // subscription; only a new session helps then.
        let subscription = self.worker.subscription();
        if subscription.is_none() || subscription != self.new_session_for {
            let stop = stopped(Arc::clone(&self.stop), Arc::clone(&self.stop_notify));
            let step = tokio::select! {
                biased;
                () = stop => return Some(RuntimeExit::Shutdown),
                step = tokio::time::timeout(
                    self.config.connect_deadline,
                    self.worker.renew_in_place_if_due(),
                ) => step,
            };
            match step {
                Ok(RenewStep::Renewed | RenewStep::NotDue) => {
                    self.renew_attempts = 0;
                    self.renew_retry_at = None;
                    return None;
                }
                // The daemon ended the session because the pairing is gone.
                // Note: `webex-bridge ui unpair` revokes live tokens, so the
                // usual unpair signal is a plain 401 on the next renew or
                // request instead; that fails the session, the reconnect's
                // create-session 401 then exits `PairingRequired` as well.
                Ok(RenewStep::PairingRemoved) => {
                    return Some(RuntimeExit::PairingRequired(SessionFailure::AuthRequired))
                }
                // Session Failed: the reconnect path (with backoff) applies.
                Ok(RenewStep::Failed(_)) => return None,
                Ok(RenewStep::NewSession) => self.new_session_for = subscription,
                // Transient failure or abandoned mid-call: the old lease is
                // untouched and still valid; retry with backoff rather than
                // every tick (expiry falls to the reconnect path).
                Ok(RenewStep::Retry) | Err(_) => {
                    self.renew_attempts = self.renew_attempts.saturating_add(1);
                    self.renew_retry_at = Some(now + backoff(&self.config, self.renew_attempts));
                    return None;
                }
            }
        }
        // 2. New session (cap reached): re-borrow the secret for one call.
        let secret = match self.secrets.load().await {
            Ok(Some(secret)) => {
                self.renew_attempts = 0;
                self.renew_retry_at = None;
                secret
            }
            // A transient store failure (e.g. locked keychain) retries with
            // the reconnect backoff while the lease is still valid; expiry
            // then falls to the reconnect path, which keeps backing off.
            Err(SecretStoreFailure::Unavailable) => {
                self.renew_attempts = self.renew_attempts.saturating_add(1);
                self.renew_retry_at = Some(now + backoff(&self.config, self.renew_attempts));
                return None;
            }
            Ok(None) | Err(SecretStoreFailure::Denied) => {
                return Some(RuntimeExit::SecretUnavailable)
            }
            Err(SecretStoreFailure::Corrupt) => return Some(RuntimeExit::SecretCorrupt),
        };
        let stop = stopped(Arc::clone(&self.stop), Arc::clone(&self.stop_notify));
        let renewed = tokio::select! {
            biased;
            () = stop => return Some(RuntimeExit::Shutdown),
            renewed = tokio::time::timeout(
                self.config.connect_deadline,
                self.worker.renew_if_due(secret.as_str()),
            ) => renewed,
        };
        drop(secret);
        match renewed {
            // Only session creation's exact `401 auth_required` means the
            // secret is not paired. A 401/403 on the event subscription right
            // after a successful create (daemon restart, or a revoke racing
            // it), or any other create refusal, fails this attempt and the
            // reconnect path retries with backoff.
            Ok(Err(failure @ SessionFailure::NotPaired)) => {
                Some(RuntimeExit::PairingRequired(failure))
            }
            // Other failures leave the session Failed; a timed-out renewal
            // leaves it Connecting. Both are picked up by the reconnect path.
            _ => None,
        }
    }

    async fn reconnect_if_due(&mut self) -> Option<RuntimeExit> {
        let now = Instant::now();
        let due = match self.reconnect_at {
            Some(at) => at,
            None => {
                let at = now + self.backoff();
                self.reconnect_at = Some(at);
                at
            }
        };
        if now < due {
            return None;
        }
        self.reconnect_at = None;
        self.attempts = self.attempts.saturating_add(1);
        let secret = match self.secrets.load().await {
            Ok(Some(secret)) => secret,
            // Transient: the next attempt follows the (already advanced) backoff.
            Err(SecretStoreFailure::Unavailable) => return None,
            Ok(None) | Err(SecretStoreFailure::Denied) => {
                return Some(RuntimeExit::SecretUnavailable)
            }
            Err(SecretStoreFailure::Corrupt) => return Some(RuntimeExit::SecretCorrupt),
        };
        let stop = stopped(Arc::clone(&self.stop), Arc::clone(&self.stop_notify));
        let connected = tokio::select! {
            biased;
            () = stop => return Some(RuntimeExit::Shutdown),
            connected = tokio::time::timeout(
                self.config.connect_deadline,
                self.worker.connect(secret.as_str()),
            ) => connected,
        };
        drop(secret);
        match connected {
            // Only session creation's exact `401 auth_required` means the
            // secret is not paired. A 401/403 on the event subscription right
            // after a successful create (daemon restart, or a revoke racing
            // it), or any other create refusal, fails this attempt and the
            // reconnect path retries with backoff.
            Ok(Err(failure @ SessionFailure::NotPaired)) => {
                Some(RuntimeExit::PairingRequired(failure))
            }
            _ => None,
        }
    }

    fn backoff(&self) -> Duration {
        backoff(&self.config, self.attempts)
    }

    async fn prepare_snapshot(&mut self) {
        let Some(subscription) = self.worker.subscription() else {
            return;
        };
        let stop = stopped(Arc::clone(&self.stop), Arc::clone(&self.stop_notify));
        let Self {
            worker,
            snapshots,
            config,
            ..
        } = self;
        let prepared = tokio::select! {
            biased;
            // An abandoned snapshot is ticket-fenced: nothing is installed.
            () = stop => return,
            prepared = tokio::time::timeout(config.snapshot_deadline, async {
                let ticket = worker.begin_snapshot().await?;
                let snapshot = {
                    let reader = worker.snapshot_reader(&ticket)?;
                    snapshots.build(&reader).await?
                };
                worker.commit_snapshot(ticket, snapshot).map(|_| ())
            }) => prepared,
        };
        let failure = match prepared {
            Ok(Ok(())) => return,
            Ok(Err(failure)) => failure,
            Err(_) => SessionFailure::Transport,
        };
        // A stale ticket after an intervening reconnect needs no action; any
        // other failure ends this subscription so the reconnect path retries
        // with backoff instead of spinning on SnapshotRequired.
        if failure != SessionFailure::StaleSnapshot
            && self.worker.phase() == SessionPhase::SnapshotRequired
        {
            self.worker.stream_failed(subscription, failure);
        }
    }
}

/// Zero for no attempt yet, then `backoff_min · 2^(attempts-1)`, capped at
/// `backoff_max`.
fn backoff(config: &RuntimeConfig, attempts: u32) -> Duration {
    if attempts == 0 {
        return Duration::ZERO;
    }
    let factor = 1_u32.checked_shl(attempts - 1).unwrap_or(u32::MAX);
    config
        .backoff_min
        .saturating_mul(factor)
        .min(config.backoff_max)
}

#[cfg(test)]
mod tests;
