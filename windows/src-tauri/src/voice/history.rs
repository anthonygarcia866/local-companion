// Dictation history (owner, 2026-10-28): the last 10 dictated texts, for the
// card that offers them to copy when Glim can't confirm the words landed.
// IN MEMORY ONLY, for this session: never written to disk, never logged, gone
// when Glim quits. A test checks this file has no file or log call.

use std::collections::VecDeque;
use std::sync::Mutex;

pub const KEPT: usize = 10;

static HISTORY: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

/// Keeps a dictated text (the newest first; at most KEPT).
pub fn push(text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    let mut h = HISTORY.lock().unwrap();
    h.retain(|t| t != text);
    h.push_front(text.to_string());
    h.truncate(KEPT);
}

/// The kept texts, newest first.
pub fn list() -> Vec<String> {
    HISTORY.lock().unwrap().iter().cloned().collect()
}

#[cfg(test)]
pub fn clear() {
    HISTORY.lock().unwrap().clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_ten_newest_first_in_memory_only() {
        clear();
        for i in 0..12 {
            push(&format!("note {i}"));
        }
        push("  ");
        let l = list();
        assert_eq!(l.len(), KEPT);
        assert_eq!(l[0], "note 11");
        assert_eq!(l[9], "note 2");
        // Said again: it moves to the front, not twice.
        push("note 5");
        assert_eq!(list()[0], "note 5");
        assert_eq!(list().iter().filter(|t| *t == "note 5").count(), 1);
        clear();
        // Nothing here writes a file or a log line.
        let src = include_str!("history.rs").replace("\r\n", "\n");
        let code = &src[..src.find("#[cfg(test)]").unwrap()];
        for banned in ["std::fs", "File", "log::", "println", "eprintln", "write!", "serde"] {
            assert!(!code.contains(banned), "{banned}");
        }
    }
}
