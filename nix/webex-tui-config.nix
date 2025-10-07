# nix/webex-tui-config.nix
#
# Nix module for webex-tui user configuration
# This can be imported into NixOS or home-manager configurations

{ lib, pkgs, config, ... }:

with lib;

let
  cfg = config.programs.webex-tui;
  
  # Convert nix config to YAML format
  configFile = pkgs.writeText "webex-tui-config.yml" (generators.toYAML {} {
    inherit (cfg) port theme messages_to_load debug;
  });
  
in {
  options.programs.webex-tui = {
    enable = mkEnableOption "webex-tui configuration";

    port = mkOption {
      type = types.int;
      default = 8080;
      description = "OAuth2 redirect port for authentication";
    };

    theme = mkOption {
      type = types.str;
      default = "default";
      description = "Theme name to load from themes directory";
    };

    messages_to_load = mkOption {
      type = types.int;
      default = 10;
      description = "Number of messages to load per room";
    };

    debug = mkOption {
      type = types.bool;
      default = false;
      description = "Enable debug logging by default";
    };

    themes = mkOption {
      type = types.attrsOf types.str;
      default = {};
      description = "Custom theme definitions as YAML content";
      example = {
        "my-theme" = ''
          name: "My Theme"
          roles:
            accent: "#bd93f9"
            border_active: "#bd93f9"
        '';
      };
    };
  };

  config = mkIf cfg.enable {
    # Set environment variable to point to nix-generated config
    home.sessionVariables = {
      WEBEX_TUI_CONFIG = "${configFile}";
    };

    # Create theme files from nix configuration
    home.file = 
      let
        themeFiles = mapAttrs' (name: content: 
          nameValuePair ".config/webex-tui/themes/${name}.yml" {
            text = content;
          }
        ) cfg.themes;
      in themeFiles;

    # Ensure webex-tui package is available
    # home.packages = [ pkgs.webex-tui ];  # Enable when package is in nixpkgs
  };
}