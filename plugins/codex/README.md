# Commander for OpenAI Codex

The same Commander dual-pane file picker, as a Codex MCP server + skill. The
`commander` binary is host-agnostic — only the registration differs from Claude
Code.

## Prerequisites

- The `commander` binary on your `PATH`. From the repo root:
  ```sh
  cargo install --path .
  ```
- Codex CLI.

## 1. Register the MCP server

Either run:

```sh
codex mcp add commander -- commander mcp
```

…or merge [`config.toml`](./config.toml) into `~/.codex/config.toml` (global) or
`.codex/config.toml` (per-project):

```toml
[mcp_servers.commander]
command = "commander"
args = ["mcp"]
startup_timeout_sec = 20
tool_timeout_sec = 600
```

> `tool_timeout_sec` is generous on purpose: `commander_open` blocks while you
> pick files. The server returns on its own after `COMMANDER_OPEN_TIMEOUT`
> (default 300s).

## 2. Install the skill

Copy the skill into your Codex skills directory so Codex knows when to reach for
the picker:

```sh
mkdir -p ~/.codex/skills/commander
cp skills/commander/SKILL.md ~/.codex/skills/commander/
```

(Project-scoped also works: `$REPO_ROOT/.agents/skills/commander/SKILL.md`.)

## 3. Use it

Start Codex and confirm the server loaded (its tools should appear). Then ask to
"pick some files," or let the skill trigger on its own. The picker opens in a new
terminal window (`$TERMINAL` on Unix); mark files with **Space**, confirm with
**a** / **r** / **e**, and the selection flows back into Codex.

## Tools

- `commander_open(path?)` — open the picker, block until confirmed, return
  `{ cwd, paths, action }`.
- `commander_get_selection(clear?)` — fallback read if `commander_open` times out.
