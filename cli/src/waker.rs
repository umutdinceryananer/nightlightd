//! A wake-up channel from the D-Bus thread to the poll loop (#18).
//!
//! An eventfd the D-Bus handlers poke so a request is applied at once instead
//! of waiting for the next minute tick. The poll loop adds it to its `poll` set.

use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::sync::Arc;

use rustix::event::{EventfdFlags, eventfd};

/// A cloneable handle to a wake-up eventfd. The D-Bus handlers call
/// [`Waker::wake`]; the poll loop polls [`Waker::as_fd`] and [`Waker::drain`]s.
#[derive(Clone)]
pub struct Waker(Arc<OwnedFd>);

/// Creates a fresh wake-up eventfd.
pub fn waker() -> std::io::Result<Waker> {
    let fd = eventfd(0, EventfdFlags::NONBLOCK | EventfdFlags::CLOEXEC)?;
    Ok(Waker(Arc::new(fd)))
}

impl Waker {
    /// Wakes the poll loop. A failed write only delays the change to the next
    /// tick, so the error is ignored.
    pub fn wake(&self) {
        let _ = rustix::io::write(self.0.as_fd(), &1u64.to_ne_bytes());
    }

    /// The fd the poll loop waits on.
    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }

    /// Drains pending wake-ups once the poll loop has been woken.
    pub fn drain(&self) {
        let mut buf = [0u8; 8];
        while rustix::io::read(self.0.as_fd(), &mut buf).is_ok() {}
    }
}
