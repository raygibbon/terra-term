use std::io;
use terra_term::{Event, Key, Style, Terminal};
fn main() -> io::Result<()> {
    let mut term = Terminal::new()?;
    term.set_mouse_enabled(true)?;
    term.put_str(1, 1, "Mouse events (Esc quits)", Style::default());
    term.present()?;
    loop {
        let event = term.read_event()?;
        if matches!(event, Event::Key(k) if k.key == Key::Escape) {
            break;
        }
        if let Event::Mouse(mouse) = event {
            term.clear();
            term.put_str(1, 1, "Mouse events (Esc quits)", Style::default());
            term.put_str(1, 3, &format!("{mouse:?}"), Style::default());
            term.present()?;
        }
    }
    Ok(())
}
