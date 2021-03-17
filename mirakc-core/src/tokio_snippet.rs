use std::io;
use std::os::unix::io::{AsRawFd, RawFd};

use mio::event::Evented;
use mio::unix::{EventedFd, UnixReady};
use mio::{Poll as MioPoll, PollOpt, Ready, Token};
use tokio::io::PollEvented;

// This module contains code snippets taken from src/process/unix/mod.rs in
// the tokio-rs/tokio GitHub repository, and provides asynchronous read/write
// functions to the following types in the `std::process` modules:
//
// * std::process::ChildStdin
// * std::process::ChildStdout
// * std::process::ChildStderr
//
// The `tokio::process` module is very useful when spawning processes in an
// async/await context.  However, there is a performance issue when implementing
// a command pipeline.
//
// `tokio::process::{ChildStdout, ChildStderr}` cannot be used as an input for
// `std::process::Command::stdin()`.  Because `std::process::Stdio` doesn't
// implement `std::convert::From` for these types.  This means that we cannot
// connect the stdout of a process to the stdin of next process directly.
// It's still possible to copy data between them by using `tokio::io::copy()`,
// but it requires a buffer which is not required when connecting them directly.
//
// This issue can be solved by simply exporting `stdio()` from the
// `tokio::process::unix` module.

pub(crate) struct Fd<T> {
    inner: T,
}

impl<T> io::Read for Fd<T>
where
    T: io::Read,
{
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.inner.read(bytes)
    }
}

impl<T> io::Write for Fd<T>
where
    T: io::Write,
{
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.inner.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl<T> AsRawFd for Fd<T>
where
    T: AsRawFd,
{
    fn as_raw_fd(&self) -> RawFd {
        self.inner.as_raw_fd()
    }
}

impl<T> Evented for Fd<T>
where
    T: AsRawFd,
{
    fn register(
        &self,
        poll: &MioPoll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        EventedFd(&self.as_raw_fd()).register(poll, token, interest | UnixReady::hup(), opts)
    }

    fn reregister(
        &self,
        poll: &MioPoll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        EventedFd(&self.as_raw_fd()).reregister(poll, token, interest | UnixReady::hup(), opts)
    }

    fn deregister(&self, poll: &MioPoll) -> io::Result<()> {
        EventedFd(&self.as_raw_fd()).deregister(poll)
    }
}

pub(crate) fn stdio<T>(option: Option<T>) -> io::Result<Option<PollEvented<Fd<T>>>>
where
    T: AsRawFd,
{
    let io = match option {
        Some(io) => io,
        None => return Ok(None),
    };

    // Set the fd to nonblocking before we pass it to the event loop
    unsafe {
        let fd = io.as_raw_fd();
        let r = libc::fcntl(fd, libc::F_GETFL);
        if r == -1 {
            return Err(io::Error::last_os_error());
        }
        let r = libc::fcntl(fd, libc::F_SETFL, r | libc::O_NONBLOCK);
        if r == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(Some(PollEvented::new(Fd { inner: io })?))
}

pub(crate) type ChildIo<T> = PollEvented<Fd<T>>;
