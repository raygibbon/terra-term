#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub(crate) use unix::Backend;
#[cfg(windows)]
pub(crate) use windows::Backend;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Wake {
    #[cfg(unix)]
    Input,
    Resize,
    #[cfg(windows)]
    Native,
    Timeout,
}
