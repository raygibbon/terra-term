# Unsafe Rust audit

The library exposes a safe public API. Unsafe code is confined to `src/platform/unix.rs` and `src/platform/windows.rs`; the input parser, terminfo parser, and renderer use safe Rust.

## Unix

`libc` calls receive valid file descriptors and initialized Rust buffers. `MaybeUninit` values are read only after successful `tcgetattr`, `ioctl`, or `sigaction` calls. The resize self-pipe descriptors are created together, checked during setup, and closed in `Drop`; the backend is the sole owner. Borrowed event descriptors cannot outlive the backend. The SIGWINCH callback loads one atomic descriptor and performs a one-byte `write`, which is async-signal-safe. The atomic is set to `-1` before the pipe is closed. Only one terminal can own the process signal handler at a time.

`from_stdio()` borrows process descriptors; callers must keep them open while the terminal exists. `new()` and `from_fds()` own their terminal descriptors. The terminal mode is restored before owned descriptors are dropped.

## Windows

Console handles are checked before use and duplicated into Rust `OwnedHandle`s. The process standard handles are never closed by the backend. Raw handle aliases stay valid through backend cleanup; the owned duplicates close afterward. A process-local atomic lease prevents overlapping terminal lifetimes from saving and restoring conflicting console state, including while suspended. Failed construction releases the lease.

Win32 calls receive initialized records or writable output pointers, and return values are checked. Input record union fields are read only after their event type is checked. UTF-16 surrogate conversion checks ranges before arithmetic and never creates an invalid Rust `char`. Key repeats are drained without allocating an event per repeat. Console writes handle partial writes and zero-byte writes, and chunks of valid UTF-8 end at character boundaries.

Initialization saves input/output modes, code pages, and cursor information before mutation. Every setup error attempts restoration. VT cleanup is emitted only after VT output was enabled. Restoration attempts every saved setting and captures each failure immediately, retaining the first restoration error. A failed output write is returned ahead of restoration errors. Active state and outstanding cleanup are separate: a partially restored terminal is suspended, and `Drop` or a subsequent resume retries remaining restoration. `Drop` does not panic. Screen cleanup after a failed console write remains best effort.

Tests use real Windows console handles and isolated child processes, including injected native records, readback, restoration failures, constructor failures, normal drop, and panic unwinding. See [Windows validation](WINDOWS_VALIDATION.md) for host results and limits.

## Remaining risk

Signal ownership is process global. Code outside terra-term that replaces SIGWINCH while a `Terminal` exists may have its handler overwritten during restoration. On Windows, unrelated code or another process attached to the same console can still change console-global modes/code pages or consume its input. The lease coordinates this crate's terminal instances only. Forced process termination, aborting panics, or detaching the console cannot guarantee restoration.
