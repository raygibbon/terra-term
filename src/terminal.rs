use crate::buffer::{Buffer, Slot};
use crate::platform::{Backend, Wake};
use crate::terminfo::Capabilities;
use crate::{Cell, Color, Event, InputMode, InputParser, Style};
use std::fmt::Write as _;
use std::io;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Size {
    pub width: u16,
    pub height: u16,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Position {
    pub x: u16,
    pub y: u16,
}

/// Owns the terminal mode and restores it on drop.
pub struct Terminal {
    backend: Backend,
    back: Buffer,
    front: Vec<Slot>,
    invalid: bool,
    parser: InputParser,
    escape_since: Option<Instant>,
    sequence_since: Option<Instant>,
    suspended: bool,
    cursor: Option<Position>,
    clear_style: Style,
    mouse_enabled: bool,
    caps: Capabilities,
}

impl Terminal {
    pub fn new() -> io::Result<Self> {
        let caps = Capabilities::load()?;
        let backend = Backend::new(caps.clone())?;
        Self::from_backend(backend, caps)
    }
    /// Connect to process stdin and stdout instead of the controlling terminal.
    #[cfg(unix)]
    pub fn from_stdio() -> io::Result<Self> {
        let caps = Capabilities::load()?;
        let backend = Backend::from_stdio(caps.clone())?;
        Self::from_backend(backend, caps)
    }
    #[cfg(unix)]
    pub fn from_fds(input: std::os::fd::OwnedFd, output: std::os::fd::OwnedFd) -> io::Result<Self> {
        let caps = Capabilities::load()?;
        let backend = Backend::from_fds(input, output, caps.clone())?;
        Self::from_backend(backend, caps)
    }
    #[cfg(unix)]
    pub fn from_tty_path(path: impl AsRef<std::path::Path>) -> io::Result<Self> {
        let tty = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?;
        let input = std::os::fd::OwnedFd::from(tty.try_clone()?);
        let output = std::os::fd::OwnedFd::from(tty);
        Self::from_fds(input, output)
    }
    fn from_backend(backend: Backend, caps: Capabilities) -> io::Result<Self> {
        let mut parser = InputParser::new();
        for (seq, key) in &caps.keys {
            parser.add_key_sequence(seq, *key);
        }
        let size = backend.size()?;
        let back = Buffer::new(size);
        let front = back.cells.clone();
        Ok(Self {
            backend,
            back,
            front,
            invalid: true,
            parser,
            escape_since: None,
            sequence_since: None,
            suspended: false,
            cursor: None,
            clear_style: Style::default(),
            mouse_enabled: false,
            caps,
        })
    }
    pub fn size(&self) -> Size {
        self.back.size
    }
    /// Unix descriptors that become readable for input and resize events.
    /// After waiting on either descriptor, call `try_event` to consume events.
    #[cfg(unix)]
    pub fn event_fds(&self) -> (std::os::fd::BorrowedFd<'_>, std::os::fd::BorrowedFd<'_>) {
        self.backend.event_fds()
    }
    pub fn clear(&mut self) {
        self.back.clear_with_style(self.clear_style);
    }
    pub fn set_clear_style(&mut self, style: Style) {
        self.clear_style = style;
    }
    pub fn clear_style(&self) -> Style {
        self.clear_style
    }
    pub fn clear_with_style(&mut self, style: Style) {
        self.back.clear_with_style(style);
    }
    /// Clear the physical screen and force the next `present` to redraw it.
    pub fn clear_screen(&mut self) -> io::Result<()> {
        self.backend.clear_screen()?;
        self.invalid = true;
        Ok(())
    }
    /// Send bytes directly to the terminal. The next presentation repaints the screen.
    pub fn send_raw(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.backend.write(bytes)?;
        self.invalid = true;
        Ok(())
    }
    pub fn put_cell(&mut self, x: u16, y: u16, cell: Cell) {
        self.back.put_cell(x, y, cell);
    }
    /// Returns the logical cell, or None for a continuation or out of bounds.
    pub fn cell(&self, x: u16, y: u16) -> Option<&Cell> {
        self.back.cell(x, y)
    }
    /// Change a cell's style, including its continuation when the cell is wide.
    pub fn set_cell_style(&mut self, x: u16, y: u16, style: Style) -> bool {
        self.back.set_cell_style(x, y, style)
    }
    /// Extend a cell with text when the result is one grapheme cluster.
    /// Returns false if the position is invalid or the text would split the cell.
    pub fn extend_cell(&mut self, x: u16, y: u16, suffix: &str) -> bool {
        self.back.extend_cell(x, y, suffix)
    }
    pub fn put_char(&mut self, x: u16, y: u16, ch: char, style: Style) {
        self.back.put_char(x, y, ch, style);
    }
    pub fn put_str(&mut self, x: u16, y: u16, text: &str, style: Style) {
        self.back.put_str(x, y, text, style);
    }
    pub fn invalidate(&mut self) {
        self.invalid = true;
    }
    pub fn show_cursor(&mut self) {
        self.cursor.get_or_insert(Position::default());
    }
    pub fn hide_cursor(&mut self) {
        self.cursor = None;
    }
    pub fn set_cursor(&mut self, x: u16, y: u16) {
        self.cursor = Some(Position { x, y });
    }
    pub fn cursor(&self) -> Option<Position> {
        self.cursor
    }
    pub fn set_input_mode(&mut self, mode: InputMode) {
        self.parser.set_mode(mode);
    }
    pub fn input_mode(&self) -> InputMode {
        self.parser.mode()
    }
    pub fn set_mouse_enabled(&mut self, enabled: bool) -> io::Result<()> {
        self.backend.set_mouse(enabled)?;
        self.mouse_enabled = enabled;
        Ok(())
    }
    /// Wait for the next event, including resize, without a polling loop.
    pub fn read_event(&mut self) -> io::Result<Event> {
        if self.suspended {
            return Err(io::Error::other("terminal is suspended"));
        }
        loop {
            if let Some(event) = self.wait_event(None)? {
                return Ok(event);
            }
        }
    }
    /// Wait up to `timeout` for an event. Incomplete escape sequences remain
    /// buffered across timeouts; only a lone ESC uses a 30 ms grace period.
    pub fn read_event_timeout(&mut self, timeout: Duration) -> io::Result<Option<Event>> {
        self.wait_event(Some(timeout))
    }
    pub fn try_event(&mut self) -> io::Result<Option<Event>> {
        self.wait_event(Some(Duration::ZERO))
    }
    fn wait_event(&mut self, timeout: Option<Duration>) -> io::Result<Option<Event>> {
        if self.suspended {
            return Ok(None);
        }
        let deadline = timeout.and_then(|d| Instant::now().checked_add(d));
        loop {
            if let Some(event) = self.parser.next_event() {
                self.escape_since = None;
                self.sequence_since = None;
                return Ok(Some(event));
            }
            if self.parser.pending_lone_escape() {
                let since = self.escape_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= Duration::from_millis(30) {
                    self.escape_since = None;
                    return Ok(self.parser.finish_escape());
                }
            } else {
                self.escape_since = None;
            }
            if self.parser.pending_sequence() {
                let since = self.sequence_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= Duration::from_millis(500) {
                    self.sequence_since = None;
                    return Ok(self.parser.finish_incomplete_sequence());
                }
            } else {
                self.sequence_since = None;
            }
            let remaining = deadline.map(|d| d.saturating_duration_since(Instant::now()));
            let wait = match (remaining, self.escape_since) {
                (Some(t), Some(since)) => {
                    Some(t.min(Duration::from_millis(30).saturating_sub(since.elapsed())))
                }
                (None, Some(since)) => {
                    Some(Duration::from_millis(30).saturating_sub(since.elapsed()))
                }
                (other, None) => other,
            };
            let wait = if let Some(since) = self.sequence_since {
                Some(
                    wait.unwrap_or(Duration::from_millis(500))
                        .min(Duration::from_millis(500).saturating_sub(since.elapsed())),
                )
            } else {
                wait
            };
            match self.backend.wait(wait)? {
                #[cfg(unix)]
                Wake::Input => {
                    let mut bytes = [0; 4096];
                    let n = self.backend.read(&mut bytes)?;
                    if n == 0 {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "terminal input closed",
                        ));
                    }
                    self.parser.feed(&bytes[..n]);
                    continue;
                }
                Wake::Resize => {
                    let size = self.backend.size()?;
                    if size != self.back.size {
                        self.back.resize(size);
                        self.back.clear_with_style(self.clear_style);
                        self.front = self.back.cells.clone();
                        self.invalid = true;
                        return Ok(Some(Event::Resize(size)));
                    }
                }
                #[cfg(windows)]
                Wake::Native => {
                    if let Some(event) = self.backend.take_event() {
                        return Ok(Some(event));
                    }
                }
                Wake::Timeout => {}
            }
            if let Some(d) = deadline
                && Instant::now() >= d
            {
                if self.parser.pending_lone_escape()
                    && self
                        .escape_since
                        .is_some_and(|s| s.elapsed() >= Duration::from_millis(30))
                {
                    self.escape_since = None;
                    return Ok(self.parser.finish_escape());
                }
                return Ok(None);
            }
        }
    }
    /// Restore cooked mode and the original screen before running another program.
    pub fn suspend(&mut self) -> io::Result<()> {
        let result = self.backend.suspend();
        self.suspended = !self.backend.is_active();
        result
    }
    /// Re-enter raw alternate-screen mode and request a full redraw.
    pub fn resume(&mut self) -> io::Result<()> {
        if !self.suspended {
            return Ok(());
        }
        self.backend.resume()?;
        self.suspended = false;
        let size = self.backend.size()?;
        self.back.resize(size);
        self.back.clear_with_style(self.clear_style);
        self.front = self.back.cells.clone();
        self.invalid = true;
        if self.mouse_enabled {
            self.backend.set_mouse(true)?;
        }
        Ok(())
    }
    /// Draw changed rows only. A failed write leaves the front buffer invalid.
    pub fn present(&mut self) -> io::Result<()> {
        if self.suspended {
            return Err(io::Error::other("terminal is suspended"));
        }
        let rows = self.back.dirty_rows(&self.front, self.invalid);
        let mut out = Vec::new();
        let width = usize::from(self.back.size.width);
        for y in rows {
            out.extend_from_slice(&self.caps.cursor_position(0, y)?);
            let mut style = None;
            for x in 0..width {
                let slot = &self.back.cells[usize::from(y) * width + x];
                if slot.continuation {
                    continue;
                }
                if style != Some(slot.cell.style) {
                    append_style(&mut out, slot.cell.style, &self.caps);
                    style = Some(slot.cell.style);
                }
                slot.cell.append_to(&mut out);
            }
        }
        out.extend_from_slice(&self.caps.reset);
        append_cursor(&mut out, self.cursor, &self.caps)?;
        self.backend.write(&out)?;
        self.front.clone_from(&self.back.cells);
        self.invalid = false;
        Ok(())
    }
}

