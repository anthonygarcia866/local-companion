// Defense in depth for names (away-mode live test, 2026-11-05): after a live
// revision, a rewrite or polished dictation, every known name of the text
// (address book, My names and businesses, the names list) must be in the
// answer as often, spelled the same (any case). A name the shield masked
// comes back through its placeholder; this catches one it didn't mask (a
// contact it didn't know yet, a lowercase word).
//
// Like facts.rs, it logs counts only, never a name.

/// Whether `after` keeps every known name word of `before` (mod.rs
/// `names_kept_with`, with the app's names). Logs "shield: names changed
/// (live): N of M" when it doesn't.
pub fn kept(what: &str, before: &str, after: &str) -> bool {
    let (lost, of) = super::with_names(|names| super::names_lost_with(before, after, names));
    if lost == 0 {
        return true;
    }
    crate::log::line(format!("shield: names changed ({what}): {lost} of {of}"));
    false
}
