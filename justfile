# Justfile for webex-tui development

# List all available targets
default:
    @just --list

# Run comprehensive quality checks (check, clippy, test, fmt)
check-all:
    just check
    just lint
    just test
    just fmt
    just check-graph

# Fail if the dependency graph pulls in Webex/OAuth/WebSocket/SQLite/TLS crates
check-graph:
    ./scripts/check-client-graph

# Compile check
check:
    cargo check

# Build the project
build:
    cargo build

# Build with release optimizations
build-release:
    cargo build --release

# Run tests
test:
    cargo test

# webex-tui talks only to the webex-bridge daemon (install it with
# `webex-bridge install-service`). Its pairing secret lives in a Keychain
# item, so it is always run as a binary signed with a persistent identity
# (com.sgrimee.webex-tui): `cargo run` relinks with an ad-hoc signature,
# which the Keychain treats as a new application. Needs a Code Signing
# certificate; set WEBEX_TUI_CODESIGN_IDENTITY to pick one. Without a Team
# ID, each rebuild still asks once for Keychain access (Always Allow).

# Build and sign with the persistent identity
build-signed:
    ./scripts/sign-tui-dev

# Build, sign and run with a timestamped log file
run *args: build-signed
    mkdir -p logs
    "${CARGO_TARGET_DIR:-target}/debug/webex-tui" --log logs/$(date +%Y%m%d-%H:%M:%S).log {{args}}

# Same as run, with debug logging
run-debug *args: build-signed
    mkdir -p logs
    "${CARGO_TARGET_DIR:-target}/debug/webex-tui" --log logs/$(date +%Y%m%d-%H:%M:%S).log --debug {{args}}

# Clean build artifacts
clean:
    cargo clean

# Remove logfiles
clean-logs:
    rm logs/*

# Format code
fmt:
    cargo fmt

# Run clippy linter (warnings are errors, as in the pre-commit hook)
lint:
    cargo clippy --all-targets -- -D warnings

# Run with debug logging to a specific log file
debug-log file="logs/debug.log" *args: build-signed
    mkdir -p logs
    "${CARGO_TARGET_DIR:-target}/debug/webex-tui" --debug --log {{file}} {{args}}
