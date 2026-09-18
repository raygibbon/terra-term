use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessHandleCount};

fn expected(key: Key) -> Event {
    Event::Key(KeyEvent::new(key, Modifiers::NONE))
}

pub(super) fn keyboard_and_unicode(backend: &mut Backend) {
    for unit in 1..=31u16 {
        let vk = match unit {
            1..=26 => unit + 64,
            27 => 0xdb,
            28 => 0xdc,
            29 => 0xdd,
            30 => 0x36,
            _ => 0xbd,
        };
        assert_eq!(
            backend.key_event(key(vk, unit, LEFT_CTRL_PRESSED, 1)),
            Some((Event::Key(control_byte(unit as u8)), 1)),
            "control byte {unit}"
        );
    }
    for vk in 0x70..=0x87 {
        assert_eq!(
            backend.key_event(key(vk, 0, 0, 1)),
            Some((expected(Key::Function((vk - 0x6f) as u8)), 1))
        );
    }
    for ch in ['é', '中', '\u{1f600}', '\u{10437}'] {
        let mut units = [0; 2];
        let encoded = ch.encode_utf16(&mut units);
        for &unit in &encoded[..encoded.len() - 1] {
            assert_eq!(backend.key_event(key(0, unit, SHIFT_PRESSED, 5)), None);
        }
        assert_eq!(
            backend.key_event(key(0, *encoded.last().unwrap(), SHIFT_PRESSED, 5)),
            Some((
                Event::Key(KeyEvent::new(Key::Char(ch), Modifiers::SHIFT)),
                5
            ))
        );
    }
    for malformed in [vec![0xd800], vec![0xdc00], vec![0xd800, 0xd801]] {
        for unit in malformed {
            assert_eq!(backend.key_event(key(0, unit, 0, 1)), None);
        }
        assert_eq!(
            backend.key_event(key(0x58, b'x' as u16, 0, 1)),
            Some((expected(Key::Char('x')), 1))
        );
        assert_eq!(backend.high_surrogate, None);
    }
    assert_eq!(backend.key_event(key(0, 0xd800, 0, 20)), None);
    assert_eq!(backend.key_event(key(0, 0xd801, 0, 2)), None);
    assert_eq!(
        backend.key_event(key(0, 0xdc37, 0, 5)),
        Some((expected(Key::Char('\u{10437}')), 2))
    );
    assert_eq!(backend.key_event(key(0x41, 65, 0, 0)), None);
}

fn mouse_record(buttons: u32, flags: u32) -> INPUT_RECORD {
    INPUT_RECORD {
        EventType: MOUSE_EVENT as u16,
        Event: INPUT_RECORD_0 {
            MouseEvent: MOUSE_EVENT_RECORD {
                dwButtonState: buttons,
                dwEventFlags: flags,
                dwMousePosition: COORD { X: 2, Y: 3 },
                ..Default::default()
            },
        },
    }
}
fn resize_record() -> INPUT_RECORD {
    INPUT_RECORD {
        EventType: WINDOW_BUFFER_SIZE_EVENT as u16,
        Event: INPUT_RECORD_0 {
            WindowBufferSizeEvent: WINDOW_BUFFER_SIZE_RECORD {
                dwSize: COORD { X: 100, Y: 30 },
            },
        },
    }
}

pub(super) fn resize_matrix(terminal: &mut crate::Terminal, input: HANDLE, output: HANDLE) {
    // Caller has left the alternate screen, so native viewport mutations and
    // the backend size query address the same buffer.
    let original = terminal.size();
    let mut screen = CONSOLE_SCREEN_BUFFER_INFO::default();
    // SAFETY: output is owned by the test; screen is writable.
    assert_ne!(
        unsafe { GetConsoleScreenBufferInfo(output, &mut screen) },
        0
    );
    let buffer = COORD {
        X: (original.width + 4) as i16,
        Y: screen.dwSize.Y.max((original.height + 4) as i16),
    };
    // SAFETY: buffer grows enough to contain every test viewport.
    assert_ne!(unsafe { SetConsoleScreenBufferSize(output, buffer) }, 0);
    terminal.set_mouse_enabled(true).unwrap();
    for (index, (width, height)) in [
        (original.width + 1, original.height),
        (original.width - 1, original.height),
        (original.width, original.height + 1),
        (original.width, original.height - 1),
        (original.width, original.height),
    ]
    .into_iter()
    .enumerate()
    {
        let rect = SMALL_RECT {
            Left: 0,
            Top: 0,
            Right: width as i16 - 1,
            Bottom: height as i16 - 1,
        };
        // SAFETY: viewport lies within the enlarged test buffer.
        assert_ne!(unsafe { SetConsoleWindowInfo(output, 1, &rect) }, 0);
        inject(
            input,
            &[
                resize_record(),
                resize_record(),
                key_record(key(0x41, 97, 0, 1)),
                mouse_record(0, MOUSE_MOVED),
            ],
        );
        assert_eq!(
            terminal.read_event_timeout(Duration::from_secs(2)).unwrap(),
            Some(Event::Resize(Size { width, height }))
        );
        assert_eq!(terminal.size(), Size { width, height });
        assert_eq!(terminal.read_event().unwrap(), expected(Key::Char('a')));
        assert!(matches!(terminal.read_event().unwrap(), Event::Mouse(_)));
        assert_eq!(
            terminal.try_event().unwrap(),
            None,
            "duplicate resize storm"
        );
        if index % 2 == 0 {
            terminal.hide_cursor();
        } else {
            terminal.show_cursor();
        }
        terminal.invalidate();
        terminal.present().unwrap();
        terminal.show_cursor();
        assert_eq!(terminal.cursor(), Some(Position { x: 4, y: 2 }));
        terminal.present().unwrap();
        // SAFETY: read back rendering cursor position in the active buffer.
        assert_ne!(
            unsafe { GetConsoleScreenBufferInfo(output, &mut screen) },
            0
        );
        assert_eq!(
            (screen.dwCursorPosition.X, screen.dwCursorPosition.Y),
            (4, 2)
        );
    }
}

