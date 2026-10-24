// The writing settings Rust acts on (1d, docs/phase1-plan.md §14), copied
// from settings.json at launch and whenever Settings saves it. The capture
// thread reads them at every reading, so a change takes effect within one
// reading (300 ms), and `version` tells it something changed.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// What the writing layer needs from the settings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prefs {
    /// Per-app switches that differ from the defaults, by lowercase exe name:
    /// true = on, false = off (14.1).
    pub apps: BTreeMap<String, bool>,
    /// When each app on neither list was last asked about (§14.2): local
    /// "YYYY-MM-DD" by lowercase exe.
    pub asked: BTreeMap<String, String>,
    /// The Google Docs tip was dismissed (writing::docs_tip).
    pub docs_tip_dismissed: bool,
    /// "Check with AI automatically when I pause typing" (§15.1).
    pub ai_auto_check: bool,
    /// Settings → Writing → Rewrite tones (§14.3): a tone id by category id
    /// or lowercase exe, only where it differs from the built-in defaults.
    pub tones: BTreeMap<String, String>,
    /// Settings → Writing → Strictness (§14.4).
    pub strictness: super::checker::Strictness,
    /// Settings → Privacy → Paused apps and sites (§14.5): the user's own
    /// entries, on top of the built-in ones.
    pub pause_apps: Vec<String>,
    pub pause_sites: Vec<String>,
    /// The mini lantern beside the field (1e, §7a.6); off by default.
    pub bubble: bool,
}

impl Prefs {
    pub fn from_settings(s: &crate::settings::Settings) -> Prefs {
        Prefs {
            apps: app_switches(&s.writing_apps),
            asked: s.writing_apps_asked.iter().map(|(exe, d)| (exe.to_ascii_lowercase(), d.clone())).collect(),
            docs_tip_dismissed: s.writing_docs_tip_dismissed,
            ai_auto_check: s.ai_auto_check,
            tones: s.rewrite_tones.iter().map(|(k, t)| (k.to_ascii_lowercase(), t.clone())).collect(),
            strictness: super::checker::Strictness::from_id(&s.checker_strictness),
            pause_apps: s.pause_apps.clone(),
            pause_sites: s.pause_sites.clone(),
            bubble: s.writing_bubble,
        }
    }
}

/// `writingApps` as stored ("on" / "off" by exe) to switches by lowercase
/// exe. Any other value reads as the default, so it is left out.
pub fn app_switches(stored: &BTreeMap<String, String>) -> BTreeMap<String, bool> {
    stored
        .iter()
        .filter_map(|(exe, v)| match v.as_str() {
            "on" => Some((exe.to_ascii_lowercase(), true)),
            "off" => Some((exe.to_ascii_lowercase(), false)),
            _ => None,
        })
        .collect()
}

static PREFS: RwLock<Option<Arc<Prefs>>> = RwLock::new(None);
static VERSION: AtomicU64 = AtomicU64::new(0);

/// The settings now in force.
pub fn get() -> Arc<Prefs> {
    PREFS.read().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}

/// Puts new settings in force. The version moves only when they differ.
pub fn set(p: Prefs) {
    let mut held = PREFS.write().unwrap_or_else(|e| e.into_inner());
    if held.as_deref() != Some(&p) {
        *held = Some(Arc::new(p));
        VERSION.fetch_add(1, Ordering::SeqCst);
    }
}

/// Moves whenever `set` changed something.
pub fn version() -> u64 {
    VERSION.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_switches_read_by_lowercase_exe_and_unknown_values_are_the_default() {
        let stored: BTreeMap<String, String> = [
            ("WINWORD.EXE", "off"),
            ("Acrobat.exe", "on"),
            ("Code.exe", "maybe"),
            ("Notepad.exe", ""),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let got = app_switches(&stored);
        assert_eq!(got.get("winword.exe"), Some(&false));
        assert_eq!(got.get("acrobat.exe"), Some(&true));
        assert_eq!(got.len(), 2);
    }
}
