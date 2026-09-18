# Windows release validation

Validation date: 2026-09-18 (Australia/Adelaide). Scope: native Windows backend and shared Rust code. No other OS backend was changed or tested for this milestone.

## Environment

| Item | Value |
| --- | --- |
| OS | Windows 11 Pro 25H2, build 26200.9457 |
| Architecture | x86_64 / AMD64 |
| Primary host | Windows Terminal 1.24.11911.0 (dedicated session) |
| Additional host | Classic `C:\Windows\System32\conhost.exe` (dedicated process) |
| Automated host | Native console child created with `CREATE_NO_WINDOW` |
| Shell | PowerShell 7.6.6 |
| rustc | `rustc 1.98.1 (48a229cea 2026-09-01)` |
| Cargo | `cargo 1.98.1 (797e8a9bc 2026-08-05)` |
| Toolchain | `stable-x86_64-pc-windows-msvc` (active, default) |

`rustc --version`, `cargo --version`, and `rustup show` were executed on this machine. The registry's legacy `ProductName` says Windows 10 Pro; DisplayVersion 25H2 and build 26200 identify Windows 11. The agent's command runner has redirected streams and `TERM=dumb`; host testing therefore ran in separate real terminal sessions. Windows Terminal reports a nonempty `WT_SESSION`; the classic-host test does not. No Visual Studio IDE was used.

## Validation commands

| Command | Result |
| --- | --- |
| `cargo fmt --check` | Pass |
| `cargo check --all-targets` | Pass |
| `cargo test --all-targets` | Pass: 39 tests; no ignored tests |
| `cargo clippy --all-targets -- -D warnings` | Pass |
| `cargo doc --no-deps` | Pass, including Windows rustdoc link fix |
| `cargo test --doc` | Pass: 1 doctest |
| `cargo build --release` | Pass |
| `cargo package` | Refuses uncommitted changes, as expected |
| `cargo package --allow-dirty` | Pass, including verification build |
| `cargo --config net.offline=false publish --dry-run --allow-dirty` | Pass, including registry access and verification build; no upload |

Machine-readable command results are in [windows-validation-results.json](windows-validation-results.json). Full logs and terminal-host reports are under `target/` on the validation machine. Packaging used `--allow-dirty` to test the actual proposed source without committing it. Network access was explicitly enabled for the dry-run because this machine's Cargo configuration defaults to offline. The dry-run's final "aborting upload" warning is expected. The Windows CI job now runs the full suite, including clean-tree packaging and publish dry-run.

## Native checks

The default Windows test suite launches an isolated, windowless native console subprocess. Failure to create or use that console fails the test; there is no silent skip. The same native checks were also run in Windows Terminal and classic Console Host. The wrapper records a PASS/FAIL marker, environment metadata, and stderr:

```powershell
pwsh -NoProfile -File tools/windows-console-test.ps1
```

Run this in a dedicated terminal tab/window, since the tests inject input and change console state. The test intentionally catches a panic to verify restoration; its stderr includes "intentional console restoration test" even on success.

Covered by real console API calls:

- Owned handle lifetime, rejection of a second terminal, failed construction, and reuse after drop.
- Saved console modes, code pages, and cursor information; idempotent suspend/resume; normal and panic Drop; restoration retry after an invalid-handle failure; cleanup after read-only output fails during initialization.
- Disabling inherited VT input, Quick Edit, processed/echo/line input; enabling VT output; mouse reporting enable/disable.
- Native input queue injection and `ReadConsoleInputW` consumption: key repeats, supplementary Unicode surrogate pairs, ignored key-up/unknown records, timeout behavior, mouse press/release, and resize wake events.
- Public `Terminal` rendering, cursor position, native events, suspended behavior, resume, and redraw.
- UTF-8 output read back as UTF-16 screen content, including accented and wide BMP characters and a multibyte character at the 16 KiB write boundary.
- A real API-driven viewport size change followed by a native resize wake. The public event uses the queried viewport rather than the record's supplied dimensions. This check uses the main screen because `SetConsoleWindowInfo` targets it.

Additional record translation checks cover Ctrl/Alt/Shift preservation, Ctrl+I versus Tab, Ctrl+Space, F1/F24, AltGr flag combinations, isolated/mismatched surrogates, simultaneous mouse buttons, release while another button remains pressed, dragging, viewport-relative coordinates, and both wheel directions on each axis.

## Audit decisions and limits

- The implementation remains Rust using `windows-sys` and standard-library owned handles. Input is native records; bracketed-paste mode is no longer enabled because this backend does not decode a VT input stream.
- Only key-down records with a positive repeat count produce keys. F1 through F24 are supported. Special keys retain native modifiers. Ctrl-letter records use their virtual-key identity. AltGr is exposed as the Ctrl/Alt bits Windows supplies, without inventing layout-dependent normalization.
- Surrogate pairs must have matching modifier state. Their repeat count is the smaller of the two records' counts. Isolated, interrupted, or mismatched surrogates are discarded, and decoding recovers on subsequent input. Key-up events do not interrupt a pending pair.
- Mouse coordinates are relative to the viewport. Extra buttons use `Other(0)` and `Other(1)`; movement without a held button uses `Other(3)`. Horizontal wheel left/right uses `Other(4)`/`Other(5)` with `MouseKind::Scroll`. Each native wheel record produces one scroll event; the public API has no delta field. A double-click is represented by its button transitions.
- Valid UTF-8 write chunks end at character boundaries. Separate `send_raw` calls should contain complete characters. Invalid raw bytes follow console code-page replacement behavior.
- Native cursor-query behavior during alternate-screen use does not reliably reflect VT cursor visibility. Visibility assertions use the main buffer; normal lifecycle assertions check restored cursor information. Screen readback of supplementary characters through legacy console APIs is not used as proof of visual glyph rendering.
- The original console-global modes/code pages are restored; applications must coordinate other console users themselves. Screen restoration after output failure is best effort. Forced termination and aborting panics bypass Rust Drop.

The host tests use injected `INPUT_RECORD`s and API-driven resizing. They do not certify physical keyboard layouts, IME composition, actual mouse hardware, visual emoji/font fallback, or dragging the terminal window while drawing. These remain manual release checks using the existing `keyboard`, `mouse`, `unicode`, `resize`, and `lifecycle` examples. Windows 10, ARM64, and other Windows versions were not tested on this machine. This report establishes the automated native Windows release baseline; it does not claim those manual or additional-platform checks have passed.

API references used during the audit: [console modes](https://learn.microsoft.com/en-us/windows/console/setconsolemode), [native mouse records](https://learn.microsoft.com/en-us/windows/console/mouse-event-record-str), and [Microsoft's console query implementation](https://github.com/microsoft/terminal/blob/main/src/host/getset.cpp).
