use super::*;
use std::fs::OpenOptions;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use windows_sys::Win32::System::Threading::{CREATE_NEW_CONSOLE, GetCurrentProcessId};

fn key(vk: u16, unit: u16, state: u32, repeat: u16) -> KEY_EVENT_RECORD {
    KEY_EVENT_RECORD {
        bKeyDown: 1,
        wRepeatCount: repeat,
        wVirtualKeyCode: vk,
        dwControlKeyState: state,
        uChar: KEY_EVENT_RECORD_0 { UnicodeChar: unit },
        ..Default::default()
    }
}

fn key_record(value: KEY_EVENT_RECORD) -> INPUT_RECORD {
    INPUT_RECORD {
        EventType: KEY_EVENT as u16,
        Event: INPUT_RECORD_0 { KeyEvent: value },
    }
}

fn inject(input: HANDLE, records: &[INPUT_RECORD]) {
    let mut count = 0;
    // SAFETY: the slice and count remain valid for this synchronous call.
    assert_ne!(
        unsafe { WriteConsoleInputW(input, records.as_ptr(), records.len() as u32, &mut count) },
        0
    );
    assert_eq!(count as usize, records.len());
}

fn mode(handle: HANDLE) -> u32 {
    let mut mode = 0;
    // SAFETY: test owns the handle and mode is writable.
    assert_ne!(unsafe { GetConsoleMode(handle, &mut mode) }, 0);
    mode
}

fn cursor(handle: HANDLE) -> (u32, i32) {
    let mut cursor = CONSOLE_CURSOR_INFO::default();
    // SAFETY: test owns the handle and cursor is writable.
    assert_ne!(unsafe { GetConsoleCursorInfo(handle, &mut cursor) }, 0);
    (cursor.dwSize, cursor.bVisible)
}

fn next(backend: &mut Backend) -> Event {
    assert_eq!(
        backend.wait(Some(Duration::from_secs(2))).unwrap(),
        Wake::Native
    );
    backend.take_event().unwrap()
}

