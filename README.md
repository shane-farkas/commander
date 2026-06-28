# Commander

A Midnight Commander–style dual-pane file picker for [Claude Code](https://code.claude.com).
You browse and mark files in a real terminal TUI; your selection — and an
optional action like *review* or *explain* — flows straight back into Claude's
context.

It's a small Rust binary that ships as a Claude Code plugin (an MCP server + a
skill + a slash command). The core is deliberately host-agnostic — MCP and
`SKILL.md` are the two primitives most agent CLIs share — so other hosts can be
added later, but **today it targets Claude Code.**

> Status: early. The select→context loop works and is pleasant to use daily.
> File operations and other hosts are not built yet — see [Roadmap](#roadmap).

## Demo

```
/commander:open
```

Opens the picker in a new terminal window. Mark files with **Space**, then press
**a** (send), **r** (review), or **e** (explain). The picks land back in Claude's
context and it acts on them.

## Why a separate window?

An agent CLI owns its terminal (raw mode / alternate screen), and any tool it
spawns gets *piped* stdout, not a real TTY — so a full-screen TUI can't render
inline. `commander_open` therefore launches the picker in a **new terminal
window** (`wt.exe` on Windows, your `$TERMINAL` on Unix). The selection returns
over MCP rather than through that window, so the round trip doesn't depend on
where the UI is drawn.

## Install

**Prerequisites**

- [Rust](https://rustup.rs) (stable). On Windows you also need the MSVC linker
  (Visual Studio Build Tools with the *C++ build tools* workload).
- Claude Code.

**1. Build and install the binary** (puts `commander` on your `PATH` via
`~/.cargo/bin`):

```sh
git clone https://github.com/<you>/commander
cd commander
cargo install --path .
```

**2. Add this repo as a plugin marketplace and install the plugin:**

```
/plugin marketplace add <path-to-clone>
/plugin install commander@commander
```

(Or via the CLI: `claude plugin marketplace add <path>` then
`claude plugin install commander@commander`.)

**3. Restart Claude Code**, then verify the MCP server connected:

```
/mcp
```

You should see **commander** with two tools. Now try `/commander:open`.

## Usage

Invoke it explicitly with `/commander:open [directory]`, or just ask in natural
language ("let me pick some files") — the skill triggers on its own.

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
        new terminal window               Claude Code
```

- `commander tui [DIR]` — the interactive dual-pane picker.
- `commander mcp` — a stdio MCP server (hand-rolled JSON-RPC, no SDK dependency)
  that Claude Code launches.
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
   └─ claude-code/    # plugin.json + .mcp.json + /commander:open + SKILL.md
```

## MCP tools

- `commander_open(path?)` — open the picker and block until the user confirms;
  returns `{ cwd, paths, action }`.
- `commander_get_selection(clear?)` — fallback to read the last selection (only
  needed if `commander_open` times out).

## Configuration

- `COMMANDER_OPEN_TIMEOUT` — seconds `commander_open` waits for confirmation
  (default `300`).

## Roadmap

- File operations (copy / move / delete / view / mkdir).
- Agent-driven navigation (the agent moves the panes; live socket transport —
  the `NavCommand` types are already defined).
- Additional hosts (Codex, Hermes, OpenClaw) via the same MCP + skill core.

These are not built yet; contributions and ideas welcome.

## Development

```sh
cargo build         # debug build
cargo test          # unit tests (core, ipc)
cargo run -- tui .  # run the picker standalone
```

After changing the binary, re-run `cargo install --path .`. Note: the running
MCP server locks `commander.exe` on Windows — stop it (or restart Claude Code)
before reinstalling. After changing plugin files, bump the version in
`plugins/claude-code/.claude-plugin/plugin.json` and reinstall the plugin so the
cached copy refreshes.

## License

[MIT](LICENSE)
