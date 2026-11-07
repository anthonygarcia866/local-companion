// The no-new-content rule for tone rewrites (owner, 2026-11-06): a rewrite
// may reword the user's paragraph, never add to what it says. Live test
// run 7 (2026-11-05) showed a Friendlier rewrite padding the paragraph out
// ("for your review", "Please don't hesitate"). The prompt forbids it
// (ai/claude.rs SYSTEM); this is the check on the answer, on this PC:
//
// - it grows by more than 60% (characters), or
// - it brings in a claim-type phrase of a kind the paragraph has none of:
//   an attachment, something already sent, an earlier conversation, an
//   apology, a promise.
//
// Either way it is discarded (Mismatch::Added). The log gets which rule,
// never the text.

/// The most a rewrite may grow: 60% longer than the paragraph.
pub const MAX_GROWTH: f64 = 0.6;

/// Claim-type phrases by kind, matched at a word start on lowercase text
/// with contractions spelled out ("I've" is "i have"). A kind the paragraph
/// already has any phrase of may be reworded freely ("the bill is attached"
/// -> "I've attached the bill").
const KINDS: [(&str, &[&str]); 5] = [
    ("attachment", &["attach", "enclos"]),
    (
        "sent",
        &[
            "have sent", "i sent", "we sent", "already sent", "have forwarded", "i forwarded", "we forwarded", "have emailed", "i emailed",
            "we emailed", "have shared", "i shared", "we shared",
        ],
    ),
    (
        "earlier-talk",
        &[
            "as discussed", "as we discussed", "as mentioned", "as we mentioned", "as promised", "as agreed", "as we agreed", "per our",
            "following up on our", "following our",
        ],
    ),
    ("apology", &["sorry", "apolog"]),
    (
        "promise",
        &[
            "i promise", "we promise", "i guarantee", "we guarantee", "rest assured", "i will make sure", "we will make sure", "i will ensure",
            "we will ensure",
        ],
    ),
];

/// Lowercase words, straight apostrophes, contractions spelled out, single
/// spaces, with a space at each end (so " stem" matches at a word start).
fn words(text: &str) -> String {
    let lower = text.to_lowercase().replace(['\u{2018}', '\u{2019}'], "'");
    let mut out = String::from(" ");
    for w in lower.split(|c: char| !(c.is_alphanumeric() || c == '\'')).filter(|w| !w.is_empty()) {
        let w = w.trim_matches('\'');
        let spelled = match w {
            "i've" => "i have",
            "we've" => "we have",
            "i'll" => "i will",
            "we'll" => "we will",
            "i'm" => "i am",
            "we're" => "we are",
            "i'd" => "i would",
            w => w,
        };
        out.push_str(spelled);
        out.push(' ');
    }
    out
}

fn has_any(words: &str, stems: &[&str]) -> bool {
    stems.iter().any(|s| words.contains(&format!(" {s}")))
}

/// Why `after` adds to `before`, if it does: "longer", or the kind of claim
/// it brings in ("attachment", "sent", "earlier-talk", "apology",
/// "promise"). None: a rewording.
pub fn added(before: &str, after: &str) -> Option<&'static str> {
    let (b, a) = (before.trim().chars().count(), after.trim().chars().count());
    if a as f64 > b as f64 * (1.0 + MAX_GROWTH) {
        return Some("longer");
    }
    let (wb, wa) = (words(before), words(after));
    KINDS.iter().find(|(_, stems)| has_any(&wa, stems) && !has_any(&wb, stems)).map(|(kind, _)| *kind)
}

/// Whether `after` only rewords `before`. Logs "shield: rewrite added
/// content (<what>): <rule>" when it doesn't.
pub fn check(what: &str, before: &str, after: &str) -> bool {
    match added(before, after) {
        None => true,
        Some(rule) => {
            crate::log::line(format!("shield: rewrite added content ({what}): {rule}"));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live test run 7's paragraph (made-up, no signature).
    const RUN7: &str = "Hi, sorry for the late reply. The repairs got finished on Monday and the bill is attached, let me know if anything looks off.";

    #[test]
    fn run_7s_rewrite_rewords_what_was_there() {
        // What Claude answered in run 7: 47% longer, and its apology and
        // attachment were in the paragraph already. The check keeps it; the
        // prompt (ai/claude.rs SYSTEM) is what now forbids its padding.
        let answered = "Hi there, I\u{2019}m so sorry for the slow reply! The repairs were wrapped up on Monday, and I\u{2019}ve attached the bill for your review. Please don\u{2019}t hesitate to let me know if anything looks off.";
        assert_eq!(added(RUN7, answered), None);
        // A plain rewording, a contraction either way.
        assert_eq!(added(RUN7, "Hi, sorry I'm late replying. The repairs were done Monday and I've attached the bill; let me know if anything's off."), None);
    }

    #[test]
    fn run_7s_paragraph_with_invented_claims_is_discarded() {
        assert_eq!(
            added(RUN7, "Hi, sorry for the late reply. As discussed, the repairs were finished on Monday and the bill is attached; let me know."),
            Some("earlier-talk")
        );
        assert_eq!(added(RUN7, "Sorry for the late reply! The repairs finished Monday; I've sent the bill, which is attached."), Some("sent"));
        assert_eq!(
            added(RUN7, "Sorry for the late reply. Repairs done Monday, bill attached. I will make sure it never happens again."),
            Some("promise")
        );
        // Padded out: more than 60% longer.
        let padded = format!("{RUN7} Thank you so much for your patience while we sorted everything out, and have a lovely rest of your week!");
        assert_eq!(added(RUN7, &padded), Some("longer"));
    }

    #[test]
    fn a_claim_kind_the_paragraph_lacks_is_never_brought_in() {
        let p = "The repairs got finished on Monday, let me know if anything looks off.";
        assert_eq!(added(p, "Sorry for the delay! The repairs were finished on Monday; let me know if anything looks off."), Some("apology"));
        assert_eq!(added(p, "The repairs were finished on Monday and the bill is attached. Let me know if anything looks off."), Some("attachment"));
        assert_eq!(added(p, "As discussed, the repairs were done Monday. Let me know if anything looks off."), Some("earlier-talk"));
        assert_eq!(added(p, "I\u{2019}ve sent the repair notes: everything was done Monday. Let me know if anything's off."), Some("sent"));
        assert_eq!(added(p, "The repairs were wrapped up Monday. Please let me know if anything looks off."), None);
        // "I will send" is not "I've sent": a promise to send never becomes a send.
        assert_eq!(added("I will send the lease tomorrow.", "I've sent the lease."), Some("sent"));
        // Exactly 60% longer is allowed; one character more is not.
        let ten = "abcdefghij";
        assert_eq!(added(ten, "abcdefghijklmnop"), None);
        assert_eq!(added(ten, "abcdefghijklmnopq"), Some("longer"));
    }

    #[test]
    fn stems_match_at_word_starts_only() {
        // "unattached" isn't an attachment; "sorry" inside another word isn't one.
        assert_eq!(added("The unit is unattached.", "The unit is unattached, sadly."), None);
        assert!(words("I've ATTACHED it").contains(" i have attached it "));
    }
}
