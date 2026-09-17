use std::io;
use terra_term::{Event, Key, Style, Terminal};
fn main() -> io::Result<()> {
    let mut term = Terminal::new()?;
    let mut last = String::from("Press keys; Esc quits");
    loop {
        term.clear();
        term.put_str(1, 1, "Input events", Style::default());
        term.put_str(1, 3, &last, Style::default());
        term.present()?;
        let event = term.read_event()?;
        if matches!(event, Event::Key(k) if k.key == Key::Escape) {
            break;
        }
        last = format!("{event:?}");
    }
    Ok(())
}
