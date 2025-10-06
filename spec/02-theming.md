# Spec 02: Theming System

## Overview

This specification defines a configurable theming system for webex-tui that allows users to customize colors and styles through external YAML files. The system decouples UI styling from code and enables easy theme switching.

## Motivation

Currently, colors and styles are hardcoded throughout the UI modules, making customization impossible without code changes. Users have requested the ability to customize the appearance, particularly for popular themes like Dracula, and to better integrate with their terminal setups.

## Goals

- **User Customization**: Allow users to define custom color schemes via config files
- **Semantic Styling**: Use role-based color assignments instead of hardcoded colors
- **Graceful Fallbacks**: Robust error handling with fallback to defaults
- **Extensibility**: Support adding new theme roles without breaking existing themes
- **Performance**: Minimal runtime overhead with single load at startup

## Non-Goals (Initial Implementation)

- Runtime theme switching (future enhancement)
- Theme validation beyond basic parsing
- Advanced styling (gradients, animations)
- Automatic light/dark detection

## Design

### Data Model

The theme system consists of three main components:

1. **Palette**: Raw color definitions (16 ANSI colors + custom)
2. **Roles**: Semantic color assignments for UI elements
3. **User Colors**: Array for cycling sender colors in messages

```rust
pub struct Theme {
    pub name: String,
    pub palette: Palette,
    pub roles: Roles,
    pub user_colors: Vec<Color>,
}

pub struct Palette {
    pub black: Color,
    pub red: Color,
    pub green: Color,
    pub yellow: Color,
    pub blue: Color,
    pub magenta: Color,
    pub cyan: Color,
    pub white: Color,
    pub bright_black: Color,
    pub bright_red: Color,
    pub bright_green: Color,
    pub bright_yellow: Color,
    pub bright_blue: Color,
    pub bright_magenta: Color,
    pub bright_cyan: Color,
    pub bright_white: Color,
}

pub struct Roles {
    pub background: Color,
    pub text_primary: Color,
    pub text_muted: Color,
    pub accent: Color,
    pub selection_bg: Color,
    pub selection_fg: Color,
    pub border: Color,
    pub border_active: Color,
    pub title: Color,
    pub hint: Color,
    pub room_unread: Color,
    pub room_team: Color,
    pub msg_timestamp: Color,
    pub log_error: Color,
    pub log_warn: Color,
    pub log_info: Color,
    pub log_debug: Color,
    pub log_trace: Color,
    pub compose_status: Color,
}
```

### File Format

Themes are defined in YAML files located at `$HOME/.config/webex-tui/themes/<name>.yml`.

Colors can be specified as:
- Named colors: `"red"`, `"light_blue"`, `"cyan"`
- Hex codes: `"#ff5555"`, `"#282a36"`
- RGB tuples: `[255, 85, 85]`

Example theme structure:
```yaml
name: "Example Theme"
palette:
  black: "#21222c"
  red: "#ff5555"
  # ... other palette colors
roles:
  background: "#282a36"
  text_primary: "#f8f8f2"
  accent: "#bd93f9"
  # ... other role assignments
user_colors:
  - "#ff5555"
  - "#50fa7b"
  - "#f1fa8c"
  # ... cycling colors for message senders
```

### Configuration Integration

Add optional theme configuration to the existing config system:

```yaml
# In client.yml
theme: "dracula"  # Optional, defaults to "default"
```

### Loading and Fallback Logic

1. Load theme name from config (default: "default")
2. Attempt to load theme file from `~/.config/webex-tui/themes/<name>.yml`
3. Parse YAML and validate color values
4. For any missing or invalid fields, use built-in default values
5. Log warnings for invalid entries but continue loading

### Error Handling

- **File not found**: Log warning, use built-in default theme
- **Parse errors**: Log warning, use default theme
- **Invalid colors**: Log warning for specific field, use default value for that field
- **Missing fields**: Silently use default values (forward compatibility)

## Integration Points

### UI Module Changes

