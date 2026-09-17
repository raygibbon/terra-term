use std::io;
use terra_term::{Event, Key, Style, Terminal};
fn main() -> io::Result<()> {
    let mut term = Terminal::new()?;
    loop {
        term.clear();
        term.put_str(2, 1, "terra-term", Style::default());
        term.put_str(2, 3, "Hello from Rust", Style::default());
        term.put_str(2, 5, "Press Esc to quit", Style::default());
        term.present()?;
        if matches!(term.read_event()?, Event::Key(k) if k.key == Key::Escape) {
            break;
        }
    }
    Ok(())
}
