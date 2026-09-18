# terra-term

terra-term is a small, low-level, cell-oriented terminal library written entirely in Rust. It manages terminal initialization and restoration, raw input, an alternate screen, buffered cell rendering, Unicode graphemes, keyboard and mouse events, bracketed paste, resize events, colours, and terminal capabilities. It has native Unix and Windows backends.

```rust
use terra_term::{Event, Key, Style, Terminal};

fn main() -> std::io::Result<()> {
    let mut terminal = Terminal::new()?;
    loop {
        terminal.clear();
        terminal.put_str(2, 2, "Hello", Style::default());
        terminal.present()?;
        if matches!(terminal.read_event()?, Event::Key(key) if key.key == Key::Escape) {
            break;
        }
    }
    Ok(())
}
```

`Terminal::new()` opens the controlling terminal (`/dev/tty`) on Unix. It works when process stdout is redirected if a controlling terminal exists. `from_stdio()` uses process stdin and stdout explicitly; `from_fds()` accepts owned terminal descriptors. The terminal restores its previous mode on drop. `suspend()` and `resume()` allow temporary use of the normal screen.

Drawing changes the back buffer. `present()` writes changed rows and restores the logical cursor position and visibility. `Cell::grapheme` stores a cluster; `put_str` segments text into clusters. `Color::Indexed(0..=15)` selects standard and bright colours, `Indexed(16..=255)` selects the extended palette, and `Rgb` selects true colour. Bold is a separate style attribute. `set_clear_style()` controls subsequent clears.

`read_event()` blocks, `read_event_timeout()` waits for a duration, and `try_event()` checks immediately. `InputMode::Escape` reports an unrecognized Escape separately; `InputMode::Alt` combines it with the following key. A lone Escape uses a 30 ms ambiguity period. Paste data arrives in bounded chunks with a completion flag. Unix and Windows normalize control-key aliases where their input sources permit it.

terra-term is synchronous and ends at the terminal abstraction. It is not a widget framework, layout engine, application framework, or async runtime. See [features](docs/FEATURES.md) for platform details and current limits.

On Windows, use a console attached to stdin and stdout (Windows Terminal is recommended). Redirected streams are rejected. The backend owns duplicate console handles and permits one `Terminal` per process, including while suspended. Native C0 characters use the shared normalization: Ctrl+H is Backspace, Ctrl+I is Tab, Ctrl+J/Ctrl+M are Enter, and Ctrl+[ is Escape. Ctrl+\\ is `Char('\\')` with Ctrl. Ctrl+C arrives as a key; Ctrl+Break is consumed as a Windows signal while active, with no key event. Suspend/drop removes that signal handler. `InputMode` affects the byte parser, not native console records. Paste arrives as key records. See the [Windows validation report](docs/WINDOWS_VALIDATION.md) for tested hosts, build commands, and native input details.

Build with `cargo build` using a normal Rust toolchain. No C compiler, terminal development headers, or external terminal utility is required. The crate is MIT licensed; adapted built-in terminal sequence data retains its required notice in [third-party notices](THIRD_PARTY_NOTICES.md).
