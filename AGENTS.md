# AGENTS.md

Guidance for coding agents working with this Rust TUI Webex client.

## Build/Test Commands
- `cargo check` - Fast compile check (preferred for development)
- `cargo clippy` - Run linter  
- `cargo test` - Run all tests with rstest framework
- `cargo test test_name` - Run specific test function
- `bacon` or `bacon check` - Continuous checking (preferred)
- `bacon clippy` - Continuous linting
- `bacon test` - Continuous testing

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

## Architecture
Multi-threaded async TUI using ratatui with crossterm backend, webex API client, and tokio runtime.