# danta-cli

FDU Hole (树洞) terminal client — TUI, CLI, and telnet server all in one.

## Features

- **TUI** — full interactive terminal UI with image rendering, search, settings
- **CLI** — all operations available as commands, with `--json` for agent/programmatic use
- **Serve** — telnet server that exposes the TUI over the network

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
