//! File-manager model: directory entries, a single pane (cwd + cursor + marks),
//! and the two-pane app state. Pure logic over the filesystem — no rendering and
//! no global state — so the navigation rules are unit-testable in isolation.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// One row in a pane.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    /// File size in bytes. `0` for directories.
    pub size: u64,
}

/// Which pane has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn other(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
}

/// A single browseable pane.
#[derive(Debug, Clone)]
pub struct Pane {
    pub cwd: PathBuf,
    pub entries: Vec<DirEntry>,
    /// Index into `entries` of the highlighted row.
    pub cursor: usize,
    /// Absolute paths the user has marked in this pane.
    pub marked: BTreeSet<PathBuf>,
}

impl Pane {
    /// Open `cwd` and read its entries. The cursor starts at the top.
    pub fn open(cwd: impl AsRef<Path>) -> Result<Pane> {
        let cwd = cwd.as_ref();
        let cwd = cwd
            .canonicalize()
            .with_context(|| format!("resolving {}", cwd.display()))?;
        let mut pane = Pane {
            cwd,
            entries: Vec::new(),
            cursor: 0,
            marked: BTreeSet::new(),
        };
        pane.reload()?;
        Ok(pane)
    }

    /// Re-read the current directory, preserving marks for paths that still
    /// exist and clamping the cursor into range.
    pub fn reload(&mut self) -> Result<()> {
        let mut entries = Vec::new();

        // A ".." row first, unless we're at a filesystem root.
        if self.cwd.parent().is_some() {
            entries.push(DirEntry {
                name: "..".to_string(),
                path: self.cwd.join(".."),
                is_dir: true,
                size: 0,
            });
        }

        let read = std::fs::read_dir(&self.cwd)
            .with_context(|| format!("reading dir {}", self.cwd.display()))?;
        for item in read {
            let item = match item {
                Ok(i) => i,
                Err(_) => continue, // skip entries we can't stat (perms, races)
            };
            let path = item.path();
            let meta = item.metadata().ok();
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let size = if is_dir {
                0
            } else {
                meta.as_ref().map(|m| m.len()).unwrap_or(0)
            };
            let name = item.file_name().to_string_lossy().into_owned();
            entries.push(DirEntry {
                name,
                path,
                is_dir,
                size,
            });
        }

        // Directories first, then files; each group alphabetical (case-insensitive).
        // The ".." row is already at index 0 and must stay there.
        let split = usize::from(entries.first().map(|e| e.name == "..").unwrap_or(false));
        entries[split..].sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        self.entries = entries;
        self.marked.retain(|p| p.exists());
        if self.cursor >= self.entries.len() {
            self.cursor = self.entries.len().saturating_sub(1);
        }
        Ok(())
    }

    pub fn current(&self) -> Option<&DirEntry> {
        self.entries.get(self.cursor)
    }

    pub fn move_cursor(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let len = self.entries.len() as isize;
        let next = (self.cursor as isize + delta).clamp(0, len - 1);
        self.cursor = next as usize;
    }

    pub fn cursor_top(&mut self) {
        self.cursor = 0;
    }

    pub fn cursor_bottom(&mut self) {
        self.cursor = self.entries.len().saturating_sub(1);
    }

    /// Descend into the directory (or `..`) under the cursor. No-op on a file.
    pub fn enter(&mut self) -> Result<()> {
        let Some(entry) = self.current() else {
            return Ok(());
        };
        if !entry.is_dir {
            return Ok(());
        }
        let target = entry.path.clone();
        // Canonicalize so ".." collapses cleanly.
        let target = target
            .canonicalize()
            .with_context(|| format!("resolving {}", target.display()))?;
        self.cwd = target;
        self.cursor = 0;
        self.reload()
    }

    /// Go up to the parent directory.
    pub fn ascend(&mut self) -> Result<()> {
        if let Some(parent) = self.cwd.parent().map(|p| p.to_path_buf()) {
            self.cwd = parent;
            self.cursor = 0;
            self.reload()?;
        }
        Ok(())
    }

