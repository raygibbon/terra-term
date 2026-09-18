use std::io;
use std::io::Write;
use terra_term::{Event, Key, Style, Terminal};
fn main() -> io::Result<()> {
    let mut log = std::env::var_os("TERRA_TERM_INPUT_LOG")
        .map(std::fs::File::create)
        .transpose()?;
    let mut term = Terminal::new()?;
    term.set_mouse_enabled(true)?;
    let mut last = String::from("Press keys; Esc quits");
    loop {
        term.clear();
        term.put_str(1, 1, "Input events", Style::default());
        term.put_str(1, 3, &last, Style::default());
        term.present()?;
        let event = term.read_event()?;
        if let Some(log) = &mut log {
            writeln!(log, "{event:?}")?;
            log.flush()?;
        }
        if matches!(event, Event::Key(k) if k.key == Key::Escape) {
            break;
        }
        last = format!("{event:?}");
    }
    Ok(())
}
