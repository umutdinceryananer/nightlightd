//! Re-apply on resume: watch logind's `PrepareForSleep` signal (#16).
//!
//! On some drivers no RandR event arrives after waking from suspend, so the
//! periodic verification tick would be the only recovery — up to a minute of
//! neutral screen. This is the safety belt: logind emits `PrepareForSleep(false)`
//! on resume, and we wake the poll loop at once. logind lives on the system bus.

use zbus::blocking::Connection;
use zbus::proxy;

use crate::waker::Waker;

/// A blocking proxy for the slice of logind we need: the sleep signal.
#[proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Login1Manager {
    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

/// Blocks watching for resume events, waking `waker` each time the system wakes
/// from sleep so the poll loop re-applies the ramp at once. Meant to run on its
/// own thread; returns only if the system bus or the subscription fails.
pub fn watch(waker: Waker) -> zbus::Result<()> {
    let connection = Connection::system()?;
    let proxy = Login1ManagerProxyBlocking::new(&connection)?;
    for signal in proxy.receive_prepare_for_sleep()? {
        // `start == true` means going to sleep; `false` means resuming.
        let going_to_sleep = *signal.args()?.start();
        if !going_to_sleep {
            waker.wake();
        }
    }
    Ok(())
}
