# Enhanced Search and Selection

## Overview

This specification defines improvements to the search and room selection functionality in webex-tui, addressing the conflict between search mode and selection operations while adding powerful multi-selection capabilities.

## Problem Statement

Currently, when using the unusedspaces filter with search (`/` shortcut), users cannot use the space key for room selection because it adds a space character to the search term instead. This prevents efficient workflow of search → filter → select multiple rooms → perform actions.

## Goals

1. **Dual-mode search**: Separate search entry mode from search filtering mode
2. **Preserve space for selection**: Space key should toggle room selection when not entering search terms
3. **Rapid multi-selection**: Support for selecting consecutive items efficiently
4. **Standard TUI shortcuts**: Implement common multi-selection patterns from other TUI applications
5. **Clear search management**: Intuitive way to clear search filters

## Solution: Search State Machine

### Search States

```rust
pub enum SearchState {
    /// Not searching - normal room browsing
    None,
    /// Actively typing search query
    Entering,
    /// Search applied, can select rooms with space
    Filtering,
}
```

### Key Bindings

#### Search Operations
- **`/`** - Enter search entering mode (or return to entering mode from filtering mode)
- **`Enter` or `Esc`** - Apply search query and switch to filtering mode (allows room selection with Space)
- **`Ctrl+U`** - Clear search filter from any mode and return to normal
- **`Backspace`** - Remove last character from search query; if query becomes empty, clear search entirely
- **Any character** - Add to search query (including `/` when already in entering mode)

#### Selection Operations (Normal & Filtering Modes)
- **`Space`** - Toggle selection of highlighted room AND move cursor to next room
- **`Ctrl+A`** - Select all visible rooms
- **`Ctrl+I`** - Invert selection of all visible rooms  
- **`Ctrl+D`** - Clear all selections (deselect all)

#### Navigation (All Modes)
- **`Up/Down`** or **`j/k`** - Navigate room list
- **`Home/End`** or **`gg/G`** - Jump to first/last room

### UI Feedback

Room panel title shows current state:
- **Normal mode**: `"Filter: {filter} ({selected} selected)"`
- **Search entering**: `"Search: {query}"`  
- **Search filtering**: `"Filtered: {query} ({selected} selected)"`

Contextual help shows only relevant actions:
- **Search entering mode**: Only shows `Esc` (exit search) and `Ctrl+C` (quit)
- **Normal/filtering modes**: Shows full action list including selection shortcuts

### Behavior Details

#### Rapid Multi-Selection
When `Space` is pressed to select a room:
1. Toggle selection state of current room
2. Automatically move cursor to next room
3. This enables rapid selection: Space, Space, Space to select consecutive rooms

#### Selection Scope
- **Select All** (`Ctrl+A`): Affects only currently visible/filtered rooms
- **Invert Selection** (`Ctrl+I`): Inverts selection state of all visible rooms
- **Clear Selection** (`Ctrl+D`): Clears selection from all rooms (including non-visible)

#### Search Filter Persistence
- Search filters remain active when switching between filtering and normal modes
- Only `Ctrl+U` or backspacing to empty clears the search
- Search query is preserved when switching between entering and filtering modes

#### Search Within Filter
- **Critical**: Search respects the current filter (e.g., UnusedSpaces, InactiveSpaces)
- When searching, only rooms matching the current filter are searched
- This ensures operations like deletion only affect the filtered subset
- Example: Search in UnusedSpaces for "Empty Title" will only find unused spaces, not 1-1 direct rooms

## Implementation Plan

### New Types
```rust
// In app/state.rs or new file
pub enum SearchState {
    None,
    Entering,
    Filtering,
}
```

### Modified Files

1. **`app/state.rs`**
   - Add `SearchState` tracking
   - Remove dependency on `ActivePane::Search`

2. **`app/actions.rs`**
   - Add `ClearSearchFilter` action
   - Add `SelectAllVisibleRooms` action (if not exists)
   - Add `InvertSelection` action
   - Add `ClearAllSelections` action

3. **`app/mod.rs`**
   - Modify `process_search_key()` to handle search states
   - Add logic for rapid selection (space + move cursor)
   - Remove `ActivePane::Search` usage

4. **`app/rooms_list.rs`**
   - Add search state management methods
   - Add `invert_selection()` method
   - Modify selection methods for cursor advancement

5. **`ui/rooms.rs`**
   - Update title display logic for new states
   - Remove search mode highlighting (use filtering state instead)

### Key Mapping Updates

```rust
// In actions.rs key mapping
Action::ClearSearchFilter => &[Key::Ctrl('u')],
Action::SelectAllVisibleRooms => &[Key::Ctrl('a')],
Action::InvertSelection => &[Key::Ctrl('i')],
Action::ClearAllSelections => &[Key::Ctrl('d')],
```

## Edge Cases

1. **Empty search results**: Clear selection when no rooms match search
2. **Filter changes**: Preserve selection when switching between different filters
3. **Search while selecting**: Entering search mode preserves existing selections
4. **Rapid key presses**: Ensure cursor advancement doesn't skip rooms

## Compatibility

- Existing `/` and `Esc` behavior preserved
- All current room filtering functionality unchanged
- Selection state maintained across filter changes
- No breaking changes to existing shortcuts

## Testing Scenarios

1. **Basic search flow**: `/` → type query → `Enter` → `Space` selects rooms
2. **Search editing**: `/` in filtering mode returns to entering mode  
3. **Rapid selection**: Multiple `Space` presses select consecutive rooms
4. **Select all**: `Ctrl+A` selects all visible rooms in filtered view
5. **Invert selection**: `Ctrl+I` toggles all visible room selections
6. **Clear operations**: `Ctrl+D` clears selections, `Ctrl+U` clears search
7. **Mode transitions**: All state transitions work correctly

## Future Enhancements

- Pattern-based selection (e.g., `+` for regex patterns like Midnight Commander)
- Range selection with Shift+arrow keys
- Bookmark search queries for quick access
- Selection persistence across application restarts