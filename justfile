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

# Run the application with timestamped log file
run *args:
    mkdir -p logs
    cargo run -- --log logs/$(date +%Y%m%d-%H:%M:%S).log {{args}}

# Run with debug logging
run-debug *args:
    mkdir -p logs
    cargo run -- --log logs/$(date +%Y%m%d-%H:%M:%S).log --debug {{args}}

# Clear cached authentication token (forces re-authentication)
clear-token:
    rm -f ~/Library/Caches/webex-tui/tokens.json
    @echo "Token cache cleared. Next run will require re-authentication."

# Clean build artifacts
clean:
    cargo clean

# Remove logfiles
clean-logs:
    rm logs/*

# Format code
fmt:
    cargo fmt

# Run clippy linter
lint:
    cargo clippy

# Run with debug logging to a specific log file
debug-log file="logs/debug.log" *args:
    mkdir -p logs
    cargo run -- --debug --log {{file}} {{args}}

# Clear token and run with debug logging (useful for testing OAuth changes)
test-auth file="logs/auth-test.log" *args:
    just clear-token
    mkdir -p logs
    cargo run -- --debug --log {{file}} {{args}}
