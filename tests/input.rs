use terra_term::{Event, InputMode, InputParser, Key, KeyEvent, Modifiers};

fn key(bytes: &[u8]) -> Option<KeyEvent> {
    let mut p = InputParser::new();
    p.feed(bytes);
    match p.next_event() {
        Some(Event::Key(k)) => Some(k),
        _ => None,
    }
}
fn alt_key(bytes: &[u8]) -> Option<KeyEvent> {
    let mut p = InputParser::new();
    p.set_mode(InputMode::Alt);
    p.feed(bytes);
    match p.next_event() {
        Some(Event::Key(k)) => Some(k),
        _ => None,
    }
}
#[test]
fn ascii_and_unicode() {
    assert_eq!(
        key(b"a"),
        Some(KeyEvent::new(Key::Char('a'), Modifiers::NONE))
    );
    assert_eq!(
        key("界".as_bytes()),
        Some(KeyEvent::new(Key::Char('界'), Modifiers::NONE))
    );
}
#[test]
fn ctrl_backslash() {
    assert_eq!(
        key(&[0x1c]),
        Some(KeyEvent::new(Key::Char('\\'), Modifiers::CTRL))
    );
}
#[test]
fn ambiguous_control_bytes_use_shared_logical_keys() {
    for (byte, expected_key, modifiers) in [
        (0x08, Key::Backspace, Modifiers::NONE),
        (0x09, Key::Tab, Modifiers::NONE),
        (0x0a, Key::Enter, Modifiers::NONE),
        (0x0d, Key::Enter, Modifiers::NONE),
        (0x1c, Key::Char('\\'), Modifiers::CTRL),
        (0x1d, Key::Char(']'), Modifiers::CTRL),
        (0x1e, Key::Char('^'), Modifiers::CTRL),
        (0x1f, Key::Char('_'), Modifiers::CTRL),
    ] {
        assert_eq!(key(&[byte]), Some(KeyEvent::new(expected_key, modifiers)));
    }
    let mut parser = InputParser::new();
    parser.feed(&[0x1b]);
    assert_eq!(parser.next_event(), None);
    assert_eq!(
        parser.finish_escape(),
        Some(Event::Key(KeyEvent::new(Key::Escape, Modifiers::NONE)))
    );
}
#[test]
fn controls_and_alt() {
    assert_eq!(
        key(&[1]),
        Some(KeyEvent::new(Key::Char('A'), Modifiers::CTRL))
    );
    assert_eq!(
        alt_key(b"\x1bx"),
        Some(KeyEvent::new(Key::Char('x'), Modifiers::ALT))
    );
    assert_eq!(
        alt_key(b"\x1b\x01"),
        Some(KeyEvent::new(
            Key::Char('A'),
            Modifiers::ALT | Modifiers::CTRL
        ))
    );
}
#[test]
fn arrows_and_modifiers() {
    for (seq, expected) in [
        (b"\x1b[A".as_slice(), Key::Up),
        (b"\x1b[B", Key::Down),
        (b"\x1b[C", Key::Right),
        (b"\x1b[D", Key::Left),
    ] {
        assert_eq!(key(seq), Some(KeyEvent::new(expected, Modifiers::NONE)));
    }
    assert_eq!(
        key(b"\x1b[1;5A"),
        Some(KeyEvent::new(Key::Up, Modifiers::CTRL))
    );
    assert_eq!(
        key(b"\x1b[Z"),
        Some(KeyEvent::new(Key::BackTab, Modifiers::SHIFT))
    );
}
#[test]
fn function_keys() {
    assert_eq!(
        key(b"\x1bOP"),
        Some(KeyEvent::new(Key::Function(1), Modifiers::NONE))
    );
    assert_eq!(
        key(b"\x1b[15~"),
        Some(KeyEvent::new(Key::Function(5), Modifiers::NONE))
    );
    assert_eq!(
        key(b"\x1b[24~"),
        Some(KeyEvent::new(Key::Function(12), Modifiers::NONE))
    );
}
#[test]
fn incomplete_and_malformed() {
    let mut p = InputParser::new();
    p.feed(b"\x1b[");
    assert_eq!(p.next_event(), None);
    p.feed(b"A");
    assert_eq!(
        p.next_event(),
        Some(Event::Key(KeyEvent::new(Key::Up, Modifiers::NONE)))
    );
    p.feed(&[0xff]);
    assert_eq!(
        p.next_event(),
        Some(Event::Key(KeyEvent::new(
            Key::Char('\u{fffd}'),
            Modifiers::NONE
        )))
    );
    p.feed(b"\x1b");
    assert_eq!(p.next_event(), None);
    assert_eq!(
        p.finish_escape(),
        Some(Event::Key(KeyEvent::new(Key::Escape, Modifiers::NONE)))
    );
    p.feed(b"\x1b[1;?Aq");
    assert_eq!(
        p.next_event(),
        Some(Event::Key(KeyEvent::new(Key::Char('q'), Modifiers::NONE)))
    );
}
#[test]
fn registered_arbitrary_escape_sequence_survives_fragmentation() {
    let sequence = b"\x1bXYZ!";
    for split in 2..sequence.len() {
        let mut parser = InputParser::new();
        parser.add_key_sequence(sequence, Key::Function(9));
        parser.feed(&sequence[..split]);
        assert_eq!(parser.next_event(), None, "split at {split}");
        assert!(parser.pending_sequence());
        parser.feed(&sequence[split..]);
        assert_eq!(
            parser.next_event(),
            Some(Event::Key(KeyEvent::new(Key::Function(9), Modifiers::NONE)))
        );
    }
    let mut parser = InputParser::new();
    parser.add_key_sequence(sequence, Key::Function(9));
    parser.feed(b"\x1bXY");
    assert_eq!(parser.next_event(), None);
    parser.feed(b"q");
    assert_eq!(
        parser.next_event(),
        Some(Event::UnknownSequence(b"\x1bXY".to_vec()))
    );
    assert_eq!(
        parser.next_event(),
        Some(Event::Key(KeyEvent::new(Key::Char('q'), Modifiers::NONE)))
    );
}

