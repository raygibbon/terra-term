use std::io;
use terra_term::{Color, Event, Key, Style, Terminal};
fn main() -> io::Result<()> {
    let mut term = Terminal::new()?;
    loop {
        term.clear();
        term.put_str(1, 0, "Colours and styles (Esc quits)", Style::default());
        for i in 0..16u8 {
            let style = Style::new(Color::Indexed(i), Color::Default);
            term.put_str(
                1,
                u16::from(i) + 2,
                &format!("Indexed {i:>3}: sample"),
                style,
            );
        }
        let style = Style {
            bold: true,
            ..Style::default()
        };
        term.put_str(28, 2, "bold", style);
        let style = Style {
            dim: true,
            ..Style::default()
        };
        term.put_str(28, 3, "dim", style);
        let style = Style {
            underline: true,
            ..Style::default()
        };
        term.put_str(28, 4, "underline", style);
        let style = Style {
            reverse: true,
            ..Style::default()
        };
        term.put_str(28, 5, "reverse", style);
        term.present()?;
        if matches!(term.read_event()?, Event::Key(k) if k.key == Key::Escape) {
            break;
        }
    }
    Ok(())
}
