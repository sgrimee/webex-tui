//! Hosting of the bridge runtime task for the terminal UI (cutover S4).
//!
//! `main` spawns `Runtime::run` on its own task. On quit it requests shutdown
//! (`App::begin_quit`) and joins the task through [`join_bounded`] while the
//! UI keeps draining and redrawing, so an attempted write can still settle
//! (the runtime waits up to its 30 s write deadline). If the bound expires
//! the task is aborted; dropping the runtime settles any write still in
//! flight as uncertain (exactly once), never as unsent.

use crate::bridge::runtime::RuntimeExit;
use std::time::Duration;
use tokio::task::JoinHandle;

/// The runtime's 30 s write deadline plus margin (§7.2 f).
pub(crate) const QUIT_JOIN_BOUND: Duration = Duration::from_secs(35);

/// How often the UI is drained and redrawn while waiting.
pub(crate) const QUIT_TICK: Duration = Duration::from_millis(200);

/// How the runtime task ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Joined {
    Exited(RuntimeExit),
    /// The task panicked or was cancelled.
    Failed,
    /// It did not stop within the bound and was aborted.
    TimedOut,
}

impl Joined {
    pub(crate) fn from_join(result: Result<RuntimeExit, tokio::task::JoinError>) -> Self {
        match result {
            Ok(exit) => Self::Exited(exit),
            Err(_) => Self::Failed,
        }
    }

    /// A message to print after the terminal is restored; `None` for a
    /// normal quit. Never contains a secret, token or message content.
    pub(crate) fn message(self) -> Option<String> {
        match self {
            Self::Exited(exit) => crate::bridge::startup::exit_message(exit),
            Self::Failed => Some(
                "The webex-bridge connection task stopped unexpectedly. Start webex-tui again."
                    .into(),
            ),
            Self::TimedOut => Some(format!(
                "A write was still in progress after {} s; webex-tui stopped waiting. \
                 Check the room before resending.",
                QUIT_JOIN_BOUND.as_secs()
            )),
        }
    }

    pub(crate) fn is_clean(self) -> bool {
        self == Self::Exited(RuntimeExit::Shutdown)
    }
}

/// Wait for `task` for at most `bound`, calling `tick` every `interval`
/// (drain + redraw). Shutdown must already have been requested.
pub(crate) async fn join_bounded(
    task: &mut JoinHandle<RuntimeExit>,
    bound: Duration,
    interval: Duration,
    mut tick: impl FnMut(),
) -> Joined {
    let deadline = tokio::time::Instant::now() + bound;
    let mut ticks = tokio::time::interval(interval);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            joined = &mut *task => return Joined::from_join(joined),
            () = tokio::time::sleep_until(deadline) => break,
            _ = ticks.tick() => tick(),
        }
    }
    task.abort();
    // Let the cancellation run the runtime's drop guards before returning.
    let _ = tokio::time::timeout(Duration::from_secs(1), &mut *task).await;
    Joined::TimedOut
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    const BOUND: Duration = Duration::from_millis(400);
    const TICK: Duration = Duration::from_millis(10);

    #[tokio::test]
    async fn a_finished_runtime_is_joined_with_its_exit() {
        let mut task = tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(150)).await;
            RuntimeExit::Shutdown
        });
        let mut ticks = 0;
        let joined = join_bounded(&mut task, BOUND, TICK, || ticks += 1).await;
        assert_eq!(joined, Joined::Exited(RuntimeExit::Shutdown));
        assert!(joined.is_clean());
        assert_eq!(joined.message(), None);
        // The UI kept draining/redrawing while waiting.
        assert!(ticks >= 5, "only {ticks} ticks");
    }

    #[tokio::test]
    async fn a_runtime_still_writing_after_the_bound_is_aborted_and_dropped() {
        struct Dropped(Arc<AtomicBool>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = Dropped(Arc::clone(&dropped));
        let mut task = tokio::spawn(async move {
            let _guard = guard;
            std::future::pending::<()>().await;
            RuntimeExit::Shutdown
        });
        let started = tokio::time::Instant::now();
        let joined = join_bounded(&mut task, BOUND, TICK, || {}).await;
        assert_eq!(joined, Joined::TimedOut);
        assert!(started.elapsed() >= BOUND);
        assert!(started.elapsed() < BOUND + Duration::from_secs(2));
        assert!(dropped.load(Ordering::SeqCst), "runtime future not dropped");
        assert!(!joined.is_clean());
        assert!(joined.message().unwrap().contains("Check the room"));
    }

    #[tokio::test]
    async fn a_panicked_runtime_is_reported_not_clean() {
        let mut task = tokio::spawn(async { panic!("boom") });
        let joined = join_bounded(&mut task, BOUND, TICK, || {}).await;
        assert_eq!(joined, Joined::Failed);
        assert!(!joined.is_clean());
        assert!(joined.message().is_some());
    }

    #[test]
    fn terminal_runtime_exits_are_not_clean_and_explain_themselves() {
        for exit in [
            RuntimeExit::PairingRequired(crate::bridge::session::SessionFailure::NotPaired),
            RuntimeExit::SecretUnavailable,
            RuntimeExit::SecretCorrupt,
        ] {
            let joined = Joined::Exited(exit);
            assert!(!joined.is_clean());
            assert!(joined.message().is_some());
        }
    }
}
