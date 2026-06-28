//! A lightweight read-only git status for the tree: which files are modified,
//! staged, or untracked (the agent's "blast radius"), plus which directories
//! contain changes. Shells out to `git status --porcelain` and caches the
//! result; refreshed on an interval by the caller.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Modified,
    Staged,
    Untracked,
}

#[derive(Default)]
pub struct GitStatus {
    files: HashMap<PathBuf, Status>,
    dirty_dirs: HashSet<PathBuf>,
}

impl GitStatus {
    /// Load status for the repo containing `cwd`. Returns an empty (non-repo)
    /// status if `cwd` isn't in a git repo or git isn't available.
    pub fn load(cwd: &Path) -> GitStatus {
        let Some(root) = git_root(cwd) else {
            return GitStatus::default();
        };
        let mut gs = GitStatus::default();

        let output = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(["status", "--porcelain"])
            .output();
        let Ok(output) = output else { return gs };

        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            // Format: "XY <path>", X = index state, Y = worktree state.
            if line.len() < 4 {
                continue;
            }
            let bytes = line.as_bytes();
            let (x, y) = (bytes[0] as char, bytes[1] as char);
            let mut path_part = &line[3..];
            // Renames look like "orig -> new"; key on the new name.
            if let Some(idx) = path_part.find(" -> ") {
                path_part = &path_part[idx + 4..];
            }
            let path_part = path_part.trim().trim_matches('"');
            let abs = root.join(path_part);

            let kind = if x == '?' && y == '?' {
                Status::Untracked
            } else if x != ' ' && x != '?' {
                Status::Staged
            } else {
                Status::Modified
            };

            // Flag ancestor directories up to the repo root so they show a hint.
            let mut ancestor = abs.parent();
            while let Some(p) = ancestor {
                gs.dirty_dirs.insert(p.to_path_buf());
                if p == root {
                    break;
                }
                ancestor = p.parent();
            }
            gs.files.insert(abs, kind);
        }
        gs
    }

    pub fn file_status(&self, path: &Path) -> Option<Status> {
        self.files.get(path).copied()
    }

    pub fn dir_has_changes(&self, path: &Path) -> bool {
        self.dirty_dirs.contains(path)
    }
}

fn git_root(cwd: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if s.is_empty() {
        return None;
    }
    // Canonicalize so the root matches the tree's canonical paths; rely on
    // Path's component comparison (which treats / and \ alike on Windows).
    let raw = PathBuf::from(s);
    Some(
        raw.canonicalize()
            .map(commander_core::simplify)
            .unwrap_or(raw),
    )
}
