use crate::key::control_byte;
use crate::{Key, KeyEvent, Modifiers, Position, Size};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputMode {
    #[default]
    Escape,
    Alt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    /// Motion or a legacy release with no identified button.
    None,
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    WheelLeft,
    WheelRight,
    Back,
    Forward,
    /// A button without a standard semantic identity.
    Other(u8),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKind {
    Press,
    Release,
    Move,
    Scroll,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseEvent {
    pub position: Position,
    /// Scroll direction, the changed/held button, or `MouseButton::None`.
    /// Each scroll event represents one notch. Native Windows partial wheel
    /// deltas accumulate per axis; larger deltas emit multiple events.
    pub button: MouseButton,
    pub kind: MouseKind,
    pub modifiers: Modifiers,
}
/// Paste data is emitted in chunks of at most 1 MiB. `complete` marks the
/// chunk terminated by the closing delimiter. Bytes preserve invalid UTF-8.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasteEvent {
    pub bytes: Vec<u8>,
    pub complete: bool,
}
impl PasteEvent {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Key(KeyEvent),
    Resize(Size),
    Mouse(MouseEvent),
    Paste(PasteEvent),
    /// An unknown, malformed, or timed-out sequence. Oversized sequences are
    /// emitted in chunks of at most 64 bytes, never as ordinary text.
    UnknownSequence(Vec<u8>),
}

const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";
const MAX_PASTE_CHUNK: usize = 1024 * 1024;
const MAX_SEQUENCE: usize = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ParseState {
    #[default]
    Ground,
    Escape,
    Csi,
    Ss3,
    Registered,
    DiscardSequence,
    Paste,
}

enum RegisteredMatch {
    Complete(Key, usize),
    CompletePrefix(Key, usize),
    Conflict(usize),
    Prefix,
    None,
}

/// Incremental parser. Only a *lone* ESC is timing-sensitive. Once `[` or `O`
/// follows, the sequence remains pending across arbitrary read boundaries.
/// Unknown or oversized sequences consume bytes and always make progress.
#[derive(Default)]
pub struct InputParser {
    bytes: VecDeque<u8>,
    paste: Vec<u8>,
    state: ParseState,
    mode: InputMode,
    key_caps: Vec<(Vec<u8>, Key)>,
    registered_prefix_len: usize,
}

impl InputParser {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set_mode(&mut self, mode: InputMode) {
        self.mode = mode;
    }
    pub fn mode(&self) -> InputMode {
        self.mode
    }
    /// Register an escape sequence of at most 64 bytes. Longest matches win;
    /// ambiguous complete prefixes wait for more input or an explicit finish.
    /// Conflicting identical registrations are treated as unknown, independent
    /// of registration order. Repeating the same registration is harmless.
    pub fn add_key_sequence(&mut self, bytes: &[u8], key: Key) {
        if !bytes.is_empty() && bytes.len() <= MAX_SEQUENCE {
            self.key_caps.push((bytes.to_vec(), key));
            self.key_caps.sort_by_key(|b| std::cmp::Reverse(b.0.len()));
        }
    }
    pub fn feed(&mut self, bytes: &[u8]) {
        self.bytes.extend(bytes);
    }
    pub fn pending_lone_escape(&self) -> bool {
        self.state == ParseState::Escape && self.bytes.len() == 1 && self.bytes[0] == 0x1b
    }
    pub fn pending_sequence(&self) -> bool {
        matches!(
            self.state,
            ParseState::Csi
                | ParseState::Ss3
                | ParseState::Registered
                | ParseState::DiscardSequence
        )
    }
    pub fn finish_incomplete_sequence(&mut self) -> Option<Event> {
        if !self.pending_sequence() {
            return None;
        }
        if let RegisteredMatch::Complete(key, len) | RegisteredMatch::CompletePrefix(key, len) =
            self.match_registered()
        {
            self.drain(len);
            self.state = ParseState::Ground;
            self.registered_prefix_len = 0;
            return Some(Event::Key(KeyEvent::new(key, Modifiers::NONE)));
        }
        self.state = ParseState::Ground;
        let count = if self.registered_prefix_len > 0 {
            self.registered_prefix_len.min(self.bytes.len())
        } else {
            self.bytes.len().min(MAX_SEQUENCE)
        };
        self.registered_prefix_len = 0;
        (count > 0).then(|| Event::UnknownSequence(self.bytes.drain(..count).collect()))
    }
    /// Resolve a lone ESC after the caller's inter-byte grace period.
    pub fn finish_escape(&mut self) -> Option<Event> {
        if !self.pending_lone_escape() {
            return None;
        }
        self.state = ParseState::Ground;
        self.bytes.pop_front();
        Some(Event::Key(KeyEvent::new(Key::Escape, Modifiers::NONE)))
    }
    pub fn next_event(&mut self) -> Option<Event> {
        loop {
            if self.state == ParseState::DiscardSequence {
                if let Some(event) = self.discard_sequence() {
                    return Some(event);
                }
                if self.state == ParseState::DiscardSequence {
                    return None;
                }
                continue;
            }
            if self.state == ParseState::Paste {
                return self.parse_paste();
            }
            let b = *self.bytes.front()?;
            if b == 0x1b {
                if self.bytes.len() == 1 {
                    self.state = ParseState::Escape;
                    return None;
                }
                if self.prefix(PASTE_START) {
                    self.drain(PASTE_START.len());
                    self.state = ParseState::Paste;
                    continue;
                }
                if PASTE_START.starts_with(&self.available()) {
                    self.state = ParseState::Csi;
                    return None;
                }
                match self.match_registered() {
                    RegisteredMatch::Conflict(len) => {
                        self.state = ParseState::Ground;
                        self.registered_prefix_len = 0;
                        return Some(Event::UnknownSequence(self.bytes.drain(..len).collect()));
                    }
                    RegisteredMatch::Complete(key, len) => {
                        self.drain(len);
                        self.registered_prefix_len = 0;
                        self.state = ParseState::Ground;
                        return Some(Event::Key(KeyEvent::new(key, Modifiers::NONE)));
                    }
                    RegisteredMatch::Prefix | RegisteredMatch::CompletePrefix(_, _) => {
                        self.registered_prefix_len = self.bytes.len();
                        self.state = ParseState::Registered;
                        return None;
                    }
                    RegisteredMatch::None if self.state == ParseState::Registered => {
                        let bytes = self.bytes.drain(..self.registered_prefix_len).collect();
                        self.registered_prefix_len = 0;
                        self.state = ParseState::Ground;
                        return Some(Event::UnknownSequence(bytes));
                    }
                    RegisteredMatch::None => {}
                }
                if self.bytes[1] == b'[' || self.bytes[1] == b'O' {
                    self.state = if self.bytes[1] == b'[' {
                        ParseState::Csi
                    } else {
                        ParseState::Ss3
                    };
                    let before = self.bytes.len();
                    if let Some(event) = self.parse_escape_sequence() {
                        if self.state != ParseState::DiscardSequence {
                            self.state = ParseState::Ground;
                        }
                        return Some(event);
                    }
                    if self.bytes.len() < before {
                        self.state = ParseState::Ground;
                        continue;
                    }
                    // Incomplete sequence is left intact; malformed sequences
                    // are consumed inside parse_escape_sequence.
                    if self.bytes.front() == Some(&0x1b) {
                        return None;
                    }
                    continue;
                }
                if self.mode == InputMode::Alt {
                    if !self.utf8_ready(1) {
                        return None;
                    }
                    self.bytes.pop_front();
                    self.state = ParseState::Ground;
                    return self.next_key(Modifiers::ALT);
                }
                self.bytes.pop_front();
                self.state = ParseState::Ground;
                return Some(Event::Key(KeyEvent::new(Key::Escape, Modifiers::NONE)));
            }
            self.state = ParseState::Ground;
            return self.next_key(Modifiers::NONE);
        }
    }
    fn available(&self) -> Vec<u8> {
        self.bytes.iter().take(MAX_SEQUENCE).copied().collect()
    }
    fn prefix(&self, seq: &[u8]) -> bool {
        self.bytes.len() >= seq.len()
            && self
                .bytes
                .iter()
                .take(seq.len())
                .copied()
                .eq(seq.iter().copied())
    }
    fn drain(&mut self, n: usize) {
        self.bytes.drain(..n);
    }
    fn match_registered(&self) -> RegisteredMatch {
        let mut prefix = false;
        let mut complete = None;
        let mut conflict = false;
        for (seq, key) in &self.key_caps {
            if self.bytes.len() >= seq.len()
                && self
                    .bytes
                    .iter()
                    .take(seq.len())
                    .copied()
                    .eq(seq.iter().copied())
            {
                match complete {
                    None => complete = Some((*key, seq.len())),
                    Some((previous, len)) if len == seq.len() && previous != *key => {
                        conflict = true
                    }
                    _ => {}
                }
            }
            if self.bytes.len() < seq.len()
                && self
                    .bytes
                    .iter()
                    .copied()
                    .eq(seq.iter().take(self.bytes.len()).copied())
            {
                prefix = true;
            }
        }
        if conflict {
            return if prefix {
                RegisteredMatch::Prefix
            } else {
                RegisteredMatch::Conflict(complete.expect("conflict requires a match").1)
            };
        }
        if let Some((key, len)) = complete {
            if prefix {
                RegisteredMatch::CompletePrefix(key, len)
            } else {
                RegisteredMatch::Complete(key, len)
            }
        } else if prefix {
            RegisteredMatch::Prefix
        } else {
            RegisteredMatch::None
        }
    }
    fn discard_sequence(&mut self) -> Option<Event> {
        let mut count = 0;
        for &byte in self.bytes.iter().take(MAX_SEQUENCE) {
            if !(0x20..=0x7e).contains(&byte) {
                self.state = ParseState::Ground;
                break; // Keep ESC, controls and UTF-8 input for the next event.
            }
            count += 1;
            if (0x40..=0x7e).contains(&byte) {
                self.state = ParseState::Ground;
                break;
            }
        }
        (count > 0).then(|| Event::UnknownSequence(self.bytes.drain(..count).collect()))
    }
    fn parse_paste(&mut self) -> Option<Event> {
        let bytes: Vec<u8> = self.bytes.iter().copied().collect();
        if let Some(pos) = bytes.windows(PASTE_END.len()).position(|w| w == PASTE_END) {
            let capacity = MAX_PASTE_CHUNK.saturating_sub(self.paste.len());
            if pos > capacity {
                self.paste.extend_from_slice(&bytes[..capacity]);
                self.drain(capacity);
                return Some(Event::Paste(PasteEvent {
                    bytes: std::mem::take(&mut self.paste),
                    complete: false,
                }));
            }
            self.paste.extend_from_slice(&bytes[..pos]);
            self.drain(pos + PASTE_END.len());
            self.state = ParseState::Ground;
            return Some(Event::Paste(PasteEvent {
                bytes: std::mem::take(&mut self.paste),
                complete: true,
            }));
        }
        // Keep the possible prefix of a closing delimiter across reads.
        let keep = (1..PASTE_END.len())
            .rev()
            .find(|&n| bytes.ends_with(&PASTE_END[..n]))
            .unwrap_or(0);
        let take = bytes
            .len()
            .saturating_sub(keep)
            .min(MAX_PASTE_CHUNK.saturating_sub(self.paste.len()));
        self.paste.extend_from_slice(&bytes[..take]);
        self.drain(take);
        if self.paste.len() >= MAX_PASTE_CHUNK {
            return Some(Event::Paste(PasteEvent {
                bytes: std::mem::take(&mut self.paste),
                complete: false,
            }));
        }
        None
    }
    fn parse_escape_sequence(&mut self) -> Option<Event> {
        let is_csi = self.bytes[1] == b'[';
        if is_csi && self.bytes.get(2) == Some(&b'M') {
            if self.bytes.len() < 6 {
                return None;
            }
            let b = self.bytes[3].saturating_sub(32) as u32;
            let x = self.bytes[4].saturating_sub(33) as u16;
            let y = self.bytes[5].saturating_sub(33) as u16;
            self.drain(6);
            return Some(Event::Mouse(mouse(b, x, y, false)));
        }
        // Linux console uses ESC [[ A..E for F1..F5.
        if is_csi && self.bytes.get(2) == Some(&b'[') {
            if self.bytes.len() < 4 {
                return None;
            }
            let key = match self.bytes[3] {
                b'A'..=b'E' => Some(Key::Function(self.bytes[3] - b'A' + 1)),
                _ => None,
            };
            let bytes = self.bytes.drain(..4).collect();
            return Some(match key {
                Some(key) => Event::Key(KeyEvent::new(key, Modifiers::NONE)),
                None => Event::UnknownSequence(bytes),
            });
        }
        let end = (2..self.bytes.len().min(MAX_SEQUENCE + 1))
            .find(|&i| !(0x20..=0x3f).contains(&self.bytes[i]));
        let Some(end) = end else {
            if self.bytes.len() > MAX_SEQUENCE {
                self.state = ParseState::DiscardSequence;
                return Some(Event::UnknownSequence(
                    self.bytes.drain(..MAX_SEQUENCE).collect(),
                ));
            }
            return None;
        };
        if !(0x40..=0x7e).contains(&self.bytes[end]) {
            return Some(Event::UnknownSequence(self.bytes.drain(..end).collect()));
        }
        if end >= MAX_SEQUENCE {
            self.state = ParseState::DiscardSequence;
            return Some(Event::UnknownSequence(
                self.bytes.drain(..MAX_SEQUENCE).collect(),
            ));
        }
        let seq: Vec<u8> = self.bytes.iter().take(end + 1).copied().collect();
        let params = &seq[2..end];
        let final_byte = seq[end];
        self.drain(end + 1);
        if is_csi
            && (final_byte == b'M' || final_byte == b'm')
            && (params.first() == Some(&b'<') || params.contains(&b';'))
        {
            let sgr = params.first() == Some(&b'<');
            let Some(numbers) = parse_numbers(if sgr { &params[1..] } else { params }) else {
                return Some(Event::UnknownSequence(seq));
            };
            if numbers.len() != 3 {
                return Some(Event::UnknownSequence(seq));
            }
            let b = if sgr {
                u32::from(numbers[0])
            } else {
                u32::from(numbers[0].saturating_sub(32))
            };
            return Some(Event::Mouse(mouse(
                b,
                numbers[1].saturating_sub(1),
                numbers[2].saturating_sub(1),
                final_byte == b'm',
            )));
        }
        let numbers = if params.is_empty() {
            Vec::new()
        } else {
            match parse_numbers(params) {
                Some(numbers) => numbers,
                None => return Some(Event::UnknownSequence(seq)),
            }
        };
        let key = if is_csi {
            match final_byte {
                b'A' => Some(Key::Up),
                b'B' => Some(Key::Down),
                b'C' => Some(Key::Right),
                b'D' => Some(Key::Left),
                b'H' => Some(Key::Home),
                b'F' => Some(Key::End),
                b'Z' => Some(Key::BackTab),
                b'P'..=b'S' => Some(Key::Function(final_byte - b'P' + 1)),
                b'~' => match numbers.first().copied() {
                    Some(1 | 7) => Some(Key::Home),
                    Some(4 | 8) => Some(Key::End),
                    Some(2) => Some(Key::Insert),
                    Some(3) => Some(Key::Delete),
                    Some(5) => Some(Key::PageUp),
                    Some(6) => Some(Key::PageDown),
                    Some(11..=15) => Some(Key::Function(numbers[0] as u8 - 10)),
                    Some(17..=21) => Some(Key::Function(numbers[0] as u8 - 11)),
                    Some(23 | 24) => Some(Key::Function(numbers[0] as u8 - 12)),
                    _ => None,
                },
                _ => None,
            }
        } else {
            match final_byte {
                b'P'..=b'S' => Some(Key::Function(final_byte - b'P' + 1)),
                b'A' => Some(Key::Up),
                b'B' => Some(Key::Down),
                b'C' => Some(Key::Right),
                b'D' => Some(Key::Left),
                b'H' => Some(Key::Home),
                b'F' => Some(Key::End),
                _ => None,
            }
        };
        let Some(key) = key else {
            return Some(Event::UnknownSequence(seq));
        };
        let code = numbers.get(1).copied().unwrap_or(1).saturating_sub(1);
        let mut mods = Modifiers::NONE;
        if code & 1 != 0 || key == Key::BackTab {
            mods = mods | Modifiers::SHIFT;
        }
        if code & 2 != 0 {
            mods = mods | Modifiers::ALT;
        }
        if code & 4 != 0 {
            mods = mods | Modifiers::CTRL;
        }
        Some(Event::Key(KeyEvent::new(key, mods)))
    }
    fn utf8_ready(&self, offset: usize) -> bool {
        let Some(&b) = self.bytes.get(offset) else {
            return false;
        };
        let n = match b {
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => 1,
        };
        self.bytes.len() >= offset + n
    }
    fn next_key(&mut self, modifiers: Modifiers) -> Option<Event> {
        let b = *self.bytes.front()?;
        let key = match b {
            0..=0x1f | 0x7f => {
                self.bytes.pop_front();
                let event = control_byte(b);
                return Some(Event::Key(KeyEvent::new(
                    event.key,
                    event.modifiers | modifiers,
                )));
            }
            0x20..=0x7e => {
                self.bytes.pop_front();
                Key::Char(b as char)
            }
            _ => {
                if !self.utf8_ready(0) {
                    return None;
                }
                let n = match b {
                    0xc2..=0xdf => 2,
                    0xe0..=0xef => 3,
                    0xf0..=0xf4 => 4,
                    _ => 1,
                };
                let candidate: Vec<u8> = self.bytes.iter().take(n).copied().collect();
                let ch = std::str::from_utf8(&candidate)
                    .ok()
                    .and_then(|s| s.chars().next())
                    .unwrap_or('\u{fffd}');
                self.drain(if ch == '\u{fffd}' { 1 } else { n });
                Key::Char(ch)
            }
        };
        Some(Event::Key(KeyEvent::new(key, modifiers)))
    }
}
fn parse_numbers(bytes: &[u8]) -> Option<Vec<u16>> {
    if bytes.is_empty() || bytes.iter().any(|b| !b.is_ascii_digit() && *b != b';') {
        return None;
    }
    std::str::from_utf8(bytes)
        .ok()?
        .split(';')
        .map(|s| s.parse().ok())
        .collect()
}
fn mouse(b: u32, x: u16, y: u16, release: bool) -> MouseEvent {
    let button = match b & 3 {
        0 if b & 64 != 0 => MouseButton::WheelUp,
        1 if b & 64 != 0 => MouseButton::WheelDown,
        2 if b & 64 != 0 => MouseButton::WheelLeft,
        3 if b & 64 != 0 => MouseButton::WheelRight,
        0 => MouseButton::Left,
        1 => MouseButton::Middle,
        2 => MouseButton::Right,
        _ => MouseButton::None,
    };
    let kind = if b & 64 != 0 {
        MouseKind::Scroll
    } else if release || b & 3 == 3 {
        MouseKind::Release
    } else if b & 32 != 0 {
        MouseKind::Move
    } else {
        MouseKind::Press
    };
    let mut modifiers = Modifiers::NONE;
    if b & 4 != 0 {
        modifiers = modifiers | Modifiers::SHIFT;
    }
    if b & 8 != 0 {
        modifiers = modifiers | Modifiers::ALT;
    }
    if b & 16 != 0 {
        modifiers = modifiers | Modifiers::CTRL;
    }
    MouseEvent {
        position: Position { x, y },
        button,
        kind,
        modifiers,
    }
}
