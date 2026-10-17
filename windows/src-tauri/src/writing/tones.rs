// The rewrite's default tone (1d, docs/phase1-plan.md §14.3): one per
// category of app, and one per app on top, from Settings → Writing →
// Rewrite tones. The default is first and highlighted on the Rewrite card,
// and a second press of the rewrite hotkey sends it.

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::ai::claude::Tone;

/// What kind of writing an app is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    /// Outlook classic and new, Gmail.
    Email,
    /// Word.
    Documents,
    /// The chat apps and their web versions.
    Chat,
    /// Everything else (Notepad, other browser pages).
    Other,
}

impl Category {
    pub const ALL: [Category; 4] = [Category::Email, Category::Documents, Category::Chat, Category::Other];

    /// The key `rewriteTones` stores it under.
    pub fn id(self) -> &'static str {
        match self {
            Category::Email => "email",
            Category::Documents => "documents",
            Category::Chat => "chat",
            Category::Other => "other",
        }
    }

    /// The tone it starts with (owner's examples, decision 3: Casual for chat).
    pub fn builtin(self) -> Tone {
        match self {
            Category::Email => Tone::Formal,
            Category::Documents => Tone::Formal,
            Category::Chat => Tone::Casual,
            Category::Other => Tone::Grammar,
        }
    }
}

/// Gmail, by the page's URL.
pub const MAIL_SITES: [&str; 1] = ["mail.google.com"];

/// The category of a field: its app, whether it is on a chat site and
/// whether it is on a mail site.
pub fn category(exe: &str, chat_site: bool, mail_site: bool) -> Category {
    let is = |list: &[&str]| list.iter().any(|a| a.eq_ignore_ascii_case(exe));
    if chat_site || is(&super::apps::CHAT) {
        Category::Chat
    } else if mail_site || is(&["OUTLOOK.EXE", "olk.exe"]) {
        Category::Email
    } else if is(&["WINWORD.EXE"]) {
        Category::Documents
    } else {
        Category::Other
    }
}

/// The default tone: the app's own (by lowercase exe), else its category's,
/// else the category's built-in one. `stored` is `rewriteTones` by lowercase
/// key; a tone id Glim doesn't know is ignored.
pub fn default_tone(exe: &str, cat: Category, stored: &BTreeMap<String, String>) -> Tone {
    let pick = |key: &str| stored.get(key).and_then(|id| Tone::from_id(id));
    pick(&exe.to_ascii_lowercase()).or_else(|| pick(cat.id())).unwrap_or_else(|| cat.builtin())
}

/// The field the checker last read text from: its app and category, nothing
/// else. What the Rewrite card's default comes from.
static FOCUS: Mutex<Option<(String, Category)>> = Mutex::new(None);

pub fn set_focus(exe: &str, cat: Category) {
    *FOCUS.lock().unwrap_or_else(|e| e.into_inner()) = Some((exe.to_string(), cat));
}

/// The exe of the field the checker last read (voice: the clean-up's context).
pub fn current_app() -> Option<String> {
    FOCUS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|(exe, _)| exe.clone())
}

/// What kind of app that field is in ("email", "documents", "chat",
/// "other"): voice's clean-up matches its tone to it (plan §4). Never text.
pub fn current_kind() -> &'static str {
    FOCUS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map_or(Category::Other.id(), |(_, cat)| cat.id())
}

/// The default tone for the field the user is writing in now.
pub fn current_default() -> Tone {
    let focus = FOCUS.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let stored = super::prefs::get().tones.clone();
    match focus {
        Some((exe, cat)) => default_tone(&exe, cat, &stored),
        None => default_tone("", Category::Other, &stored),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_focused_fields_kind_is_its_category_id() {
        set_focus("OUTLOOK.EXE", Category::Email);
        assert_eq!(current_kind(), "email");
        set_focus("WINWORD.EXE", Category::Documents);
        assert_eq!(current_kind(), "documents");
        *FOCUS.lock().unwrap() = None;
        assert_eq!(current_kind(), "other");
    }

    #[test]
    fn email_and_documents_are_formal_chat_casual_and_the_rest_grammar() {
        let none = BTreeMap::new();
        let tone = |exe: &str, chat: bool, mail: bool| default_tone(exe, category(exe, chat, mail), &none);
        assert_eq!(tone("OUTLOOK.EXE", false, false), Tone::Formal);
        assert_eq!(tone("olk.exe", false, false), Tone::Formal);
        assert_eq!(tone("chrome.exe", false, true), Tone::Formal, "Gmail");
        assert_eq!(tone("WINWORD.EXE", false, false), Tone::Formal);
        assert_eq!(tone("slack.exe", false, false), Tone::Casual);
        assert_eq!(tone("chrome.exe", true, false), Tone::Casual, "claude.ai in Chrome");
        assert_eq!(tone("Notepad.exe", false, false), Tone::Grammar);
        assert_eq!(tone("chrome.exe", false, false), Tone::Grammar);
        assert_eq!(tone("Acrobat.exe", false, false), Tone::Grammar);
    }

    #[test]
    fn the_apps_own_tone_wins_then_its_categorys_and_unknown_tones_are_ignored() {
        let stored: BTreeMap<String, String> =
            [("email", "friendlier"), ("outlook.exe", "shorter"), ("chat", "pirate"), ("other", "casual")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
        assert_eq!(default_tone("OUTLOOK.EXE", Category::Email, &stored), Tone::Shorter);
        assert_eq!(default_tone("olk.exe", Category::Email, &stored), Tone::Friendlier);
        assert_eq!(default_tone("slack.exe", Category::Chat, &stored), Tone::Casual, "pirate is no tone");
        assert_eq!(default_tone("Notepad.exe", Category::Other, &stored), Tone::Casual);
    }

    #[test]
    fn categories_have_distinct_ids() {
        let ids: Vec<_> = Category::ALL.iter().map(|c| c.id()).collect();
        assert_eq!(ids, ["email", "documents", "chat", "other"]);
    }
}
