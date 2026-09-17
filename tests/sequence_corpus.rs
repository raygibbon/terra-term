//! Terminal input sequence fixtures covering common keys, Unicode, and mouse input.
use terra_term::{Event, InputParser, Key, KeyEvent, Modifiers, MouseButton, MouseKind};

#[test]
fn key_sequence_corpus() {
    let cases: &[(&[u8], KeyEvent)] = &[
        (b"a", KeyEvent::new(Key::Char('a'), Modifiers::NONE)),
        (&[0x1c], KeyEvent::new(Key::Char('\\'), Modifiers::CTRL)),
        (b"\x1bOP", KeyEvent::new(Key::Function(1), Modifiers::NONE)),
        (
            b"\x1b[15~",
            KeyEvent::new(Key::Function(5), Modifiers::NONE),
        ),
        (b"\x1b[1;5A", KeyEvent::new(Key::Up, Modifiers::CTRL)),
        (
            b"\x1b[24~",
            KeyEvent::new(Key::Function(12), Modifiers::NONE),
        ),
        (
            "界".as_bytes(),
            KeyEvent::new(Key::Char('界'), Modifiers::NONE),
        ),
    ];
    for (bytes, expected) in cases {
        let mut parser = InputParser::new();
        parser.feed(bytes);
        assert_eq!(
            parser.next_event(),
            Some(Event::Key(*expected)),
            "{bytes:?}"
        );
    }
}
#[test]
fn mouse_sequence_corpus() {
    let mut parser = InputParser::new();
    parser.feed(b"\x1b[<0;10;20M");
    let Event::Mouse(mouse) = parser.next_event().unwrap() else {
        panic!("mouse")
    };
    assert_eq!((mouse.position.x, mouse.position.y), (9, 19));
    assert_eq!(mouse.button, MouseButton::Left);
    assert_eq!(mouse.kind, MouseKind::Press);
}
#[test]
fn large_paste_is_chunked_without_key_interpretation() {
    let mut parser = InputParser::new();
    let mut bytes = b"\x1b[200~".to_vec();
    bytes.extend(std::iter::repeat_n(b'X', 1024 * 1024 + 21));
    bytes.extend_from_slice(b"\x1b[A\x1b[201~");
    parser.feed(&bytes);
    let Event::Paste(first) = parser.next_event().unwrap() else {
        panic!("paste")
    };
    assert_eq!(first.bytes.len(), 1024 * 1024);
    assert!(!first.complete);
    let Event::Paste(last) = parser.next_event().unwrap() else {
        panic!("paste")
    };
    assert_eq!(&last.bytes[21..], b"\x1b[A");
    assert!(last.complete);
}
