//! `cockpit` is the standalone Commander workbench: a file tree on the left and
//! a docked coding agent (or shell) on the right, running in a real PTY. Unlike
//! the plugin, the cockpit is the top-level app, so it owns the screen and can
//! host a live agent pane itself.
//!
//! Usage:
//!   cockpit [DIR]                 file tree + a shell in the dock
//!   cockpit --agent claude [DIR]  auto-start `claude` in the dock
//!
//! Keys:
//!   Ctrl-O   toggle focus (tree <-> dock)
//!   Ctrl-Q   quit the cockpit
//!   tree focused:  ↑/k ↓/j move, Space mark, Enter open dir / send file(s),
//!                  Backspace up, q quit
//!   dock focused:  every other key goes to the agent (Tab, Ctrl-C, ...)
//!
//! "Send" types the selected paths into the dock as `@path` mentions (no
//! newline) and focuses the dock, so you add your instruction and submit.

mod dock;

use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Result;
use commander_core::Pane;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use dock::Dock;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Tree,
    Dock,
}

struct App {
    tree: Pane,
    dock: Dock,
    focus: Focus,
    /// The directory the agent was launched in; `@` mentions are made relative
    /// to it so they match what the agent sees.
    agent_cwd: PathBuf,
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
    // Parse `[DIR]` and `--agent <cmd>`.
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
    let program = agent.unwrap_or_else(default_shell);

    let mut terminal = setup_terminal()?;
    let result = run_loop(&mut terminal, &start, &program);
    restore_terminal(&mut terminal)?;
    result
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
    program: &str,
) -> Result<()> {
    let tree = Pane::open(start)?;
    // Capture the canonical launch dir before any navigation; this matches the
    // agent's working directory for relative `@` mentions.
    let agent_cwd = tree.cwd.clone();
    // Spawn with a placeholder size; the first draw resizes it to the real pane.
    let dock = Dock::spawn(program, start, 24, 80)?;
    let mut app = App {
        tree,
        dock,
        focus: Focus::Dock,
        agent_cwd,
    };

    let result = loop {
        terminal.draw(|f| draw(f, &mut app))?;

        // Short poll so PTY output is reflected promptly even without keypresses.
        if !event::poll(Duration::from_millis(50))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if handle_key(key, &mut app) {
                    break Ok(());
                }
            }
            _ => {} // resize is picked up by the next draw
        }
    };

    app.dock.kill();
    result
}

/// Returns `true` when the cockpit should quit.
fn handle_key(key: KeyEvent, app: &mut App) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // Global chords, reserved from the agent (tmux-style).
    if ctrl && key.code == KeyCode::Char('q') {
        return true;
    }
    if ctrl && key.code == KeyCode::Char('o') {
        app.focus = match app.focus {
            Focus::Tree => Focus::Dock,
            Focus::Dock => Focus::Tree,
        };
        return false;
    }

    match app.focus {
        Focus::Dock => {
            if let Some(bytes) = key_to_bytes(key) {
                app.dock.send(&bytes);
            }
        }
        Focus::Tree => match key.code {
            KeyCode::Up | KeyCode::Char('k') => app.tree.move_cursor(-1),
            KeyCode::Down | KeyCode::Char('j') => app.tree.move_cursor(1),
            KeyCode::Char(' ') => app.tree.toggle_mark(),
            KeyCode::Enter => {
                // Marks take priority (send them). Otherwise open a directory,
                // or send the file under the cursor.
                let on_dir = app.tree.current().map(|e| e.is_dir).unwrap_or(false);
                if app.tree.marked.is_empty() && on_dir {
                    let _ = app.tree.enter();
                } else {
                    send_files(app);
                }
            }
            KeyCode::Backspace => {
                let _ = app.tree.ascend();
            }
            KeyCode::Char('q') | KeyCode::Esc => return true,
            _ => {}
        },
    }
    false
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

/// Send the tree's selection (marked files, or the file under the cursor) into
/// the dock as `@path` mentions, then focus the dock so the user can add their
/// instruction. Clears the marks afterward.
fn send_files(app: &mut App) {
    let paths = app.tree.effective_selection();
    if paths.is_empty() {
        return;
    }
    let text = mention_string(&paths, &app.agent_cwd);
    app.dock.send(text.as_bytes());
    app.tree.marked.clear();
    app.focus = Focus::Dock;
}

/// Build a space-separated list of `@path` mentions, relative to `agent_cwd`
/// when the path is under it (absolute otherwise), with forward slashes so the
/// agent parses them cleanly. Trailing space and no newline: the user types
/// their instruction next and submits it themselves.
fn mention_string(paths: &[PathBuf], agent_cwd: &Path) -> String {
    let mut out = String::new();
    for p in paths {
        let rel = p.strip_prefix(agent_cwd).unwrap_or(p);
        let display = rel.to_string_lossy().replace('\\', "/");
        // Quote the path when it contains a space so the agent treats it as one
        // mention rather than several tokens.
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
        // Ctrl-C -> 0x03, Ctrl-A -> 0x01.
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

fn draw(f: &mut Frame, app: &mut App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(f.area());

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(18), Constraint::Percentage(82)])
        .split(rows[0]);

    draw_tree(f, cols[0], app);
    draw_dock(f, cols[1], app);

    let exited = if app.dock.is_alive() { "" } else { " [exited]" };
    let hint = match app.focus {
        Focus::Tree => {
            let n = app.tree.marked.len();
            let sel = if n > 0 {
                format!("Enter send {n} marked")
            } else {
                "Enter open/send".to_string()
            };
            format!("Ctrl-O dock · Space mark · {sel} · Backspace up · q quit · dock: {}{}", app.dock.program, exited)
        }
        Focus::Dock => format!(
            "Ctrl-O tree · Ctrl-Q quit · keys → {}{}",
            app.dock.program, exited
        ),
    };
    f.render_widget(
        Paragraph::new(hint).style(Style::default().fg(Color::DarkGray)),
        rows[1],
    );
}

fn draw_tree(f: &mut Frame, area: Rect, app: &App) {
    let focused = app.focus == Focus::Tree;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style(focused))
        .title(format!(" {} ", app.tree.cwd.to_string_lossy()));

    let items: Vec<ListItem> = app
        .tree
        .entries
        .iter()
        .map(|e| {
            let marked = app.tree.is_marked(&e.path);
            let glyph = if e.name == ".." {
                "↑"
            } else if e.is_dir {
                "▸"
            } else {
                " "
            };
            let mark = if marked { "*" } else { " " };
            let mut style = Style::default();
            if e.is_dir {
                style = style.fg(Color::Blue).add_modifier(Modifier::BOLD);
            }
            if marked {
                style = style.fg(Color::Yellow).add_modifier(Modifier::BOLD);
            }
            ListItem::new(format!("{mark}{glyph} {}", e.name)).style(style)
        })
        .collect();

    let list = List::default()
        .items(items)
        .block(block)
        .highlight_style(highlight_style(focused));

    let mut state = ListState::default();
    if !app.tree.entries.is_empty() {
        state.select(Some(app.tree.cursor));
    }
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_dock(f: &mut Frame, area: Rect, app: &mut App) {
    let focused = app.focus == Focus::Dock;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style(focused))
        .title(format!(" {} ", app.dock.program));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Keep the PTY sized to the visible pane, then paint its screen.
    app.dock.resize(inner.height, inner.width);
    let cursor = app.dock.render(inner, f.buffer_mut());
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
