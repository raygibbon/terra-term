use std::io;
use terra_term::{Event, Key, Style, Terminal};
fn main() -> io::Result<()> {
    let mut term = Terminal::new()?;
    loop {
        let size = term.size();
        term.clear();
        term.put_str(1, 1, "Resize the terminal (Esc quits)", Style::default());
        term.put_str(
            1,
            3,
            &format!("{} columns × {} rows", size.width, size.height),
            Style::default(),
        );
        term.present()?;
        match term.read_event()? {
            Event::Key(k) if k.key == Key::Escape => break,
            Event::Resize(_) => {}
            _ => {}
        }
    }
    Ok(())
}
