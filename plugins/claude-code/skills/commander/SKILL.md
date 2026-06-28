---
name: commander
description: >
  Use when the user wants to visually browse the filesystem or pick files/folders
  to work on, rather than naming paths by hand. Opens a Midnight Commander-style
  dual-pane file UI in a new terminal window; the user marks files and the
  selection flows back to you. Triggers: "let me pick the files", "open a file
  browser", "show me a file picker", "I'll choose what to work on".
---

# Commander — visual file picker for the agent

Commander is a dual-pane terminal file manager (Midnight Commander style) that
the user drives, handing their selection back to you through MCP.

## When to use it

- The user wants to choose files visually instead of typing paths.
- The user has a vague target ("the config files", "those test fixtures") and
  would rather point at them.
- You want the user to confirm exactly which files an action applies to before
  you run something destructive or wide-reaching.

## How it works

1. Call `commander_open` (optionally with a `path` to root the panes at). This
   spawns the UI in a **new terminal window** — the current chat terminal is not
   taken over — and **blocks until the user confirms or cancels**, then returns
   structured `{ cwd, paths, action }` directly.
2. The user browses: arrows to move, **Space** to mark files, **Enter** to
   descend, **Backspace** to go up. They confirm with **a** (send), **r**
   (review), or **e** (explain), or quit with **q**.
3. You normally do **not** need `commander_get_selection` — `commander_open`
   already returns the selection. Only use `commander_get_selection` as a
   fallback if `commander_open` reports a timeout (the user took too long) and
   the picker is still open.

## After you get a selection

- `action: "review"` → review the selected files for bugs/quality.
- `action: "explain"` → explain what the selected files do.
- `action: null` → the files are just being added to context; ask what's next.

If `commander_get_selection` returns empty, the user hasn't confirmed yet — wait
or ask them to.

## Notes

- The terminal handoff (new window) is deliberate: agent CLI hosts own the
  current TTY, so an interactive TUI must run in its own window. Selections come
  back over MCP, not the terminal, so this works regardless of host.
