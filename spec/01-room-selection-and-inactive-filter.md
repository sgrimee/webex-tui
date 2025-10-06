---
title: Multi-Room Selection, Inactive Filter, Batch Leave
status: Implemented
priority: Medium
category: UX / Feature
implementation_date: 2025-10-06
dependencies: ["webex-rust client (leave_room)", "tui_logger >= 0.14.x", "tokio"]
---

# Multi-Room Selection, Inactive Filter & Batch Leave

## Problem Statement

Managing a large number of Webex rooms is inefficient:
- Users cannot select multiple rooms to leave them in bulk.
- No quick way to identify rooms that have been dormant for a long period.
- Leaving old / inactive rooms requires manual per-room navigation.

## Current State Analysis

Before this change:
- Rooms list supported filters: All, Direct, Teams, Recent, Spaces, Unread.
- No notion of selection state for rooms.
- No batch operation for leaving spaces.
- UI room list rows were passive (no indicators beyond unread state).
- Reducing noise required external cleanup (manual via Webex GUI or API).

## Proposed Solution

Introduce:
1. A new "Inactive" filter (rooms with no activity for ≥ 365 days).
2. Multi-select capability with visual checkbox indicators.
3. Actions to: toggle selection, select all visible, clear selection, delete (leave) selected rooms.
4. Batch leave implemented via a new `AppCmdEvent::LeaveRoom` dispatched per selected room.
5. Enhance rooms table title to show selection count.

## Implementation Details

- Enum additions:
  - `RoomsListFilter::Inactive`
  - `Action::{ToggleRoomSelection, SelectAllVisibleRooms, ClearRoomSelections, DeleteSelectedRooms}`
  - `AppCmdEvent::LeaveRoom(RoomId)`
- Data structure: `RoomsList` adds `selected_rooms: HashSet<RoomId>`.
- UI: Each room row prefixed with "☑ " (selected) or "☐ " (unselected).
- Batch delete: App gathers selected room IDs and dispatches one `LeaveRoom` per ID; Teams handler calls `client.leave_room(...)` then prunes from cache via `cb_space_left`.
- Filter logic: `Inactive` → `!room.has_activity_since(Duration::days(365))`.
- Key bindings:
  - Space: Toggle current room selection
  - A: Select all visible rooms
  - C: Clear selections
  - X: Delete (leave) selected rooms
- Logger: Updated to new `tui_logger` API (`TuiLoggerFile`).
- Toolchain: Bumped to Rust 1.88 (enables dependency refresh & logger update compatibility).

## Files to Create/Modify

- Modified:
  - `src/app/actions.rs`
  - `src/app/cache/room_list_filter.rs`
  - `src/app/cache/rooms.rs`
  - `src/app/rooms_list.rs`
  - `src/app/mod.rs`
  - `src/app/state.rs`
  - `src/ui/rooms.rs`
  - `src/teams/app_handler.rs`
  - `src/logger.rs`
  - `Cargo.toml` (webex git dependency)
  - `rust-toolchain.toml`, `flake.nix`, `Cargo.lock`
- New:
  - This spec file

## Migration Strategy

- No data migration; purely runtime/UI enhancements.
- Behavior change is additive; existing workflows unaffected.
- Dependency uses git commit SHA for reproducible builds.

## Testing Strategy

Manual:
- Launch app; verify new key bindings in help (if help enumerates actions).
- Apply each filter & ensure Inactive only shows long-idle rooms.
- Select multiple rooms; verify selection count in table title.
- Press X; verify rooms disappear (left) and no stale selection.
- Attempt delete with zero selections → expect warning/error path (currently returns eyre error; confirm logged).

Targeted (possible future unit tests):
- `RoomsList::toggle_room_selection` logic.
- `RoomsList::select_all_visible_rooms` idempotency.
- Filter predicate for `Inactive`.

Integration:
- Mock/record `leave_room` calls (if test harness evolves to support it).

## Benefits

- Faster inbox/space hygiene.
- Visual clarity of bulk operations.
- Enables future batch actions (mute, archive if supported later).
- Inactive filter reduces clutter and encourages workspace pruning.

## Implementation Steps

1. Extend enums (actions, filter, app command).
2. Add selection set + API to `RoomsList`.
3. Wire key handling in main App.
4. Implement batch leave dispatch + Teams handler.
5. Augment UI rendering with selection state.
6. Add Inactive filter logic.
7. Update logger API (dependency-driven).
8. Update spec + changelog + version.

## Acceptance Criteria

- User can select/unselect rooms with space.
- Selecting all visible adds every row currently rendered.
- Title reflects selection count when > 0.
- Deleting selected dispatches one leave command per room; rooms removed from UI.
- Inactive filter shows only rooms with last activity older than threshold.
- No panics on empty selection delete attempt.
- Logger still writes to file when configured.
- Build passes on Rust 1.88.

## Future Enhancements

- Configurable inactivity threshold.
- Confirmation prompt before bulk leave.
- Persistent selection across filters (currently resets implicitly when items disappear).
- Batch mute / mark read / export transcripts.
- Undo (grace period) after leave.