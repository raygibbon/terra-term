//! Feed a compiled terminfo entry from stdin without using the host database.
use std::io::{self, Read};
pub use terra_term::Key;
#[path = "../../../src/terminfo/mod.rs"]
#[allow(dead_code)]
mod terminfo;

fn main() -> io::Result<()> {
    let mut input = Vec::new();
    io::stdin().take(1024 * 1024).read_to_end(&mut input)?;
    if let Some(entry) = terminfo::parse_entry(&input) {
        let caps = terminfo::Capabilities::from_entry(&entry);
        let _ = caps.cursor_position(1, 2);
    }
    Ok(())
}
