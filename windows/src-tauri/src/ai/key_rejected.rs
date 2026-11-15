// "Glim's Claude key was rejected" (owner 2026-11-14): when the Claude API
// answers 401, one clear card on the island instead of a line in the log
// only, at most once an hour. Every request goes through claude::send, which
// calls `on_401` there; the app sets the notifier at launch (an event to the
// island). The log gets the fact, never the key or the request.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// At most one card per this long.
pub const QUIET: Duration = Duration::from_secs(60 * 60);

/// The island's event.
pub const EVENT: &str = "api-key-rejected";

static LAST: Mutex<Option<Instant>> = Mutex::new(None);
type Notify = Box<dyn Fn() + Send + Sync>;
static NOTIFY: OnceLock<Notify> = OnceLock::new();

/// Whether a 401 at `now` shows the card, given when it last did.
pub fn shows(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|t| now.duration_since(t) >= QUIET)
}

/// The app's notifier (lib.rs setup); later calls are ignored.
pub fn set_notifier(f: impl Fn() + Send + Sync + 'static) {
    let _ = NOTIFY.set(Box::new(f));
}

/// The API refused the key (401). Shows the card unless it did within the hour.
pub fn on_401() {
    let now = Instant::now();
    let show = {
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        let show = shows(*last, now);
        if show {
            *last = Some(now);
        }
        show
    };
    if show {
        crate::log::line("api: key rejected (401): card shown");
        if let Some(f) = NOTIFY.get() {
            f();
        }
    } else {
        crate::log::line("api: key rejected (401): card already shown this hour");
    }
}

/// When the card last showed (tests).
#[cfg(test)]
pub fn last_shown() -> Option<Instant> {
    *LAST.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_shows_once_an_hour_at_most() {
        let t0 = Instant::now();
        assert!(shows(None, t0), "the first 401");
        assert!(!shows(Some(t0), t0 + Duration::from_secs(1)));
        assert!(!shows(Some(t0), t0 + QUIET - Duration::from_secs(1)));
        assert!(shows(Some(t0), t0 + QUIET));
        assert_eq!(QUIET, Duration::from_secs(3600));
    }

    /// Every Claude request goes through claude::send, and its 401 reaches
    /// on_401 before the failure is returned.
    #[test]
    fn claude_send_reports_a_401() {
        let code = include_str!("claude.rs").replace("\r\n", "\n");
        let send = &code[code.find("pub(super) async fn send(").unwrap()..];
        let send = &send[..send.find("\n}\n").unwrap()];
        let hook = send.find("super::key_rejected::on_401();").expect("send calls on_401");
        assert!(send[..hook].contains("if status == 401 {"));
        assert!(hook < send.find("return Err(Failure::Api { status, kind, retry_after });").unwrap());
        let lib = include_str!("../lib.rs");
        assert!(lib.contains("ai::key_rejected::set_notifier("), "the app wires the card");
    }
}
