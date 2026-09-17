/// A key reported by the terminal, without application interpretation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Escape,
    Backspace,
    Tab,
    BackTab,
    Up,
    Down,
    Right,
    Left,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    Function(u8),
}

/// Modifier flags; terminals may not report every physical modifier.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers(u8);

impl Modifiers {
    pub const NONE: Self = Self(0);
    pub const SHIFT: Self = Self(1);
    pub const ALT: Self = Self(2);
    pub const CTRL: Self = Self(4);
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for Modifiers {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: Key,
    pub modifiers: Modifiers,
}

impl KeyEvent {
    pub const fn new(key: Key, modifiers: Modifiers) -> Self {
        Self { key, modifiers }
    }
}

/// Logical interpretation of ASCII control bytes shared by terminal backends.
pub(crate) const fn control_byte(byte: u8) -> KeyEvent {
    match byte {
        8 | 127 => KeyEvent::new(Key::Backspace, Modifiers::NONE),
        9 => KeyEvent::new(Key::Tab, Modifiers::NONE),
        10 | 13 => KeyEvent::new(Key::Enter, Modifiers::NONE),
        27 => KeyEvent::new(Key::Escape, Modifiers::NONE),
        0..=31 => KeyEvent::new(
            Key::Char(if byte == 0 { '@' } else { (byte + 64) as char }),
            Modifiers::CTRL,
        ),
        _ => KeyEvent::new(Key::Char(byte as char), Modifiers::NONE),
    }
}
