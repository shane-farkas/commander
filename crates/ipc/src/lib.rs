//! Shared state channel between the `commander tui` process (the human-facing
//! dual-pane UI) and the `commander mcp` process (the agent-facing MCP server).
//!
//! Milestone 1 uses a plain JSON file in a per-user session directory. The TUI
//! writes a [`Selection`] when the human confirms; the MCP server reads it back
//! when the agent calls `commander_get_selection`. This is intentionally the
//! simplest cross-platform transport that proves the round trip.
//!
//! Live, bidirectional agent-driven navigation (the agent pushing nav commands
//! into a *running* TUI) will need a real socket. The message types for that
//! already live here ([`NavCommand`]) so the protocol is defined in one place;
//! only the transport changes.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// How a TUI session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// The human confirmed a selection.
    #[default]
    Sent,
    /// The human quit without sending.
    Cancelled,
}

/// What the human picked in the TUI and handed back to the agent. Written on
/// *every* TUI exit (a cancel writes `status: cancelled` with no paths) so a
/// waiting reader always unblocks.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Selection {
    /// How the session ended. Defaults to `Sent` for backward compatibility
    /// with files written before this field existed.
    #[serde(default)]
    pub status: Status,
    /// Working directory the panes were rooted at when confirmed.
    pub cwd: PathBuf,
    /// Absolute paths the human marked (or the cursor path if none marked).
    pub paths: Vec<PathBuf>,
    /// Optional action the human chose, e.g. "review", "refactor", "explain".
    /// `None` means a plain "select -> add to context".
    pub action: Option<String>,
    /// Unix-millis timestamp the selection was confirmed, best-effort.
    pub submitted_at_ms: u128,
}

/// Navigation/action commands the agent can push to a running TUI. Not wired to
/// a live transport yet — defined here so the protocol has a single home.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NavCommand {
    /// Change the active pane's directory.
    Cd { path: PathBuf },
    /// Move the cursor to a named entry in the active pane.
    Focus { name: String },
    /// Toggle the mark on the entry under the cursor.
    ToggleMark,
    /// Switch which pane is active.
    SwitchPane,
}

/// Resolve the per-user session directory, creating it if needed.
///
/// Windows: `%LOCALAPPDATA%\commander`. Unix: `$XDG_STATE_HOME/commander` or
/// `~/.local/state/commander`. Falls back to the system temp dir.
pub fn session_dir() -> Result<PathBuf> {
    let base = local_state_base();
    let dir = base.join("commander");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating session dir {}", dir.display()))?;
    Ok(dir)
}

fn local_state_base() -> PathBuf {
    if let Ok(v) = std::env::var("LOCALAPPDATA") {
        if !v.is_empty() {
            return PathBuf::from(v);
        }
    }
    if let Ok(v) = std::env::var("XDG_STATE_HOME") {
        if !v.is_empty() {
            return PathBuf::from(v);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home).join(".local").join("state");
        }
    }
    std::env::temp_dir()
}

fn selection_path() -> Result<PathBuf> {
    Ok(session_dir()?.join("selection.json"))
}

/// Current unix-millis, best-effort (0 if the clock is before the epoch).
pub fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Persist a selection for the agent to pick up. Writes atomically via a temp
/// file + rename so a reader never sees a half-written file.
pub fn write_selection(sel: &Selection) -> Result<()> {
    let path = selection_path()?;
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_vec_pretty(sel)?;
    std::fs::write(&tmp, json).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("finalizing {}", path.display()))?;
    Ok(())
}

/// Read the pending selection, if any. Returns `Ok(None)` when nothing is
/// pending (no file yet).
pub fn read_selection() -> Result<Option<Selection>> {
    let path = selection_path()?;
    match std::fs::read(&path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Block until a selection newer than `since_ms` is written, or until `timeout`
/// elapses. Polls the session file a few times a second. Returns `Ok(None)` on
/// timeout. Intended for `commander_open` to wait on the human's confirmation.
pub fn wait_for_outcome(
    since_ms: u128,
    timeout: std::time::Duration,
) -> Result<Option<Selection>> {
    let start = std::time::Instant::now();
    let poll = std::time::Duration::from_millis(150);
    loop {
        if let Some(sel) = read_selection()? {
            if sel.submitted_at_ms >= since_ms {
                return Ok(Some(sel));
            }
        }
        if start.elapsed() >= timeout {
            return Ok(None);
        }
        std::thread::sleep(poll);
    }
}

/// Clear any pending selection. No-op if there is none.
pub fn clear_selection() -> Result<()> {
    let path = selection_path()?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

/// True if `path` is inside `root` (or equal to it). Used to keep agent-driven
/// navigation from escaping a sandbox root later.
pub fn is_within(root: &Path, path: &Path) -> bool {
    match (root.canonicalize(), path.canonicalize()) {
        (Ok(r), Ok(p)) => p.starts_with(r),
        _ => false,
    }
}
