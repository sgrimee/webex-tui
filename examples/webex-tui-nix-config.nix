# examples/webex-tui-nix-config.nix
#
# Example configuration for webex-tui using the nix module
# This can be imported into your home-manager configuration

{ pkgs, ... }:

{
  # Import the webex-tui module
  imports = [ ../nix/webex-tui-config.nix ];

  # Configure webex-tui
  programs.webex-tui = {
    enable = true;
    
    # Basic settings
    port = 8080;
    theme = "dracula";
    messages_to_load = 20;
    debug = false;
    
    # Define custom themes directly in nix
    themes = {
      # Dracula theme
      "dracula" = ''
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
      '';
      
      # Solarized Dark theme
      "solarized-dark" = ''
        name: "Solarized Dark"
        roles:
          background: "#002b36"
          text_primary: "#839496"
          accent: "#268bd2"
          border_active: "#268bd2"
          room_unread: "#2aa198"
          selection_bg: "#073642"
          selection_fg: "#eee8d5"
        user_colors:
          - "#dc322f"
          - "#859900"
          - "#b58900"
          - "#268bd2"
          - "#d33682"
          - "#2aa198"
      '';
      
      # Custom minimal theme
      "minimal" = ''
        name: "Minimal"
        roles:
          accent: "#4dabf7"
          border_active: "#4dabf7"
          room_unread: "#74c0fc"
      '';
    };
  };
}