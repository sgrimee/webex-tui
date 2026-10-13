//! Ordered worker → App presentation pump with a snapshot install/ack barrier.
//!
//! The worker-side `PumpHost` is the `PresentationHost` inside the worker's
//! controller; the App-side `PresentationEndpoint` drains events synchronously
//! and acknowledges snapshot installation. No await, task, timer or logging.
//!
//! Ordering: events are drained exactly in the order the controller produced
//! them, so post-boundary lifecycle metadata replayed after a snapshot commit,
//! and read results dispatched after it, always follow that snapshot.
//! Bounds without silent loss:
//! - adjacent phase updates coalesce to the latest;
//! - adjacent lifecycle updates merge (latest projection, union of hints);
//! - at most `LIFECYCLE_CAP` lifecycle entries are queued; beyond that new
//!   hints merge into the newest entry, which is upgraded to a whole-mirror
//!   lag resync, and redaction keys are always retained;
//! - an undrained snapshot superseded by a newer install is dropped; only the
//!   latest install can be acknowledged;
//! - read terminals are bounded by the worker's read capacity (one terminal
//!   per admitted read) and are never coalesced or dropped.
//!
//! Barrier: until the App has drained and acknowledged the latest install,
//! `snapshot_acknowledged()` is false and the worker keeps write admission
//! revoked, so Active cannot race ahead of App installation.

use super::{controller::PresentationHost, read_dispatch::ReadTerminal, session::SessionPhase};
use crate::frontend::{
    events::{ChangeScope, ResyncReason},
    lifecycle::{LifecycleProjection, ProjectionEffects, RefreshTarget},
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, MutexGuard},
};

pub(crate) const LIFECYCLE_CAP: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InstallId(u64);

/// No Debug: snapshots carry presentation data.
pub(crate) enum PumpEvent<S> {
    Phase(SessionPhase),
    Lifecycle {
        projection: LifecycleProjection,
        effects: ProjectionEffects,
    },
    /// Install, then `PresentationEndpoint::ack(install)`.
    Snapshot {
        install: InstallId,
        snapshot: S,
    },
    /// Bounded by the worker's read capacity plus one in-flight read.
    Read(ReadTerminal),
}

struct Pump<S> {
    queue: VecDeque<PumpEvent<S>>,
    lifecycle: usize,
    last_install: u64,
    latest: Option<InstallId>,
    drained_latest: bool,
    acked: Option<InstallId>,
    closed: bool,
}

type Shared<S> = Arc<Mutex<Pump<S>>>;

fn lock<S>(shared: &Shared<S>) -> MutexGuard<'_, Pump<S>> {
    // No callbacks, awaits or logging under this short internal lock.
    shared.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// Create the worker-side host and the App-side endpoint.
pub(crate) fn pump<S>() -> (PumpHost<S>, PresentationEndpoint<S>) {
    let shared = Arc::new(Mutex::new(Pump {
        queue: VecDeque::new(),
        lifecycle: 0,
        last_install: 0,
        latest: None,
        drained_latest: false,
        acked: None,
        closed: false,
    }));
    (
        PumpHost {
            shared: Arc::clone(&shared),
        },
        PresentationEndpoint { shared },
    )
}

/// Worker side; owned by the controller. No accessor returns queued data.
pub(crate) struct PumpHost<S> {
    shared: Shared<S>,
}

impl<S> PresentationHost for PumpHost<S> {
    type Snapshot = S;

    fn session_phase(&mut self, phase: SessionPhase) {
        let mut pump = lock(&self.shared);
        if let Some(PumpEvent::Phase(last)) = pump.queue.back_mut() {
            *last = phase;
        } else {
            pump.queue.push_back(PumpEvent::Phase(phase));
        }
    }

    fn lifecycle(&mut self, projection: &LifecycleProjection, effects: ProjectionEffects) {
        let mut pump = lock(&self.shared);
        if let Some(PumpEvent::Lifecycle {
            projection: last,
            effects: queued,
        }) = pump.queue.back_mut()
        {
            if mergeable(queued, &effects) {
                *last = projection.clone();
                merge(queued, effects);
                return;
            }
        }
        if pump.lifecycle < LIFECYCLE_CAP {
            pump.lifecycle += 1;
            pump.queue.push_back(PumpEvent::Lifecycle {
                projection: projection.clone(),
                effects,
            });
            return;
        }
        // Overflow: never drop hints silently. Merge into the newest queued
        // lifecycle entry and upgrade it to a whole-mirror lag reconcile.
        let target = pump.queue.iter_mut().rev().find_map(|event| match event {
            PumpEvent::Lifecycle {
                projection,
                effects,
            } => Some((projection, effects)),
            _ => None,
        });
        if let Some((last, queued)) = target {
            *last = projection.clone();
            let membership = queued.membership.clone();
            merge(queued, effects);
            // A conflicting membership hint is covered by the Rooms refresh.
            queued.membership = membership;
            lag_resync(queued);
        }
    }