/// A child process gives these tests an actual Windows console with no shared
/// input queue or modes. CREATE_NEW_CONSOLE allocates a genuine attached console.
/// Set TERRA_TERM_CONSOLE_TEST=attached in a dedicated terminal to test that host.
#[test]
fn native_console() {
    if std::env::var_os("TERRA_TERM_CONSOLE_TEST").is_none() {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "platform::windows::tests::native_console",
                "--nocapture",
            ])
            .env("TERRA_TERM_CONSOLE_TEST", "child")
            .creation_flags(CREATE_NEW_CONSOLE)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "native console child failed: {status}");
                return;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("native console child exceeded 30 seconds");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    let result = std::panic::catch_unwind(console_checks);
    if let Some(path) = std::env::var_os("TERRA_TERM_CONSOLE_REPORT") {
        std::fs::write(path, if result.is_ok() { "PASS\n" } else { "FAIL\n" }).unwrap();
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

fn console_checks() {
    let mut processes = [0u32; 64];
    // SAFETY: the process list is writable and its element count is correct.
    let count = unsafe { GetConsoleProcessList(processes.as_mut_ptr(), processes.len() as u32) };
    assert!(
        count > 0 && count as usize <= processes.len(),
        "no usable attached Windows console: {}",
        io::Error::last_os_error()
    );
    // SAFETY: GetCurrentProcessId takes no pointers or handles.
    assert!(processes[..count as usize].contains(&unsafe { GetCurrentProcessId() }));
    let input = OpenOptions::new()
        .read(true)
        .write(true)
        .open("CONIN$")
        .unwrap();
    let output = OpenOptions::new()
        .read(true)
        .write(true)
        .open("CONOUT$")
        .unwrap();
    let input_handle = input.as_raw_handle();
    let output_handle = output.as_raw_handle();
    // A dedicated console is used, but restore its initial code pages even when
    // an assertion unwinds. Native W input must not require a UTF-8 input page.
    let _pages = CodePageGuard::new();
    // SAFETY: 850 and 437 are valid installed console code pages.
    unsafe {
        assert_ne!(SetConsoleCP(850), 0);
        assert_ne!(SetConsoleOutputCP(437), 0);
    }
    // SAFETY: this is an isolated process. Files remain open throughout the test.
    unsafe {
        assert_ne!(SetStdHandle(STD_INPUT_HANDLE, input_handle), 0);
        assert_ne!(SetStdHandle(STD_OUTPUT_HANDLE, output_handle), 0);
        assert_ne!(FlushConsoleInputBuffer(input_handle), 0);
        // Exercise inherited VT input and Quick Edit instead of only defaults.
        assert_ne!(
            SetConsoleMode(
                input_handle,
                mode(input_handle)
                    | ENABLE_VIRTUAL_TERMINAL_INPUT
                    | ENABLE_QUICK_EDIT_MODE
                    | ENABLE_EXTENDED_FLAGS
            ),
            0
        );
    }
    let before_input = mode(input_handle);
    let before_output = mode(output_handle);
    let before_cursor = cursor(output_handle);
    // SAFETY: console is attached to this process.
    let before_cp = unsafe { (GetConsoleCP(), GetConsoleOutputCP()) };
    let restored = || {
        assert_eq!(mode(input_handle), before_input);
        assert_eq!(mode(output_handle), before_output);
        assert_eq!(cursor(output_handle), before_cursor);
        // SAFETY: console remains attached.
        assert_eq!(unsafe { (GetConsoleCP(), GetConsoleOutputCP()) }, before_cp);
    };
    let caps = Capabilities::load().unwrap();
    let mut backend = Backend::new(caps.clone()).unwrap();
    assert_eq!(
        mode(input_handle)
            & (ENABLE_ECHO_INPUT
                | ENABLE_LINE_INPUT
                | ENABLE_PROCESSED_INPUT
                | ENABLE_QUICK_EDIT_MODE
                | ENABLE_VIRTUAL_TERMINAL_INPUT),
        0
    );
    assert_ne!(mode(output_handle) & ENABLE_VIRTUAL_TERMINAL_PROCESSING, 0);
    assert_eq!(mode(input_handle) & ENABLE_MOUSE_INPUT, 0);
    assert!(backend.size().unwrap().width > 0);
    assert_eq!(
        Backend::new(caps.clone()).err().unwrap().kind(),
        io::ErrorKind::AlreadyExists
    );
    keyboard_checks(&mut backend);
    regressions::keyboard_and_unicode(&mut backend);
    // Alternate-screen entry can generate a native resize record.
    // SAFETY: the dedicated test process owns this console input handle.
    unsafe {
        assert_ne!(FlushConsoleInputBuffer(input_handle), 0);
    }
    regressions::repeats_and_bursts(&mut backend, input_handle);
    // Actual console queue: repeated Unicode input and surrogate pairs in order.
    inject(
        input_handle,
        &[
            key_record(key(0x41, 'a' as u16, 0, 3)),
            key_record(key(0, 0xd83d, 0, 2)),
            key_record(key(0, 0xde00, 0, 2)),
        ],
    );
    for ch in ['a', 'a', 'a', '😀', '😀'] {
        assert_eq!(
            next(&mut backend),
            Event::Key(KeyEvent::new(Key::Char(ch), Modifiers::NONE))
        );
    }
    let mut up = key(0x41, 'a' as u16, 0, 1);
    up.bKeyDown = 0;
    inject(input_handle, &[key_record(up), INPUT_RECORD::default()]);
    assert_eq!(
        backend.wait(Some(Duration::from_millis(10))).unwrap(),
        Wake::Timeout
    );
    let start = Instant::now();
    assert_eq!(
        backend.wait(Some(Duration::from_millis(15))).unwrap(),
        Wake::Timeout
    );
    assert!(start.elapsed() >= Duration::from_millis(15));
    inject(
        input_handle,
        &[INPUT_RECORD {
            EventType: WINDOW_BUFFER_SIZE_EVENT as u16,
            Event: INPUT_RECORD_0 {
                WindowBufferSizeEvent: WINDOW_BUFFER_SIZE_RECORD {
                    dwSize: COORD { X: 80, Y: 25 },
                },
            },
        }],
    );
    assert_eq!(
        backend.wait(Some(Duration::from_secs(1))).unwrap(),
        Wake::Resize
    );
    backend.set_mouse(true).unwrap();
    for (buttons, kind) in [
        (FROM_LEFT_1ST_BUTTON_PRESSED, MouseKind::Press),
        (0, MouseKind::Release),
    ] {
        inject(
            input_handle,
            &[INPUT_RECORD {
                EventType: MOUSE_EVENT as u16,
                Event: INPUT_RECORD_0 {
                    MouseEvent: MOUSE_EVENT_RECORD {
                        dwButtonState: buttons,
                        dwMousePosition: COORD { X: 2, Y: 3 },
                        ..Default::default()
                    },
                },
            }],
        );
        assert!(
            matches!(next(&mut backend), Event::Mouse(e) if e.button == MouseButton::Left && e.kind == kind)
        );
    }
    backend.set_mouse(false).unwrap();
    inject(
        input_handle,
        &[INPUT_RECORD {
            EventType: MOUSE_EVENT as u16,
            Event: INPUT_RECORD_0 {
                MouseEvent: MOUSE_EVENT_RECORD::default(),
            },
        }],
    );
    assert_eq!(
        backend.wait(Some(Duration::from_millis(10))).unwrap(),
        Wake::Timeout
    );

    // Read back UTF-16 screen cells after UTF-8 output, including a chunk boundary.
    backend.write(b"\x1b[2J\x1b[H").unwrap();
    backend.write("Aé中".as_bytes()).unwrap();
    let mut text = [0u16; 6];
    let mut read = 0;
    // SAFETY: text and read are writable, output_handle belongs to this console.
    assert_ne!(
        unsafe {
            ReadConsoleOutputCharacterW(
                backend.output,
                text.as_mut_ptr(),
                6,
                COORD { X: 0, Y: 0 },
                &mut read,
            )
        },
        0
    );
    // The API's length counts cells; wide characters consume two cells.
    assert!(read >= 3);
    let text = [text[0], text[1], text[2]];
    assert_eq!(text, ['A' as u16, 'é' as u16, '中' as u16]);
    backend.write(b"\x1b[H").unwrap();
    let long = format!("{}abc\u{e9}Z", "\x1b[0m".repeat(4095));
    backend.write(long.as_bytes()).unwrap();
    let mut wide = [0u16; 10];
    // SAFETY: the output buffer and count are writable for this call.
    assert_ne!(
        unsafe {
            ReadConsoleOutputCharacterW(
                backend.output,
                wide.as_mut_ptr(),
                wide.len() as u32,
                COORD { X: 0, Y: 0 },
                &mut read,
            )
        },
        0
    );
    assert_eq!(&wide[..5], &[97, 98, 99, 233, 90]);
    backend.suspend().unwrap();
    restored();
    backend.suspend().unwrap();
    backend.resume().unwrap();
    backend.resume().unwrap();
    // Failed restoration must still attempt every other saved setting.
    let saved_input = backend.input;
    backend.input = std::ptr::null_mut();
    assert_eq!(backend.suspend().unwrap_err().raw_os_error(), Some(6));
    assert!(!backend.is_active());
    assert_eq!(mode(output_handle), before_output);
    assert_eq!(cursor(output_handle), before_cursor);
    backend.input = saved_input;
    drop(backend); // retries the failed input restoration
    restored();

    // Exercise the public renderer, cursor, timeout and lifecycle together.
    let mut terminal = crate::Terminal::new().unwrap();
    terminal.put_str(0, 0, "release test", crate::Style::default());
    terminal.set_cursor(4, 2);
    terminal.present().unwrap();
    let mut screen = CONSOLE_SCREEN_BUFFER_INFO::default();
    // SAFETY: screen is writable and this process owns output_handle.
    assert_ne!(
        unsafe { GetConsoleScreenBufferInfo(output_handle, &mut screen) },
        0
    );
    assert_eq!(
        (screen.dwCursorPosition.X, screen.dwCursorPosition.Y),
        (4, 2)
    );
    // SAFETY: discard only setup records from the dedicated test input queue.
    unsafe {
        assert_ne!(FlushConsoleInputBuffer(input_handle), 0);
    }
    inject(
        input_handle,
        &[key_record(key(0x0d, 13, LEFT_CTRL_PRESSED, 1))],
    );
    assert_eq!(
        terminal.read_event_timeout(Duration::from_secs(1)).unwrap(),
        Some(Event::Key(KeyEvent::new(Key::Enter, Modifiers::NONE)))
    );
    assert_eq!(terminal.try_event().unwrap(), None);
    terminal.hide_cursor();
    terminal.present().unwrap();
    assert_eq!(terminal.cursor(), None);
    terminal.show_cursor();
    assert_eq!(terminal.cursor(), Some(Position { x: 4, y: 2 }));
    terminal.present().unwrap();
    // SetConsoleWindowInfo targets the main buffer, unlike the size query.
    // Leave the alternate buffer explicitly for this API-driven resize check.
    terminal.send_raw(b"\x1b[?1049l").unwrap();
    let size = terminal.size();
    let mut smaller = screen.srWindow;
    smaller.Right -= 1;
    smaller.Bottom -= 1;
    // SAFETY: the new viewport is inside the existing screen buffer.
    assert_ne!(
        unsafe { SetConsoleWindowInfo(output_handle, 1, &smaller) },
        0
    );
    // A viewport-only change need not create a buffer-size notification. Supply
    // the wake record but require Terminal to query the real, changed viewport.
    inject(
        input_handle,
        &[INPUT_RECORD {
            EventType: WINDOW_BUFFER_SIZE_EVENT as u16,
            Event: INPUT_RECORD_0 {
                WindowBufferSizeEvent: WINDOW_BUFFER_SIZE_RECORD {
                    dwSize: COORD { X: 1, Y: 1 },
                },
            },
        }],
    );
    assert_eq!(
        terminal.read_event_timeout(Duration::from_secs(1)).unwrap(),
        Some(Event::Resize(Size {
            width: size.width - 1,
            height: size.height - 1
        }))
    );
    terminal.present().unwrap();
    regressions::resize_matrix(&mut terminal, input_handle, output_handle);
    terminal.suspend().unwrap();
    restored();
    assert!(terminal.read_event().is_err());
    assert!(terminal.present().is_err());
    terminal.resume().unwrap();
    terminal.present().unwrap();
    drop(terminal);
    restored();

    regressions::lifecycle_and_failures(&caps, input_handle, output_handle);
    restored();

    // Rust unwinding runs the same Drop cleanup as a normal return.
    let panic = std::panic::catch_unwind(|| {
        let _terminal = Backend::new(caps.clone()).unwrap();
        panic!("intentional console restoration test");
    });
    assert!(panic.is_err());
    assert_eq!(
        panic.unwrap_err().downcast_ref::<&str>(),
        Some(&"intentional console restoration test")
    );
    restored();

    // GetConsoleCursorInfo reports main-buffer visibility even while VT uses
    // an alternate buffer. Exercise visibility without an alternate buffer.
    let mut cursor_caps = caps.clone();
    cursor_caps.enter_screen.clear();
    cursor_caps.exit_screen.clear();
    let backend = Backend::new(cursor_caps).unwrap();
    assert_eq!(cursor(output_handle).1, 0);
    drop(backend);
    restored();

    // Constructor failure releases ownership and leaves original state intact.
    // SAFETY: swapping process standard handles is confined to this test process.
    unsafe {
        assert_ne!(SetStdHandle(STD_OUTPUT_HANDLE, input_handle), 0);
    }
    assert!(Backend::new(caps.clone()).is_err());
    // SAFETY: restore the still-open output file as this process's standard handle.
    unsafe {
        assert_ne!(SetStdHandle(STD_OUTPUT_HANDLE, output_handle), 0);
    }
    restored();
    // Fail screen entry after mode changes with a read-only console output handle.
    let readonly = OpenOptions::new().read(true).open("CONOUT$").unwrap();
    // SAFETY: readonly stays alive until after this temporary standard-handle use.
    unsafe {
        assert_ne!(SetStdHandle(STD_OUTPUT_HANDLE, readonly.as_raw_handle()), 0);
    }
    assert!(Backend::new(caps.clone()).is_err());
    // SAFETY: restore the still-open output file after the expected setup failure.
    unsafe {
        assert_ne!(SetStdHandle(STD_OUTPUT_HANDLE, output_handle), 0);
    }
    restored();

    // Owned duplicate handles survive the caller closing its standard handles.
    let backend = Backend::new(caps).unwrap();
    drop(input);
    drop(output);
    assert!(backend.size().is_ok());
    backend.write(b"owned handles").unwrap();
    drop(backend);
}

struct CodePageGuard {
    input: u32,
    output: u32,
}
impl CodePageGuard {
    fn new() -> Self {
        // SAFETY: this test process is attached to a console.
        unsafe {
            Self {
                input: GetConsoleCP(),
                output: GetConsoleOutputCP(),
            }
        }
    }
}
impl Drop for CodePageGuard {
    fn drop(&mut self) {
        // SAFETY: restore previously queried code pages, including on unwind.
        unsafe {
            SetConsoleCP(self.input);
            SetConsoleOutputCP(self.output);
        }
    }
}

mod regressions;

fn keyboard_checks(backend: &mut Backend) {
    for (vk, unit, flags, expected, mods) in [
        (0x0d, 13, LEFT_ALT_PRESSED, Key::Enter, Modifiers::ALT),
        (0x08, 8, LEFT_CTRL_PRESSED, Key::Backspace, Modifiers::NONE),
        (0x09, 9, SHIFT_PRESSED, Key::BackTab, Modifiers::SHIFT),
        (0x49, 9, LEFT_CTRL_PRESSED, Key::Tab, Modifiers::NONE),
        (
            0x43,
            3,
            LEFT_CTRL_PRESSED | LEFT_ALT_PRESSED,
            Key::Char('C'),
            Modifiers::CTRL | Modifiers::ALT,
        ),
        (0x20, 0, LEFT_CTRL_PRESSED, Key::Char('@'), Modifiers::CTRL),
        (0x26, 0, SHIFT_PRESSED, Key::Up, Modifiers::SHIFT),
        (0x70, 0, 0, Key::Function(1), Modifiers::NONE),
        (0x87, 0, 0, Key::Function(24), Modifiers::NONE),
        (
            0,
            '€' as u16,
            RIGHT_ALT_PRESSED | LEFT_CTRL_PRESSED,
            Key::Char('€'),
            Modifiers::ALT | Modifiers::CTRL,
        ),
    ] {
        assert_eq!(
            backend.key_event(key(vk, unit, flags, 1)),
            Some((Event::Key(KeyEvent::new(expected, mods)), 1))
        );
    }
    assert!(backend.key_event(key(0x10, 0, SHIFT_PRESSED, 1)).is_none());
    assert!(backend.key_event(key(0, 0xdc00, 0, 1)).is_none());
    assert!(backend.key_event(key(0, 0xd800, 0, 1)).is_none());
    assert!(backend.key_event(key(0x26, 0, 0, 1)).is_some());
    assert!(backend.key_event(key(0, 0xdc00, 0, 1)).is_none());
    assert!(backend.key_event(key(0, 0xd83d, 0, 1)).is_none());
    assert!(
        backend
            .key_event(key(0, 0xde00, SHIFT_PRESSED, 1))
            .is_none()
    );
}

#[test]
fn mouse_transitions_and_viewport() {
    let mut state = MouseState::default();
    let window = SMALL_RECT {
        Left: 10,
        Top: 20,
        Right: 89,
        Bottom: 44,
    };
    let mut rec = MOUSE_EVENT_RECORD {
        dwMousePosition: COORD { X: 12, Y: 23 },
        dwButtonState: 3,
        dwControlKeyState: SHIFT_PRESSED,
        ..Default::default()
    };
    let events = mouse_events(rec, &mut state, window);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].position, Position { x: 2, y: 3 });
    assert_eq!(events[0].modifiers, Modifiers::SHIFT);
    rec.dwButtonState = 2;
    let events = mouse_events(rec, &mut state, window);
    assert_eq!(
        (events[0].button, events[0].kind),
        (MouseButton::Left, MouseKind::Release)
    );
    rec.dwEventFlags = MOUSE_MOVED;
    assert_eq!(
        mouse_events(rec, &mut state, window)[0].kind,
        MouseKind::Move
    );
    for (flag, delta, button) in [
        (MOUSE_WHEELED, 120i16, MouseButton::WheelUp),
        (MOUSE_WHEELED, -120, MouseButton::WheelDown),
        (MOUSE_HWHEELED, 120, MouseButton::WheelRight),
        (MOUSE_HWHEELED, -120, MouseButton::WheelLeft),
    ] {
        rec.dwEventFlags = flag;
        rec.dwButtonState = u32::from(delta as u16) << 16;
        let event = mouse_events(rec, &mut state, window)[0];
        assert_eq!((event.button, event.kind), (button, MouseKind::Scroll));
    }
}
