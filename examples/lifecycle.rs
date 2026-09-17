use std::io;
use terra_term::{Style, Terminal};
fn main() -> io::Result<()> {
    let mut term = Terminal::new()?;
    term.put_str(1, 1, "Press any key to suspend", Style::default());
    term.present()?;
    let _ = term.read_event()?;
    term.suspend()?;
    println!("Normal terminal mode restored");
    term.resume()?;
    term.put_str(1, 1, "Press any key to quit", Style::default());
    term.present()?;
    let _ = term.read_event()?;
    Ok(())
}