| Module | Current Hard-coded Colors | Theme Role Replacement |
|--------|---------------------------|------------------------|
| `ui/style.rs` | `Color::LightBlue`, `Color::LightCyan` | `room_unread`, `room_team` |
| `ui/title.rs` | `Color::LightCyan`, `Color::White` | `title`, `border` |
| `ui/rooms.rs` | `Color::Cyan`, `Color::Yellow`, `Color::Black` | `border_active`, `selection_bg`, `selection_fg` |
| `ui/messages.rs` | Hard-coded color array, `Style::new().gray()` | `user_colors`, `msg_timestamp` |
| `ui/message_editor.rs` | `Color::Yellow`, `Color::Gray` | `compose_status`, `hint` |
| `ui/help.rs` | `Color::LightCyan`, `Color::Gray` | `accent`, `text_muted` |
| `ui/logs.rs` | `Color::Red`, `Color::Yellow`, etc. | `log_error`, `log_warn`, etc. |

### AppState Integration

Add theme field to `AppState`:
```rust
pub struct AppState<'a> {
    // ... existing fields
    pub theme: Theme,
}
```

## Implementation Plan

### Phase 1: Core Infrastructure
1. Create `src/theme/` module with data structures
2. Implement YAML parsing and color conversion
3. Add theme loading function with fallback logic
4. Create built-in default theme

### Phase 2: Integration
1. Add theme field to `AppState`
2. Update config system to include theme setting
3. Integrate theme loading in main initialization

### Phase 3: UI Refactoring
1. Create style helper functions using theme roles
2. Update each UI module to use theme-based styling
3. Replace hardcoded colors with semantic roles

### Phase 4: Testing and Documentation
1. Add unit tests for theme loading and parsing
2. Create example theme files
3. Update documentation

## Example: Dracula Theme

```yaml
name: "Dracula"
palette:
  black: "#21222c"
  red: "#ff5555"
  green: "#50fa7b"
  yellow: "#f1fa8c"
  blue: "#6272a4"
  magenta: "#ff79c6"
  cyan: "#8be9fd"
  white: "#f8f8f2"
  bright_black: "#6272a4"
  bright_red: "#ff6e6e"
  bright_green: "#69ff94"
  bright_yellow: "#ffffa5"
  bright_blue: "#8185d6"
  bright_magenta: "#ff92df"
  bright_cyan: "#a4ffff"
  bright_white: "#ffffff"

roles:
  background: "#282a36"
  text_primary: "#f8f8f2"
  text_muted: "#6272a4"
  accent: "#bd93f9"
  selection_bg: "#44475a"
  selection_fg: "#f8f8f2"
  border: "#44475a"
  border_active: "#bd93f9"
  title: "#50fa7b"
  hint: "#6272a4"
  room_unread: "#8be9fd"
  room_team: "#ffb86c"
  msg_timestamp: "#6272a4"
  log_error: "#ff5555"
  log_warn: "#ffb86c"
  log_info: "#8be9fd"
  log_debug: "#50fa7b"
  log_trace: "#6272a4"
  compose_status: "#f1fa8c"

user_colors:
  - "#ff5555"
  - "#50fa7b"
  - "#f1fa8c"
  - "#bd93f9"
  - "#ff79c6"
  - "#8be9fd"
  - "#ffb86c"
  - "#69ff94"
  - "#ffffa5"
  - "#ff92df"
  - "#a4ffff"
```

## Backwards Compatibility

- Existing installations continue to work with built-in default theme
- No breaking changes to existing APIs
- Theme system is opt-in through configuration

## Future Enhancements

1. **Runtime Theme Switching**: Hot-reload themes without restart
2. **Theme Validation**: CLI command to validate theme files
3. **Theme Export**: Export current built-in theme to file
4. **Advanced Styling**: Support for modifiers (bold, italic, underline)
5. **Accessibility**: High contrast themes, colorblind-friendly palettes
6. **Auto-detection**: Light/dark theme switching based on terminal
7. **Theme Inheritance**: Allow themes to extend base themes

