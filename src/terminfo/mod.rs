//! Small reader for compiled terminfo's standard string section.
use crate::Key;
mod builtins;
mod expand;
use std::io;
#[cfg(not(windows))]
use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[cfg(any(not(windows), test))]
const KEYS: &[(usize, Key)] = &[
    (66, Key::Function(1)),
    (68, Key::Function(2)),
    (69, Key::Function(3)),
    (70, Key::Function(4)),
    (71, Key::Function(5)),
    (72, Key::Function(6)),
    (73, Key::Function(7)),
    (74, Key::Function(8)),
    (75, Key::Function(9)),
    (67, Key::Function(10)),
    (216, Key::Function(11)),
    (217, Key::Function(12)),
    (77, Key::Insert),
    (59, Key::Delete),
    (76, Key::Home),
    (164, Key::End),
    (82, Key::PageUp),
    (81, Key::PageDown),
    (87, Key::Up),
    (61, Key::Down),
    (79, Key::Left),
    (83, Key::Right),
    (148, Key::BackTab),
];

#[derive(Clone)]
pub(crate) struct Capabilities {
    pub keys: Vec<(Vec<u8>, Key)>,
    pub enter_screen: Vec<u8>,
    pub exit_screen: Vec<u8>,
    pub show_cursor: Vec<u8>,
    pub hide_cursor: Vec<u8>,
    pub clear: Vec<u8>,
    cursor_address: Option<Vec<u8>>,
    vt_cursor_fallback: bool,
    pub reset: Vec<u8>,
    #[cfg_attr(windows, allow(dead_code))]
    pub enter_keypad: Vec<u8>,
    #[cfg_attr(windows, allow(dead_code))]
    pub exit_keypad: Vec<u8>,
    /// bold, dim, underline, italic, blink, reverse, invisible
    pub style: [Vec<u8>; 7],
}
impl Capabilities {
    pub fn load() -> io::Result<Self> {
        #[cfg(windows)]
        {
            // The native Windows backend enables VT output; host TERM databases
            // (including Wine's) do not describe that console connection.
            Ok(Self::builtin("xterm"))
        }
        #[cfg(not(windows))]
        {
            let term = env::var("TERM").unwrap_or_default();
            if term.is_empty() {
                return Err(io::Error::new(io::ErrorKind::NotFound, "TERM is not set"));
            }
            let known = ["xterm", "linux", "screen", "tmux", "rxvt", "Eterm"]
                .iter()
                .any(|family| term == *family || term.starts_with(&format!("{family}-")));
            let db = find_entry(&term);
            if !known && db.is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!("no usable capabilities for TERM={term}"),
                ));
            }
            if let Some(db) = db {
                return Ok(Self::from_entry(&db));
            }
            Ok(Self::builtin(&term))
        }
    }
    #[cfg(any(not(windows), test))]
    pub(crate) fn from_entry(db: &[Option<Vec<u8>>]) -> Self {
        let mut result = Self {
            keys: Vec::new(),
            enter_screen: Vec::new(),
            exit_screen: Vec::new(),
            show_cursor: Vec::new(),
            hide_cursor: Vec::new(),
            clear: Vec::new(),
            cursor_address: db.get(10).and_then(|value| value.clone()),
            vt_cursor_fallback: false,
            reset: Vec::new(),
            enter_keypad: Vec::new(),
            exit_keypad: Vec::new(),
            style: std::array::from_fn(|_| Vec::new()),
        };
        for &(index, key) in KEYS {
            if let Some(bytes) = db.get(index).and_then(|v| v.as_ref()) {
                result.keys.push((bytes.clone(), key));
            }
        }
        for (field, index) in [
            (&mut result.enter_screen, 28),
            (&mut result.exit_screen, 40),
            (&mut result.show_cursor, 16),
            (&mut result.hide_cursor, 13),
            (&mut result.clear, 5),
            (&mut result.reset, 39),
            (&mut result.enter_keypad, 89),
            (&mut result.exit_keypad, 88),
        ] {
            if let Some(bytes) = db.get(index).and_then(|v| v.as_ref()) {
                *field = bytes.clone();
            }
        }
        for (slot, index) in [27, 30, 36, 311, 26, 34, 32].into_iter().enumerate() {
            if let Some(bytes) = db.get(index).and_then(|v| v.as_ref()) {
                result.style[slot] = bytes.clone();
            }
        }
        result
    }
    pub(crate) fn builtin(term: &str) -> Self {
        let table = if term.contains("xterm") {
            &builtins::XTERM
        } else if term == "linux" {
            &builtins::LINUX
        } else if term.contains("screen") || term.contains("tmux") {
            &builtins::SCREEN
        } else if term.contains("rxvt-256color") {
            &builtins::RXVT_256COLOR
        } else if term.contains("rxvt") {
            &builtins::RXVT_UNICODE
        } else if term.contains("Eterm") {
            &builtins::ETERM
        } else {
            &builtins::XTERM
        };
        let key = |i| match i {
            0..=11 => Key::Function(i as u8 + 1),
            12 => Key::Insert,
            13 => Key::Delete,
            14 => Key::Home,
            15 => Key::End,
            16 => Key::PageUp,
            17 => Key::PageDown,
            18 => Key::Up,
            19 => Key::Down,
            20 => Key::Left,
            21 => Key::Right,
            _ => Key::BackTab,
        };
        let keys = (0..23)
            .filter(|&i| !table[i].is_empty())
            .map(|i| (table[i].to_vec(), key(i)))
            .collect();
        Self {
            keys,
            enter_screen: table[23].to_vec(),
            exit_screen: table[24].to_vec(),
            show_cursor: table[25].to_vec(),
            hide_cursor: table[26].to_vec(),
            clear: table[27].to_vec(),
            cursor_address: None,
            vt_cursor_fallback: true,
            reset: table[28].to_vec(),
            enter_keypad: table[34].to_vec(),
            exit_keypad: table[35].to_vec(),
            style: [
                table[30].to_vec(),
                table[36].to_vec(),
                table[29].to_vec(),
                table[32].to_vec(),
                table[31].to_vec(),
                table[33].to_vec(),
                table[37].to_vec(),
            ],
        }
    }
    pub fn cursor_position(&self, x: u16, y: u16) -> io::Result<Vec<u8>> {
        if let Some(template) = &self.cursor_address {
            return expand::expand(template, [i32::from(y), i32::from(x)])
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
        }
        if self.vt_cursor_fallback {
            return Ok(format!("\x1b[{};{}H", u32::from(y) + 1, u32::from(x) + 1).into_bytes());
        }
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cursor positioning capability is absent",
        ))
    }
}
#[cfg(not(windows))]
fn find_entry(term: &str) -> Option<Vec<Option<Vec<u8>>>> {
    if term.is_empty()
        || term.len() > 128
        || !term
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
    {
        return None;
    }
    let mut dirs = Vec::<PathBuf>::new();
    if let Some(path) = env::var_os("TERMINFO") {
        dirs.push(path.into());
    }
    if let Some(home) = env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".terminfo"));
    }
    if let Some(paths) = env::var_os("TERMINFO_DIRS") {
        for path in env::split_paths(&paths) {
            if path.as_os_str().is_empty() {
                dirs.push(PathBuf::from("/usr/share/terminfo"));
                dirs.push(PathBuf::from("/usr/lib/terminfo"));
            } else {
                dirs.push(path);
            }
        }
    }
    #[cfg(target_os = "android")]
    if let Some(prefix) = env::var_os("PREFIX") {
        dirs.push(PathBuf::from(prefix).join("share/terminfo"));
    }
    for path in [
        "/usr/local/etc/terminfo",
        "/usr/local/share/terminfo",
        "/usr/local/lib/terminfo",
        "/etc/terminfo",
        "/usr/share/terminfo",
        "/usr/lib/terminfo",
        "/usr/share/lib/terminfo",
        "/lib/terminfo",
    ] {
        dirs.push(path.into());
    }
    for dir in dirs {
        let first = term.as_bytes()[0];
        for subdir in [
            (first as char).to_string(),
            format!("{first:x}"),
            format!("{first:02x}"),
        ] {
            let path = dir.join(subdir).join(term);
            if let Ok(bytes) = read_limited(&path)
                && let Some(entry) = parse_entry(&bytes)
            {
                return Some(entry);
            }
        }
    }
    None
}
#[cfg(not(windows))]
fn read_limited(path: &Path) -> std::io::Result<Vec<u8>> {
    if fs::metadata(path)?.len() > 1024 * 1024 {
        return Err(std::io::Error::other("terminfo entry too large"));
    }
    fs::read(path)
}
#[cfg(any(not(windows), test))]
pub(crate) fn parse_entry(bytes: &[u8]) -> Option<Vec<Option<Vec<u8>>>> {
    fn n(bytes: &[u8], off: usize) -> Option<usize> {
        Some(usize::from(u16::from_le_bytes(
            bytes.get(off..off + 2)?.try_into().ok()?,
        )))
    }
    let magic = n(bytes, 0)?;
    let int_width = match magic {
        0x11a => 2,
        0x21e => 4,
        _ => return None,
    };
    let names = n(bytes, 2)?;
    let bools = n(bytes, 4)?;
    let ints = n(bytes, 6)?;
    let count = n(bytes, 8)?;
    let strings = n(bytes, 10)?;
    let offsets_at = 12usize
        .checked_add(names)?
        .checked_add(bools)?
        .checked_add((names + bools) & 1)?
        .checked_add(ints.checked_mul(int_width)?)?;
    let table_at = offsets_at.checked_add(count.checked_mul(2)?)?;
    let end = table_at.checked_add(strings)?;
    if end > bytes.len() || count > 4096 {
        return None;
    }
    let mut result = Vec::with_capacity(count);
    for i in 0..count {
        let off = n(bytes, offsets_at + i * 2)?;
        if off == u16::MAX as usize || off == u16::MAX as usize - 1 {
            result.push(None);
            continue;
        }
        let start = table_at.checked_add(off)?;
        if start >= end {
            result.push(None);
            continue;
        }
        let len = bytes[start..end].iter().position(|b| *b == 0)?;
        result.push(Some(bytes[start..start + len].to_vec()));
    }
    Some(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn decode_fixture(hex: &str) -> Vec<u8> {
        let digits: Vec<u8> = hex.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
        digits
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }
    #[test]
    fn parses_binary_entry() {
        // Header: 0 names, 0 bools, 0 nums, 2 strings, 4 string bytes.
        let bytes = [
            0x1a, 0x01, 0, 0, 0, 0, 0, 0, 2, 0, 4, 0, 0, 0, 2, 0, b'a', 0, b'b', 0,
        ];
        let caps = parse_entry(&bytes).unwrap();
        assert_eq!(caps[0], Some(b"a".to_vec()));
        assert_eq!(caps[1], Some(b"b".to_vec()));
    }
    #[test]
    fn fixture_missing_capabilities_stay_missing() {
        let bytes = decode_fixture(include_str!("../../tests/fixtures/minimal-terminfo.hex"));
        let entry = parse_entry(&bytes).unwrap();
        let caps = Capabilities::from_entry(&entry);
        assert_eq!(caps.clear, b"\x1b[2J");
        assert!(caps.show_cursor.is_empty());
        assert!(caps.reset.is_empty());
    }
    #[test]
    fn cursor_capability_fixture_expands_row_and_column() {
        let bytes = decode_fixture(include_str!("../../tests/fixtures/cursor-terminfo.hex"));
        let entry = parse_entry(&bytes).unwrap();
        let caps = Capabilities::from_entry(&entry);
        assert_eq!(caps.cursor_position(9, 4).unwrap(), b"\x1b[5;10H");
    }
    #[test]
    fn rejects_corruption() {
        assert!(parse_entry(&[0; 12]).is_none());
    }
    #[test]
    fn valid_entry_does_not_inherit_builtin_capabilities() {
        let mut entry = vec![None; 40];
        entry[5] = Some(b"clear".to_vec());
        let caps = Capabilities::from_entry(&entry);
        assert_eq!(caps.clear, b"clear");
        assert!(caps.enter_screen.is_empty());
        assert!(caps.show_cursor.is_empty());
        assert!(caps.keys.is_empty());
    }
    #[test]
    fn arbitrary_terminfo_bytes_do_not_panic() {
        let mut state = 0x517cc1b727220a95u64;
        for len in 0..512 {
            let mut bytes = vec![0; len];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            let _ = parse_entry(&bytes);
        }
    }
    #[test]
    fn builtin_families_use_distinct_reference_sequences() {
        assert!(Capabilities::builtin("linux").enter_screen.is_empty());
        assert!(
            Capabilities::builtin("tmux-256color")
                .keys
                .iter()
                .any(|(s, k)| s == b"\x1bOP" && *k == Key::Function(1))
        );
        assert!(
            Capabilities::builtin("rxvt-unicode")
                .keys
                .iter()
                .any(|(s, k)| s == b"\x1b[11~" && *k == Key::Function(1))
        );
        assert!(
            !Capabilities::builtin("Eterm")
                .keys
                .iter()
                .any(|(_, k)| *k == Key::BackTab)
        );
    }
}
