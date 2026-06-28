//! `cockpit` is the standalone Commander workbench: tabbed workspaces, each with
//! a file tree on the left and a docked coding agent (or shell) on the right,
//! running in a real PTY. Each tab is rooted at its own folder, so different
//! tabs can drive different projects/agents. Unlike the plugin, the cockpit is
//! the top-level app, so it owns the screen and can host live agents itself.
//!
//! Usage:
//!   cockpit [DIR]                 file tree + a shell in the dock
//!   cockpit --agent claude [DIR]  auto-start `claude` in the dock
//!
//! Keys (reserved from the agent):
//!   Ctrl-O          toggle focus (tree <-> dock)
//!   Ctrl-Q          quit the cockpit
//!   Ctrl-T          new tab (rooted at the current tree folder)
//!   Ctrl-W          close the current tab
//!   Ctrl-PageUp/Dn  previous / next tab
//!   Alt-PageUp/Dn   scroll the dock (shell-style output; full-screen agents
//!                   scroll themselves)
//!   tree focused:   ↑/k ↓/j move, Space mark, Enter open dir / send file(s),
//!                   Backspace up, q quit
//!   dock focused:   every other key goes to the agent

mod dock;
mod git;

use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::Result;
use commander_core::Pane;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs};

use dock::Dock;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Tree,
    Dock,
}

/// One tab: a file tree + a docked agent, rooted at a folder.
struct Workspace {
    tree: Pane,
    dock: Dock,
    /// Directory the agent launched in; `@` mentions are relative to it.
    agent_cwd: PathBuf,
    focus: Focus,
    /// Short folder name shown on the tab.
    label: String,
    /// Cached git status of the repo, refreshed on an interval.
    git: git::GitStatus,
    last_git: Instant,
}

impl Workspace {
    fn new(start: &Path, launch: &Launch) -> Result<Workspace> {
        let tree = Pane::open(start)?;
        let agent_cwd = tree.cwd.clone();
        let dock = Dock::spawn(&launch.exec, &launch.args, &launch.label, &agent_cwd, 24, 80)?;
        let label = agent_cwd
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| agent_cwd.to_string_lossy().into_owned());
        let git = git::GitStatus::load(&agent_cwd);
        Ok(Workspace {
            tree,
            dock,
            agent_cwd,
            focus: Focus::Dock,
            label,
            git,
            last_git: Instant::now(),
        })
    }

    /// Reload git status if the cache is stale (cheap interval check; the reload
    /// itself shells out, so we keep the interval coarse).
    fn refresh_git_if_stale(&mut self) {
        if self.last_git.elapsed() >= Duration::from_millis(2000) {
            self.git = git::GitStatus::load(&self.agent_cwd);
            self.last_git = Instant::now();
        }
    }
}

struct App {
    workspaces: Vec<Workspace>,
    active: usize,
    /// Template used to spawn agents in new tabs.
    launch: Launch,
}

impl App {
    fn ws_mut(&mut self) -> &mut Workspace {
        &mut self.workspaces[self.active]
    }
    /// Open a new tab rooted at the active tab's current folder.
    fn new_tab(&mut self) {
        let start = self.workspaces[self.active].tree.cwd.clone();
        if let Ok(w) = Workspace::new(&start, &self.launch) {
            self.workspaces.push(w);
            self.active = self.workspaces.len() - 1;
        }
    }
    /// Close the active tab. Returns `true` if that was the last one (quit).
    fn close_tab(&mut self) -> bool {
        self.workspaces[self.active].dock.kill();
        self.workspaces.remove(self.active);
        if self.workspaces.is_empty() {
            return true;
        }
        if self.active >= self.workspaces.len() {
            self.active = self.workspaces.len() - 1;
        }
        false
    }
    fn next_tab(&mut self) {
        let n = self.workspaces.len();
        if n > 0 {
            self.active = (self.active + 1) % n;
        }
    }
    fn prev_tab(&mut self) {
        let n = self.workspaces.len();
        if n > 0 {
            self.active = (self.active + n - 1) % n;
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cockpit: error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let mut agent: Option<String> = None;
    let mut dir: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--agent" => agent = args.next(),
            _ if dir.is_none() => dir = Some(a),
            _ => {}
        }
    }
    let start = match dir {
        Some(d) => PathBuf::from(d),
        None => std::env::current_dir()?,
    };
    let launch = resolve_launch(agent);

    let mut terminal = setup_terminal()?;
    let result = run_loop(&mut terminal, &start, launch);
    restore_terminal(&mut terminal)?;
    result
}

