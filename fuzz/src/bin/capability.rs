//! Feed a parameterized cursor capability from stdin.
use std::io::{self, Read};
#[path = "../../../src/terminfo/expand.rs"]
mod expand;

fn main() -> io::Result<()> {
    let mut input = Vec::new();
    io::stdin().take(4096).read_to_end(&mut input)?;
    let row = u16::from_le_bytes([*input.first().unwrap_or(&0), *input.get(1).unwrap_or(&0)]);
    let column = u16::from_le_bytes([*input.get(2).unwrap_or(&0), *input.get(3).unwrap_or(&0)]);
    let template = input.get(4..).unwrap_or_default();
    let _ = expand::expand(template, [i32::from(row), i32::from(column)]);
    Ok(())
}