    /// Toggle the mark on the entry under the cursor (never the ".." row).
    pub fn toggle_mark(&mut self) {
        if let Some(entry) = self.current() {
            if entry.name == ".." {
                return;
            }
            let p = entry.path.clone();
            if !self.marked.remove(&p) {
                self.marked.insert(p);
            }
        }
    }

    pub fn is_marked(&self, path: &Path) -> bool {
        self.marked.contains(path)
    }

    /// Paths to hand to the agent: the marked set if non-empty, else the single
    /// path under the cursor (excluding "..").
    pub fn effective_selection(&self) -> Vec<PathBuf> {
        if !self.marked.is_empty() {
            return self.marked.iter().cloned().collect();
        }
        match self.current() {
            Some(e) if e.name != ".." => vec![e.path.clone()],
            _ => Vec::new(),
        }
    }
}

/// The whole dual-pane app: two panes and which one is active.
#[derive(Debug, Clone)]
pub struct AppState {
    pub left: Pane,
    pub right: Pane,
    pub active: Side,
}

impl AppState {
    /// Both panes start at the same directory (classic MC behaviour).
    pub fn new(cwd: impl AsRef<Path>) -> Result<AppState> {
        let cwd = cwd.as_ref();
        Ok(AppState {
            left: Pane::open(cwd)?,
            right: Pane::open(cwd)?,
            active: Side::Left,
        })
    }

    pub fn active_pane(&self) -> &Pane {
        match self.active {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
    }

    pub fn active_pane_mut(&mut self) -> &mut Pane {
        match self.active {
            Side::Left => &mut self.left,
            Side::Right => &mut self.right,
        }
    }

    pub fn switch_pane(&mut self) {
        self.active = self.active.other();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_tree() -> (tempdir_lite::TempDir, PathBuf) {
        let dir = tempdir_lite::TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir(root.join("sub")).unwrap();
        std::fs::write(root.join("b.txt"), b"hi").unwrap();
        std::fs::write(root.join("a.txt"), b"yo").unwrap();
        (dir, root)
    }

    #[test]
    fn dirs_sort_before_files() {
        let (_g, root) = tmp_tree();
        let pane = Pane::open(&root).unwrap();
        // index 0 is "..", 1 is the "sub" dir, then files alphabetically.
        let names: Vec<_> = pane.entries.iter().map(|e| e.name.clone()).collect();
        assert_eq!(names, vec!["..", "sub", "a.txt", "b.txt"]);
    }

    #[test]
    fn cursor_clamps() {
        let (_g, root) = tmp_tree();
        let mut pane = Pane::open(&root).unwrap();
        pane.move_cursor(-10);
        assert_eq!(pane.cursor, 0);
        pane.move_cursor(100);
        assert_eq!(pane.cursor, pane.entries.len() - 1);
    }

    #[test]
    fn marking_and_effective_selection() {
        let (_g, root) = tmp_tree();
        let mut pane = Pane::open(&root).unwrap();
        // Move off ".." onto "sub", mark it.
        pane.move_cursor(1);
        pane.toggle_mark();
        assert_eq!(pane.effective_selection().len(), 1);
        pane.toggle_mark();
        // No marks -> cursor path used instead.
        assert_eq!(pane.effective_selection().len(), 1);
    }

    #[test]
    fn cannot_mark_dotdot() {
        let (_g, root) = tmp_tree();
        let mut pane = Pane::open(&root).unwrap();
        pane.toggle_mark(); // cursor on ".."
        assert!(pane.marked.is_empty());
    }
}

/// Tiny dependency-free temp-dir helper for tests only, so the core crate stays
/// dependency-light. Not part of the public API.
#[cfg(test)]
mod tempdir_lite {
    use std::path::{Path, PathBuf};

    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new() -> std::io::Result<TempDir> {
            let mut base = std::env::temp_dir();
            let unique = format!(
                "commander-test-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            );
            base.push(unique);
            std::fs::create_dir_all(&base)?;
            Ok(TempDir(base))
        }
        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
}
