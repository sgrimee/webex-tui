# webex-tui

[webex-tui](https://github.com/sgrimee/webex-tui) is an unofficial [Webex](https://www.webex.com/) chat client for the terminal.

It is work in progress but usable to some extent. Feedback is welcome.

## How it works

webex-tui is a client of the local [webex-bridge](https://github.com/sgrimee/webex-bridge)
daemon. The daemon owns the only Webex connection (login, real-time events, an
encrypted local mirror); webex-tui talks to it over a private Unix socket and
holds no Webex credentials. webex-tui never installs, starts or stops the
daemon. macOS only.

## Features

- View a list of rooms, with team names, unread marks and filters
- Select a room and send, reply to, edit and delete messages
- Live incoming messages, edits and deletes
- Nested conversations (threads), with older history and threads loaded on demand
- Browse the mirrored rooms while Webex is offline (writes are disabled)

## Getting started

1. Install and log in the daemon (once), from the webex-bridge checkout or release:

   ```shell
   webex-bridge install-service
   webex-bridge login
   ```

   `webex-bridge health` shows whether it runs and is connected.

2. Start webex-tui. On the first start the daemon shows a macOS dialog:
   "Allow <path to webex-tui> full access to your Webex mirror?" Click
   **Allow**. webex-tui stores its pairing secret in your login Keychain
   (`com.cisco.webex-tui.ui-pairing`); later starts reuse it without a dialog.

If the daemon is not running, webex-tui prints the exact `install-service`
command and exits. Denying the dialog (or not answering within 60 s) pairs
nothing; the next start asks again.

To unpair: `webex-bridge ui pairings`, then `webex-bridge ui unpair <path>`.
Quit with `q` or `Ctrl-C`; a write still being sent is finished (up to 35 s)
and a one-line, content-free summary of unfinished writes is printed.

## Upgrading from the standalone (OAuth) version

Older webex-tui versions connected to Webex themselves with their own
integration (`~/.config/webex-tui/client.yml`) and token cache. This version
ignores those files; it does not read, import or delete them. Quit any old
webex-tui first: it would register its own Webex device next to the daemon.

Optional cleanup, once you no longer need the old version:

```shell
rm ~/.config/webex-tui/client.yml               # old integration client ID/secret
rm ~/Library/Caches/webex-tui/tokens*.json      # old OAuth token cache
```

You may also delete the old webex-tui integration on
[developer.webex.com](https://developer.webex.com/my-apps). Do not delete the
webex-bridge integration or its Keychain items.

## Installing

### Homebrew

```shell
brew install sgrimee/tap/webex-tui
```

### From source

With [Rust installed](https://www.rust-lang.org/tools/install), you can install with:

```shell
cargo install --git https://github.com/sgrimee/webex-tui
```

For development, build a signed binary with `just build-signed` and run it
with `just run` (see [CONFIGURATION.md](CONFIGURATION.md#keychain-and-code-signing)).

### Pre-build binaries

Pre-build binaries are available on the [releases page](https://github.com/sgrimee/webex-tui/releases).

## Configuration

Themes, page size and debug logging: see [CONFIGURATION.md](CONFIGURATION.md)
and [THEMING.md](THEMING.md).

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
