//! A small, low-level, cell-oriented terminal library written in Rust.
//!
//! [`Terminal`] owns a terminal connection, keeps a back buffer of cells, and
//! restores terminal state when dropped. Drawing methods change the buffer;
//! [`Terminal::present`] writes changes and restores the requested cursor.
//! Events are read synchronously with [`Terminal::read_event`],
//! [`Terminal::read_event_timeout`], or [`Terminal::try_event`].
//!
//! On Unix, [`Terminal::new`] opens the controlling terminal. Applications that
//! want process standard streams can use [`Terminal::from_stdio`].
//!
//! ```no_run
//! use terra_term::{Event, Key, Style, Terminal};
//!
//! # fn main() -> std::io::Result<()> {
//! let mut terminal = Terminal::new()?;
//! loop {
//!     terminal.clear();
//!     terminal.put_str(0, 0, "Hello", Style::default());
//!     terminal.present()?;
//!     match terminal.read_event()? {
//!         Event::Key(key) if key.key == Key::Escape => break,
//!         Event::Resize(_) => continue,
//!         _ => {}
//!     }
//! }
//! # Ok(())
//! # }
//! ```

mod buffer;
mod cell;
mod color;
mod event;
mod key;
mod platform;
mod terminal;
mod terminfo;

pub use cell::Cell;
pub use color::{Color, Style};
pub use event::{Event, InputMode, InputParser, MouseButton, MouseEvent, MouseKind, PasteEvent};
pub use key::{Key, KeyEvent, Modifiers};
pub use terminal::{Position, Size, Terminal};