/// What the dock actually runs: an executable, its args, and a display label.
struct Launch {
    exec: String,
    args: Vec<String>,
    label: String,
}

/// Resolve `--agent <cmd>` (or the default shell) into something spawnable.
///
/// On Windows, an agent like `claude` is usually an npm `.cmd` shim or a script,
/// which `CreateProcessW` can't launch directly, so route it through
/// `cmd.exe /c <agent>` (cmd resolves the shim via PATHEXT). The default shell
/// is a real binary and spawns directly.
fn resolve_launch(agent: Option<String>) -> Launch {
    match agent {
        Some(cmd) => {
            if cfg!(windows) {
                Launch {
                    exec: "cmd.exe".to_string(),
                    args: vec!["/c".to_string(), cmd.clone()],
                    label: cmd,
                }
            } else {
                Launch {
                    exec: cmd.clone(),
                    args: Vec::new(),
                    label: cmd,
                }
            }
        }
        None => {
            let shell = default_shell();
            let label = Path::new(&shell)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| shell.clone());
            Launch {
                exec: shell,
                args: Vec::new(),
                label,
            }
        }
    }
}

fn default_shell() -> String {
    if cfg!(windows) {
        "powershell.exe".to_string()
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
    }
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    start: &Path,
    launch: Launch,
) -> Result<()> {
    let first = Workspace::new(start, &launch)?;
    let mut app = App {
        workspaces: vec![first],
        active: 0,
        launch,
    };

    let result = loop {
        app.ws_mut().refresh_git_if_stale();
        terminal.draw(|f| draw(f, &mut app))?;

        if !event::poll(Duration::from_millis(50))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if handle_key(key, &mut app) {
                    break Ok(());
                }
            }
            _ => {}
        }
    };

    for w in &mut app.workspaces {
        w.dock.kill();
    }
    result
}

/// Returns `true` when the cockpit should quit.
fn handle_key(key: KeyEvent, app: &mut App) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // Global cockpit chords, reserved from the agent.
    if ctrl {
        match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Char('o') => {
                let f = &mut app.ws_mut().focus;
                *f = match *f {
                    Focus::Tree => Focus::Dock,
                    Focus::Dock => Focus::Tree,
                };
                return false;
            }
            KeyCode::Char('t') => {
                app.new_tab();
                return false;
            }
            KeyCode::Char('w') => return app.close_tab(),
            KeyCode::PageUp => {
                app.prev_tab();
                return false;
            }
            KeyCode::PageDown => {
                app.next_tab();
                return false;
            }
            _ => {}
        }
    }

    let ws = app.ws_mut();
    match ws.focus {
        Focus::Dock => {
            // Alt (not Shift): Windows Terminal reserves Shift+PageUp/Down for
            // its own scrollback, so those never reach us.
            let alt = key.modifiers.contains(KeyModifiers::ALT);
            match key.code {
                KeyCode::PageUp if alt => ws.dock.scroll_page_up(),
                KeyCode::PageDown if alt => ws.dock.scroll_page_down(),
                _ => {
                    ws.dock.scroll_reset();
                    if let Some(bytes) = key_to_bytes(key) {
                        ws.dock.send(&bytes);
                    }
                }
            }
        }
        Focus::Tree => match key.code {
            KeyCode::Up | KeyCode::Char('k') => ws.tree.move_cursor(-1),
            KeyCode::Down | KeyCode::Char('j') => ws.tree.move_cursor(1),
            KeyCode::Char(' ') => ws.tree.toggle_mark(),
            KeyCode::Enter => {
                let on_dir = ws.tree.current().map(|e| e.is_dir).unwrap_or(false);
                if ws.tree.marked.is_empty() && on_dir {
                    let _ = ws.tree.enter();
                } else {
                    send_files(ws);
                }
            }
            KeyCode::Backspace => {
                let _ = ws.tree.ascend();
            }
            KeyCode::Char('q') | KeyCode::Esc => return true,
            _ => {}
        },
    }
    false
}

/// Send the workspace's selection into its dock as `@path` mentions, then focus
/// the dock. Clears the marks.
fn send_files(ws: &mut Workspace) {
    let paths = ws.tree.effective_selection();
    if paths.is_empty() {
        return;
    }
    let text = mention_string(&paths, &ws.agent_cwd);
    ws.dock.send(text.as_bytes());
    ws.tree.marked.clear();
    ws.focus = Focus::Dock;
}

