# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build and Development Commands

### Standard Cargo Commands
- `cargo build` - Build the project
- `cargo run` - Run the application  
- `cargo test` - Run tests with rstest framework
- `cargo check` - Fast compile check
- `cargo clippy` - Run linter
- `cargo fmt` - Format the code
- `cargo doc --no-deps` - Generate documentation

Always run cargo check, test, fmt and clipy before committing.

### Bacon Integration (preferred for continuous development)
Uses `bacon.toml` configuration:
- `bacon` or `bacon check` - Continuous cargo check
- `bacon clippy` - Continuous linting
- `bacon test` - Continuous testing
- `bacon doc` - Generate docs
- `bacon doc-open` - Generate and open docs in browser

### Development Environment
- Rust toolchain: 1.84 (specified in `rust-toolchain.toml`)
- Uses Nix flakes for environment management (`flake.nix`, `flake.lock`)

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
- **webex**: Webex API client (version 0.10.0)
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
