# Commander

A Midnight Commander–style dual-pane file UI that plugs into agentic CLIs
(Claude Code, Codex, Hermes, OpenClaw). You browse and pick files in a real
terminal TUI; your selection — and an optional action like *review* or
*explain* — flows back into the agent's context.

## Why it's one plugin for four hosts

All four hosts converge on two extension primitives:

- **MCP** (Model Context Protocol) — the agent-facing control channel. Works
  everywhere.
- **Skills** (`SKILL.md`) — the human-facing launcher + "when to use" guidance.
  Works everywhere, with minor frontmatter dialect differences.

So Commander ships as **one MCP server + one skill**, registered per host. No
per-host UI code.

## Architecture

```
        ┌─────────────────────────────────────────────┐
        │  commander  (single Rust binary, ratatui)    │
        │                                              │
        │   commander tui   ←── IPC ──→  commander mcp │
        │   (dual-pane UI)   (session file) (stdio MCP)│
        └─────────────────────────────────────────────┘
              ▲                              ▲
              │ human keys                   │ agent tool calls
        new terminal window           Claude Code / Codex /
        (wt.exe / VSCode term)        Hermes / OpenClaw
```

- `commander tui [DIR]` — the interactive dual-pane manager.
- `commander mcp` — the stdio MCP server the host launches.
- The two share a per-user **session file** (`selection.json`). The TUI writes
  the user's confirmed selection; the MCP server reads it back when the agent
  calls `commander_get_selection`.

### The terminal-handoff constraint

An agent CLI owns the current terminal (raw mode / alt screen), and tools it
spawns get *piped* stdout, not a real TTY — so a TUI can't render inline. The
`commander_open` tool therefore launches the UI in a **new terminal window**;
selections return over MCP, not the terminal, so it works in every host.

## Workspace layout

```
commander/
├─ src/main.rs        # arg dispatch: `commander tui` | `commander mcp`
├─ crates/
│  ├─ core/           # fs model, panes, cursor/marks (unit-tested)
│  ├─ ipc/            # session-file protocol shared by tui + mcp
│  ├─ tui/            # ratatui dual-pane UI + keymap
│  └─ mcp/            # hand-rolled stdio JSON-RPC MCP server
└─ plugins/
   ├─ claude-code/    # plugin.json + /commander:open command + SKILL.md
   ├─ codex/          # (todo) config.toml snippet + skill
   ├─ hermes/         # (todo) plugin manifest
   └─ openclaw/       # (todo) SKILL.md for ClawHub
```

## Build & run

```sh
cargo build --release
# try the UI standalone:
./target/release/commander tui .
# run the MCP server (a host normally launches this):
./target/release/commander mcp
```

Put `commander` on your `PATH` (or edit the plugin manifest to use an absolute
path) so the host can launch it.

## TUI keymap

| Key | Action |
|-----|--------|
| ←/h, →/l, Tab | switch active pane |
| ↑/k, ↓/j | move cursor |
| Enter | descend into dir / `..` |
| Backspace | go up a directory |
| Space | mark/unmark file or dir |
| g / G | jump to top / bottom |
| **a** | send selection (no action) |
| **r** | send + action `review` |
| **e** | send + action `explain` |
| q / Esc | cancel |

## MCP tools

- `commander_open(path?)` — open the UI in a new window.
- `commander_get_selection(clear?)` — return `{ cwd, paths, action }` the user
  confirmed; clears the pending selection by default.

## Status

Milestone 1 (this scaffold): dual-pane browse + mark, select→context round trip,
Claude Code plugin. Next: agent-driven navigation (live socket + `NavCommand`),
file operations (copy/move/delete/view), and the Codex/Hermes/OpenClaw manifests.

## License

MIT
