# Unsafe Rust audit

The library exposes a safe public API. Unsafe code is confined to `src/platform/unix.rs` and `src/platform/windows.rs`; the input parser, terminfo parser, and renderer use safe Rust.

## Unix

`libc` calls receive valid file descriptors and initialized Rust buffers. `MaybeUninit` values are read only after successful `tcgetattr`, `ioctl`, or `sigaction` calls. The resize self-pipe descriptors are created together, checked during setup, and closed in `Drop`; the backend is the sole owner. Borrowed event descriptors cannot outlive the backend. The SIGWINCH callback loads one atomic descriptor and performs a one-byte `write`, which is async-signal-safe. The atomic is set to `-1` before the pipe is closed. Only one terminal can own the process signal handler at a time.

`from_stdio()` borrows process descriptors; callers must keep them open while the terminal exists. `new()` and `from_fds()` own their terminal descriptors. The terminal mode is restored before owned descriptors are dropped.

## Windows

Console handles are checked before use. Win32 calls receive initialized records or writable output pointers, and return values are checked. Input record union fields are read only after their event type is checked. The backend borrows process standard handles and restores their saved modes, code pages, and cursor settings when it shuts down.

## Remaining risk

Signal ownership is process global. Code outside terra-term that replaces SIGWINCH while a `Terminal` exists may have its handler overwritten during restoration. Windows console mouse and resize behavior still needs real-console testing.
