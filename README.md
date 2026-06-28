# Commander

A Midnight Commander style dual-pane file picker for terminal coding agents:
[Claude Code](https://code.claude.com), [OpenAI Codex](https://developers.openai.com/codex),
and [Grok Build](https://x.ai/cli). You browse and mark files in a real terminal
TUI, and your selection (plus an optional action like *review* or *explain*)
flows straight back into the agent's context.

It's a small Rust binary that exposes an MCP server plus a `SKILL.md`, the two
primitives these agents share, so the **same binary** works across all three
hosts. Only the one-time registration differs.

```
mark files with Space  →  press a / r / e  →  paths land in the agent's context
```

> Status: early. The select-to-context loop works and is pleasant to use daily.
> File operations and richer features are still ahead; see [Roadmap](#roadmap).

## Why a separate window?

An agent CLI owns its terminal (raw mode and alternate screen), and any tool it
spawns gets *piped* stdout, not a real TTY, so a full-screen TUI can't render
inline. `commander_open` therefore launches the picker in a **new terminal
window** (`wt.exe` on Windows, your `$TERMINAL` on Unix). The selection returns
over MCP rather than through that window, so the round trip doesn't depend on
where the UI is drawn.

## Install

Build the binary once, then register it with whichever agent you use. Every
host's instructions are right here; you don't need to open any other file.

### Step 1: Build the binary (all hosts)

```sh
git clone https://github.com/shane-farkas/commander
cd commander
cargo install --path .
```

This puts `commander` on your `PATH` via `~/.cargo/bin`.

**Prerequisites:** [Rust](https://rustup.rs) (stable). On Windows you also need
the MSVC linker, from Visual Studio Build Tools with the *C++ build tools*
workload.

### Step 2: Register with your host

#### Claude Code

```
/plugin marketplace add ./commander
/plugin install commander@commander
```

(Or via the CLI: `claude plugin marketplace add ./commander` then
`claude plugin install commander@commander`.)

Restart Claude Code, then verify with `/mcp`. You should see **commander** with
two tools. Invoke with `/commander:open [directory]`, or just ask in natural
language.

#### OpenAI Codex

Register the MCP server:

```sh
codex mcp add commander -- commander mcp
```

Or merge this into `~/.codex/config.toml` (global) or `.codex/config.toml`
(per-project). `tool_timeout_sec` is generous on purpose, because
`commander_open` blocks while you pick files:

```toml
[mcp_servers.commander]
command = "commander"
args = ["mcp"]
startup_timeout_sec = 20
tool_timeout_sec = 600
```

Install the skill:

```sh
mkdir -p ~/.codex/skills/commander
cp plugins/codex/skills/commander/SKILL.md ~/.codex/skills/commander/
```

Start Codex, confirm the server's tools appear, then ask to "pick some files."

#### Grok Build (xAI)

Register the MCP server:

```sh
grok mcp add commander commander mcp   # NAME, then the command + args
grok mcp list                          # confirm it registered
```

(If your build complains about argument parsing, use the explicit separator:
`grok mcp add commander -- commander mcp`.)

Install the skill (Grok recognizes the Anthropic `SKILL.md` format):

```sh
mkdir -p ~/.grok/skills/commander
cp plugins/grok/skills/commander/SKILL.md ~/.grok/skills/commander/
```

Run `grok inspect` to confirm the skill and MCP server loaded, then ask to "pick
some files."

> Grok Build is early beta and its config layout is still moving, so the `grok
> mcp` commands above (which abstract the on-disk path) are the most durable
> route. Grok Build can also read an existing Claude Code `.mcp.json` directly.

## Usage

Ask in natural language ("let me pick some files") and the skill triggers on its
own, or, in Claude Code, invoke `/commander:open [directory]` explicitly.

`commander_open` **blocks until you confirm or cancel** and returns the selection
directly, so there's no second step in the normal flow.

### Keymap

| Key | Action |
|-----|--------|
| ←/h, →/l, Tab | switch active pane |
| ↑/k, ↓/j | move cursor |
| Enter | descend into directory / `..` |
| Backspace | go up a directory |
| Space | mark / unmark file or dir |
| g / G | jump to top / bottom |
| **a** | send selection (no action) |
| **r** | send + action `review` |
| **e** | send + action `explain` |
| q / Esc | cancel |

If nothing is marked, the item under the cursor is sent.

## How it works

```
        ┌─────────────────────────────────────────────┐
        │  commander  (single Rust binary, ratatui)    │
        │                                              │
        │   commander tui   ←── IPC ──→  commander mcp │
        │   (dual-pane UI)   (session file) (stdio MCP)│
        └─────────────────────────────────────────────┘
              ▲                              ▲
              │ human keys                   │ agent tool calls
        new terminal window        Claude Code · Codex · Grok Build
```

- `commander tui [DIR]`: the interactive dual-pane picker.
- `commander mcp`: a stdio MCP server (hand-rolled JSON-RPC, no SDK dependency)
  that the host launches.
- The two share a per-user **session file** (`selection.json` under your local
  app-data dir). `commander_open` clears it, spawns the TUI, and blocks polling
  for the confirmed selection; the TUI writes it (or a "cancelled" marker) on
  exit.

### Workspace layout

```
commander/
├─ src/main.rs        # arg dispatch: `commander tui` | `commander mcp`
├─ crates/
│  ├─ core/           # fs model, panes, cursor/marks (unit-tested)
│  ├─ ipc/            # session-file protocol shared by tui + mcp
│  ├─ tui/            # ratatui dual-pane UI + keymap
│  └─ mcp/            # hand-rolled stdio JSON-RPC MCP server
└─ plugins/
   ├─ claude-code/    # plugin.json + .mcp.json + /commander:open + SKILL.md
   ├─ codex/          # config.toml (mcp_servers) + SKILL.md
   └─ grok/           # SKILL.md (registered via `grok mcp add`)
```

## MCP tools

- `commander_open(path?)`: open the picker and block until the user confirms,
  returning `{ cwd, paths, action }`.
- `commander_get_selection(clear?)`: fallback for reading the last selection
  (only needed if `commander_open` times out).

## Configuration

- `COMMANDER_OPEN_TIMEOUT`: seconds `commander_open` waits for confirmation
  (default `300`).

## Roadmap

- File operations (copy / move / delete / view / mkdir).
- Agent-driven navigation (the agent moves the panes via a live socket
  transport; the `NavCommand` types are already defined).
- Inline picker via `tmux split-window` when run inside tmux, instead of a
  separate window.
- More hosts via the same MCP and skill core.

These are not built yet; contributions and ideas welcome.

## Development

```sh
cargo build                          # debug build (all binaries)
cargo test                           # unit tests (core, ipc)
cargo run -- tui .                   # run the picker standalone (plugin binary)
cargo run -p commander-cockpit -- .  # run the standalone cockpit workbench
```

After changing the binary, re-run `cargo install --path .`. Note: a running MCP
server locks `commander.exe` on Windows, so stop it (or restart your host)
before reinstalling. After changing Claude Code plugin files, bump the version
in `plugins/claude-code/.claude-plugin/plugin.json` and reinstall the plugin so
the cached copy refreshes.

## License

[MIT](LICENSE)