## Testing Strategy

### Unit Tests
- Color parsing (hex, named colors, RGB)
- Theme loading with various error conditions
- Fallback behavior for missing/invalid fields
- YAML deserialization edge cases

### Integration Tests
- End-to-end theme loading from file
- UI rendering with custom themes
- Configuration integration

### Manual Testing
- Create test themes with extreme values
- Test with missing theme files
- Verify fallback behavior
- Test with partially invalid theme files

## Security Considerations

- Theme files are read from user's home directory only
- No remote theme loading
- No code execution in theme files
- Limit user_colors array size to prevent memory issues

## Performance Impact

- Single file read at startup (minimal I/O)
- Theme data cached in memory
- No runtime parsing overhead
- Negligible memory footprint (<1KB per theme)

## Dependencies

- `serde_yaml`: For YAML parsing (already in Cargo.toml)
- `regex`: For hex color validation (add if not present)
- No new major dependencies required

## Migration Guide

For users:
1. No action required - existing installations use default theme
2. To customize: Create `~/.config/webex-tui/themes/<name>.yml`
3. Add `theme: "<name>"` to `client.yml`

For developers:
1. Replace hardcoded `Color::*` with `state.theme.roles.*`
2. Use `state.theme.user_colors` for sender cycling
3. Create style helper functions for common patterns

## Implementation Status

✅ **COMPLETED** - All core features implemented and tested.

### Completed Features

- ✅ **Theme System Architecture**: Complete `src/theme/` module with models, parser, and loader
- ✅ **YAML Configuration**: Support for theme files in `~/.config/webex-tui/themes/`
- ✅ **Color Parsing**: Hex codes (`#ff5555`), short hex (`#f55`), and named colors (`red`, `light_blue`)
- ✅ **Semantic Roles**: 19 UI element roles for consistent theming
- ✅ **Config Integration**: Added `theme` field to `client.yml` with "default" fallback
- ✅ **UI Refactoring**: All modules use theme roles instead of hardcoded colors
- ✅ **Graceful Fallbacks**: Invalid themes fall back to built-in defaults with logging
- ✅ **Dracula Theme**: Complete example theme with authentic Dracula colors
- ✅ **Testing**: Comprehensive unit tests for parsing, loading, and fallback behavior
- ✅ **Documentation**: Complete spec and user guide (`THEMING.md`)

### Verification

All acceptance criteria met:
- ✅ Users can create custom theme files
- ✅ Theme is loaded at startup and applied to all UI elements  
- ✅ Invalid themes fall back gracefully to defaults
- ✅ Dracula theme renders correctly
- ✅ No performance regression
- ✅ Backwards compatibility maintained
- ✅ Documentation updated

## Next Steps

### Future Enhancements

1. **Runtime Theme Switching**
   - Add `CycleTheme` action with hotkey
   - File system watcher for hot-reload
   - Theme selection UI in help panel

2. **Advanced Features**
   - High contrast accessibility themes
   - Light/dark auto-detection based on terminal
   - Theme inheritance (extend base themes)
   - Style modifiers (bold, italic, underline) in YAML

3. **Developer Experience**
   - CLI command `webex-tui --validate-theme <file>`
   - Theme export: `webex-tui --export-theme default > mytheme.yml`
   - Theme preview without starting full application

4. **Additional Themes**
   - Solarized Light/Dark
   - Monokai Pro
   - GitHub themes
   - Terminal-adaptive themes

### Technical Debt

- Consider extracting SerializableColor to separate crate if other projects need it
- Add theme validation beyond basic YAML parsing
- Optimize theme loading for faster startup (currently negligible impact)

### Known Limitations

- No runtime theme switching (requires restart)
- Limited to ratatui's Color enum (no gradients/transparency)
- User color array size limited to 64 entries (security measure)

## Migration Notes

This is a non-breaking change:
- Existing installations continue using built-in default theme
- No configuration changes required
- All existing functionality preserved