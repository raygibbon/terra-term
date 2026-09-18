# terra-term features

| Capability | Status | Detail |
|---|---|---|
| Terminal lifecycle and restoration | Supported | Raw input, alternate screen, RAII shutdown, suspend/resume |
| Buffered cell rendering | Supported | Back/front comparison and changed-row output |
| Unicode and grapheme clusters | Supported | Wide cells, combining sequences, and visible replacement for standalone controls |
| Standard, bright, indexed, and RGB colour | Supported | Explicit `Color` values; bold is a separate attribute |
| Keyboard and modified keys | Supported | CSI, SS3, registered terminal sequences, and platform control-key normalization |
| Mouse input | Supported | Press, release, movement, wheel, and modifiers where reported |
| Bracketed paste | Supported on Unix VT input | Bounded payload chunks; Windows console paste currently arrives as key records |
| Resize events | Supported | SIGWINCH wake pipe on Unix; native console events on Windows |
| Compiled terminfo | Supported subset | Standard string capabilities and bounded numeric parameter expansion for cursor position |
| Built-in terminal models | Supported | Used when a matching compiled entry is unavailable |
| Controlling terminal connection | Unix | `Terminal::new()` opens `/dev/tty` |
| Explicit stdio or owned descriptors | Unix | `from_stdio()`, `from_fds()`, `from_tty_path()` |
| Native Windows console backend | Tested on Windows 11 x64 | Native API checks passed in Windows Terminal and classic Console Host; see [validation report](WINDOWS_VALIDATION.md) for scope and remaining manual checks |

The API is synchronous. `InputParser` is available independently of a terminal for protocol decoding and tests. The terminfo evaluator deliberately supports the operations needed by consumed capabilities; unsupported expressions return an error. Terminal output assumes VT-style colour sequences. There is no widget or layout layer.
