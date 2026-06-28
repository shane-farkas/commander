//! A minimal, dependency-light MCP server over stdio.
//!
//! Rather than pull in a fast-moving MCP SDK, we speak the protocol directly:
//! MCP's stdio transport is newline-delimited JSON-RPC 2.0. We implement just
//! the handful of methods a host needs — `initialize`, `tools/list`,
//! `tools/call` — plus the two Commander tools. Swapping in `rmcp` later is a
//! drop-in change behind this same crate boundary.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::Command;

use serde_json::{json, Value};

const PROTOCOL_VERSION: &str = "2024-11-05";

/// Run the stdio server loop until stdin closes. Blocks the calling thread.
pub fn serve() -> anyhow::Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue, // ignore malformed frames
        };

        // Notifications have no "id" and need no reply.
        let Some(id) = msg.get("id").cloned() else {
            continue;
        };
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        let response = match dispatch(method, &params) {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(err) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32000, "message": err.to_string() }
            }),
        };

        writeln!(out, "{}", serde_json::to_string(&response)?)?;
        out.flush()?;
    }
    Ok(())
}

fn dispatch(method: &str, params: &Value) -> anyhow::Result<Value> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "commander", "version": env!("CARGO_PKG_VERSION") }
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_defs() })),
        "tools/call" => call_tool(params),
        other => anyhow::bail!("unknown method: {other}"),
    }
}

fn tool_defs() -> Value {
    json!([
        {
            "name": "commander_open",
            "description": "Open the Commander dual-pane file UI in a new terminal window so the \
                user can browse and pick files/dirs. Call this when the user wants to choose \
                files visually. After they confirm in the UI, call commander_get_selection to \
                retrieve what they picked.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory to open the panes at. Defaults to the current working directory."
                    }
                }
            }
        },
        {
            "name": "commander_get_selection",
            "description": "Return the files/dirs the user confirmed in the Commander UI, plus any \
                action they chose (e.g. 'review', 'explain'). Returns empty if nothing is pending. \
                By default this clears the pending selection once read.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clear": {
                        "type": "boolean",
                        "description": "Clear the pending selection after reading. Default true."
                    }
                }
            }
        }
    ])
}

fn call_tool(params: &Value) -> anyhow::Result<Value> {
    let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);
    match name {
        "commander_open" => tool_open(&args),
        "commander_get_selection" => tool_get_selection(&args),
        other => anyhow::bail!("unknown tool: {other}"),
    }
}

fn tool_open(args: &Value) -> anyhow::Result<Value> {
    let start_dir = args
        .get("path")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);

    spawn_tui(&start_dir)?;
    Ok(text_result(&format!(
        "Opened Commander at {}. Ask the user to browse, mark files with Space, then press \
         'a' (send), 'r' (review), or 'e' (explain). Once they confirm, call \
         commander_get_selection.",
        start_dir.display()
    )))
}

fn tool_get_selection(args: &Value) -> anyhow::Result<Value> {
    let clear = args.get("clear").and_then(Value::as_bool).unwrap_or(true);
    match commander_ipc::read_selection()? {
        None => Ok(text_result(
            "No pending selection. The user may not have confirmed in the Commander UI yet.",
        )),
        Some(sel) => {
            if clear {
                commander_ipc::clear_selection()?;
            }
            let paths: Vec<String> = sel.paths.iter().map(|p| p.display().to_string()).collect();
            let summary = format!(
                "User selected {} item(s){} from {}:\n{}",
                paths.len(),
                sel.action
                    .as_ref()
                    .map(|a| format!(" with action '{a}'"))
                    .unwrap_or_default(),
                sel.cwd.display(),
                paths.join("\n")
            );
            // Return both a human-readable text block and a structured payload.
            Ok(json!({
                "content": [{ "type": "text", "text": summary }],
                "structuredContent": {
                    "cwd": sel.cwd,
                    "paths": sel.paths,
                    "action": sel.action
                }
            }))
        }
    }
}

fn text_result(text: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }] })
}

/// Launch `commander tui <dir>` in a fresh terminal window. The host owns the
/// current TTY, so an interactive pane must live in its own window. Best-effort
/// across platforms; errors bubble up so the agent can report them.
fn spawn_tui(start_dir: &std::path::Path) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let dir = start_dir.to_string_lossy().to_string();

    #[cfg(windows)]
    {
        // Prefer Windows Terminal (tabs); fall back to a classic console window.
        if Command::new("wt.exe")
            .args(["new-tab", &exe.to_string_lossy(), "tui", &dir])
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
        Command::new("cmd")
            .args(["/c", "start", "", &exe.to_string_lossy(), "tui", &dir])
            .spawn()?;
        return Ok(());
    }

    #[cfg(not(windows))]
    {
        // Try a few common terminal emulators; -e runs the command.
        let term = std::env::var("TERMINAL").unwrap_or_default();
        let candidates = [
            term.as_str(),
            "x-terminal-emulator",
            "kitty",
            "alacritty",
            "wezterm",
            "gnome-terminal",
            "xterm",
        ];
        for t in candidates.iter().filter(|t| !t.is_empty()) {
            if Command::new(t)
                .args(["-e", &exe.to_string_lossy(), "tui", &dir])
                .spawn()
                .is_ok()
            {
                return Ok(());
            }
        }
        anyhow::bail!("could not find a terminal emulator to open Commander; set $TERMINAL");
    }
}
