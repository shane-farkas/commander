# Commander for Grok Build (xAI)

The same Commander dual-pane file picker, registered with [Grok Build](https://x.ai/cli),
xAI's terminal coding agent. Grok Build is MCP-native and recognizes the
Anthropic skill format, so the host-agnostic `commander` binary drops straight in.

> Grok Build is early beta and its docs/config layout are still moving. The
> `grok mcp` CLI commands below abstract over the on-disk config, so they're the
> most durable way to wire this up. Verify what loaded with `grok inspect`.

## Prerequisites

- The `commander` binary on your `PATH`. From the repo root:
  ```sh
  cargo install --path .
  ```
- Grok Build CLI (`curl -fsSL https://x.ai/cli/install.sh | bash`, then
  `grok login`).

## 1. Register the MCP server

```sh
grok mcp add commander -t stdio -c commander -a mcp
```

Confirm it registered:

```sh
grok mcp list
grok mcp test commander    # optional connectivity check
```

> Because `commander_open` blocks while the user picks files, make sure Grok's
> tool timeout is generous (the server returns on its own after
> `COMMANDER_OPEN_TIMEOUT`, default 300s). If a long pick is cut off, the
> `commander_get_selection` tool is the fallback.

Already have a Claude Code `.mcp.json`? Grok Build can read it directly, so you
can point it at the one in `plugins/claude-code/.mcp.json` instead of re-adding.

## 2. Install the skill

Copy the skill where Grok auto-loads skills (Anthropic SKILL.md format):

```sh
mkdir -p ~/.grok/skills/commander
cp skills/commander/SKILL.md ~/.grok/skills/commander/
```

(A project-local skills folder works too.) Verify with `grok inspect`, which
lists the skills, plugins, hooks, and MCP servers Grok loaded.

## 3. Use it

Start Grok Build, then ask to "pick some files," or let the skill trigger on its
own. The picker opens in a new terminal window (`$TERMINAL` on Unix); mark files
with **Space**, confirm with **a** / **r** / **e**, and the selection flows back
into Grok.

## Tools

- `commander_open(path?)` — open the picker, block until confirmed, return
  `{ cwd, paths, action }`.
- `commander_get_selection(clear?)` — fallback read if `commander_open` times out.