/// Build a space-separated list of `@path` mentions, relative to `agent_cwd`
/// when the path is under it (absolute otherwise), with forward slashes so the
/// agent parses them cleanly. Paths containing a space are quoted. Trailing
/// space and no newline: the user types their instruction next.
fn mention_string(paths: &[PathBuf], agent_cwd: &Path) -> String {
    let mut out = String::new();
    for p in paths {
        let rel = p.strip_prefix(agent_cwd).unwrap_or(p);
        let display = rel.to_string_lossy().replace('\\', "/");
        if display.contains(' ') {
            out.push_str(&format!("@\"{display}\""));
        } else {
            out.push('@');
            out.push_str(&display);
        }
        out.push(' ');
    }
    out
}

/// Translate a key event into the byte sequence a terminal expects.
fn key_to_bytes(key: KeyEvent) -> Option<Vec<u8>> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let bytes = match key.code {
        KeyCode::Char(c) => {
            if ctrl {
                let up = c.to_ascii_uppercase();
                if ('@'..='_').contains(&up) {
                    vec![(up as u8) & 0x1f]
                } else if c == ' ' {
                    vec![0]
                } else {
                    char_bytes(c)
                }
            } else {
                char_bytes(c)
            }
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => vec![0x1b, b'[', b'Z'],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => vec![0x1b, b'[', b'A'],
        KeyCode::Down => vec![0x1b, b'[', b'B'],
        KeyCode::Right => vec![0x1b, b'[', b'C'],
        KeyCode::Left => vec![0x1b, b'[', b'D'],
        KeyCode::Home => vec![0x1b, b'[', b'H'],
        KeyCode::End => vec![0x1b, b'[', b'F'],
        KeyCode::PageUp => vec![0x1b, b'[', b'5', b'~'],
        KeyCode::PageDown => vec![0x1b, b'[', b'6', b'~'],
        KeyCode::Delete => vec![0x1b, b'[', b'3', b'~'],
        KeyCode::Insert => vec![0x1b, b'[', b'2', b'~'],
        _ => return None,
    };
    Some(bytes)
}

fn char_bytes(c: char) -> Vec<u8> {
    let mut tmp = [0u8; 4];
    c.encode_utf8(&mut tmp).as_bytes().to_vec()
}

fn draw(f: &mut Frame, app: &mut App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // tab bar
            Constraint::Min(3),    // tree + dock
            Constraint::Length(1), // status
        ])
        .split(f.area());

    draw_tabs(f, rows[0], app);

    let active = app.active;
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(18), Constraint::Percentage(82)])
        .split(rows[1]);

    let ws = &mut app.workspaces[active];
    draw_tree(f, cols[0], ws);
    draw_dock(f, cols[1], ws);
    draw_status(f, rows[2], ws);
}

fn draw_tabs(f: &mut Frame, area: Rect, app: &App) {
    let titles: Vec<String> = app
        .workspaces
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let live = if w.dock.is_alive() { "" } else { "·exited" };
            format!(" {}:{}{} ", i + 1, w.label, live)
        })
        .collect();
    let tabs = Tabs::new(titles)
        .select(app.active)
        .style(Style::default().fg(Color::Gray))
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    f.render_widget(tabs, area);
}

fn draw_status(f: &mut Frame, area: Rect, ws: &Workspace) {
    let exited = if ws.dock.is_alive() { "" } else { " [exited]" };
    let hint = match ws.focus {
        Focus::Tree => {
            let n = ws.tree.marked.len();
            let sel = if n > 0 {
                format!("Enter send {n} marked")
            } else {
                "Enter open/send".to_string()
            };
            format!("Ctrl-O dock · Ctrl-T/W tab · Space mark · {sel} · q quit · {}{}", ws.dock.program, exited)
        }
        Focus::Dock => format!(
            "Ctrl-O tree · Ctrl-T/W tab · Ctrl-PgUp/Dn switch · Alt-PgUp/Dn scroll{} · keys → {}{}",
            if ws.dock.is_scrolled() { " [SCROLLED]" } else { "" },
            ws.dock.program,
            exited
        ),
    };
    f.render_widget(
        Paragraph::new(hint).style(Style::default().fg(Color::DarkGray)),
        area,
    );
}

