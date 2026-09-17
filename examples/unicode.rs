use std::io;
use terra_term::{Event, Key, Style, Terminal};
fn main() -> io::Result<()> {
    let mut term = Terminal::new()?;
    loop {
        term.clear();
        term.put_str(1, 1, "Unicode (Esc quits)", Style::default());
        term.put_str(
            1,
            3,
            "Combining: e\u{301}  Wide: 界  Emoji: 👩‍💻",
            Style::default(),
        );
        term.present()?;
        match term.read_event()? {
            Event::Key(k) if k.key == Key::Escape => break,
            _ => {}
        }
    }
    Ok(())
}
