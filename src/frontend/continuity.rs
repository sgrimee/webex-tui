//! Neutral UI stream metadata/controls, independent of bridge protocol types.
//!
//! These values do not subscribe, recreate sessions or apply snapshots. A future
//! stream owner must fence each subscription and process controls before changes.

use super::events::{ChangeScope, LifecycleEvent, ResyncReason};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DaemonInstanceId(String);

impl DaemonInstanceId {
    pub(crate) fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ContinuityBoundary {
    pub(crate) instance_id: DaemonInstanceId,
    pub(crate) watermark: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotRelation {
    DifferentInstance,
    Covered,
    After,
}

impl ContinuityBoundary {
    /// Only compare watermarks within one daemon instance. Earlier/equal events
    /// can be covered by a snapshot; they are not malformed/backwards counters.
    /// This is a pure relation, not permission to discard a stream control.
    pub(crate) fn relative_to(&self, snapshot: &Self) -> SnapshotRelation {
        if self.instance_id != snapshot.instance_id {
            SnapshotRelation::DifferentInstance
        } else if self.watermark <= snapshot.watermark {
            SnapshotRelation::Covered
        } else {
            SnapshotRelation::After
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LifecycleControl {
    Ready(ContinuityBoundary),
    ResyncRequired {
        boundary: ContinuityBoundary,
        scope: ChangeScope,
        reason: ResyncReason,
    },
    /// Terminal for its owning subscription, not an upstream OAuth logout.
    /// Expiry has no daemon watermark; attach local subscription identity in the
    /// future runtime and never suppress it through snapshot/generation filtering.
    SessionExpired,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum UiLifecycleFrame {
    Control(LifecycleControl),
    Change {
        /// None is allowed only for the shared unsequenced connection event.
        boundary: Option<ContinuityBoundary>,
        event: LifecycleEvent,
    },
    /// Retain a future/unknown UI event's valid marker without retaining any
    /// payload content. A stream owner must account for it before ignoring it.
    Unrecognized {
        boundary: Option<ContinuityBoundary>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boundary(instance: &str, watermark: u64) -> ContinuityBoundary {
        ContinuityBoundary {
            instance_id: DaemonInstanceId::new(instance),
            watermark,
        }
    }

    #[test]
    fn instance_identity_precedes_numeric_watermark_comparison() {
        assert_eq!(
            boundary("new-instance", 0).relative_to(&boundary("old-instance", u64::MAX)),
            SnapshotRelation::DifferentInstance
        );
    }

    #[test]
    fn snapshot_covers_equal_older_and_zero_without_arithmetic_or_wrap() {
        let snapshot = boundary("instance", 10);
        assert_eq!(
            boundary("instance", 0).relative_to(&snapshot),
            SnapshotRelation::Covered
        );
        assert_eq!(
            boundary("instance", 10).relative_to(&snapshot),
            SnapshotRelation::Covered
        );
        assert_eq!(
            boundary("instance", 11).relative_to(&snapshot),
            SnapshotRelation::After
        );
        assert_eq!(
            boundary("instance", u64::MAX).relative_to(&snapshot),
            SnapshotRelation::After
        );
        assert_eq!(
            boundary("instance", u64::MAX).relative_to(&boundary("instance", u64::MAX)),
            SnapshotRelation::Covered
        );
    }
}
