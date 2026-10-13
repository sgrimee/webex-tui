# Configuration Guide

webex-tui has one configuration file, for user preferences. It holds no Webex
credentials: the [webex-bridge](https://github.com/sgrimee/webex-bridge) daemon
owns the Webex login and connection (see the [Readme](Readme.md#getting-started)).

## User Configuration (`config.yml`)

**Location**: `~/.config/webex-tui/config.yml`
**Purpose**: User preferences and UI settings
**Auto-creation**: If this file doesn't exist on first run, it will be automatically created with default values, and all bundled theme files will be copied to `~/.config/webex-tui/themes/`
**Contents**:
```yaml
# Theme to use (default: "default")
theme: "default"

# Messages per room page, 1-100 (default: 10). Values outside the range are
# clamped with a warning. --messages overrides it.
messages_to_load: 10

# Enable debug logging (default: false)
debug: false
```

A `client.yml` from older, standalone versions is ignored (see
[Upgrading](Readme.md#upgrading-from-the-standalone-oauth-version)).

## Configuration Methods

### Method 1: Automatic Configuration (First Run)

On first run, if `~/.config/webex-tui/config.yml` doesn't exist, webex-tui will:
1. Create `config.yml` with default settings
2. Copy all bundled theme files to `~/.config/webex-tui/themes/`
3. Set the default theme to "default" (a clean, functional theme)

You can then edit `~/.config/webex-tui/config.yml` to customize your preferences:

```yaml
theme: "dracula"  # Switch to Dracula theme
messages_to_load: 15
debug: false
```

Available bundled themes:
- `default` - Clean, functional theme with standard terminal colors
- `dracula` - Popular dark theme with vibrant colors

### Method 2: Manual Configuration

Create `~/.config/webex-tui/config.yml` manually:

```yaml
theme: "dracula"
messages_to_load: 15
debug: false
```

### Method 3: Environment Variable

Set `WEBEX_TUI_CONFIG` to point to a custom config file:

```bash
export WEBEX_TUI_CONFIG="/path/to/my/webex-config.yml"
webex-tui
```

### Method 4: Nix Configuration (Recommended)

For NixOS or home-manager users, use the provided nix module:

```nix
# In your home-manager configuration
{
  imports = [ ./path/to/webex-tui/nix/webex-tui-config.nix ];
  
  programs.webex-tui = {
    enable = true;
    theme = "dracula";
    messages_to_load = 20;
    debug = false;
    
    # Define themes directly in nix
    themes = {
      "my-theme" = ''
        name: "My Custom Theme"
        roles:
          accent: "#bd93f9"
          border_active: "#bd93f9"
          room_unread: "#8be9fd"
      '';
    };
  };
}
```

## Priority Order

Configuration values are resolved in this order (highest to lowest priority):

1. **Command Line Arguments** (e.g., `--debug`, `--messages 25`; `--messages` accepts 1-100)
2. **Environment Variable Config** (`WEBEX_TUI_CONFIG`)  
3. **Standard User Config** (`~/.config/webex-tui/config.yml`)
4. **Built-in Defaults**

## Nix Integration Features

The nix module provides several advantages:

### Declarative Configuration
Define your entire webex-tui setup in your nix configuration:

```nix
programs.webex-tui = {
  enable = true;
  theme = "dracula";
  messages_to_load = 25;
  
  themes.dracula = ''
    name: "Dracula"
    roles:
      accent: "#bd93f9"
      # ... theme definition
  '';
};
```

### Automatic Theme Management
Themes defined in nix are automatically written to the correct location.

### Reproducible Environments
Your webex-tui configuration is version-controlled and reproducible across systems.

## Troubleshooting

### Config Not Loading
- Check file permissions on config files
- Verify YAML syntax with `yamllint`
- Check logs for configuration errors

### Environment Variable Issues
```bash
# Check if variable is set
echo $WEBEX_TUI_CONFIG

# Test config file loading
webex-tui --debug 2>&1 | grep -i config
```

### Nix Module Issues
```bash
# Check generated config
cat $(nix-build --no-out-link -A config.home.sessionVariables.WEBEX_TUI_CONFIG)

# Verify theme files
ls ~/.config/webex-tui/themes/
```

## Keychain and Code Signing

webex-tui keeps one secret, its pairing secret for the daemon, in the login
Keychain (service `com.cisco.webex-tui.ui-pairing`, account `ui-secret-v1`).
macOS lets the binary that created the item read it without asking. Run a
binary signed with a persistent identity (`just build-signed`, `just run`),
never `cargo run`, whose ad-hoc signature changes on every build. Without a
signing certificate that has a Team ID, each rebuilt binary still asks once
("webex-tui wants to use your confidential information…"): choose
**Always Allow**.

If the item is damaged, webex-tui says so and never replaces it. Remove it and
start again to pair once more:

```bash
security delete-generic-password -s com.cisco.webex-tui.ui-pairing -a ui-secret-v1
```

## Security Considerations

- **No Webex credentials**: webex-tui stores no Webex token or integration secret; the daemon does.
- **Pairing secret**: only in the Keychain item above; never logged or printed.
- **Logs**: the bridge layer logs metadata only (phases, renewals, counts), never message bodies or secrets.
- **Config files**: User preferences are safe to version control
- **Environment variables**: Be cautious with config paths in shared environments
