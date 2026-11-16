# danta-cli

FDU Hole (树洞) terminal client — TUI, CLI, and telnet server all in one.

## Features

- **TUI** — full interactive terminal UI with image rendering, search, settings
- **CLI** — all operations available as commands, with `--json` for agent/programmatic use
- **Serve** — telnet server that exposes the TUI over the network
- **WebVPN** — UIS login for WebVPN access to forum authentication, posts, and images

## Install

```bash
cargo build --release
# binary at target/release/danta-cli
```

## Usage

### Login

```bash
danta login -e <email> -p <password>
```

### UIS / WebVPN

To access the forum through Fudan WebVPN, first log in with your UIS account:

```bash
danta webvpn login -u <UIS_ID>  # prompts for the UIS password without echoing it
danta webvpn status            # verifies the saved session and forum gateway
danta login -e <email> -p <password>  # only needed if not already logged into FDU Hole
danta                         # prefers direct access, falls back to saved WebVPN
```

UIS authenticates the WebVPN connection; your FDU Hole email/password still authenticates
your forum account. With a saved WebVPN session, the CLI and TUI probe direct access
briefly at startup. If the probe fails, they use WebVPN for forum authentication, content,
and supported images. A direct connection failure can switch later requests in the same
process to WebVPN. Requests are not replayed automatically after a connection failure.
Restart an already-running TUI after logging in or removing the WebVPN session.

Use `danta --connection webvpn status` to test the forum over WebVPN, or
`danta --connection direct status` to force direct access. `danta webvpn status`
always checks the saved WebVPN session independently of the routing preference.

The client saves WebVPN session cookies and keeps them updated when the gateway
rotates them. It does not save the UIS password. `webvpn.json` lives next to `token.json`
in `~/Library/Application Support/danta-cli/` on macOS and in
`$XDG_CONFIG_HOME/danta-cli/` or `~/.config/danta-cli/` on Linux. On Unix, the file is
readable and writable only by its owner. If WebVPN requires renewed authentication,
run `danta webvpn login -u <UIS_ID>` again.

For automation, `danta webvpn login -u <UIS_ID> --password-stdin` reads the password
from the first line of standard input. Supply it from your secret manager or another
private input source; there is no UIS password command-line argument.

```bash
danta webvpn logout  # removes the local WebVPN session and restores direct access
```

This removes this client's saved session only; it does not sign out other browser
sessions. CAPTCHA and additional verification cannot currently be completed by the CLI;
use the university WebVPN in a browser when these challenges are required. Browser
sign-in does not transfer its session into this CLI. Weak-password and maintenance
responses also stop login; resolve those conditions before retrying.

The authentication and routing protocol follows the Flutter
[DanXi main repository](https://github.com/DanXi-Dev/DanXi) at commit
`3c1f5f026fbc7ca50e151865f813cdb381807abe`:
[identity login](https://github.com/DanXi-Dev/DanXi/blob/3c1f5f026fbc7ca50e151865f813cdb381807abe/lib/repository/fdu/neo_login_tool.dart)
and [WebVPN routing](https://github.com/DanXi-Dev/DanXi/blob/3c1f5f026fbc7ca50e151865f813cdb381807abe/lib/util/webvpn_proxy.dart).
It supports the current `id.fudan.edu.cn` password and ticket flow, with a legacy
UIS form fallback. Interactive browser verification is not embedded in this CLI.

### TUI Mode (default)

```bash
danta          # or
danta tui
```

Keybindings:
- `j/k` — navigate up/down
- `Enter` — open hole
- `n` — new post
- `r` — reply (in hole detail)
- `/` — search
- `Tab` — switch division
- `m` — messages
- `,` — settings
- `q` — back / quit

### CLI Mode

```bash
danta holes                       # list holes
danta view 12345                  # view hole and floors
danta post "hello" -d 1 -t "tag" # create post
danta reply 12345 "content"      # reply to hole
danta search "keyword"           # search
danta me                         # user info
```

All commands support `--json` for machine-readable output:

```bash
danta --json holes -l 5
danta --json view 12345
danta --json me
```

Full command list: `danta --help`

### Telnet Server

Expose the TUI over TCP so other machines can connect:

```bash
danta serve                          # localhost:2323
danta serve -p 8080                  # custom port
danta serve -b 0.0.0.0 -p 2323      # listen on all interfaces
```

Connect from another machine:

```bash
stty raw -echo; nc <host> 2323; stty sane
# or
socat TCP:<host>:2323 STDIO,raw,echo=0
```

Each connection spawns an isolated TUI session via PTY.

## Config

Settings are stored at `~/.config/danta-cli/config.toml`. Editable in-app via `,` key in TUI.

Options: border style, sort order, floors per page, image protocol (Kitty/iTerm2/Sixel/auto), thumbnail mode, and more.

## License

MIT