#[test]
fn multiple_events_in_one_read_keep_order() {
    let mut parser = InputParser::new();
    parser.feed(b"abc\x1bOP\x1bOQ\x1b[A\x1b[<0;2;3Mx\x1b[200~paste\x1b[201~z");
    let mut events = Vec::new();
    while let Some(event) = parser.next_event() {
        events.push(event);
    }
    assert_eq!(events.len(), 10);
    assert_eq!(
        events[0],
        Event::Key(KeyEvent::new(Key::Char('a'), Modifiers::NONE))
    );
    assert_eq!(
        events[1],
        Event::Key(KeyEvent::new(Key::Char('b'), Modifiers::NONE))
    );
    assert_eq!(
        events[2],
        Event::Key(KeyEvent::new(Key::Char('c'), Modifiers::NONE))
    );
    assert_eq!(
        events[3],
        Event::Key(KeyEvent::new(Key::Function(1), Modifiers::NONE))
    );
    assert_eq!(
        events[4],
        Event::Key(KeyEvent::new(Key::Function(2), Modifiers::NONE))
    );
    assert_eq!(
        events[5],
        Event::Key(KeyEvent::new(Key::Up, Modifiers::NONE))
    );
    assert!(matches!(events[6], Event::Mouse(_)));
    assert_eq!(
        events[7],
        Event::Key(KeyEvent::new(Key::Char('x'), Modifiers::NONE))
    );
    assert!(matches!(&events[8], Event::Paste(paste) if paste.bytes == b"paste" && paste.complete));
    assert_eq!(
        events[9],
        Event::Key(KeyEvent::new(Key::Char('z'), Modifiers::NONE))
    );
}
