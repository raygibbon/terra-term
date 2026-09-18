# Windows 0.1.0 release validation

Validation date: 2026-09-18, Australia/Adelaide. Scope: native Windows and shared Rust. No other OS backend was changed or tested.

## Readiness decision

Automated correctness and release-artifact checks pass on this Windows machine. **Full 0.1.0 Windows sign-off remains pending the requested human interactive checks.** The Windows Terminal manual runner was opened, but no observations or physical input results were submitted. Do not treat automated console injection as manual testing or claim that physical Ctrl+backslash, IME, fonts, hardware mouse input or visual RGB rendering was manually verified.

## Environment

| Item | Recorded value |
| --- | --- |
| Windows | Windows 11 Pro 25H2, build 26200.9457 |
| Architecture | AMD64 / x86_64 |
| Windows Terminal | 1.24.11911.0; dedicated attached test sessions |
| Classic Console Host | System32/conhost.exe; dedicated attached test process |
| Isolated automated console | CREATE_NEW_CONSOLE; process attachment verified before opening CONIN$/CONOUT$ |
| Shell | PowerShell 7.6.6 |
| Rust | rustc 1.98.1 (48a229cea 2026-09-01) |
| Cargo | cargo 1.98.1 (797e8a9bc 2026-08-05) |
| Toolchain | stable-x86_64-pc-windows-msvc, active/default |

Versions were queried directly. The registry's legacy ProductName says Windows 10 Pro; build 26200/25H2 identifies Windows 11. No Visual Studio IDE was used. The command runner's redirected streams are distinct from both attached host sessions. Wine was not used.

## Commands and results

| Exact command | Outcome |
| --- | --- |
| cargo fmt --check | Pass |
| cargo check --all-targets | Pass |
| cargo test --all-targets | Pass: 51 tests, zero ignored |
| cargo test --lib platform::windows -- --nocapture | Pass: 5 Windows groups; native group contains the lifecycle, input and failure matrices |
| cargo test --test sequence_recovery | Pass: 9 parser regression tests |
| cargo clippy --all-targets -- -D warnings | Pass |
| cargo doc --no-deps | Pass |
| cargo test --doc | Pass: 1 doctest |
| cargo build --release | Pass |
| cargo package | Pass in the clean release-source snapshot; working checkout correctly refuses uncommitted edits |
| cargo package --allow-dirty | Pass in working checkout, including verification build |
| cargo publish --dry-run | Pass in clean release-source snapshot with CARGO_NET_OFFLINE=false; nothing published |
| cargo --config net.offline=false publish --dry-run --allow-dirty | Pass in working checkout; nothing published |
| cargo package --list | Pass in clean snapshot; working checkout requires --allow-dirty |
| cargo package --list --allow-dirty | Package file inventory checked |
| cargo tree | Rust dependencies: unicode-segmentation, unicode-width, windows-sys, windows-link |
| cargo tree -e build | No build dependencies |
| Get-ChildItem -Recurse -Include *.c,*.h,*.cc,*.cpp | No C/C++ implementation files found |
| rg -n "\bunsafe\b" src | Windows blocks reviewed; other backend not audited or modified in this pass |

Development-only machine results are in docs/windows-validation-results.json in the repository, with logs in target/. They are excluded from the crate. Package and dry-run commands were also run from a fresh temporary snapshot copied from Cargo's package inventory, with source hashes checked against the working copy. This permits clean packaging without creating a commit or discarding existing edits. Cargo's expected "aborting upload due to dry run" message is not an upload.

## Parser corrections

The registered matcher distinguishes no match, prefix, complete match, and complete match with a possible longer registration. A complete prefix waits; finish_incomplete_sequence resolves the longest completed registration on timeout. A mismatch resolves the longest completed match and leaves subsequent bytes queued. Registration order does not affect the result. Identical registrations are harmless; conflicting mappings for identical bytes produce UnknownSequence deterministically.

Tests cover shorter sequences alone, two and three overlaps, every split of the longer sequence, duplicate/common prefixes, reverse registration, conflicting duplicates and immediately following events. Registrations longer than 64 bytes are rejected by the existing bounded registration API; oversized CSI input matching such an attempted registration is tested as malformed input.

