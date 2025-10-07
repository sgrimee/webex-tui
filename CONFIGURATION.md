# Configuration Guide

webex-tui uses a dual configuration system that separates authentication credentials from user preferences.

## Configuration Files

### 1. Client Configuration (`client.yml`)
**Location**: `~/.config/webex-tui/client.yml`  
**Purpose**: Authentication credentials and OAuth settings (managed by sops/secrets)  
**Contents**:
```yaml
client_id: "your-webex-integration-client-id"
client_secret: "your-webex-integration-client-secret"
port: 8080  # OAuth2 redirect port (optional, defaults to 8080)
```

### 2. User Configuration (`config.yml`)
**Location**: `~/.config/webex-tui/config.yml`  
**Purpose**: User preferences and UI settings  
**Contents**:
```yaml
# Theme to use (default: "default")
theme: "dracula"

# Messages to load per room (default: 10)
messages_to_load: 20

# Enable debug logging (default: false)
debug: true
```

## Configuration Methods

### Method 1: Manual Configuration

Create `~/.config/webex-tui/config.yml` manually:

```yaml
theme: "dracula"
messages_to_load: 15
debug: false
```

### Method 2: Environment Variable

Set `WEBEX_TUI_CONFIG` to point to a custom config file:

```bash
export WEBEX_TUI_CONFIG="/path/to/my/webex-config.yml"
webex-tui
```

### Method 3: Nix Configuration (Recommended)

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

1. **Command Line Arguments** (e.g., `--debug`, `--messages 25`)
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

### Integration with Secrets Management
Keep credentials separate while managing preferences declaratively:

```nix
# Authentication handled by sops-nix or similar
sops.secrets."webex/client-id" = {};
sops.secrets."webex/client-secret" = {};

# Preferences managed declaratively
programs.webex-tui = {
  enable = true;
  theme = "dracula";
};
```

## Migration from Legacy Config

If you have a `client.yml` with user preferences, they will continue to work with deprecation warnings. To migrate:

1. **Move user preferences** from `client.yml` to `config.yml`
2. **Keep only credentials** in `client.yml`
3. **Consider using nix** for declarative management

Example migration:

**Old `client.yml`** (if it had user preferences mixed in):
```yaml
client_id: "abc123..."
client_secret: "def456..."
port: 8080
theme: "dracula"
```

**New `client.yml`** (authentication and OAuth only):
```yaml
client_id: "abc123..."
client_secret: "def456..."
port: 8080  # OAuth redirect port (keep here as it's tied to integration setup)
```

**New `config.yml`** (user preferences only):
```yaml
theme: "dracula"
```

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

## Security Considerations

- **Client credentials**: Store securely using sops, age, or similar
- **Config files**: User preferences are safe to version control
- **File permissions**: Config files use standard user permissions (600)
- **Environment variables**: Be cautious with config paths in shared environments