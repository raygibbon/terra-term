use std::io;
use terra_term::{Event, Key, Style, Terminal};
fn main() -> io::Result<()> {
    let mut term = Terminal::new()?;
    term.put_str(1, 1, "Paste text here (Esc quits)", Style::default());
    term.present()?;
    loop {
        let event = term.read_event()?;
        if matches!(event, Event::Key(k) if k.key == Key::Escape) {
            break;
        }
        if let Event::Paste(paste) = event {
            term.clear();
            term.put_str(1, 1, "Paste text here (Esc quits)", Style::default());
            term.put_str(
                1,
                3,
                &format!("{} bytes, complete: {}", paste.bytes.len(), paste.complete),
                Style::default(),
            );
            term.put_str(1, 5, &paste.text(), Style::default());
            term.present()?;
        }
    }
    Ok(())
}
