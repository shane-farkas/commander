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
  doing something wide-reaching.

## How it works

1. Call the `commander_open` MCP tool (optionally with a `path` to root the panes
   at). It opens the picker in a **new terminal window** and **blocks until the
   user confirms or cancels**, then returns structured `{ cwd, paths, action }`
   directly.
2. The user browses: arrows to move, **Space** to mark files, **Enter** to
   descend, **Backspace** to go up. They confirm with **a** (send), **r**
   (review), or **e** (explain), or quit with **q**.
3. You normally do **not** need `commander_get_selection` — `commander_open`
   already returns the selection. Only use it as a fallback if `commander_open`
   reports a timeout and the picker is still open.

## After you get a selection

- `action: "review"` → review the selected files for bugs/quality.
- `action: "explain"` → explain what the selected files do.
- `action: null` → the files are just being added to context; ask what's next.

If the result says the user cancelled, acknowledge and stop.
