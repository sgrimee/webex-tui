# webex-tui

[webex-tui](https://github.com/sgrimee/webex-tui) is an unofficial [Webex](https://www.webex.com/) chat client for the terminal.

It is work in progress but usable to some extent. Feedback is welcome.

## Features

- View a list of rooms
  - from all user rooms (not all rooms are shown at this time)
  - that have received a message since launch
  - that have been updated since some Duration
- Select a room and send messages to the room

## Missing features

This is early work in progress. The following will be added:

- Handle message edits, both incoming and outgoing
- Handle nested conversations within a room
- Search for rooms/users
- Send messages to a new room/user

See also the [TODO list](TODO.md).

## Installing

## Homebrew

```shell
brew install sgrimee/webex-tui/webex-tui
```

## From source

With [Rust installed](https://www.rust-lang.org/tools/install), you can install with:

```shell
cargo install --git https://github.com/sgrimee/webex-tui
```

## Pre-build binaries

Pre-build binaries are available on the [releases page](https://github.com/sgrimee/webex-tui/releases).

## Inspirations

- This [tutorial on TUI](https://blog.logrocket.com/rust-and-tui-building-a-command-line-interface-in-rust/)
- This [article on concurrency with TUI](https://www.monkeypatch.io/blog/2021-05-31-rust-tui)
- The [webex-rust](https://github.com/shutton/webex-rust) crate and its authors
- Initial work for [webexterm](https://github.com/Nabushika/webexterm)
- [spotify-tui](https://github.com/sgrimee/webex-tui/tree/main)

## DISCLAIMER

This crate is not maintained by Cisco, and not an official SDK. The authors are current developers at Cisco, but have no direct affiliation with the Webex development team.

## License

webex-tui is provided under the MIT license. See [LICENSE](LICENSE).
