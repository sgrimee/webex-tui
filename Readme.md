[webex-tui](https://github.com/sgrimee/webex-tui) is an unofficial [Webex](https://www.webex.com/) chat client for the terminal.

## Features
- View a list of rooms that have received a message since launch (for now...)
- Select a room and send messages to the room

## Missing features

This is early work in progress. The following will be added:
- View a list of all of the user's rooms
- Handle message edits, both incoming and outgoing
- Handle conversations within a room
- Search for rooms/users
- Send messages to a new room/user

## Installing

Currently the best way to use it is by [installing Rust](https://www.rust-lang.org/tools/install) and then:
```shell
cargo install --git https://github.com/sgrimee/webex-tui
```

More options like pre-built binaries, homebrew and nix packages will be available once the tool becomes more stable.

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

