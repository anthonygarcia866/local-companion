// Ask first (owner, 2026-11-04: "nothing goes to the cloud without asking").
// The cloud features that send text by themselves, without a key press —
// live revisions, polished dictation, and "Check with AI automatically" —
// each start off. The first time one would send, nothing is sent: the
// island shows a one-time card listing exactly what that feature sends
// (PRIVACY.md's table), with "Turn on" and "Not now". Only "Turn on" (or the
// feature's switch in Settings) lets it send; "Not now" keeps it off and the
// card never comes back by itself. A manual rewrite, a "Check with AI"
// click and Ask Glim are the user asking (a key or a click) and need no card.
//
// The answers live in settings.json (`cloud_consent`); this module holds
// them for the threads that send, and asks.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// The island hears this with the feature to ask about.
pub const EVENT: &str = "consent-ask";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {
    Live,
    Polish,
    AutoCheck,
}

impl Feature {
    pub const ALL: [Feature; 3] = [Feature::Live, Feature::Polish, Feature::AutoCheck];

    /// The settings' and the page's name for it.
    pub fn key(self) -> &'static str {
        match self {
            Feature::Live => "live",
            Feature::Polish => "polish",
            Feature::AutoCheck => "autoCheck",
        }
    }

    pub fn from_key(k: &str) -> Option<Feature> {
        Feature::ALL.into_iter().find(|f| f.key() == k)
    }

    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Never asked: the first send asks instead.
    Unasked,
    On,
    Off,
}

impl Answer {
    fn code(self) -> u8 {
        match self {
            Answer::Unasked => 0,
            Answer::On => 1,
            Answer::Off => 2,
        }
    }

    fn from_code(c: u8) -> Answer {
        match c {
            1 => Answer::On,
            2 => Answer::Off,
            _ => Answer::Unasked,
        }
    }

    /// From the stored word: "on", "off", anything else is unasked.
    pub fn from_word(w: &str) -> Answer {
        match w {
            "on" => Answer::On,
            "off" => Answer::Off,
            _ => Answer::Unasked,
        }
    }
}

/// What the settings say for `f`. A settings file from before the cards:
/// auto-check switched on was already the user asking; live revisions
/// switched off stays off.
pub fn answer_in(s: &crate::settings::Settings, f: Feature) -> Answer {
    let c = &s.cloud_consent;
    let stored = Answer::from_word(match f {
        Feature::Live => &c.live,
        Feature::Polish => &c.polish,
        Feature::AutoCheck => &c.auto_check,
    });
    match (stored, f) {
        (Answer::Unasked, Feature::AutoCheck) if s.ai_auto_check => Answer::On,
        (Answer::Unasked, Feature::Live) if !s.live_revisions => Answer::Off,
        (a, _) => a,
    }
}

static ANSWERS: [AtomicU8; 3] = [AtomicU8::new(0), AtomicU8::new(0), AtomicU8::new(0)];
/// The card being asked: 0 none, else the feature's index + 1.
static ASKING: AtomicU8 = AtomicU8::new(0);
static APP: OnceLock<AppHandle> = OnceLock::new();

/// lib.rs, at launch: where the card is asked.
pub fn init(app: AppHandle) {
    let _ = APP.set(app);
}

/// From the settings, at launch and on every save.
pub fn apply_settings(s: &crate::settings::Settings) {
    for f in Feature::ALL {
        let a = answer_in(s, f);
        ANSWERS[f.index()].store(a.code(), Ordering::Relaxed);
        // Answered (here or in Settings): the card is done.
        if a != Answer::Unasked && ASKING.load(Ordering::Relaxed) == f.index() as u8 + 1 {
            ASKING.store(0, Ordering::Relaxed);
        }
    }
}

pub fn answer(f: Feature) -> Answer {
    Answer::from_code(ANSWERS[f.index()].load(Ordering::Relaxed))
}

/// Whether `f` may send now.
pub fn allowed(f: Feature) -> bool {
    answer(f) == Answer::On
}

#[derive(Clone, Serialize)]
struct Ask {
    feature: &'static str,
}

/// Call where `f` would send by itself. True: send. False: don't; and if
/// it was never asked, the island asks (one card at a time, once).
pub fn gate(f: Feature) -> bool {
    match answer(f) {
        Answer::On => true,
        Answer::Off => false,
        Answer::Unasked => {
            if ASKING.compare_exchange(0, f.index() as u8 + 1, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
                crate::log::line(format!("consent: asked ({})", f.key()));
                if let Some(app) = APP.get() {
                    let _ = app.emit_to(crate::island::WINDOW_LABEL, EVENT, Ask { feature: f.key() });
                }
            }
            false
        }
    }
}

/// The card's answer was saved (lib.rs consent_answer): the next card may come.
pub fn answered() {
    ASKING.store(0, Ordering::Relaxed);
}

/// The settings change for an answer: the stored word, and the feature's
/// own switch with it.
pub fn record(s: &mut crate::settings::Settings, f: Feature, on: bool) {
    let word = if on { "on" } else { "off" }.to_string();
    match f {
        Feature::Live => {
            s.cloud_consent.live = word;
            if on {
                s.live_revisions = true;
            }
        }
        Feature::Polish => s.cloud_consent.polish = word,
        Feature::AutoCheck => {
            s.cloud_consent.auto_check = word;
            s.ai_auto_check = on;
        }
    }
}

#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    #[test]
    fn every_cloud_feature_starts_off_and_asks_first() {
        let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let s = Settings::default();
        for f in Feature::ALL {
            assert_eq!(answer_in(&s, f), Answer::Unasked, "{f:?}");
        }
        apply_settings(&s);
        answered();
        // The first send asks and doesn't send; one card at a time.
        assert!(!gate(Feature::Live));
        assert_eq!(ASKING.load(Ordering::Relaxed), 1);
        assert!(!gate(Feature::AutoCheck));
        assert_eq!(ASKING.load(Ordering::Relaxed), 1, "the live card is still up");
        // Turn on.
        let mut s = s;
        record(&mut s, Feature::Live, true);
        apply_settings(&s);
        assert_eq!(ASKING.load(Ordering::Relaxed), 0);
        assert!(gate(Feature::Live));
        // Not now: off, and never asked again.
        record(&mut s, Feature::AutoCheck, false);
        apply_settings(&s);
        assert!(!gate(Feature::AutoCheck));
        assert_eq!(ASKING.load(Ordering::Relaxed), 0);
        assert!(!s.ai_auto_check);
        apply_settings(&Settings::default());
        answered();
    }

    #[test]
    fn a_settings_file_from_before_the_cards_keeps_what_the_user_chose() {
        let mut s = Settings::default();
        s.ai_auto_check = true;
        s.live_revisions = false;
        assert_eq!(answer_in(&s, Feature::AutoCheck), Answer::On);
        assert_eq!(answer_in(&s, Feature::Live), Answer::Off);
        assert_eq!(answer_in(&s, Feature::Polish), Answer::Unasked);
        assert_eq!(Feature::from_key("autoCheck"), Some(Feature::AutoCheck));
        assert_eq!(Feature::from_key("rewrite"), None);
    }
}
