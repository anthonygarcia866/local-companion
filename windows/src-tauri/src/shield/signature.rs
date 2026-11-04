// Signature blocks (owner, 2026-11-04) are never sent or rewritten: from a
// "--" line to the end, or a sign-off line ("Best," "Thanks," "Regards,")
// and the few short lines after it at the end of the text. The caller sends
// only the body and puts the block back, byte for byte, after the answer.

const SIGN_OFFS: &[&str] = &[
    "best", "best regards", "best wishes", "kind regards", "warm regards", "warmest regards", "regards", "thanks", "thank you",
    "thanks so much", "thanks again", "many thanks", "cheers", "sincerely", "sincerely yours", "yours truly", "respectfully",
    "all the best", "take care", "talk soon", "warmly", "with thanks",
];

/// At most this many lines after a sign-off are its signature.
const MAX_LINES_AFTER: usize = 6;
/// A signature line is short (a name, a title, a phone, a business).
const MAX_LINE_CHARS: usize = 80;

fn is_sign_off(line: &str) -> bool {
    let l = line.trim().trim_end_matches([',', '.', '!']).trim().to_lowercase();
    SIGN_OFFS.contains(&l.as_str())
}

/// Where the signature block starts in `text` (a byte index at a line
/// start), or None.
pub fn start(text: &str) -> Option<usize> {
    // Line starts and lines.
    let mut lines: Vec<(usize, &str)> = Vec::new();
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        lines.push((at, line.trim_end_matches(['\n', '\r'])));
        at += line.len();
    }
    // A "--" line.
    if let Some(&(i, _)) = lines.iter().find(|(_, l)| l.trim_end() == "--") {
        return Some(i);
    }
    // The last sign-off line followed only by a few short lines.
    for (k, &(i, l)) in lines.iter().enumerate().rev() {
        if is_sign_off(l) {
            let after = &lines[k + 1..];
            let filled = after.iter().filter(|(_, l)| !l.trim().is_empty()).count();
            if filled <= MAX_LINES_AFTER && after.iter().all(|(_, l)| l.chars().count() <= MAX_LINE_CHARS) {
                return Some(i);
            }
            return None;
        }
    }
    None
}

/// `text` as (body, tail): the body without its trailing blank space, and
/// everything after it (the blank space and the signature), so that
/// `body + tail == text`. The tail is empty when there's no signature.
pub fn split(text: &str) -> (&str, &str) {
    match start(text) {
        Some(i) => {
            let body = text[..i].trim_end();
            (body, &text[body.len()..])
        }
        None => (text, ""),
    }
}

/// The answer for the body, with the tail back as it was.
pub fn rejoin(answer: &str, tail: &str) -> String {
    if tail.is_empty() {
        return answer.to_string();
    }
    let mut out = answer.trim_end().to_string();
    out.push_str(tail);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dash_line_or_a_sign_off_starts_the_signature() {
        let text = "Hi team,\nThe pool closes at 6.\n\nBest,\nJordan Vale\nVale Property Co.\n(555) 123-4567";
        let (body, tail) = split(text);
        assert_eq!(body, "Hi team,\nThe pool closes at 6.");
        assert_eq!(tail, "\n\nBest,\nJordan Vale\nVale Property Co.\n(555) 123-4567");
        assert_eq!(format!("{body}{tail}"), text);
        let dashed = "See you then.\n-- \nJ. Vale";
        assert_eq!(split(dashed), ("See you then.", "\n-- \nJ. Vale"));
        for s in ["Thanks,", "Thank you!", "Kind regards,", "Regards"] {
            assert_eq!(start(&format!("Body.\n{s}\nJordan")), Some(6), "{s}");
        }
    }

    #[test]
    fn no_signature_leaves_the_text_whole() {
        for text in [
            "Thanks for the update, see you tomorrow.",
            "Best of luck with the move!",
            "One line only",
            // A sign-off followed by a long paragraph isn't a signature.
            "Thanks,\nI looked at the lease again and the clause about pets is the one we discussed, so I think we're fine to go ahead with it next month as planned.",
        ] {
            assert_eq!(split(text), (text, ""), "{text}");
        }
    }

    #[test]
    fn the_signature_comes_back_byte_for_byte() {
        let text = "pls fix the sink\r\n\r\nThanks,\r\nJordan  Vale \r\n";
        let (body, tail) = split(text);
        assert_eq!(body, "pls fix the sink");
        assert_eq!(rejoin("Please fix the sink.\n", tail), "Please fix the sink.\r\n\r\nThanks,\r\nJordan  Vale \r\n");
        assert_eq!(rejoin("Same.", ""), "Same.");
    }
}
