# Theming Guide

webex-tui supports customizable themes through YAML configuration files.

## Using Themes

1. **Set theme in config**: Add `theme: "theme_name"` to your `~/.config/webex-tui/client.yml`

2. **Create theme file**: Place your theme YAML file at `~/.config/webex-tui/themes/theme_name.yml`

3. **Fallback**: If the theme file is missing or invalid, webex-tui will use the built-in default theme

## Theme File Format

Themes are defined in YAML files with three main sections:

```yaml
name: "My Theme"

# Raw color palette (optional)
palette:
  black: "#000000"
  red: "#ff0000"
  # ... other colors

# Semantic color roles (controls UI appearance)
roles:
  background: "#282a36"
  text_primary: "#f8f8f2"
  accent: "#bd93f9"
  border_active: "#bd93f9"
  selection_bg: "#44475a"
  selection_fg: "#f8f8f2"
  room_unread: "#8be9fd"
  room_team: "#ffb86c"
  msg_timestamp: "#6272a4"
  compose_status: "#f1fa8c"
  hint: "#6272a4"
  # ... other roles

# Colors for cycling message senders
user_colors:
  - "#ff5555"
  - "#50fa7b"
  - "#f1fa8c"
  # ... more colors
```

## Color Formats

Colors can be specified as:
- **Hex codes**: `"#ff5555"` or `"#f55"`
- **Named colors**: `"red"`, `"light_blue"`, `"cyan"`

## Theme Roles

The `roles` section defines semantic color assignments:

| Role | Purpose |
|------|---------|
| `background` | Main background color |
| `text_primary` | Primary text color |
| `text_muted` | Secondary/muted text |
| `accent` | Accent color for highlights |
| `selection_bg` | Background for selected items |
| `selection_fg` | Foreground for selected items |
| `border` | Default border color |
| `border_active` | Border for active/focused panes |
| `title` | Application title color |
| `hint` | Hint text color |
| `room_unread` | Unread room indicators |
| `room_team` | Team name color |
| `msg_timestamp` | Message timestamp color |
| `log_error` | Error log color |
| `log_warn` | Warning log color |
| `log_info` | Info log color |
| `log_debug` | Debug log color |
| `log_trace` | Trace log color |
| `compose_status` | Message composition status |

## Example: Dracula Theme

```yaml
name: "Dracula"
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
```

## Partial Themes

You can define partial themes that only override specific colors:

```yaml
name: "Dark Blue"
roles:
  accent: "#4dabf7"
  border_active: "#4dabf7"
  room_unread: "#74c0fc"
```

Missing fields will use the built-in defaults.

## Error Handling

- **Invalid colors**: Warnings logged, default color used for that field
- **Missing file**: Warning logged, full default theme used
- **Parse errors**: Warning logged, full default theme used

The application will always start successfully even with theme issues.