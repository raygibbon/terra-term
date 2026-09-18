use terra_term::{Event, InputParser, Key, KeyEvent, Modifiers};

fn event(key: Key) -> Option<Event> {
    Some(Event::Key(KeyEvent::new(key, Modifiers::NONE)))
}

fn registered(reverse: bool) -> InputParser {
    let mut p = InputParser::new();
    let mut sequences = [
        (b"\x1b[1".as_slice(), Key::Function(1)),
        (b"\x1b[1~", Key::Function(2)),
        (b"\x1b[1~!", Key::Function(3)),
    ];
    if reverse {
        sequences.reverse();
    }
    for (bytes, key) in sequences {
        p.add_key_sequence(bytes, key);
    }
    p
}

#[test]
fn complete_prefix_waits_and_finishes_shortest_or_middle() {
    for reverse in [false, true] {
        for (bytes, key) in [
            (b"\x1b[1".as_slice(), Key::Function(1)),
            (b"\x1b[1~", Key::Function(2)),
        ] {
            let mut p = registered(reverse);
            p.feed(bytes);
            assert_eq!(p.next_event(), None);
            assert!(p.pending_sequence());
            assert_eq!(p.finish_incomplete_sequence(), event(key));
            assert_eq!(p.next_event(), None);
        }
    }
}

#[test]
fn longest_match_at_every_fragment_and_registration_order() {
    let bytes = b"\x1b[1~!x\x1b[A";
    for reverse in [false, true] {
        for split in 0..=5 {
            let mut p = registered(reverse);
            p.feed(&bytes[..split]);
            if split < 5 {
                assert_eq!(p.next_event(), None);
            }
            p.feed(&bytes[split..]);
            assert_eq!(p.next_event(), event(Key::Function(3)));
            assert_eq!(p.next_event(), event(Key::Char('x')));
            assert_eq!(p.next_event(), event(Key::Up));
            assert_eq!(p.next_event(), None);
        }
    }
}

#[test]
fn two_overlaps_and_identical_prefixes_keep_following_bytes() {
    for reverse in [false, true] {
        let mut p = InputParser::new();
        let mut entries = [(b"\x1bXY".as_slice(), Key::Home), (b"\x1bXYZ", Key::End)];
        if reverse {
            entries.reverse();
        }
        for (seq, key) in entries {
            p.add_key_sequence(seq, key);
        }
        p.add_key_sequence(b"\x1bXY", Key::Home);
        p.feed(b"\x1bXY");
        assert_eq!(p.next_event(), None);
        p.feed(b"q");
        assert_eq!(p.next_event(), event(Key::Home));
        assert_eq!(p.next_event(), event(Key::Char('q')));
        p.feed(b"\x1bXYZ");
        assert_eq!(p.next_event(), event(Key::End));
    }
}

#[test]
fn conflicting_identical_registration_is_order_independent() {
    for keys in [[Key::Home, Key::End], [Key::End, Key::Home]] {
        let mut p = InputParser::new();
        for key in keys {
            p.add_key_sequence(b"\x1bXY", key);
        }
        p.feed(b"\x1bXYq");
        assert_eq!(
            p.next_event(),
            Some(Event::UnknownSequence(b"\x1bXY".to_vec()))
        );
        assert_eq!(p.next_event(), event(Key::Char('q')));
    }
}

fn collect_unknown(p: &mut InputParser) -> Vec<u8> {
    let mut bytes = Vec::new();
    for _ in 0..20000 {
        match p.next_event() {
            Some(Event::UnknownSequence(chunk)) => {
                assert!(!chunk.is_empty() && chunk.len() <= 64);
                bytes.extend(chunk);
            }
            None => return bytes,
            other => panic!("malformed contents became a key: {other:?}"),
        }
    }
    panic!("parser failed to make progress")
}

#[test]
fn oversized_csi_and_ss3_are_bounded_unknown_chunks() {
    for introducer in *b"[O" {
        for chunk_size in [1, 7, 64, 4096] {
            let mut bytes = vec![27, introducer];
            bytes.extend(vec![b'1'; 8192]);
            bytes.push(b'~');
            let mut p = InputParser::new();
            let mut unknown = Vec::new();
            for chunk in bytes.chunks(chunk_size) {
                p.feed(chunk);
                unknown.extend(collect_unknown(&mut p));
            }
            assert_eq!(unknown, bytes);
            assert!(!p.pending_sequence());
            p.feed(b"\x1b[Aq");
            assert_eq!(p.next_event(), event(Key::Up));
            assert_eq!(p.next_event(), event(Key::Char('q')));
            assert_eq!(p.next_event(), None);
        }
    }
}

#[test]
fn oversized_registration_is_rejected_and_input_recovers() {
    let mut sequence = b"\x1b[".to_vec();
    sequence.extend(vec![b'1'; 80]);
    sequence.push(b'~');
    let mut p = InputParser::new();
    p.add_key_sequence(&sequence, Key::Function(24));
    p.feed(&sequence);
    assert_eq!(collect_unknown(&mut p), sequence);
    p.feed(b"z");
    assert_eq!(p.next_event(), event(Key::Char('z')));
}

#[test]
fn malformed_sequences_preserve_immediately_following_keys() {
    for prefix in [
        b"\x1b[1;".as_slice(),
        b"\x1bO1;",
        b"\x1b[999999999999999999999~",
        b"\x1b[?A",
    ] {
        let mut p = InputParser::new();
        p.feed(prefix);
        p.feed(b"\x1b[Aq");
        assert_eq!(
            p.next_event(),
            Some(Event::UnknownSequence(prefix.to_vec()))
        );
        assert_eq!(p.next_event(), event(Key::Up));
        assert_eq!(p.next_event(), event(Key::Char('q')));
    }
}

#[test]
fn truncated_sequences_and_repeated_escape_recover() {
    for prefix in [b"\x1b[1;".as_slice(), b"\x1bO1;"] {
        let mut p = InputParser::new();
        p.feed(prefix);
        assert_eq!(p.next_event(), None);
        assert_eq!(
            p.finish_incomplete_sequence(),
            Some(Event::UnknownSequence(prefix.to_vec()))
        );
        p.feed(b"\x1b\x1b\x1b[Aq");
        assert_eq!(p.next_event(), event(Key::Escape));
        assert_eq!(p.next_event(), event(Key::Escape));
        assert_eq!(p.next_event(), event(Key::Up));
        assert_eq!(p.next_event(), event(Key::Char('q')));
    }
}

#[test]
fn random_escape_parameters_never_swallow_the_next_key() {
    let mut seed = 7u64;
    for length in 0..512 {
        let mut malformed = b"\x1b[".to_vec();
        for _ in 0..length {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            malformed.push(0x20 + ((seed >> 32) as u8 % 32));
        }
        let mut p = InputParser::new();
        p.feed(&malformed);
        let mut recovered = collect_unknown(&mut p);
        if let Some(Event::UnknownSequence(chunk)) = p.finish_incomplete_sequence() {
            recovered.extend(chunk);
        }
        assert_eq!(recovered, malformed);
        p.feed(b"\x1bOPx");
        assert_eq!(p.next_event(), event(Key::Function(1)));
        assert_eq!(p.next_event(), event(Key::Char('x')));
    }
}
