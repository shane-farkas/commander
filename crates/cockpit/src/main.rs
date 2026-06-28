//! `cockpit` is the standalone Commander workbench: a file tree on the left,
//! and a dock on the right where a coding agent will eventually live in an
//! embedded PTY. Unlike the plugin (where the host owns the terminal), the
//! cockpit is the top-level app, so it owns the screen and can host an agent
//! pane itself.
//!
//! This is a skeleton. The file tree is real and reuses `commander-core`; the
//! agent dock is a placeholder until PTY embedding lands. It deliberately shares
//! nothing with, and changes nothing in, the plugin binary.
//!
//! Keymap:
//!   ↑/k, ↓/j     move in the tree
//!   Enter        descend into dir / `..`
//!   Backspace    go up a directory
//!   Tab          switch focus (tree <-> dock)
//!   q / Esc      quit

use std::io::{self, Stdout};
use std::time::Duration;

use anyhow::Result;
use commander_core::Pane;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Tree,
    Dock,
}

struct Cockpit {
    tree: Pane,
    focus: Focus,
    status: String,
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cockpit: error: {e:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    // Optional starting directory: `cockpit [DIR]`.
    let start = match std::env::args().nth(1) {
        Some(d) => std::path::PathBuf::from(d),
        None => std::env::current_dir()?,
    };
    let mut terminal = setup_terminal()?;
    let result = run_loop(&mut terminal, &start);
    restore_terminal(&mut terminal)?;
    result
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

fn run_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>, start: &std::path::Path) -> Result<()> {
    let mut app = Cockpit {
        tree: Pane::open(start)?,
        focus: Focus::Tree,
        status: String::from("Tab=switch focus  Enter=open  Backspace=up  q=quit"),
    };

    loop {
        terminal.draw(|f| draw(f, &app))?;

        if !event::poll(Duration::from_millis(200))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if handle_key(key, &mut app)? {
            return Ok(());
        }
    }
}

/// Returns `true` when the app should quit.
fn handle_key(key: KeyEvent, app: &mut Cockpit) -> Result<bool> {
    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), _) | (KeyCode::Esc, _) => return Ok(true),
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(true),
        (KeyCode::Tab, _) => {
            app.focus = match app.focus {
                Focus::Tree => Focus::Dock,
                Focus::Dock => Focus::Tree,
            };
        }
        // Tree navigation only matters while the tree has focus.
        (KeyCode::Up, _) | (KeyCode::Char('k'), _) if app.focus == Focus::Tree => {
            app.tree.move_cursor(-1)
        }
        (KeyCode::Down, _) | (KeyCode::Char('j'), _) if app.focus == Focus::Tree => {
            app.tree.move_cursor(1)
        }
        (KeyCode::Enter, _) if app.focus == Focus::Tree => {
            if let Err(e) = app.tree.enter() {
                app.status = format!("cannot open: {e}");
            }
        }
        (KeyCode::Backspace, _) if app.focus == Focus::Tree => {
            if let Err(e) = app.tree.ascend() {
                app.status = format!("cannot go up: {e}");
            }
        }
        _ => {}
    }
    Ok(false)
}

fn draw(f: &mut Frame, app: &Cockpit) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(f.area());

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(rows[0]);

    draw_tree(f, cols[0], app);
    draw_dock(f, cols[1], app);

    let help = Paragraph::new(app.status.as_str()).style(Style::default().fg(Color::DarkGray));
    f.render_widget(help, rows[1]);
}

fn draw_tree(f: &mut Frame, area: Rect, app: &Cockpit) {
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
            let glyph = if e.name == ".." {
                "↑"
            } else if e.is_dir {
                "▸"
            } else {
                " "
            };
            let mut style = Style::default();
            if e.is_dir {
                style = style.fg(Color::Blue).add_modifier(Modifier::BOLD);
            }
            ListItem::new(format!("{glyph} {}", e.name)).style(style)
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

fn draw_dock(f: &mut Frame, area: Rect, app: &Cockpit) {
    let focused = app.focus == Focus::Dock;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style(focused))
        .title(" Agent dock ");

    let body = Text::from(vec![
        Line::from(""),
        Line::from("  This pane will host a coding agent in an embedded terminal."),
        Line::from("  (Claude Code / Codex / Grok), running in a PTY the cockpit owns."),
        Line::from(""),
        Line::from("  Because the cockpit is the top-level app, it owns the screen,"),
        Line::from("  so it can dock a live agent here. That PTY embedding is the"),
        Line::from("  next milestone; this is the skeleton."),
        Line::from(""),
        Line::from(Span::styled(
            "  [ not yet wired ]",
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
        )),
    ]);

    f.render_widget(Paragraph::new(body).block(block).wrap(Wrap { trim: false }), area);
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