pub(super) fn repeats_and_bursts(backend: &mut Backend, input: HANDLE) {
    for count in [1, 2, 5, 20, u16::MAX] {
        inject(
            input,
            &[
                key_record(key(0x41, 97, 0, count)),
                key_record(key(0x42, 98, 0, 1)),
            ],
        );
        for _ in 0..count {
            assert_eq!(next(backend), expected(Key::Char('a')));
            assert!(backend.pending.len() <= 1);
        }
        assert_eq!(next(backend), expected(Key::Char('b')));
    }
    backend.set_mouse(true).unwrap();
    for records in [
        vec![
            key_record(key(0x41, 97, 0, 1)),
            key_record(key(0x42, 98, 0, 1)),
            key_record(key(0x43, 99, 0, 1)),
        ],
        vec![
            key_record(key(0x41, 97, 0, 1)),
            mouse_record(0, MOUSE_MOVED),
            key_record(key(0x42, 98, 0, 1)),
        ],
        vec![resize_record(), key_record(key(0x41, 97, 0, 1))],
        vec![
            mouse_record(0, MOUSE_MOVED),
            resize_record(),
            key_record(key(0x41, 97, 0, 1)),
        ],
    ] {
        inject(input, &records);
        for record in records {
            match u32::from(record.EventType) {
                KEY_EVENT => {
                    // SAFETY: EventType selects KeyEvent; these records were built above.
                    let rec = unsafe { record.Event.KeyEvent };
                    let expected = backend.key_event(rec).unwrap().0;
                    assert_eq!(next(backend), expected);
                }
                MOUSE_EVENT => assert!(matches!(next(backend), Event::Mouse(_))),
                WINDOW_BUFFER_SIZE_EVENT => {
                    assert_eq!(backend.wait(Some(Duration::ZERO)).unwrap(), Wake::Resize)
                }
                _ => unreachable!(),
            }
        }
    }
    inject(
        input,
        &[
            key_record(key(0, 0xd83d, 0, 1)),
            key_record(key(0, 0xde00, 0, 1)),
            key_record(key(0x41, 97, 0, 1)),
        ],
    );
    assert_eq!(next(backend), expected(Key::Char('\u{1f600}')));
    assert_eq!(next(backend), expected(Key::Char('a')));
    backend.set_mouse(false).unwrap();
    let start = Instant::now();
    assert_eq!(
        backend.wait(Some(Duration::from_millis(30))).unwrap(),
        Wake::Timeout
    );
    assert!(
        start.elapsed() >= Duration::from_millis(30) && start.elapsed() < Duration::from_secs(2)
    );
}

fn handles() -> u32 {
    let mut count = 0;
    // SAFETY: the pseudo-handle is valid and count is writable.
    assert_ne!(
        unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) },
        0
    );
    count
}

static BREAK_OBSERVED: AtomicUsize = AtomicUsize::new(0);
unsafe extern "system" fn observe_break(kind: u32) -> i32 {
    if kind == CTRL_BREAK_EVENT {
        BREAK_OBSERVED.fetch_add(1, Ordering::SeqCst);
        return 1;
    }
    0
}

