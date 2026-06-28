//! The agent dock: a real pseudo-terminal running a shell or coding agent,
//! parsed by a `vt100` emulator and rendered into a ratatui pane.
//!
//! A background thread pumps the PTY's output into a shared `vt100::Parser`;
//! the UI thread locks it to render the current screen. Input flows the other
//! way: the UI writes keystroke bytes to the PTY writer.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

/// How many lines of scrollback the dock keeps.
const SCROLLBACK_LEN: usize = 5000;

pub struct Dock {
    parser: Arc<Mutex<vt100::Parser>>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    alive: Arc<AtomicBool>,
    size: (u16, u16),
    /// Rows scrolled back from the live bottom (0 == following output).
    scroll: usize,
    /// The command running in the dock (shown in the title/status).
    pub program: String,
}

impl Dock {
    /// Spawn `exec` (with `args`) in a PTY rooted at `cwd`, sized `rows` x
    /// `cols`. `label` is what shows in the title (e.g. "claude" even when
    /// `exec` is `cmd.exe /c claude`).
    pub fn spawn(
        exec: &str,
        args: &[String],
        label: &str,
        cwd: &Path,
        rows: u16,
        cols: u16,
    ) -> Result<Dock> {
        let rows = rows.max(1);
        let cols = cols.max(1);

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .context("openpty")?;

        let mut cmd = CommandBuilder::new(exec);
        for a in args {
            cmd.arg(a);
        }
        cmd.cwd(cwd);
        // Propagate the environment so PATH (and thus `claude`/`codex`/`grok`)
        // resolves, and advertise a capable terminal.
        for (k, v) in std::env::vars() {
            cmd.env(k, v);
        }
        cmd.env("TERM", "xterm-256color");

        let child = pair
            .slave
            .spawn_command(cmd)
            .with_context(|| format!("spawning '{label}'"))?;
        // Drop the slave so the child sees EOF / we get a clean exit signal.
        drop(pair.slave);

        let reader = pair.master.try_clone_reader().context("clone pty reader")?;
        let writer = pair.master.take_writer().context("take pty writer")?;
        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, SCROLLBACK_LEN)));
        let alive = Arc::new(AtomicBool::new(true));

        // Pump PTY output into the parser until EOF.
        {
            let parser = parser.clone();
            let alive = alive.clone();
            let mut reader = reader;
            thread::spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if let Ok(mut p) = parser.lock() {
                                p.process(&buf[..n]);
                            }
                        }
                    }
                }
                alive.store(false, Ordering::Relaxed);
            });
        }

        Ok(Dock {
            parser,
            writer,
            master: pair.master,
            child,
            alive,
            size: (rows, cols),
            scroll: 0,
            program: label.to_string(),
        })
    }

    /// Resize the PTY and emulator to match the pane (no-op if unchanged).
    pub fn resize(&mut self, rows: u16, cols: u16) {
        let rows = rows.max(1);
        let cols = cols.max(1);
        if (rows, cols) == self.size {
            return;
        }
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        if let Ok(mut p) = self.parser.lock() {
            p.set_size(rows, cols);
        }
        self.size = (rows, cols);
    }

    /// Forward input bytes to the child.
    pub fn send(&mut self, bytes: &[u8]) {
        let _ = self.writer.write_all(bytes);
        let _ = self.writer.flush();
    }

    /// Scroll the view up/down by half a page through the scrollback buffer.
    pub fn scroll_page_up(&mut self) {
        let page = (self.size.0 as usize / 2).max(1);
        // vt100 0.15.2's visible_rows() underflows if the scrollback offset
        // exceeds the visible row count, so cap one screen back.
        let cap = self.size.0.saturating_sub(1) as usize;
        self.scroll = (self.scroll + page).min(cap);
    }

    pub fn scroll_page_down(&mut self) {
        let page = (self.size.0 as usize / 2).max(1);
        self.scroll = self.scroll.saturating_sub(page);
    }

    /// Jump back to the live bottom (called on input, like a real terminal).
    pub fn scroll_reset(&mut self) {
        self.scroll = 0;
    }

    pub fn is_scrolled(&self) -> bool {
        self.scroll > 0
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }

    /// Render the current terminal screen into `area`. Returns the absolute
    /// cursor position if it should be drawn.
    pub fn render(&self, area: Rect, buf: &mut Buffer) -> Option<(u16, u16)> {
        let mut guard = self.parser.lock().ok()?;
        guard.set_scrollback(self.scroll);
        let screen = guard.screen();
        for row in 0..area.height {
            for col in 0..area.width {
                let target = &mut buf[(area.x + col, area.y + row)];
                match screen.cell(row, col) {
                    Some(cell) => {
                        let contents = cell.contents();
                        let sym: &str = if contents.is_empty() { " " } else { contents.as_str() };
                        target.set_symbol(sym);
                        target.set_style(cell_style(cell));
                    }
                    None => {
                        target.set_symbol(" ");
                        target.set_style(Style::default());
                    }
                }
            }
        }
        if screen.hide_cursor() {
            return None;
        }
        let (crow, ccol) = screen.cursor_position();
        if crow < area.height && ccol < area.width {
            Some((area.x + ccol, area.y + crow))
        } else {
            None
        }
    }
}

fn cell_style(cell: &vt100::Cell) -> Style {
    let mut style = Style::default();
    if let Some(fg) = conv_color(cell.fgcolor()) {
        style = style.fg(fg);
    }
    if let Some(bg) = conv_color(cell.bgcolor()) {
        style = style.bg(bg);
    }
    if cell.bold() {
        style = style.add_modifier(Modifier::BOLD);
    }
    if cell.italic() {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if cell.underline() {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if cell.inverse() {
        style = style.add_modifier(Modifier::REVERSED);
    }
    style
}

fn conv_color(c: vt100::Color) -> Option<Color> {
    match c {
        vt100::Color::Default => None,
        vt100::Color::Idx(i) => Some(Color::Indexed(i)),
        vt100::Color::Rgb(r, g, b) => Some(Color::Rgb(r, g, b)),
    }
}
