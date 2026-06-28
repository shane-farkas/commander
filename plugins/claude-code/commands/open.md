---
description: Open the Commander dual-pane file UI to visually pick files for the agent
argument-hint: "[directory]"
---

Open the Commander file UI so the user can visually browse and select files.

1. Call the `commander_open` MCP tool. If the user passed a directory in `$ARGUMENTS`, pass it as the `path` argument; otherwise omit `path` to use the current working directory.
2. Tell the user the UI has opened in a new window and to: navigate with arrows, mark files with **Space**, then press **a** (send), **r** (review), or **e** (explain) to confirm — or **q** to cancel.
3. Wait for the user to say they've confirmed (or to ask you to check). Then call `commander_get_selection`.
4. If a selection comes back:
   - Read the returned paths into context.
   - If an `action` is present (`review`, `explain`, etc.), perform it on the selected files.
   - If there's no action, just acknowledge the files are now in context and ask what they'd like to do.
5. If the selection is empty, let the user know nothing was confirmed yet.
