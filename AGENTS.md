# AGENTS.md

Guidance for coding agents working with this Rust TUI Webex client.

## Build and Development Commands

### Standard Cargo Commands
- `cargo build` - Build the project
- `cargo test` - Run tests with rstest framework
- `cargo check` - Fast compile check (preferred for development)
- `cargo clippy --all-targets -- -D warnings` - Run linter (as the pre-commit hook does)
- `cargo fmt` - Format the code
- `cargo doc --no-deps` - Generate documentation
- `just check-graph` - Fail if the dependency graph pulls in Webex/OAuth/WebSocket/SQLite/TLS crates
- `UPDATE_GOLDEN=1 cargo test ui::render_tests` - Regenerate the golden full-frame renders in `src/ui/golden/` after an intended UI change; review the diff before committing

Always run cargo check, test, fmt and clippy before committing.

### Running
- Never `cargo run` against the real daemon: its ad-hoc signature makes every
  build a new Keychain application. Use `just build-signed` / `just run`
  (signed as `com.sgrimee.webex-tui`, see `scripts/sign-tui-dev`).
- A live run touches the user's Keychain item, the daemon pairing dialog and
  real Webex data: only with the user's explicit consent. Tests are fake-only
  (no socket, daemon, Keychain or Webex).

### Development Environment
- Rust toolchain: 1.88 (specified in `rust-toolchain.toml`)
- Uses Nix flakes for environment management (`flake.nix`, `flake.lock`)
- `webex-bridge` is a git dependency pinned by `rev` on
  `github.com/sgrimee/webex-bridge` (client feature only). To bump it, change
  the `rev` and run `cargo update -p webex-bridge`. See the workspace AGENTS.md
  for the push order.
- To develop against an unpushed sibling `../webex-bridge`, override the git
  dependency locally without committing the override:
  `cargo --config 'patch."https://github.com/sgrimee/webex-bridge".webex-bridge.path="../webex-bridge"' check`
  (or the same `[patch."https://github.com/sgrimee/webex-bridge"]` entry in an
  uncommitted `.cargo/config.toml`). The override rewrites `Cargo.lock`: never
  commit a lock or `Cargo.toml` that points at the path, and run the gates
  without the override before pushing.

## Code Style Guidelines
- **Modules**: Use `mod.rs` pattern, declare submodules with `pub(crate) mod`
- **Imports**: Group std/external crates, then local crate imports with `use crate::`
- **Types**: Use explicit types for public APIs, prefer `pub(crate)` over `pub`
- **Naming**: snake_case for functions/variables, PascalCase for types/enums
- **Error handling**: Use `color_eyre::Result<T>` for fallible functions
- **Documentation**: Use `//!` for module docs, `///` for item docs
- **Async**: tokio; the bridge runtime runs on its own task, the App drains it every tick
- **Testing**: Use `rstest` framework with `#[cfg(test)]` mod tests blocks
- **Separation**: App/UI (`app`, `ui`), bridge boundary (`bridge`), pure frontend models (`frontend`)
- **Cache pattern**: In-memory state management with typed IDs (RoomId, MessageId)

## Architecture Overview

A terminal Webex client built with ratatui and tokio. It is a client of the
local webex-bridge daemon, which owns the Webex connection; webex-tui talks to
it over a private Unix socket (`webex_bridge::client`) and holds no Webex
credentials.

```
main ─ bridge::startup (before raw mode: socket, status, session-first pairing)
  │
  ├─ tokio task: bridge::runtime::Runtime  ──UDS──►  webex-bridged ──► Webex
  │     session lifecycle, SSE events, renewal, reads/writes (CommandWorker)
  │        ▲ mailbox (reads, one write slot)   │ presentation pump (events)
  │        │                                   ▼
  └─ render loop: App ── app::bridge_glue (drained every tick) ── app::cache
                 ▲
             inputs (terminal events thread)
```

### Key Components

- **`main.rs` / `host.rs`**: CLI, pre-terminal startup, spawning the runtime,
  `RuntimeExit` handling, the bounded quit join and the quit summary.
- **`src/bridge/`**: the only code that imports bridge DTOs. `startup`
  (socket probe, status, actionable errors), `pairing` (Keychain secret,
  session-first pairing), `session`/`controller`/`commands`/`runtime`
  (lease, renewal, reconnect, write slot), `read_dispatch`/`write_dispatch`/
  `projection` (DTO → frontend values), `presentation` (event pump).
- **`src/frontend/`**: pure, backend-neutral models: intents/results,
  `RoomMessageView` (canonical per-room view, revisions, tombstones,
  placeholders), editor write ledger, lifecycle projection.
- **`src/app/`**: `App` (input → actions), `AppState`, `cache` (rooms, teams,
  persons, one `RoomMessageView` per opened room), `bridge_glue` (reduces
  runtime events into the cache, issues reads/writes), `bridge_app` (routing
  and the status line).
- **`src/tui.rs`, `src/ui/`**: terminal setup and rendering (crossterm + ratatui).
- **`src/inputs/`**: terminal event capture and key mapping.

### Key Dependencies
- **webex-bridge** (client feature): typed UDS client and DTOs
- **keyring** (apple-native): the pairing secret in the login Keychain
- **ratatui**, **crossterm**, **tui-textarea**, **tui-logger**
- **tokio**: async runtime (multi-threaded)

### Configuration
- User preferences in `~/.config/webex-tui/config.yml` (or `WEBEX_TUI_CONFIG`); see CONFIGURATION.md
- Pairing secret: Keychain service `com.cisco.webex-tui.ui-pairing`, account `ui-secret-v1`
- Debug logging controlled via CLI flags and modules; never log message bodies or secrets

### Testing
- Uses `rstest` framework for unit tests
- Bridge, pairing, runtime and glue tests use fakes: no socket, daemon, Keychain or Webex
- Live validation against the real daemon is a separate, consented pass (see the workspace docs)

## Commit Guidelines
- Never include 'Generated with' or 'Co authored by' OpenCode in commit messages or pull requests
