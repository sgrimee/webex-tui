{
  description = "Rust dev using fenix";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils = {
      url = "github:numtide/flake-utils";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nixpkgs,
    fenix,
    flake-utils,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [
            fenix.overlays.default
          ];
        };

        # get Rust version from toolchain file
        toolchain = with fenix.packages.${system};
          fromToolchainFile {
            file = ./rust-toolchain.toml;
            sha256 = "sha256-lMLAupxng4Fd9F1oDw8gx+qA0RuF7ou7xhNU8wgs0PU=";
          };
      in {
        devShell = pkgs.mkShell {
          # build environment
          nativeBuildInputs = with pkgs; [
            openssl.dev
            pkg-config
            toolchain
          ];

          # runtime environment
          buildInputs = with pkgs;
            [
              bacon
              cargo-udeps
              clippy
              git-cliff
              rust-analyzer
              toolchain
            ]
            ++ lib.optionals pkgs.stdenv.isDarwin [
              pkgs.darwin.apple_sdk.frameworks.CoreServices
              pkgs.darwin.apple_sdk.frameworks.Security
              pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
              pkgs.libiconv
            ];
        };
      }
    );
}
