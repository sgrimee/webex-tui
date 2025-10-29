# Justfile for webex-tui development

# Build the project
build:
    cargo build

# Build with release optimizations
build-release:
    cargo build --release

# Run tests
test:
    cargo test

# Run the application
run *args:
    cargo run -- {{args}}

# Run with debug logging
run-debug *args:
    cargo run -- --debug {{args}}

# Clear cached authentication token (forces re-authentication)
clear-token:
    rm -f ~/Library/Caches/webex-tui/tokens.json
    @echo "Token cache cleared. Next run will require re-authentication."

# Clean build artifacts
clean:
    cargo clean

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