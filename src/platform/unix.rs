use super::Wake;
use crate::Size;
use crate::terminfo::Capabilities;
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::Duration;

static SIGNAL_FD: AtomicI32 = AtomicI32::new(-1);
static SIGNAL_OWNER: AtomicBool = AtomicBool::new(false);
extern "C" fn on_resize(_: libc::c_int) {
    let fd = SIGNAL_FD.load(Ordering::Relaxed);
    if fd >= 0 {
        let byte: u8 = 1; /* SAFETY: signal-safe write to our pipe. */
        unsafe {
            libc::write(fd, (&byte as *const u8).cast(), 1);
        }
    }
}

pub(crate) struct Backend {
    input: RawFd,
    output: RawFd,
    original: libc::termios,
    active: bool,
    resize_read: RawFd,
    resize_write: RawFd,
    previous_signal: libc::sigaction,
    caps: Capabilities,
    _owned_fds: Option<(OwnedFd, OwnedFd)>,
}

impl Backend {
    pub fn event_fds(&self) -> (BorrowedFd<'_>, BorrowedFd<'_>) {
        // SAFETY: the backend owns both descriptors for at least this borrow.
        unsafe {
            (
                BorrowedFd::borrow_raw(self.input),
                BorrowedFd::borrow_raw(self.resize_read),
            )
        }
    }
    pub fn is_active(&self) -> bool {
        self.active
    }
    pub fn new(caps: Capabilities) -> io::Result<Self> {
        let tty = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")?;
        let input = OwnedFd::from(tty.try_clone()?);
        let output = OwnedFd::from(tty);
        Self::from_fds(input, output, caps)
    }
    pub fn from_stdio(caps: Capabilities) -> io::Result<Self> {
        Self::new_impl(libc::STDIN_FILENO, libc::STDOUT_FILENO, caps, None)
    }
    pub fn from_fds(input: OwnedFd, output: OwnedFd, caps: Capabilities) -> io::Result<Self> {
        let input_fd = input.as_raw_fd();
        let output_fd = output.as_raw_fd();
        Self::new_impl(input_fd, output_fd, caps, Some((input, output)))
    }
    fn new_impl(
        input: RawFd,
        output: RawFd,
        caps: Capabilities,
        owned_fds: Option<(OwnedFd, OwnedFd)>,
    ) -> io::Result<Self> {
        // SAFETY: isatty examines valid process file descriptors.
        if unsafe { libc::isatty(input) } != 1 || unsafe { libc::isatty(output) } != 1 {
            return Err(io::Error::other("input and output must be terminals"));
        }
        if SIGNAL_OWNER
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(io::Error::other(
                "a terminal already owns SIGWINCH in this process",
            ));
        }
        let result = Self::create(input, output, caps, owned_fds);
        if result.is_err() {
            SIGNAL_OWNER.store(false, Ordering::Release);
        }
        result
    }
    fn create(
        input: RawFd,
        output: RawFd,
        caps: Capabilities,
        owned_fds: Option<(OwnedFd, OwnedFd)>,
    ) -> io::Result<Self> {
        let mut original = std::mem::MaybeUninit::<libc::termios>::uninit();
        // SAFETY: tcgetattr initializes original for a valid terminal descriptor.
        if unsafe { libc::tcgetattr(input, original.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: tcgetattr succeeded.
        let original = unsafe { original.assume_init() };
        let mut pipe = [-1; 2];
        // SAFETY: pipe points to two writable descriptors.
        if unsafe { libc::pipe(pipe.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        for fd in pipe {
            // SAFETY: both descriptors were created by pipe and remain open.
            if unsafe { libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK) } < 0
                || unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0
            {
                let error = io::Error::last_os_error();
                // SAFETY: both pipe descriptors are still owned here.
                unsafe {
                    libc::close(pipe[0]);
                    libc::close(pipe[1]);
                }
                return Err(error);
            }
        }
        // SAFETY: zeroed sigaction is initialized before use.
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = on_resize as *const () as usize;
        // SAFETY: action's mask is valid memory.
        unsafe {
            libc::sigemptyset(&mut action.sa_mask);
        }
        let mut previous = std::mem::MaybeUninit::<libc::sigaction>::uninit();
        SIGNAL_FD.store(pipe[1], Ordering::Release);
        // SAFETY: sigaction installs our handler and initializes previous.
        if unsafe { libc::sigaction(libc::SIGWINCH, &action, previous.as_mut_ptr()) } != 0 {
            SIGNAL_FD.store(-1, Ordering::Release);
            // SAFETY: pipe descriptors are open.
            unsafe {
                libc::close(pipe[0]);
                libc::close(pipe[1]);
            }
            return Err(io::Error::last_os_error());
        }
        // SAFETY: sigaction succeeded.
        let previous_signal = unsafe { previous.assume_init() };
        let mut backend = Self {
            input,
            output,
            original,
            active: false,
            resize_read: pipe[0],
            resize_write: pipe[1],
            previous_signal,
            caps,
            _owned_fds: owned_fds,
        };
        backend.resume()?;
        Ok(backend)
    }
    pub fn size(&self) -> io::Result<Size> {
        let mut ws = std::mem::MaybeUninit::<libc::winsize>::zeroed();
        // SAFETY: ioctl fills winsize for a valid terminal descriptor.
        if unsafe { libc::ioctl(self.output, libc::TIOCGWINSZ, ws.as_mut_ptr()) } < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: zeroed winsize is initialized, and ioctl succeeded.
        let ws = unsafe { ws.assume_init() };
        Ok(Size {
            width: ws.ws_col,
            height: ws.ws_row,
        })
    }
    pub fn wait(&mut self, timeout: Option<Duration>) -> io::Result<Wake> {
        let ms = timeout
            .map(|d| d.as_millis().min(i32::MAX as u128) as i32)
            .unwrap_or(-1);
        let mut fds = [
            libc::pollfd {
                fd: self.input,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: self.resize_read,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: fds points to two initialized pollfd values.
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), 2, ms) };
        if ready < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                return Ok(Wake::Resize);
            }
            return Err(e);
        }
        if ready == 0 {
            return Ok(Wake::Timeout);
        }
        if fds[1].revents & libc::POLLIN != 0 {
            let mut bytes = [0u8; 64];
            // SAFETY: pipe is nonblocking and bytes is writable.
            unsafe {
                libc::read(self.resize_read, bytes.as_mut_ptr().cast(), bytes.len());
            }
            return Ok(Wake::Resize);
        }
        if fds[0].revents & libc::POLLIN != 0 {
            return Ok(Wake::Input);
        }
        Err(io::Error::other("terminal input hangup or poll error"))
    }
    pub fn read(&self, bytes: &mut [u8]) -> io::Result<usize> {
        // SAFETY: bytes is writable for its length and input is a valid descriptor.
        let n = unsafe { libc::read(self.input, bytes.as_mut_ptr().cast(), bytes.len()) };
        if n < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(n as usize)
        }
    }
    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
        let mut rest = bytes;
        while !rest.is_empty() {
            // SAFETY: output is a valid descriptor and rest points to readable bytes.
            let n = unsafe { libc::write(self.output, rest.as_ptr().cast(), rest.len()) };
            if n < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "terminal write returned zero",
                ));
            }
            rest = &rest[n as usize..];
        }
        Ok(())
    }
    pub fn suspend(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        let mut seq = b"\x1b[?2004l\x1b[?1006l\x1b[?1015l\x1b[?1002l\x1b[?1000l".to_vec();
        seq.extend_from_slice(&self.caps.reset);
        seq.extend_from_slice(b"\x1b[?7h");
        seq.extend_from_slice(&self.caps.show_cursor);
        seq.extend_from_slice(&self.caps.exit_keypad);
        seq.extend_from_slice(&self.caps.exit_screen);
        let output = self.write(&seq);
        // SAFETY: original was obtained from this terminal descriptor.
        let raw = unsafe { libc::tcsetattr(self.input, libc::TCSAFLUSH, &self.original) };
        if raw == 0 {
            self.active = false;
        }
        output?;
        if raw != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    pub fn resume(&mut self) -> io::Result<()> {
        if self.active {
            return Ok(());
        }
        let mut raw = self.original;
        // SAFETY: raw points to a valid termios structure.
        unsafe {
            libc::cfmakeraw(&mut raw);
        }
        // SAFETY: input is a terminal descriptor and raw is initialized.
        if unsafe { libc::tcsetattr(self.input, libc::TCSAFLUSH, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        self.active = true;
        let mut seq = self.caps.enter_screen.clone();
        seq.extend_from_slice(&self.caps.enter_keypad);
        seq.extend_from_slice(b"\x1b[?7l");
        seq.extend_from_slice(&self.caps.hide_cursor);
        seq.extend_from_slice(b"\x1b[?2004h");
        seq.extend_from_slice(&self.caps.reset);
        if let Err(e) = self.write(&seq) {
            let _ = self.suspend();
            return Err(e);
        }
        Ok(())
    }
    pub fn set_mouse(&self, enabled: bool) -> io::Result<()> {
        if enabled {
            self.write(b"\x1b[?1000h\x1b[?1002h\x1b[?1015h\x1b[?1006h")
        } else {
            self.write(b"\x1b[?1006l\x1b[?1015l\x1b[?1002l\x1b[?1000l")
        }
    }
    pub fn clear_screen(&self) -> io::Result<()> {
        self.write(&self.caps.clear)
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        let _ = self.suspend();
        SIGNAL_FD.store(-1, Ordering::Release);
        // SAFETY: restore prior handler and close descriptors owned by this backend.
        unsafe {
            libc::sigaction(libc::SIGWINCH, &self.previous_signal, std::ptr::null_mut());
            libc::close(self.resize_read);
            libc::close(self.resize_write);
        }
        SIGNAL_OWNER.store(false, Ordering::Release);
    }
}
