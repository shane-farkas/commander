//! The dual-pane ratatui UI. Milestone 1: browse both panes, mark files, and
//! confirm a selection that gets written to the IPC channel for the agent.
//!
//! Keymap (Midnight Commander-flavoured):
//!   ←/h, →/l     switch active pane
//!   ↑/k, ↓/j     move cursor
//!   Enter        descend into dir / open ".."
//!   Backspace    go up a directory
//!   Space        toggle mark on the file/dir under the cursor
//!   g / G        jump to top / bottom
//!   a            confirm: send selection to the agent (no action tag)
//!   r            confirm with action = "review"
//!   e            confirm with action = "explain"
//!   q / Esc      quit without sending

use std::io::{self, Stdout};
use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use commander_core::{AppState, Pane, Side};
use commander_ipc::{now_ms, Selection};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

/// Outcome of a TUI session, surfaced to the caller for the exit message.
pub enum Outcome {
    /// The user confirmed; this many paths were written to the IPC channel.
    Sent { count: usize, action: Option<String> },
    /// The user quit without sending.
    Cancelled,
}

/// Run the dual-pane UI rooted at `start_dir`. Sets up and tears down the
/// terminal, restoring it even on error.
pub fn run(start_dir: impl AsRef<Path>) -> Result<Outcome> {
    let mut terminal = setup_terminal()?;
    let result = run_loop(&mut terminal, start_dir.as_ref());
    restore_terminal(&mut terminal)?;
    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    start_dir: &Path,
) -> Result<Outcome> {
    let mut app = AppState::new(start_dir)?;
    let mut status = String::from("Space=mark  a=send  r=review  e=explain  q=quit");

    loop {
        terminal.draw(|f| draw(f, &app, &status))?;

        // Poll so a resize or future async nav event can wake us; 200ms is
        // imperceptible but keeps the loop responsive.
        if !event::poll(Duration::from_millis(200))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        // Windows reports both press and release; act on press only.
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match handle_key(key, &mut app, &mut status)? {
            Flow::Continue => {}
            Flow::Quit => {
                write_cancelled(&app);
                return Ok(Outcome::Cancelled);
            }
            Flow::Send(action) => {
                let count = confirm(&app, action.clone())?;
                return Ok(Outcome::Sent { count, action });
            }
        }
    }
}

enum Flow {
    Continue,
    Quit,
    Send(Option<String>),
}

fn handle_key(key: KeyEvent, app: &mut AppState, status: &mut String) -> Result<Flow> {
    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), _) | (KeyCode::Esc, _) => return Ok(Flow::Quit),
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(Flow::Quit),

        (KeyCode::Left, _) | (KeyCode::Char('h'), _) => app.active = Side::Left,
        (KeyCode::Right, _) | (KeyCode::Char('l'), _) => app.active = Side::Right,
        (KeyCode::Tab, _) => app.switch_pane(),

        (KeyCode::Up, _) | (KeyCode::Char('k'), _) => app.active_pane_mut().move_cursor(-1),
        (KeyCode::Down, _) | (KeyCode::Char('j'), _) => app.active_pane_mut().move_cursor(1),
        (KeyCode::Char('g'), _) => app.active_pane_mut().cursor_top(),
        (KeyCode::Char('G'), _) => app.active_pane_mut().cursor_bottom(),

        (KeyCode::Enter, _) => {
            if let Err(e) = app.active_pane_mut().enter() {
                *status = format!("cannot open: {e}");
            }
        }
        (KeyCode::Backspace, _) => {
            if let Err(e) = app.active_pane_mut().ascend() {
                *status = format!("cannot go up: {e}");
            }
        }

        (KeyCode::Char(' '), _) => app.active_pane_mut().toggle_mark(),

        (KeyCode::Char('a'), _) => return Ok(Flow::Send(None)),
        (KeyCode::Char('r'), _) => return Ok(Flow::Send(Some("review".into()))),
        (KeyCode::Char('e'), _) => return Ok(Flow::Send(Some("explain".into()))),

        _ => {}
    }
    Ok(Flow::Continue)
}

/// Build a [`Selection`] from the active pane and write it to the IPC channel.
fn confirm(app: &AppState, action: Option<String>) -> Result<usize> {
    let pane = app.active_pane();
    let paths = pane.effective_selection();
    let sel = Selection {
        status: commander_ipc::Status::Sent,
        cwd: pane.cwd.clone(),
        paths: paths.clone(),
        action,
        submitted_at_ms: now_ms(),
    };
    commander_ipc::write_selection(&sel)?;
    Ok(paths.len())
}

/// Write a `Cancelled` outcome so a waiting `commander_open` returns promptly
/// instead of polling until timeout.
fn write_cancelled(app: &AppState) {
    let sel = Selection {
        status: commander_ipc::Status::Cancelled,
        cwd: app.active_pane().cwd.clone(),
        paths: Vec::new(),
        action: None,
        submitted_at_ms: now_ms(),
    };
    // Best-effort: cancelling shouldn't fail the whole session on a write error.
    let _ = commander_ipc::write_selection(&sel);
}

fn draw(f: &mut Frame, app: &AppState, status: &str) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(f.area());

    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[0]);

    draw_pane(f, panes[0], &app.left, app.active == Side::Left);
    draw_pane(f, panes[1], &app.right, app.active == Side::Right);

    let help = Paragraph::new(status).style(Style::default().fg(Color::DarkGray));
    f.render_widget(help, chunks[1]);
}

fn draw_pane(f: &mut Frame, area: Rect, pane: &Pane, active: bool) {
    let border_style = if active {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let title = format!(" {} ", truncate(&pane.cwd.to_string_lossy(), area.width.saturating_sub(4) as usize));
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(title);

    let items: Vec<ListItem> = pane
        .entries
        .iter()
        .map(|e| {
            let marked = pane.is_marked(&e.path);
            let glyph = if e.name == ".." {
                "↑"
            } else if e.is_dir {
                "▸"
            } else {
                " "
            };
            let mark = if marked { "*" } else { " " };
            let size = if e.is_dir {
                String::new()
            } else {
                human_size(e.size)
            };
            // name padded left, size pushed right-ish via a single space gap.
            let label = format!("{mark}{glyph} {:<24} {:>8}", truncate(&e.name, 24), size);
            let mut style = Style::default();
            if e.is_dir {
                style = style.fg(Color::Blue).add_modifier(Modifier::BOLD);
            }
            if marked {
                style = style.fg(Color::Yellow).add_modifier(Modifier::BOLD);
            }
            ListItem::new(label).style(style)
        })
        .collect();

    let highlight = if active {
        Style::default()
            .bg(Color::Cyan)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::REVERSED)
    };

    let list = List::default()
        .items(items)
        .block(block)
        .highlight_style(highlight);

    let mut state = ListState::default();
    if !pane.entries.is_empty() {
        state.select(Some(pane.cursor));
    }
    f.render_stateful_widget(list, area, &mut state);
}

fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push('…');
    out
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes}{}", UNITS[0])
    } else {
        format!("{size:.1}{}", UNITS[unit])
    }
}