pub(super) fn lifecycle_and_failures(caps: &Capabilities, input: HANDLE, output: HANDLE) {
    let initial_modes = (mode(input), mode(output));
    let initial_cursor = cursor(output);
    // SAFETY: attached console, queries take no pointers.
    let pages = unsafe { (GetConsoleCP(), GetConsoleOutputCP()) };
    let baseline = handles();
    let marker: Vec<u16> = "TERRA-TERM-MAIN-SCREEN".encode_utf16().collect();
    let mut written = 0;
    // SAFETY: marker is readable for its supplied length; count is writable.
    assert_ne!(
        unsafe {
            WriteConsoleOutputCharacterW(
                output,
                marker.as_ptr(),
                marker.len() as u32,
                COORD { X: 0, Y: 0 },
                &mut written,
            )
        },
        0
    );
    assert_eq!(written as usize, marker.len());
    let main_screen_restored = || {
        let mut text = vec![0u16; marker.len()];
        let mut read = 0;
        // SAFETY: destination has the specified number of writable UTF-16 units.
        assert_ne!(
            unsafe {
                ReadConsoleOutputCharacterW(
                    output,
                    text.as_mut_ptr(),
                    text.len() as u32,
                    COORD { X: 0, Y: 0 },
                    &mut read,
                )
            },
            0
        );
        assert_eq!(text, marker);
    };
    for stage in [
        "handles",
        "code_page",
        "input_mode",
        "output_mode",
        "mouse",
        "alternate_screen",
        "cursor",
    ] {
        FAIL_AFTER.with(|failure| failure.set(Some(stage)));
        let result = Backend::new(caps.clone());
        FAIL_AFTER.with(|failure| failure.set(None));
        assert!(result.is_err(), "missing injection at {stage}");
        assert_eq!((mode(input), mode(output)), initial_modes, "stage {stage}");
        assert_eq!(cursor(output), initial_cursor, "stage {stage}");
        // SAFETY: console stays attached after failed initialization.
        assert_eq!(unsafe { (GetConsoleCP(), GetConsoleOutputCP()) }, pages);
        assert_eq!(handles(), baseline, "leak after {stage}");
        main_screen_restored();
        drop(Backend::new(caps.clone()).unwrap()); // lease was released
    }
    for _ in 0..100 {
        let mut backend = Backend::new(caps.clone()).unwrap();
        backend.write(b"ALTERNATE SCREEN CONTENTS").unwrap();
        for _ in 0..3 {
            backend.set_mouse(true).unwrap();
            backend.suspend().unwrap();
            assert_eq!((mode(input), mode(output)), initial_modes);
            // SAFETY: same attached console throughout the test.
            assert_eq!(unsafe { (GetConsoleCP(), GetConsoleOutputCP()) }, pages);
            backend.resume().unwrap();
            assert_ne!(mode(input) & ENABLE_MOUSE_INPUT, 0);
        }
        drop(backend);
        assert_eq!(cursor(output), initial_cursor);
        main_screen_restored();
    }
    assert_eq!(handles(), baseline, "duplicated handles leaked");

    let mut backend = Backend::new(caps.clone()).unwrap();
    backend.set_mouse(true).unwrap();
    backend.suspend().unwrap();
    FAIL_AFTER.with(|failure| failure.set(Some("mouse")));
    let resumed = backend.resume();
    FAIL_AFTER.with(|failure| failure.set(None));
    assert!(resumed.is_err());
    assert_eq!((mode(input), mode(output)), initial_modes);
    assert!(!backend.active && !backend.needs_restore);
    let saved_output = backend.output;
    backend.output = std::ptr::null_mut();
    assert!(backend.resume().is_err()); // code page/input mode changed before output failure
    assert!(!backend.active && backend.needs_restore);
    assert_eq!(mode(input), initial_modes.0);
    // SAFETY: query remains valid even while an intentionally invalid output alias is installed.
    assert_eq!(unsafe { GetConsoleOutputCP() }, pages.1);
    backend.output = saved_output;
    backend.resume().unwrap(); // retry outstanding restoration, then re-enter
    let saved_input = backend.input;
    backend.input = std::ptr::null_mut();
    assert!(backend.set_mouse(true).is_err());
    backend.input = saved_input;
    drop(backend);
    assert_eq!((mode(input), mode(output)), initial_modes);
    assert_eq!(handles(), baseline);

    // Signal broadcast is safe only in our isolated child, not a manual host
    // shared with PowerShell. A pre-existing handler proves removal on suspend.
    if std::env::var("TERRA_TERM_CONSOLE_TEST").as_deref() == Ok("child") {
        // SAFETY: callback is static and uses only an atomic counter.
        assert_ne!(unsafe { SetConsoleCtrlHandler(Some(observe_break), 1) }, 0);
        let mut backend = Backend::new(caps.clone()).unwrap();
        // SAFETY: only this test process is attached to its newly allocated console.
        assert_ne!(unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, 0) }, 0);
        let end = Instant::now() + Duration::from_secs(2);
        while BREAK_COUNT.load(Ordering::SeqCst) == 0 && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(BREAK_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(BREAK_OBSERVED.load(Ordering::SeqCst), 0);
        backend.suspend().unwrap();
        // SAFETY: same isolated console; observer catches the signal after removal.
        assert_ne!(unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, 0) }, 0);
        let end = Instant::now() + Duration::from_secs(2);
        while BREAK_OBSERVED.load(Ordering::SeqCst) == 0 && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(BREAK_OBSERVED.load(Ordering::SeqCst), 1);
        // SAFETY: remove exactly the observer installed above.
        assert_ne!(unsafe { SetConsoleCtrlHandler(Some(observe_break), 0) }, 0);
    }
}

