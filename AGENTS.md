# AGENTS.md

Guidance for coding agents working with this Rust TUI Webex client.

## Build and Development Commands

### Standard Cargo Commands
- `cargo build` - Build the project
- `cargo run` - Run the application  
- `cargo test` - Run tests with rstest framework
- `cargo check` - Fast compile check (preferred for development)
- `cargo clippy` - Run linter
- `cargo fmt` - Format the code
- `cargo doc --no-deps` - Generate documentation

Always run cargo check, test, fmt and clippy before committing.

### Development Environment
- Rust toolchain: 1.88 (specified in `rust-toolchain.toml`)
- Uses Nix flakes for environment management (`flake.nix`, `flake.lock`)

## Code Style Guidelines
- **Modules**: Use `mod.rs` pattern, declare submodules with `pub(crate) mod`
- **Imports**: Group std/external crates, then local crate imports with `use crate::`
- **Types**: Use explicit types for public APIs, prefer `pub(crate)` over `pub`
- **Naming**: snake_case for functions/variables, PascalCase for types/enums
- **Error handling**: Use `color_eyre::Result<T>` for fallible functions
- **Documentation**: Use `//!` for module docs, `///` for item docs
- **Async**: Multi-threaded tokio design with message passing via channels
- **Testing**: Use `rstest` framework with `#[cfg(test)]` mod tests blocks
- **Threading**: Separate concerns: App (UI), Teams (API), Input (events)
- **Cache pattern**: In-memory state management with typed IDs (RoomId, MessageId)

## Architecture Overview

This is a terminal-based Webex Teams client built with Rust, using the ratatui library for the TUI and tokio for async runtime.

### Core Architecture Pattern
Multi-threaded async design with message passing between components:

```
Main Thread (App) ←→ Teams Thread ←→ Webex Event Stream Thread
       ↑                   ↓
   Input Handler      Webex API Client
```

### Key Components

#### Main Application (`main.rs`)
- Entry point handling CLI arguments and initialization
- Manages authentication flow via OAuth2
- Spawns and coordinates all threads
- High/low priority message queues between App and Teams threads

#### App Controller (`src/app/`)
- **`App`**: Main UI state controller, handles user input and UI logic
- **`AppState`**: Central state management (rooms, messages, UI state)
- **Cache system** (`cache/`): In-memory storage for rooms, messages, persons
- **Actions**: Key binding to action mapping system
- **Message Editor**: Text composition and editing functionality

#### Teams Thread (`src/teams/`)
- **`Teams`**: Handles all Webex API interactions
- **`app_handler`**: Processes commands from App thread 
- **`webex_handler`**: Processes events from Webex stream
- **`auth`**: OAuth2 authentication flow
- **`client`**: Webex API client wrapper

#### Terminal UI (`src/tui.rs` & `src/ui/`)
- **`Tui`**: Terminal setup, event handling, drawing coordination
- **UI Modules**: Specialized rendering for rooms, messages, logs, help
- Uses crossterm for terminal control and ratatui for widgets

#### Input System (`src/inputs/`)
- **`EventHandler`**: Async input event processing
- **Key mapping**: Translates terminal events to application actions

### Threading Model
- **Main/App Thread**: UI rendering and user input processing
- **Teams Thread**: All Webex API calls and business logic  
- **Event Stream Thread**: Dedicated Webex real-time event listening
- **Input Thread**: Terminal input event capture

Communication via tokio channels with priority queues (high/low priority from App to Teams).

### Key Dependencies
- **ratatui**: TUI framework with crossterm backend
- **webex**: Webex API client (git dependency from sgrimee/webex-rust)
- **tokio**: Async runtime with unbounded channels
- **oauth2**: Authentication flow
- **tui-textarea**: Rich text editing widget
- **tui-logger**: Integrated logging display

### Configuration
- Client credentials stored in `$HOME/.config/webex-tui/client.yml`
- OAuth2 integration setup required for Webex API access
- Debug logging controlled via CLI flags and modules

### Testing
- Uses `rstest` framework for unit tests
- Integration tests should consume real external resources (following user's global CLAUDE.md)

## Commit Guidelines
- Never include 'Generated with' or 'Co authored by' OpenCode in commit messages or pull requests