fn draw_tree(f: &mut Frame, area: Rect, ws: &Workspace) {
    let focused = ws.focus == Focus::Tree;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style(focused))
        .title(format!(" {} ", ws.tree.cwd.to_string_lossy()));

    let items: Vec<ListItem> = ws
        .tree
        .entries
        .iter()
        .map(|e| {
            let marked = ws.tree.is_marked(&e.path);
            let glyph = if e.name == ".." {
                "↑"
            } else if e.is_dir {
                "▸"
            } else {
                " "
            };
            let mark = if marked { "*" } else { " " };

            // Git status: a colored single-char column.
            let (git_char, git_color) = match ws.git.file_status(&e.path) {
                Some(git::Status::Untracked) => ("?", Color::Red),
                Some(git::Status::Staged) => ("+", Color::Green),
                Some(git::Status::Modified) => ("M", Color::Yellow),
                None if e.is_dir && ws.git.dir_has_changes(&e.path) => ("·", Color::DarkGray),
                None => (" ", Color::Reset),
            };

            let mut name_style = Style::default();
            if e.is_dir {
                name_style = name_style.fg(Color::Blue).add_modifier(Modifier::BOLD);
            }
            if marked {
                name_style = name_style.fg(Color::Yellow).add_modifier(Modifier::BOLD);
            }

            let line = Line::from(vec![
                Span::raw(mark),
                Span::styled(git_char, Style::default().fg(git_color)),
                Span::styled(format!("{glyph} {}", e.name), name_style),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::default()
        .items(items)
        .block(block)
        .highlight_style(highlight_style(focused));

    let mut state = ListState::default();
    if !ws.tree.entries.is_empty() {
        state.select(Some(ws.tree.cursor));
    }
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_dock(f: &mut Frame, area: Rect, ws: &mut Workspace) {
    let focused = ws.focus == Focus::Dock;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style(focused))
        .title(format!(" {} ", ws.dock.program));
    let inner = block.inner(area);
    f.render_widget(block, area);

    ws.dock.resize(inner.height, inner.width);
    let cursor = ws.dock.render(inner, f.buffer_mut());
    if focused {
        if let Some((x, y)) = cursor {
            f.set_cursor_position((x, y));
        }
    }
}

fn border_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn highlight_style(focused: bool) -> Style {
    if focused {
        Style::default()
            .bg(Color::Cyan)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::REVERSED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode, ctrl: bool) -> KeyEvent {
        let mods = if ctrl {
            KeyModifiers::CONTROL
        } else {
            KeyModifiers::NONE
        };
        KeyEvent::new(code, mods)
    }

    #[test]
    fn plain_chars_and_enter() {
        assert_eq!(key_to_bytes(k(KeyCode::Char('a'), false)), Some(b"a".to_vec()));
        assert_eq!(key_to_bytes(k(KeyCode::Enter, false)), Some(vec![b'\r']));
        assert_eq!(key_to_bytes(k(KeyCode::Backspace, false)), Some(vec![0x7f]));
    }

    #[test]
    fn ctrl_letters_map_to_control_bytes() {
        assert_eq!(key_to_bytes(k(KeyCode::Char('c'), true)), Some(vec![0x03]));
        assert_eq!(key_to_bytes(k(KeyCode::Char('a'), true)), Some(vec![0x01]));
    }

    #[test]
    fn arrows_are_csi_sequences() {
        assert_eq!(key_to_bytes(k(KeyCode::Up, false)), Some(b"\x1b[A".to_vec()));
        assert_eq!(key_to_bytes(k(KeyCode::Left, false)), Some(b"\x1b[D".to_vec()));
    }

    #[test]
    fn mentions_are_relative_and_forward_slashed() {
        let cwd = PathBuf::from(if cfg!(windows) { r"C:\proj" } else { "/proj" });
        let under = cwd.join("src").join("main.rs");
        assert_eq!(mention_string(&[under], &cwd), "@src/main.rs ");
    }

    #[test]
    fn mention_with_space_is_quoted() {
        let cwd = PathBuf::from(if cfg!(windows) { r"C:\proj" } else { "/proj" });
        let spaced = cwd.join("my notes.md");
        assert_eq!(mention_string(&[spaced], &cwd), "@\"my notes.md\" ");
    }

    #[test]
    fn mention_outside_cwd_stays_absolute() {
        let cwd = PathBuf::from(if cfg!(windows) { r"C:\proj" } else { "/proj" });
        let outside = PathBuf::from(if cfg!(windows) { r"D:\other\x.rs" } else { "/other/x.rs" });
        let s = mention_string(&[outside], &cwd);
        assert!(s.starts_with('@') && s.ends_with(' '));
        assert!(s.contains("x.rs"));
    }
}
