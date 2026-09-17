use super::Wake;
use crate::key::control_byte;
use crate::terminfo::Capabilities;
use crate::{Event, Key, KeyEvent, Modifiers, MouseButton, MouseEvent, MouseKind, Position, Size};
use std::io;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::Storage::FileSystem::WriteFile;
use windows_sys::Win32::System::Console::*;
use windows_sys::Win32::System::Threading::WaitForSingleObject;

pub(crate) struct Backend {
    input: HANDLE,
    output: HANDLE,
    input_mode: u32,
    output_mode: u32,
    input_cp: u32,
    output_cp: u32,
    cursor: CONSOLE_CURSOR_INFO,
    active: bool,
    mouse_enabled: bool,
    pending: Option<Event>,
    repeated: Option<(Event, u16)>,
    high_surrogate: Option<u16>,
    caps: Capabilities,
}
impl Backend {
    pub fn new(caps: Capabilities) -> io::Result<Self> {
        // SAFETY: process standard handles and console queries have valid output pointers.
        unsafe {
            let input = GetStdHandle(STD_INPUT_HANDLE);
            let output = GetStdHandle(STD_OUTPUT_HANDLE);
            if input.is_null()
                || output.is_null()
                || input == INVALID_HANDLE_VALUE
                || output == INVALID_HANDLE_VALUE
            {
                return Err(io::Error::other("a Windows console is required"));
            }
            let (mut input_mode, mut output_mode) = (0, 0);
            if GetConsoleMode(input, &mut input_mode) == 0
                || GetConsoleMode(output, &mut output_mode) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let mut cursor = CONSOLE_CURSOR_INFO::default();
            if GetConsoleCursorInfo(output, &mut cursor) == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut backend = Self {
                input,
                output,
                input_mode,
                output_mode,
                input_cp: GetConsoleCP(),
                output_cp: GetConsoleOutputCP(),
                cursor,
                active: false,
                mouse_enabled: false,
                pending: None,
                repeated: None,
                high_surrogate: None,
                caps,
            };
            backend.resume()?;
            Ok(backend)
        }
    }
    pub fn is_active(&self) -> bool {
        self.active
    }
    pub fn size(&self) -> io::Result<Size> {
        let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
        // SAFETY: info is writable and output is a valid console handle.
        if unsafe { GetConsoleScreenBufferInfo(self.output, &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Size {
            width: (i32::from(info.srWindow.Right) - i32::from(info.srWindow.Left) + 1).max(0)
                as u16,
            height: (i32::from(info.srWindow.Bottom) - i32::from(info.srWindow.Top) + 1).max(0)
                as u16,
        })
    }
    pub fn wait(&mut self, timeout: Option<Duration>) -> io::Result<Wake> {
        if let Some((event, left)) = self.repeated.take() {
            self.pending = Some(event.clone());
            if left > 1 {
                self.repeated = Some((event, left - 1));
            }
            return Ok(Wake::Native);
        }
        let deadline = timeout.and_then(|d| Instant::now().checked_add(d));
        loop {
            let ms = deadline
                .map(|d| {
                    d.saturating_duration_since(Instant::now())
                        .as_millis()
                        .min(u32::MAX as u128 - 1) as u32
                })
                .unwrap_or(u32::MAX);
            // SAFETY: input is a valid waitable console input handle.
            match unsafe { WaitForSingleObject(self.input, ms) } {
                WAIT_TIMEOUT => return Ok(Wake::Timeout),
                WAIT_OBJECT_0 => {}
                _ => return Err(io::Error::last_os_error()),
            }
            let mut record = INPUT_RECORD::default();
            let mut read = 0;
            // SAFETY: record and read are writable, input is a console handle.
            if unsafe { ReadConsoleInputW(self.input, &mut record, 1, &mut read) } == 0 {
                return Err(io::Error::last_os_error());
            }
            if read != 1 {
                continue;
            }
            match u32::from(record.EventType) {
                KEY_EVENT => {
                    // SAFETY: EventType identifies the active union member.
                    let key = unsafe { record.Event.KeyEvent };
                    if key.bKeyDown == 0 {
                        continue;
                    }
                    if let Some(event) = self.key_event(key) {
                        if key.wRepeatCount > 1 {
                            self.repeated = Some((event.clone(), key.wRepeatCount - 1));
                        }
                        self.pending = Some(event);
                        return Ok(Wake::Native);
                    }
                }
                WINDOW_BUFFER_SIZE_EVENT => return Ok(Wake::Resize),
                MOUSE_EVENT if self.mouse_enabled => {
                    // SAFETY: EventType identifies the active union member.
                    let mouse = unsafe { record.Event.MouseEvent };
                    self.pending = Some(Event::Mouse(mouse_event(mouse)));
                    return Ok(Wake::Native);
                }
                _ => {}
            }
            if deadline.is_some_and(|d| Instant::now() >= d) {
                return Ok(Wake::Timeout);
            }
        }
    }
    fn key_event(&mut self, rec: KEY_EVENT_RECORD) -> Option<Event> {
        let mut modifiers = win_modifiers(rec.dwControlKeyState);
        let vk = rec.wVirtualKeyCode;
        let key = if (0x70..=0x7b).contains(&vk) {
            Some(Key::Function((vk - 0x70 + 1) as u8))
        } else {
            match vk {
                0x2d => Some(Key::Insert),
                0x2e => Some(Key::Delete),
                0x24 => Some(Key::Home),
                0x23 => Some(Key::End),
                0x21 => Some(Key::PageUp),
                0x22 => Some(Key::PageDown),
                0x26 => Some(Key::Up),
                0x28 => Some(Key::Down),
                0x25 => Some(Key::Left),
                0x27 => Some(Key::Right),
                0x1b => Some(Key::Escape),
                0x0d => Some(Key::Enter),
                0x08 => Some(Key::Backspace),
                0x09 => Some(if modifiers.contains(Modifiers::SHIFT) {
                    Key::BackTab
                } else {
                    Key::Tab
                }),
                _ => None,
            }
        };
        if let Some(key) = key {
            if matches!(key, Key::Tab | Key::Enter | Key::Backspace | Key::Escape) {
                modifiers = Modifiers::NONE;
            }
            return Some(Event::Key(KeyEvent::new(key, modifiers)));
        }
        // SAFETY: KEY_EVENT_RECORD's UnicodeChar is the active union field for W calls.
        let unit = unsafe { rec.uChar.UnicodeChar };
        if (0xd800..=0xdbff).contains(&unit) {
            self.high_surrogate = Some(unit);
            return None;
        }
        let ch = if (0xdc00..=0xdfff).contains(&unit) {
            let high = self.high_surrogate.take()?;
            char::from_u32(0x10000 + ((u32::from(high) - 0xd800) << 10) + u32::from(unit) - 0xdc00)?
        } else {
            self.high_surrogate = None;
            char::from_u32(u32::from(unit))?
        };
        if ch == '\0' {
            return None;
        }
        if (ch as u32) < 32 {
            let event = control_byte(ch as u8);
            return Some(Event::Key(event));
        }
        Some(Event::Key(KeyEvent::new(Key::Char(ch), modifiers)))
    }
    pub fn take_event(&mut self) -> Option<Event> {
        self.pending.take()
    }
    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
        let mut rest = bytes;
        while !rest.is_empty() {
            let mut written = 0;
            let len = rest.len().min(16384) as u32;
            // SAFETY: output is a valid console handle and rest is readable for len bytes.
            if unsafe {
                WriteFile(
                    self.output,
                    rest.as_ptr(),
                    len,
                    &mut written,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            if written == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "console write returned zero",
                ));
            }
            rest = &rest[written as usize..];
        }
        Ok(())
    }
    pub fn suspend(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        let mut seq = b"\x1b[?2004l\x1b[?1006l\x1b[?1002l\x1b[?1000l".to_vec();
        seq.extend_from_slice(&self.caps.reset);
        seq.extend_from_slice(&self.caps.show_cursor);
        seq.extend_from_slice(&self.caps.exit_screen);
        let output = self.write(&seq);
        // SAFETY: restore modes/code pages/cursor saved at construction.
        let restored = unsafe {
            let a = SetConsoleMode(self.input, self.input_mode);
            let b = SetConsoleMode(self.output, self.output_mode);
            let c = SetConsoleCP(self.input_cp);
            let d = SetConsoleOutputCP(self.output_cp);
            let e = SetConsoleCursorInfo(self.output, &self.cursor);
            a != 0 && b != 0 && c != 0 && d != 0 && e != 0
        };
        if restored {
            self.active = false;
        }
        output?;
        if !restored {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    pub fn resume(&mut self) -> io::Result<()> {
        if self.active {
            return Ok(());
        }
        self.active = true;
        // SAFETY: standard console handles are valid; restoration is attempted on any failure.
        let ok = unsafe {
            SetConsoleCP(65001) != 0
                && SetConsoleOutputCP(65001) != 0
                && SetConsoleMode(
                    self.input,
                    (self.input_mode
                        | ENABLE_WINDOW_INPUT
                        | ENABLE_MOUSE_INPUT
                        | ENABLE_EXTENDED_FLAGS)
                        & !(ENABLE_ECHO_INPUT
                            | ENABLE_LINE_INPUT
                            | ENABLE_PROCESSED_INPUT
                            | ENABLE_QUICK_EDIT_MODE),
                ) != 0
                && SetConsoleMode(
                    self.output,
                    self.output_mode
                        | ENABLE_PROCESSED_OUTPUT
                        | ENABLE_VIRTUAL_TERMINAL_PROCESSING
                        | DISABLE_NEWLINE_AUTO_RETURN,
                ) != 0
        };
        if !ok {
            let e = io::Error::last_os_error();
            let _ = self.suspend();
            return Err(e);
        }
        let mut seq = self.caps.enter_screen.clone();
        seq.extend_from_slice(&self.caps.hide_cursor);
        seq.extend_from_slice(b"\x1b[?2004h");
        seq.extend_from_slice(&self.caps.reset);
        if let Err(e) = self.write(&seq) {
            let _ = self.suspend();
            return Err(e);
        }
        Ok(())
    }
    pub fn set_mouse(&mut self, enabled: bool) -> io::Result<()> {
        self.mouse_enabled = enabled;
        Ok(())
    }
    pub fn clear_screen(&self) -> io::Result<()> {
        self.write(&self.caps.clear)
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        let _ = self.suspend();
    }
}
fn win_modifiers(state: u32) -> Modifiers {
    let mut mods = Modifiers::NONE;
    if state & SHIFT_PRESSED != 0 {
        mods = mods | Modifiers::SHIFT;
    }
    if state & (LEFT_ALT_PRESSED | RIGHT_ALT_PRESSED) != 0 {
        mods = mods | Modifiers::ALT;
    }
    if state & (LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED) != 0 {
        mods = mods | Modifiers::CTRL;
    }
    mods
}
fn mouse_event(rec: MOUSE_EVENT_RECORD) -> MouseEvent {
    let button = if rec.dwEventFlags == MOUSE_WHEELED {
        if (rec.dwButtonState >> 16) as i16 > 0 {
            MouseButton::WheelUp
        } else {
            MouseButton::WheelDown
        }
    } else if rec.dwButtonState & FROM_LEFT_1ST_BUTTON_PRESSED != 0 {
        MouseButton::Left
    } else if rec.dwButtonState & FROM_LEFT_2ND_BUTTON_PRESSED != 0 {
        MouseButton::Middle
    } else if rec.dwButtonState & RIGHTMOST_BUTTON_PRESSED != 0 {
        MouseButton::Right
    } else {
        MouseButton::Other(3)
    };
    let kind = if rec.dwEventFlags == MOUSE_WHEELED {
        MouseKind::Scroll
    } else if rec.dwEventFlags == MOUSE_MOVED {
        MouseKind::Move
    } else if rec.dwButtonState == 0 {
        MouseKind::Release
    } else {
        MouseKind::Press
    };
    MouseEvent {
        position: Position {
            x: rec.dwMousePosition.X.max(0) as u16,
            y: rec.dwMousePosition.Y.max(0) as u16,
        },
        button,
        kind,
        modifiers: win_modifiers(rec.dwControlKeyState),
    }
}
