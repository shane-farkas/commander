---
description: Open the Commander dual-pane file UI to visually pick files for the agent
argument-hint: "[directory]"
---

Open the Commander file UI so the user can visually browse and select files.

1. Call the `commander_open` MCP tool. If the user passed a directory in `$ARGUMENTS`, pass it as the `path` argument; otherwise omit `path` to use the current working directory. Briefly tell the user the picker has opened in a new window and that they should mark files with **Space**, then press **a** (send), **r** (review), or **e** (explain) — or **q** to cancel.
2. `commander_open` **blocks until the user confirms** and returns the selection directly. You do **not** need to call `commander_get_selection` in the normal flow.
3. When it returns a selection:
   - Read the returned `paths` into context.
   - If an `action` is present (`review`, `explain`, etc.), perform it on the selected files.
   - If there's no action, acknowledge the files are now in context and ask what they'd like to do.
4. If it returns that the user cancelled, acknowledge and stop.
5. Only if it reports a **timeout** (the user took longer than the wait window): the picker is still open — once the user says they've confirmed, call `commander_get_selection` to retrieve the picks.
