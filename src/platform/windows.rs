use super::Wake;
use crate::key::control_byte;
use crate::terminfo::Capabilities;
use crate::{Event, Key, KeyEvent, Modifiers, MouseButton, MouseEvent, MouseKind, Position, Size};
use std::collections::VecDeque;
use std::io;
use std::os::windows::io::{AsRawHandle, BorrowedHandle, OwnedHandle};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::Storage::FileSystem::WriteFile;
use windows_sys::Win32::System::Console::*;
use windows_sys::Win32::System::Threading::WaitForSingleObject;

pub(crate) struct Backend {
    input: HANDLE,
    output: HANDLE,
    // Own duplicates, never close the process standard handles. These outlive Drop::suspend.
    _input_owner: OwnedHandle,
    _output_owner: OwnedHandle,
    _lease: ConsoleLease,
    input_mode: u32,
    output_mode: u32,
    output_cp: u32,
    cursor: CONSOLE_CURSOR_INFO,
    active: bool,
    needs_restore: bool,
    vt_ready: bool,
    ctrl_handler: bool,
    mouse_enabled: bool,
    pending: VecDeque<Event>,
    repeated: Option<(Event, u16)>,
    high_surrogate: Option<(u16, Modifiers, u16)>,
    mouse_buttons: u32,
    caps: Capabilities,
}
// Console modes and code pages are shared even through duplicated handles.
static CONSOLE_OWNED: AtomicBool = AtomicBool::new(false);
#[cfg(test)]
static BREAK_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
thread_local! { static FAIL_AFTER: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) }; }
#[cfg(test)]
fn setup_checkpoint(stage: &'static str) -> io::Result<()> {
    if FAIL_AFTER.with(|failure| failure.get() == Some(stage)) {
        Err(io::Error::other(format!(
            "injected setup failure after {stage}"
        )))
    } else {
        Ok(())
    }
}
// Ctrl+Break is a signal even with processed input disabled. Consume only that
// signal while this terminal owns the handler; no Rust allocation or console
// I/O is performed on Windows' control-handler thread.
unsafe extern "system" fn console_control(kind: u32) -> i32 {
    #[cfg(test)]
    if kind == CTRL_BREAK_EVENT {
        BREAK_COUNT.fetch_add(1, Ordering::SeqCst);
    }
    i32::from(kind == CTRL_BREAK_EVENT)
}
struct ConsoleLease;
impl ConsoleLease {
    fn acquire() -> io::Result<Self> {
        CONSOLE_OWNED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "a Windows terminal is already active",
                )
            })?;
        Ok(Self)
    }
}
impl Drop for ConsoleLease {
    fn drop(&mut self) {
        CONSOLE_OWNED.store(false, Ordering::Release);
    }
}
impl Backend {
    pub fn new(caps: Capabilities) -> io::Result<Self> {
        let lease = ConsoleLease::acquire()?;
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
            let input_owner = BorrowedHandle::borrow_raw(input).try_clone_to_owned()?;
            let output_owner = BorrowedHandle::borrow_raw(output).try_clone_to_owned()?;
            let input_cp = GetConsoleCP();
            let output_cp = GetConsoleOutputCP();
            if input_cp == 0 || output_cp == 0 {
                return Err(io::Error::other("console code pages are unavailable"));
            }
            let mut backend = Self {
                input: input_owner.as_raw_handle(),
                output: output_owner.as_raw_handle(),
                _input_owner: input_owner,
                _output_owner: output_owner,
                _lease: lease,
                input_mode,
                output_mode,
                output_cp,
                cursor,
                active: false,
                needs_restore: false,
                vt_ready: false,
                ctrl_handler: false,
                mouse_enabled: false,
                pending: VecDeque::new(),
                repeated: None,
                high_surrogate: None,
                mouse_buttons: 0,
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
        if !self.pending.is_empty() {
            return Ok(Wake::Native);
        }
        if let Some((event, left)) = self.repeated.take() {
            self.pending.push_back(event.clone());
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
                        .as_nanos()
                        .div_ceil(1_000_000)
                        .min(u32::MAX as u128 - 1) as u32
                })
                .unwrap_or(u32::MAX);
            // SAFETY: input is a valid waitable console input handle.
            match unsafe { WaitForSingleObject(self.input, ms) } {
                WAIT_TIMEOUT => {
                    if deadline.is_none_or(|d| Instant::now() < d) {
                        continue;
                    }
                    return Ok(Wake::Timeout);
                }
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
                    if let Some((event, count)) = self.key_event(key) {
                        if count > 1 {
                            self.repeated = Some((event.clone(), count - 1));
                        }
                        self.pending.push_back(event);
                        return Ok(Wake::Native);
                    }
                }
                WINDOW_BUFFER_SIZE_EVENT => return Ok(Wake::Resize),
                MOUSE_EVENT if self.mouse_enabled => {
                    // SAFETY: EventType identifies the active union member.
                    let mouse = unsafe { record.Event.MouseEvent };
                    let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
                    // SAFETY: output is owned and info is writable.
                    if unsafe { GetConsoleScreenBufferInfo(self.output, &mut info) } == 0 {
                        return Err(io::Error::last_os_error());
                    }
                    self.pending.extend(
                        mouse_events(mouse, &mut self.mouse_buttons, info.srWindow)
                            .into_iter()
                            .map(Event::Mouse),
                    );
                    if !self.pending.is_empty() {
                        return Ok(Wake::Native);
                    }
                }
                _ => {}
            }
            if deadline.is_some_and(|d| Instant::now() >= d) {
                return Ok(Wake::Timeout);
            }
        }
    }
    fn key_event(&mut self, rec: KEY_EVENT_RECORD) -> Option<(Event, u16)> {
        if rec.bKeyDown == 0 || rec.wRepeatCount == 0 {
            return None;
        }
        let modifiers = win_modifiers(rec.dwControlKeyState);
        let mut count = rec.wRepeatCount;
        let vk = rec.wVirtualKeyCode;
        // SAFETY: UnicodeChar is the active member for ReadConsoleInputW records.
        let unit = unsafe { rec.uChar.UnicodeChar };
        let control = (1..=31).contains(&unit)
            || unit == 127
            || (unit == 0
                && modifiers.contains(Modifiers::CTRL)
                && matches!(vk, 0x20 | 0x32 | 0xc0));
        if control && !(vk == 0x09 && modifiers.contains(Modifiers::SHIFT)) {
            self.high_surrogate = None;
            let mut event = control_byte(unit as u8);
            // C0 aliases have the same logical meaning on every input source.
            // An independently reported Alt modifier is still meaningful.
            if modifiers.contains(Modifiers::ALT) {
                event.modifiers = event.modifiers | Modifiers::ALT;
            }
            return Some((Event::Key(event), count));
        }
        let key = if (0x70..=0x87).contains(&vk) {
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
            self.high_surrogate = None;
            return Some((Event::Key(KeyEvent::new(key, modifiers)), count));
        }
        if (0xd800..=0xdbff).contains(&unit) {
            self.high_surrogate = Some((unit, modifiers, count));
            return None;
        }
        let ch = if (0xdc00..=0xdfff).contains(&unit) {
            let (high, high_modifiers, high_count) = self.high_surrogate.take()?;
            if high_modifiers != modifiers {
                return None;
            }
            count = count.min(high_count);
            char::from_u32(0x10000 + ((u32::from(high) - 0xd800) << 10) + u32::from(unit) - 0xdc00)?
        } else {
            self.high_surrogate = None;
            char::from_u32(u32::from(unit))?
        };
        if ch == '\0' {
            return None;
        }
        Some((Event::Key(KeyEvent::new(Key::Char(ch), modifiers)), count))
    }
    pub fn take_event(&mut self) -> Option<Event> {
        self.pending.pop_front()
    }
    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
        let utf8 = std::str::from_utf8(bytes).is_ok();
        let mut rest = bytes;
        while !rest.is_empty() {
            let mut written = 0;
            let len = output_chunk_len(rest, utf8) as u32;
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
        if !self.needs_restore {
            return Ok(());
        }
        // A partial restoration is not an active terminal. Retain a separate
        // cleanup flag so Drop and a later resume can retry failed API calls.
        self.active = false;
        let mut seq = self.caps.reset.clone();
        seq.extend_from_slice(&self.caps.show_cursor);
        seq.extend_from_slice(&self.caps.exit_screen);
        // A failed mode setup must not print escape sequences in cooked mode.
        let output = if self.vt_ready {
            self.write(&seq)
        } else {
            Ok(())
        };
        self.vt_ready = false;
        let mut restore_error = None;
        let mut check = |result| {
            if result == 0 {
                let error = io::Error::last_os_error();
                if restore_error.is_none() {
                    restore_error = Some(error);
                }
            }
        };
        // SAFETY: restore modes/code pages/cursor saved at construction.
        unsafe {
            check(SetConsoleMode(self.input, self.input_mode));
            check(SetConsoleMode(self.output, self.output_mode));
            check(SetConsoleOutputCP(self.output_cp));
            check(SetConsoleCursorInfo(self.output, &self.cursor));
            if self.ctrl_handler {
                let removed = SetConsoleCtrlHandler(Some(console_control), 0);
                check(removed);
                if removed != 0 {
                    self.ctrl_handler = false;
                }
            }
        }
        if restore_error.is_none() {
            self.needs_restore = false;
            self.pending.clear();
            self.repeated = None;
            self.high_surrogate = None;
            self.mouse_buttons = 0;
        }
        output?;
        if let Some(error) = restore_error {
            return Err(error);
        }
        Ok(())
    }
    pub fn resume(&mut self) -> io::Result<()> {
        if self.active {
            return Ok(());
        }
        if self.needs_restore {
            self.suspend()?;
        }
        self.active = true;
        self.needs_restore = true;
        // SAFETY: the callback has the Win32 ABI and a static lifetime.
        if unsafe { SetConsoleCtrlHandler(Some(console_control), 1) } == 0 {
            self.active = false;
            self.needs_restore = false;
            return Err(io::Error::last_os_error());
        }
        self.ctrl_handler = true;
        let setup = (|| {
            #[cfg(test)]
            setup_checkpoint("handles")?;
            // SAFETY: UTF-8 is a valid output code page for the attached console.
            win32_result(unsafe { SetConsoleOutputCP(65001) })?;
            #[cfg(test)]
            setup_checkpoint("code_page")?;
            // SAFETY: input is an owned console handle; all flags are input flags.
            win32_result(unsafe {
                SetConsoleMode(
                    self.input,
                    (self.input_mode | ENABLE_WINDOW_INPUT | ENABLE_EXTENDED_FLAGS)
                        & !(ENABLE_ECHO_INPUT
                            | ENABLE_LINE_INPUT
                            | ENABLE_PROCESSED_INPUT
                            | ENABLE_VIRTUAL_TERMINAL_INPUT
                            | ENABLE_MOUSE_INPUT
                            | ENABLE_QUICK_EDIT_MODE),
                )
            })?;
            #[cfg(test)]
            setup_checkpoint("input_mode")?;
            // SAFETY: output is an owned console handle; all flags are output flags.
            win32_result(unsafe {
                SetConsoleMode(
                    self.output,
                    self.output_mode
                        | ENABLE_PROCESSED_OUTPUT
                        | ENABLE_VIRTUAL_TERMINAL_PROCESSING
                        | DISABLE_NEWLINE_AUTO_RETURN,
                )
            })?;
            self.vt_ready = true;
            #[cfg(test)]
            setup_checkpoint("output_mode")?;
            self.set_mouse(self.mouse_enabled)?;
            #[cfg(test)]
            setup_checkpoint("mouse")?;
            self.write(&self.caps.enter_screen)?;
            #[cfg(test)]
            setup_checkpoint("alternate_screen")?;
            self.write(&self.caps.hide_cursor)?;
            #[cfg(test)]
            setup_checkpoint("cursor")?;
            self.write(&self.caps.reset)
        })();
        if let Err(e) = setup {
            let _ = self.suspend();
            return Err(e);
        }
        Ok(())
    }
    pub fn set_mouse(&mut self, enabled: bool) -> io::Result<()> {
        if self.active {
            let mut mode = 0;
            // SAFETY: input is an owned console handle and mode is writable.
            if unsafe { GetConsoleMode(self.input, &mut mode) } == 0 {
                return Err(io::Error::last_os_error());
            }
            mode = if enabled {
                mode | ENABLE_MOUSE_INPUT
            } else {
                mode & !ENABLE_MOUSE_INPUT
            };
            // SAFETY: input is an owned console handle.
            if unsafe { SetConsoleMode(self.input, mode) } == 0 {
                return Err(io::Error::last_os_error());
            }
        }
        self.mouse_buttons = 0;
        self.mouse_enabled = enabled;
        Ok(())
    }
    pub fn clear_screen(&self) -> io::Result<()> {
        self.write(&self.caps.clear)
    }
}
fn win32_result(result: i32) -> io::Result<()> {
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        let _ = self.suspend();
    }
}
fn output_chunk_len(bytes: &[u8], utf8: bool) -> usize {
    let mut len = bytes.len().min(16384);
    if utf8 {
        while len < bytes.len() && bytes[len] & 0xc0 == 0x80 {
            len -= 1;
        }
    }
    len
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
fn mouse_events(
    rec: MOUSE_EVENT_RECORD,
    previous: &mut u32,
    window: SMALL_RECT,
) -> Vec<MouseEvent> {
    let buttons = [
        (FROM_LEFT_1ST_BUTTON_PRESSED, MouseButton::Left),
        (RIGHTMOST_BUTTON_PRESSED, MouseButton::Right),
        (FROM_LEFT_2ND_BUTTON_PRESSED, MouseButton::Middle),
        (FROM_LEFT_3RD_BUTTON_PRESSED, MouseButton::Other(0)),
        (FROM_LEFT_4TH_BUTTON_PRESSED, MouseButton::Other(1)),
    ];
    let current = rec.dwButtonState & 0x1f;
    let changed = *previous ^ current;
    *previous = current;
    let event = |button, kind| MouseEvent {
        position: Position {
            x: (i32::from(rec.dwMousePosition.X) - i32::from(window.Left)).max(0) as u16,
            y: (i32::from(rec.dwMousePosition.Y) - i32::from(window.Top)).max(0) as u16,
        },
        button,
        kind,
        modifiers: win_modifiers(rec.dwControlKeyState),
    };
    if rec.dwEventFlags == MOUSE_WHEELED || rec.dwEventFlags == MOUSE_HWHEELED {
        let delta = (rec.dwButtonState >> 16) as i16;
        if delta == 0 {
            return Vec::new();
        }
        let button = match (rec.dwEventFlags == MOUSE_HWHEELED, delta > 0) {
            (false, true) => MouseButton::WheelUp,
            (false, false) => MouseButton::WheelDown,
            (true, true) => MouseButton::Other(5), // horizontal right
            (true, false) => MouseButton::Other(4), // horizontal left
        };
        return vec![event(button, MouseKind::Scroll)];
    }
    let mut events = Vec::new();
    // Report each changed button, including releases while another remains down.
    for (mask, button) in buttons {
        if changed & mask != 0 {
            events.push(event(
                button,
                if current & mask == 0 {
                    MouseKind::Release
                } else {
                    MouseKind::Press
                },
            ));
        }
    }
    if rec.dwEventFlags == MOUSE_MOVED {
        let button = buttons
            .into_iter()
            .find(|(mask, _)| current & mask != 0)
            .map_or(MouseButton::Other(3), |(_, button)| button);
        events.push(event(button, MouseKind::Move));
    }
    events
}

#[cfg(test)]
mod tests;