#[test]
fn all_mouse_button_transitions() {
    for (from, to, changes) in [
        (0, 1, vec![(MouseButton::Left, MouseKind::Press)]),
        (1, 0, vec![(MouseButton::Left, MouseKind::Release)]),
        (0, 2, vec![(MouseButton::Right, MouseKind::Press)]),
        (2, 0, vec![(MouseButton::Right, MouseKind::Release)]),
        (0, 4, vec![(MouseButton::Middle, MouseKind::Press)]),
        (4, 0, vec![(MouseButton::Middle, MouseKind::Release)]),
        (1, 3, vec![(MouseButton::Right, MouseKind::Press)]),
        (3, 2, vec![(MouseButton::Left, MouseKind::Release)]),
        (3, 1, vec![(MouseButton::Right, MouseKind::Release)]),
        (3, 3, vec![]),
    ] {
        let mut previous = MouseState { buttons: from, ..Default::default() };
        let events = mouse_events(
            MOUSE_EVENT_RECORD {
                dwButtonState: to,
                ..Default::default()
            },
            &mut previous,
            SMALL_RECT::default(),
        );
        assert_eq!(
            events
                .iter()
                .map(|e| (e.button, e.kind))
                .collect::<Vec<_>>(),
            changes
        );
        assert_eq!(previous.buttons, to);
    }
}

#[test]
fn mouse_edges_resized_viewports_drag_and_wheels() {
    for window in [
        SMALL_RECT {
            Left: 10,
            Top: 20,
            Right: 89,
            Bottom: 44,
        },
        SMALL_RECT {
            Left: 5,
            Top: 7,
            Right: 44,
            Bottom: 16,
        },
    ] {
        for (x, y) in [(window.Left, window.Top), (window.Right, window.Bottom)] {
            for (flags, buttons, kind) in [
                (MOUSE_MOVED, 0, MouseKind::Move),
                (MOUSE_MOVED, 1, MouseKind::Move),
                (MOUSE_WHEELED, 120 << 16, MouseKind::Scroll),
                (MOUSE_HWHEELED, 120 << 16, MouseKind::Scroll),
            ] {
                let mut state = MouseState { buttons: buttons & 0x1f, ..Default::default() };
                let events = mouse_events(
                    MOUSE_EVENT_RECORD {
                        dwMousePosition: COORD { X: x, Y: y },
                        dwButtonState: buttons,
                        dwEventFlags: flags,
                        ..Default::default()
                    },
                    &mut state,
                    window,
                );
                assert_eq!(events.len(), 1);
                assert_eq!(events[0].kind, kind);
                assert_eq!(
                    events[0].position,
                    Position {
                        x: (x - window.Left) as u16,
                        y: (y - window.Top) as u16
                    }
                );
                if flags == MOUSE_MOVED {
                    assert_eq!(
                        events[0].button,
                        if buttons == 0 {
                            MouseButton::None
                        } else {
                            MouseButton::Left
                        }
                    );
                }
            }
        }
    }
}

#[test]
fn utf8_chunks_preserve_every_character_at_boundaries() {
    for text in [
        "a",
        "é",
        "中",
        "\u{1f600}",
        "e\u{301}",
        "\u{1f469}\u{200d}\u{1f4bb}",
    ] {
        for offset in 0..5 {
            let bytes = format!("{}{text}tail", "a".repeat(16384 - offset)).into_bytes();
            let mut rest = bytes.as_slice();
            let mut joined = Vec::new();
            while !rest.is_empty() {
                let len = output_chunk_len(rest, true);
                assert!((1..=16384).contains(&len));
                assert!(std::str::from_utf8(&rest[..len]).is_ok());
                joined.extend_from_slice(&rest[..len]);
                rest = &rest[len..];
            }
            assert_eq!(joined, bytes);
        }
    }
}