    fn install_snapshot(&mut self, snapshot: S) {
        let mut pump = lock(&self.shared);
        let Some(next) = pump.last_install.checked_add(1) else {
            // Exhausted install IDs can never be acknowledged unambiguously.
            pump.closed = true;
            return;
        };
        pump.last_install = next;
        // An undrained older snapshot is superseded; only the latest matters.
        pump.queue
            .retain(|event| !matches!(event, PumpEvent::Snapshot { .. }));
        let install = InstallId(next);
        pump.latest = Some(install);
        pump.drained_latest = false;
        pump.queue
            .push_back(PumpEvent::Snapshot { install, snapshot });
    }

    fn deliver_read(&mut self, terminal: ReadTerminal) {
        lock(&self.shared)
            .queue
            .push_back(PumpEvent::Read(terminal));
    }

    fn snapshot_acknowledged(&self) -> bool {
        let pump = lock(&self.shared);
        !pump.closed && pump.latest.is_none_or(|latest| pump.acked == Some(latest))
    }
}

impl<S> Drop for PumpHost<S> {
    fn drop(&mut self) {
        lock(&self.shared).closed = true;
    }
}

/// App side. Synchronous drain/ack only; no capability or lifecycle access.
pub(crate) struct PresentationEndpoint<S> {
    shared: Shared<S>,
}

impl<S> PresentationEndpoint<S> {
    /// Up to `max` events in production order.
    pub(crate) fn drain(&self, max: usize) -> Vec<PumpEvent<S>> {
        let mut pump = lock(&self.shared);
        let mut events = Vec::new();
        while events.len() < max {
            let Some(event) = pump.queue.pop_front() else {
                break;
            };
            match &event {
                PumpEvent::Lifecycle { .. } => pump.lifecycle -= 1,
                PumpEvent::Snapshot { install, .. } if pump.latest == Some(*install) => {
                    pump.drained_latest = true;
                }
                _ => {}
            }
            events.push(event);
        }
        events
    }

    /// Acknowledge installing a drained snapshot. Only the latest install,
    /// after it was drained, is accepted; stale or early acks return false.
    pub(crate) fn ack(&self, install: InstallId) -> bool {
        let mut pump = lock(&self.shared);
        if pump.closed || pump.latest != Some(install) || !pump.drained_latest {
            return false;
        }
        pump.acked = Some(install);
        true
    }

    /// The worker/host is gone: treat every outstanding request as lost.
    pub(crate) fn is_closed(&self) -> bool {
        lock(&self.shared).closed
    }
}

fn mergeable(queued: &ProjectionEffects, next: &ProjectionEffects) -> bool {
    queued.membership.is_none() || next.membership.is_none() || queued.membership == next.membership
}

fn merge(queued: &mut ProjectionEffects, next: ProjectionEffects) {
    for target in next.refresh {
        push_unique(&mut queued.refresh, target);
    }
    for key in next.redact {
        push_unique(&mut queued.redact, key);
    }
    if queued.membership.is_none() {
        queued.membership = next.membership;
    }
    queued.resync = match (queued.resync.take(), next.resync) {
        (None, next) => next,
        (current, None) => current,
        (Some(current), Some(next)) if current == next => Some(current),
        (Some(_), Some((_, reason))) => {
            // Different scopes/reasons: reconcile everything loaded.
            for target in whole_mirror_refresh() {
                push_unique(&mut queued.refresh, target);
            }
            Some((ChangeScope::WholeMirror, reason))
        }
    };
}

fn lag_resync(queued: &mut ProjectionEffects) {
    queued.resync = Some((ChangeScope::WholeMirror, ResyncReason::EventLag));
    for target in whole_mirror_refresh() {
        push_unique(&mut queued.refresh, target);
    }
}

fn whole_mirror_refresh() -> [RefreshTarget; 3] {
    [
        RefreshTarget::LoadedMessages(ChangeScope::WholeMirror),
        RefreshTarget::Status,
        RefreshTarget::Rooms,
    ]
}

fn push_unique<T: PartialEq>(items: &mut Vec<T>, item: T) {
    if !items.contains(&item) {
        items.push(item);
    }
}

#[cfg(test)]
mod tests;
