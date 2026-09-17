#![cfg(target_os = "linux")]

use std::ffi::CStr;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::process::Command;
use std::time::Duration;
use terra_term::{Event, Key, Position, Size, Style, Terminal};

fn open_pty() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut master = -1;
    let mut slave = -1;
    // SAFETY: openpty writes two descriptors into initialized integer slots.
    if unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful openpty transfers ownership of both fresh descriptors.
    Ok(unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) })
}

fn clone_fd(fd: &OwnedFd) -> io::Result<OwnedFd> {
    // SAFETY: fd is open; dup returns an independent owned descriptor.
    let copy = unsafe { libc::dup(fd.as_raw_fd()) };
    if copy < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: dup returned a new descriptor owned by this function.
    Ok(unsafe { OwnedFd::from_raw_fd(copy) })
}

fn termios(fd: &OwnedFd) -> io::Result<libc::termios> {
    let mut value = std::mem::MaybeUninit::uninit();
    // SAFETY: fd is a PTY slave and tcgetattr writes a termios value on success.
    if unsafe { libc::tcgetattr(fd.as_raw_fd(), value.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: tcgetattr initialized the value.
    Ok(unsafe { value.assume_init() })
}

fn send(fd: &OwnedFd, bytes: &[u8]) {
    // SAFETY: fd is open and bytes points to readable memory.
    let written = unsafe { libc::write(fd.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) };
    assert_eq!(written, bytes.len() as isize);
}

fn drain(fd: &OwnedFd) -> Vec<u8> {
    let mut output = Vec::new();
    loop {
        let mut bytes = [0u8; 8192];
        // SAFETY: fd is open and bytes is writable.
        let count = unsafe { libc::read(fd.as_raw_fd(), bytes.as_mut_ptr().cast(), bytes.len()) };
        if count > 0 {
            output.extend_from_slice(&bytes[..count as usize]);
        } else if count == 0 || io::Error::last_os_error().kind() == io::ErrorKind::WouldBlock {
            break;
        } else {
            panic!("PTY read failed: {}", io::Error::last_os_error());
        }
    }
    output
}

fn tty_path(fd: &OwnedFd) -> io::Result<String> {
    let mut bytes = [0i8; 256];
    // SAFETY: ttyname_r writes a NUL-terminated path into the provided buffer.
    let code = unsafe { libc::ttyname_r(fd.as_raw_fd(), bytes.as_mut_ptr(), bytes.len()) };
    if code != 0 {
        return Err(io::Error::from_raw_os_error(code));
    }
    // SAFETY: ttyname_r succeeded and terminated the buffer.
    Ok(unsafe { CStr::from_ptr(bytes.as_ptr()) }
        .to_string_lossy()
        .into_owned())
}

#[test]
fn full_pty_lifecycle() -> io::Result<()> {
    // SAFETY: this integration-test binary does not concurrently modify TERM.
    unsafe {
        std::env::set_var("TERM", "xterm");
    }
    let (master, slave) = open_pty()?;
    let original = termios(&slave)?;
    let size = libc::winsize {
        ws_row: 12,
        ws_col: 40,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: the master is a PTY and size is initialized.
    assert_eq!(
        unsafe { libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ, &size) },
        0
    );
    // SAFETY: F_GETFL succeeds for this open descriptor; F_SETFL changes only our master.
    let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
    assert!(flags >= 0);
    assert_eq!(
        unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) },
        0
    );

    {
        let mut terminal = Terminal::from_fds(clone_fd(&slave)?, clone_fd(&slave)?)?;
        assert_eq!(
            terminal.size(),
            Size {
                width: 40,
                height: 12
            }
        );
        assert_eq!(termios(&slave)?.c_lflag & (libc::ICANON | libc::ECHO), 0);
        drain(&master);

        send(&master, b"a");
        assert!(
            matches!(terminal.read_event_timeout(Duration::from_secs(1))?, Some(Event::Key(key)) if key.key == Key::Char('a'))
        );
        send(&master, b"\x1b[");
        assert_eq!(
            terminal.read_event_timeout(Duration::from_millis(10))?,
            None
        );
        send(&master, b"A");
        assert!(
            matches!(terminal.read_event_timeout(Duration::from_secs(1))?, Some(Event::Key(key)) if key.key == Key::Up)
        );
        let unicode = "界".as_bytes();
        send(&master, &unicode[..1]);
        assert_eq!(
            terminal.read_event_timeout(Duration::from_millis(10))?,
            None
        );
        send(&master, &unicode[1..]);
        assert!(
            matches!(terminal.read_event_timeout(Duration::from_secs(1))?, Some(Event::Key(key)) if key.key == Key::Char('界'))
        );

        terminal.put_str(1, 1, "rendered", Style::default());
        terminal.set_cursor(10, 5);
        terminal.present()?;
        let output = drain(&master);
        assert!(output.windows(8).any(|bytes| bytes == b"rendered"));
        assert!(output.windows(7).any(|bytes| bytes == b"\x1b[6;11H"));
        assert!(output.ends_with(b"\x1b[?12l\x1b[?25h") || output.ends_with(b"\x1b[?25h"));
        assert_eq!(terminal.cursor(), Some(Position { x: 10, y: 5 }));

        let resized = libc::winsize {
            ws_row: 15,
            ws_col: 50,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: the master is open and resized is initialized.
        assert_eq!(
            unsafe { libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ, &resized) },
            0
        );
        // SAFETY: the library has installed its SIGWINCH handler for this process.
        assert_eq!(unsafe { libc::raise(libc::SIGWINCH) }, 0);
        assert_eq!(
            terminal.read_event_timeout(Duration::from_secs(1))?,
            Some(Event::Resize(Size {
                width: 50,
                height: 15
            }))
        );

        terminal.suspend()?;
        assert_eq!(
            termios(&slave)?.c_lflag & (libc::ICANON | libc::ECHO),
            original.c_lflag & (libc::ICANON | libc::ECHO)
        );
        terminal.resume()?;
        assert_eq!(termios(&slave)?.c_lflag & (libc::ICANON | libc::ECHO), 0);
    }
    assert_eq!(
        termios(&slave)?.c_lflag & (libc::ICANON | libc::ECHO),
        original.c_lflag & (libc::ICANON | libc::ECHO)
    );

    let path = tty_path(&slave)?;
    let output = Command::new(std::env::current_exe()?)
        .arg("--exact")
        .arg("redirected_stdout_child")
        .arg("--nocapture")
        .env("TERRA_TERM_PTY_CHILD", path)
        .env("TERM", "xterm")
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[test]
fn redirected_stdout_child() -> io::Result<()> {
    let Ok(path) = std::env::var("TERRA_TERM_PTY_CHILD") else {
        return Ok(());
    };
    // SAFETY: this is a separate child process; setsid creates its own session.
    assert!(unsafe { libc::setsid() } >= 0);
    let tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?;
    // SAFETY: tty is an open PTY slave and this process is the session leader.
    assert_eq!(
        unsafe { libc::ioctl(tty.as_raw_fd(), libc::TIOCSCTTY, 0) },
        0
    );
    let null = std::fs::File::options().write(true).open("/dev/null")?;
    // SAFETY: null is open and STDOUT_FILENO is the child process's descriptor.
    assert_eq!(
        unsafe { libc::dup2(null.as_raw_fd(), libc::STDOUT_FILENO) },
        libc::STDOUT_FILENO
    );
    let mut terminal = Terminal::new()?;
    terminal.set_cursor(1, 1);
    terminal.present()?;
    drop(terminal);
    assert!(Terminal::from_stdio().is_err());
    Ok(())
}
