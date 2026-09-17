//! Feed bytes from stdin; suitable for a process-based Rust fuzzing engine.
use std::io::{self, Read};
use terra_term::{InputParser, Key};

fn main() -> io::Result<()> {
    let mut input = Vec::new();
    io::stdin().take(1024 * 1024).read_to_end(&mut input)?;
    for chunk_size in [1, 2, 3, 7, 64, 4096] {
        let mut parser = InputParser::new();
        parser.add_key_sequence(b"\x1bXYZ!", Key::Function(9));
        parser.add_key_sequence(b"\x1b[1;5A", Key::Up);
        for chunk in input.chunks(chunk_size) {
            parser.feed(chunk);
            for _ in 0..=chunk.len() {
                if parser.next_event().is_none() {
                    break;
                }
            }
        }
        if parser.pending_sequence() {
            let _ = parser.finish_incomplete_sequence();
        }
        if parser.pending_lone_escape() {
            let _ = parser.finish_escape();
        }
    }
    Ok(())
}