fn append_cursor(
    out: &mut Vec<u8>,
    cursor: Option<Position>,
    caps: &Capabilities,
) -> io::Result<()> {
    if let Some(pos) = cursor {
        out.extend_from_slice(&caps.cursor_position(pos.x, pos.y)?);
        out.extend_from_slice(&caps.show_cursor);
    } else {
        out.extend_from_slice(&caps.hide_cursor);
    }
    Ok(())
}
fn append_style(out: &mut Vec<u8>, style: Style, caps: &Capabilities) {
    out.extend_from_slice(&caps.reset);
    let mut color = String::new();
    append_color(&mut color, style.foreground, true);
    append_color(&mut color, style.background, false);
    if !color.is_empty() {
        out.extend_from_slice(b"\x1b[");
        out.extend_from_slice(&color.as_bytes()[1..]);
        out.push(b'm');
    }
    for (active, index) in [
        (style.bold, 0),
        (style.dim, 1),
        (style.underline, 2),
        (style.italic, 3),
        (style.blink, 4),
        (style.reverse, 5),
        (style.invisible, 6),
    ] {
        if active {
            out.extend_from_slice(&caps.style[index]);
        }
    }
    if style.strikeout {
        out.extend_from_slice(b"\x1b[9m");
    }
    if style.double_underline {
        out.extend_from_slice(b"\x1b[21m");
    }
    if style.overline {
        out.extend_from_slice(b"\x1b[53m");
    }
}
fn append_color(out: &mut String, color: Color, foreground: bool) {
    match color {
        Color::Default => {}
        Color::Indexed(i) => {
            if i < 16 {
                let base = if foreground { 30 } else { 40 };
                let code = if i < 8 {
                    base + u32::from(i)
                } else {
                    base + 60 + u32::from(i - 8)
                };
                write!(out, ";{code}").expect("String write");
            } else {
                write!(out, ";{};5;{i}", if foreground { 38 } else { 48 }).expect("String write");
            }
        }
        Color::Rgb(r, g, b) => {
            write!(out, ";{};2;{r};{g};{b}", if foreground { 38 } else { 48 })
                .expect("String write");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn colour_and_attribute_sequences() {
        let mut out = Vec::new();
        let caps = Capabilities::builtin("xterm");
        let style = Style {
            foreground: Color::Indexed(9),
            background: Color::Rgb(1, 2, 3),
            italic: true,
            strikeout: true,
            ..Style::default()
        };
        append_style(&mut out, style, &caps);
        assert_eq!(out, b"\x1b(B\x1b[m\x1b[91;48;2;1;2;3m\x1b[3m\x1b[9m");
        out.clear();
        append_style(
            &mut out,
            Style::new(Color::Indexed(5), Color::Default),
            &caps,
        );
        assert_eq!(out, b"\x1b(B\x1b[m\x1b[35m");
        out.clear();
        append_style(
            &mut out,
            Style::new(Color::Indexed(234), Color::Default),
            &caps,
        );
        assert_eq!(out, b"\x1b(B\x1b[m\x1b[38;5;234m");
    }
    #[test]
    fn cursor_sequence_follows_rendering() {
        let caps = Capabilities::builtin("xterm");
        let mut out = b"\x1b[3;1Hdrawn".to_vec();
        append_cursor(&mut out, Some(Position { x: 10, y: 5 }), &caps).unwrap();
        assert!(out.ends_with(&[b"\x1b[6;11H".as_slice(), caps.show_cursor.as_slice()].concat()));
        let mut hidden = Vec::new();
        append_cursor(&mut hidden, None, &caps).unwrap();
        assert_eq!(hidden, caps.hide_cursor);
    }
}
