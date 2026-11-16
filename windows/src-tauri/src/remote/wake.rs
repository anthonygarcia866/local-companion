// The courier reconnects at once when Windows says the old line can't be
// trusted (owner 2026-11-15): on resume from sleep (a suspend-resume
// notification, PowerRegisterSuspendResumeNotification with a callback: no
// window needed) and when an IP address changes (NotifyAddrChange, blocking
// on its own thread). Both only call net::relay::kick; neither opens a socket
// or reads anything but the event itself. The session's own checks (60 s of
// silence, a wall-clock jump) still catch whatever these miss.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::net::relay::{kick, Kick};

/// Address changes come in bursts (an adapter going up brings IPv4 and IPv6
/// addresses one after another): one kick per burst.
pub const NETWORK_SETTLE: Duration = Duration::from_secs(5);

/// One kick per burst of address changes: true when this change (at `now`)
/// should kick, i.e. the last kick was NETWORK_SETTLE or more ago.
pub fn network_kick_due(last: &mut Option<Instant>, now: Instant) -> bool {
    if last.is_some_and(|t| now.duration_since(t) < NETWORK_SETTLE) {
        return false;
    }
    *last = Some(now);
    true
}

static STARTED: AtomicBool = AtomicBool::new(false);

/// Starts both watchers, once, for Glim's life.
pub fn start() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    #[cfg(windows)]
    {
        power::register();
        let _ = std::thread::Builder::new().name("remote-network-watch".into()).spawn(|| {
            let mut last: Option<Instant> = None;
            loop {
                // SAFETY: both pointers null = wait (blocking) for the next
                // change of any IPv4 address on this PC.
                let r = unsafe { windows::Win32::NetworkManagement::IpHelper::NotifyAddrChange(std::ptr::null_mut(), std::ptr::null()) };
                if r != 0 {
                    // Not supported or failing: give up quietly; the session's
                    // own silence check still notices a dead line.
                    crate::log::line(format!("remote: network watch unavailable ({r})"));
                    return;
                }
                if network_kick_due(&mut last, Instant::now()) {
                    crate::log::line("remote: network changed; reconnecting");
                    kick(Kick::Network);
                }
            }
        });
    }
}

#[cfg(windows)]
mod power {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Power::{PowerRegisterSuspendResumeNotification, DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS};
    use windows::Win32::UI::WindowsAndMessaging::{DEVICE_NOTIFY_CALLBACK, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND};

    /// Windows calls this on its own thread for each power event.
    unsafe extern "system" fn on_power(_context: *const core::ffi::c_void, kind: u32, _setting: *const core::ffi::c_void) -> u32 {
        if kind == PBT_APMRESUMEAUTOMATIC || kind == PBT_APMRESUMESUSPEND {
            crate::log::line("remote: resumed from sleep; reconnecting");
            super::kick(super::Kick::Resumed);
        }
        0
    }

    pub fn register() {
        // Kept for Glim's life: Windows reads it for as long as the
        // registration stands (never unregistered).
        let params: &'static mut DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS = Box::leak(Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS { Callback: Some(on_power), Context: std::ptr::null_mut() }));
        let mut handle: *mut core::ffi::c_void = std::ptr::null_mut();
        // SAFETY: DEVICE_NOTIFY_CALLBACK takes a pointer to the subscribe
        // parameters as the recipient; they live forever (leaked above).
        let r = unsafe { PowerRegisterSuspendResumeNotification(DEVICE_NOTIFY_CALLBACK, HANDLE(params as *mut _ as *mut core::ffi::c_void), &mut handle) };
        if r.0 != 0 {
            crate::log::line(format!("remote: resume watch unavailable ({})", r.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_of_address_changes_kicks_once() {
        let t0 = Instant::now();
        let mut last = None;
        assert!(network_kick_due(&mut last, t0));
        assert!(!network_kick_due(&mut last, t0 + Duration::from_secs(1)));
        assert!(!network_kick_due(&mut last, t0 + Duration::from_secs(4)));
        assert!(network_kick_due(&mut last, t0 + NETWORK_SETTLE + Duration::from_secs(1)));
    }
}
