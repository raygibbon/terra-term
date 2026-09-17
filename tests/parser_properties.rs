use terra_term::{Event, InputMode, InputParser, Key, KeyEvent, Modifiers, MouseButton, MouseKind};

#[test]
fn fragmented_escape_state() {
    let mut p = InputParser::new();
    p.feed(b"\x1b");
    assert_eq!(p.next_event(), None);
    p.feed(b"[");
    assert_eq!(p.next_event(), None);
    assert_eq!(p.finish_escape(), None, "CSI prefix must survive a timeout");
    p.feed(b"1;5A");
    assert_eq!(
        p.next_event(),
        Some(Event::Key(KeyEvent::new(Key::Up, Modifiers::CTRL)))
    );
    p.feed(b"\x1bO");
    assert_eq!(p.next_event(), None);
    p.feed(b"P");
    assert_eq!(
        p.next_event(),
        Some(Event::Key(KeyEvent::new(Key::Function(1), Modifiers::NONE)))
    );
}
#[test]
fn escape_and_alt_modes() {
    let mut p = InputParser::new();
    p.feed(b"\x1bx");
    assert_eq!(
        p.next_event(),
        Some(Event::Key(KeyEvent::new(Key::Escape, Modifiers::NONE)))
    );
    assert_eq!(
        p.next_event(),
        Some(Event::Key(KeyEvent::new(Key::Char('x'), Modifiers::NONE)))
    );
    p.set_mode(InputMode::Alt);
    p.feed("\x1bé".as_bytes());
    assert_eq!(
        p.next_event(),
        Some(Event::Key(KeyEvent::new(Key::Char('é'), Modifiers::ALT)))
    );
}
#[test]
fn paste_preserves_escapes_and_fragments() {
    let mut p = InputParser::new();
    p.feed(b"\x1b[200~hello\n\x1b[A\x1b[20");
    assert_eq!(p.next_event(), None);
    p.feed(b"1~x");
    let Event::Paste(paste) = p.next_event().unwrap() else {
        panic!("expected paste")
    };
    assert_eq!(paste.bytes, b"hello\n\x1b[A");
    assert!(paste.complete);
    assert_eq!(
        p.next_event(),
        Some(Event::Key(KeyEvent::new(Key::Char('x'), Modifiers::NONE)))
    );
}
#[test]
fn mouse_protocols() {
    let mut p = InputParser::new();
    p.feed(b"\x1b[<0;10;20M");
    let Event::Mouse(mouse) = p.next_event().unwrap() else {
        panic!("expected mouse")
    };
    assert_eq!((mouse.position.x, mouse.position.y), (9, 19));
    assert_eq!(mouse.button, MouseButton::Left);
    assert_eq!(mouse.kind, MouseKind::Press);
    p.feed(b"\x1b[M !!");
    assert!(matches!(p.next_event(), Some(Event::Mouse(_))));
    p.feed(b"\x1b[96;3;4M");
    let Event::Mouse(mouse) = p.next_event().unwrap() else {
        panic!("expected mouse")
    };
    assert_eq!(mouse.button, MouseButton::WheelUp);
}
#[test]
fn arbitrary_bytes_make_progress() {
    let mut p = InputParser::new();
    let bytes = (0..=255u8).cycle().take(4096).collect::<Vec<_>>();
    p.feed(&bytes);
    for _ in 0..bytes.len() {
        if p.next_event().is_none() {
            break;
        }
    }
}

#[test]
fn every_split_of_known_sequences() {
    let cases: &[(&[u8], Event)] = &[
        (
            b"\x1b[A",
            Event::Key(KeyEvent::new(Key::Up, Modifiers::NONE)),
        ),
        (
            b"\x1bOP",
            Event::Key(KeyEvent::new(Key::Function(1), Modifiers::NONE)),
        ),
        (
            b"\x1b[1;8D",
            Event::Key(KeyEvent::new(
                Key::Left,
                Modifiers::CTRL | Modifiers::ALT | Modifiers::SHIFT,
            )),
        ),
        (
            b"\x1b[<0;10;20M",
            Event::Mouse(terra_term::MouseEvent {
                position: terra_term::Position { x: 9, y: 19 },
                button: MouseButton::Left,
                kind: MouseKind::Press,
                modifiers: Modifiers::NONE,
            }),
        ),
    ];
    for (bytes, expected) in cases {
        for split in 1..bytes.len() {
            let mut p = InputParser::new();
            p.feed(&bytes[..split]);
            assert_eq!(
                p.next_event(),
                None,
                "premature event for {bytes:?} at {split}"
            );
            p.feed(&bytes[split..]);
            assert_eq!(
                p.next_event(),
                Some(expected.clone()),
                "{bytes:?} at {split}"
            );
        }
    }
}

#[test]
fn fragmented_random_streams_never_panic() {
    let mut state = 0x9e3779b97f4a7c15u64;
    for _case in 0..256 {
        let mut p = InputParser::new();
        for _chunk in 0..32 {
            let mut bytes = [0; 11];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            p.feed(&bytes);
            for _ in 0..bytes.len() + 2 {
                if p.next_event().is_none() {
                    break;
                }
            }
        }
        if p.pending_sequence() {
            let _ = p.finish_incomplete_sequence();
        }
        if p.pending_lone_escape() {
            let _ = p.finish_escape();
        }
    }
}