Oversized CSI/SS3 enters explicit discard/recovery state and emits UnknownSequence chunks of at most 64 bytes until the terminator. ESC, control or non-ASCII input interrupts recovery without being swallowed. Invalid parameters and unknown final sequences also produce UnknownSequence. No malformed payload is reinterpreted as ordinary text. Tests cover large fragmented sequences, truncation, repeated ESC, randomized parameter bytes and an immediately following valid key. Sequence state/output is bounded; feed still accepts caller-provided queued input, so callers should drain events between bounded feeds.

## Windows results

| Area | Evidence / behavior |
| --- | --- |
| Harness | CREATE_NO_WINDOW removed. CREATE_NEW_CONSOLE child verifies its PID in GetConsoleProcessList; CONIN$/CONOUT$, mode queries and native input/output must work. Failures fail clearly; no tests silently skipped. |
| Mouse transitions | All requested left/right/middle press/release transitions, combined buttons and unchanged buttons pass. Release identity uses previous state. Movement/drag and vertical/horizontal scroll remain distinct. |
| Coordinates | Both edges of offset and resized viewports tested, including movement, drag and wheel. Coordinates are viewport-relative. |
| Keyboard repeats | 1, 2, 5, 20 and 65,535 repeats preserve order with a following key. No per-repeat bulk allocation; at most one repeated event is queued at a time. |
| Control normalization | Ctrl+A-Z and C0 punctuation use shared control_byte semantics. Ctrl+H -> Backspace; Ctrl+I -> Tab; Ctrl+J/Ctrl+M -> Enter; Ctrl+[ -> Escape; these aliases do not retain Ctrl. Alt is retained when separately reported. Ctrl+Space -> Ctrl+@. |
| Ctrl+backslash | Automated native record check yields Char('\\') with CTRL, not Escape or a control signal. Physical press remains unverified. |
| Ctrl+C | ENABLE_PROCESSED_INPUT is cleared. Normal Ctrl+C is a key event, Char('C') with CTRL. |
| Ctrl+Break | Windows always treats it as a signal. A scoped handler consumes only CTRL_BREAK_EVENT while active; no key event is invented. A real GenerateConsoleCtrlEvent test confirms delivery and survival in the isolated console. A pre-existing observer receives Break after suspend removes the handler. |
| Unicode | BMP, emoji, supplementary scalar, valid pairs, orphan high/low, two high units, interrupted pairs, modifier mismatch and repeat counts pass. Malformed units are discarded; following ordinary keys survive. Matching pairs use the smaller repeat count. |
| UTF-8 output | Boundary tests cover ASCII, 2/3/4-byte scalars, combining graphemes and ZWJ graphemes. Each chunk is valid UTF-8 and concatenation equals the original bytes. Native accented/wide-BMP screen readback also passes. |
| Resize | Native wake records plus real API viewport width/height increases/decreases, rapid successive changes, identical notifications, key and mouse ordering pass. Public size and rendering dimensions update. Duplicate notifications do not emit duplicate resize events. |
| Cursor | Hide/show now remembers logical position. set/present, hide/present, show/present, visible/hidden resize, invalidate/full redraw and suspend/resume preserve position. Restored native cursor information matches the baseline. |
| Alternate screen | Recognizable main-buffer marker survives alternate-screen output, repeated drop and injected setup failures. Both attached hosts pass API readback. Visual human confirmation remains pending. |
| Ownership/lease | Standard handles are borrowed only long enough to query/duplicate. Rust OwnedHandle fields own duplicates and close after cleanup; raw aliases never close anything. Originals remain usable. Simultaneous terminal B is rejected; dropping/failing A releases the lease. |
| Repeated lifecycle | 20 create/drop cycles, each with 3 suspend/resume cycles, preserve modes/cursor/code pages and leave the process handle count unchanged. Double suspend/resume is harmless. |
| Partial failures | Test checkpoints after handle acquisition, output page, input mode, output mode, mouse, alternate screen and cursor all restore saved state and release the lease. Read-only output, invalid handles and failed restoration/retry also pass. |
| Code pages | Native W input leaves the input page unchanged. Only the output page changes to 65001. Tests start with input 850/output 437 and verify restoration after errors and repeated cycles; the fixture restores the original host pages on exit. |
| Queues | key/key/key, key/mouse/key, resize/key, mouse/resize/key, Unicode pair/key and repeat/key preserve order. Unsupported key-up records are intentionally ignored. |
| Waits | Empty, queued and timed cases pass for try_event/read_event_timeout/read_event. Native waits use WaitForSingleObject; measured timed waits do not return early and have a generous scheduling upper bound. |
| Suspend/resume | Modes, VT output, mouse selection, dimensions, cursor and redraw recover repeatedly; suspended reads/presents behave as documented. |
| Unsafe | Pointer extents, initialized structs, union discriminators, owned handle lifetime, callback ABI/lifetime and immediate Win32 error capture reviewed. See UNSAFE_AUDIT.md. |

Horizontal wheel uses Other(4) for left and Other(5) for right; extra buttons use Other(0)/Other(1), and motion without a button uses Other(3). A wheel record produces one scroll event because the public model has no delta field. Native paste remains key records. InputMode only affects the byte parser.

## Attached hosts versus manual validation

Automated attached-console checks passed in Windows Terminal and classic Console Host, separately from the newly allocated-console test. Reports under target/windows-terminal-test.txt and target/classic-console-test.txt include PASS markers and host metadata. The native group intentionally catches a panic to verify Drop; its stderr includes "intentional console restoration test" on a passing run.

Manual diagnostic tooling is retained in the repository (not the crate package):

- tools/windows-console-test.ps1 runs attached API checks.
- tools/windows-interactive.ps1 runs the existing input, Unicode, colors, resize and lifecycle examples and records the operator's actual observations.
- examples/input.rs can log observed events using TERRA_TERM_INPUT_LOG.

The Windows Terminal interactive runner was launched. Its observation array remained empty when the user requested finishing. No physical keyboard/mouse or visual result is claimed. Classic Console Host was tested automatically; its human interactive pass remains unverified. These are the remaining release acceptance items, not failed automated tests.

## Package and implementation

The package inventory includes Rust source, tests/fixtures, examples, manifest/lockfile, static documentation, LICENSE and THIRD_PARTY_NOTICES.md. It excludes target output, executables, PDBs, console logs, temporary reports, tooling, fuzz artifacts and nested archives. Cargo's generated metadata/normalized manifest are expected. The crate introduces no C/C++ sources, C compiler build dependency, UI framework or architecture replacement.

Changed areas: src/event.rs and parser regression tests; src/platform/windows.rs and its test modules; src/terminal.rs cursor persistence; examples/input.rs diagnostic logging; Cargo.toml package exclusions; README and Windows audit/validation documentation; Windows diagnostic scripts. The preceding pass's Windows CI and shared Clippy/rustdoc fixes remain. No non-Windows backend changed.

## Limits and references

Human visual/input sign-off is outstanding. Windows 10, ARM64, alternate keyboard layouts/IME, physical F13-F24 and font fallback were not verified. Legacy screen-read APIs are not proof of supplementary glyph rendering. GetConsoleCursorInfo's alternate-buffer behavior requires visibility checks on the main screen. Resize API tests deliberately leave the alternate buffer because SetConsoleWindowInfo targets the main buffer; user-driven resizing during alternate-screen drawing remains a manual check.

Applications must coordinate other console users. Another application handler installed later can intercept control signals before terra-term. Detaching the console resets handlers. While suspended, normal application/Windows Ctrl+Break behavior applies. Forced termination and aborting panics bypass Drop. Screen recovery after a failed output write is best effort.

References: [console modes](https://learn.microsoft.com/en-us/windows/console/setconsolemode), [Ctrl+C and Ctrl+Break](https://learn.microsoft.com/en-us/windows/console/ctrl-c-and-ctrl-break-signals), [control handler lifetime](https://learn.microsoft.com/en-us/windows/console/setconsolectrlhandler), [mouse records](https://learn.microsoft.com/en-us/windows/console/mouse-event-record-str).
