// What the user is writing: the paragraph around the caret, with the email
// signature and quoted reply text left out. Glim checks only that.

use crate::capture::{AFTER_CARET, BEFORE_CARET};

/// The paragraph to check, and where it starts in the field (in characters).
#[derive(Clone, PartialEq)]
pub struct Paragraph {
    pub text: String,
    pub start: usize,
}

impl std::fmt::Debug for Paragraph {
    /// Never the text: formatting a Paragraph into a log line can't carry it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Paragraph").field("start", &self.start).field("chars", &self.text.chars().count()).finish()
    }
}

/// The paragraph the caret is in, inside the user's own text. None when the
/// caret is in the signature or the quoted part, or the paragraph is blank.
/// Without a caret (ValuePattern fields), the end of the user's own text.
pub fn paragraph_to_check(field: &str, caret: Option<usize>) -> Option<Paragraph> {
    let chars: Vec<char> = field.chars().collect();
    let own_end = own_text_end(&chars);
    let caret = caret.unwrap_or(own_end).min(chars.len());
    // own_end is where the signature or quote line starts: a caret there is
    // in it, not at the end of the user's text.
    if caret > own_end || (caret == own_end && own_end < chars.len()) {
        return None;
    }
    let par_start = chars[..caret].iter().rposition(|c| is_break(*c)).map(|i| i + 1).unwrap_or(0);
    let par_end = chars[caret..own_end].iter().position(|c| is_break(*c)).map(|i| caret + i).unwrap_or(own_end);
    let mut start = par_start.max(caret.saturating_sub(BEFORE_CARET));
    let mut end = par_end.min(caret + AFTER_CARET);
    // A window cut inside a long paragraph starts and ends on whole words.
    if start > par_start {
        while start < caret && !chars[start - 1].is_whitespace() {
            start += 1;
        }
    }
    if end < par_end {
        while end > caret && !chars[end].is_whitespace() {
            end -= 1;
        }
    }
    let text: String = chars[start..end].iter().collect();
    (!text.trim().is_empty()).then_some(Paragraph { text, start })
}

fn is_break(c: char) -> bool {
    c == '\n' || c == '\r' || c == '\u{b}' || c == '\u{2029}'
}

/// Where the user's own text ends: the first line that starts a signature or
/// quoted reply text, or the end of the field.
pub fn own_text_end(chars: &[char]) -> usize {
    let lines = lines_of(chars);
    for (i, &(start, end)) in lines.iter().enumerate() {
        let line: String = chars[start..end].iter().collect();
        if starts_quote(&line, &lines[i + 1..], chars) || starts_signature(&line, &lines[i + 1..], chars) {
            return start;
        }
    }
    chars.len()
}

/// (start, end) of every line, in characters, without its line break.
fn lines_of(chars: &[char]) -> Vec<(usize, usize)> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (i, &c) in chars.iter().enumerate() {
        if is_break(c) {
            lines.push((start, i));
            start = i + 1;
        }
    }
    lines.push((start, chars.len()));
    lines
}

fn text_of(chars: &[char], (start, end): (usize, usize)) -> String {
    chars[start..end].iter().collect()
}

/// Quoted reply text starts here: a `>` line, "On … wrote:", Outlook's
/// "-----Original Message-----" or underscore rule, or a "From:" header
/// followed by "Sent:" / "Date:" within a few lines.
fn starts_quote(line: &str, rest: &[(usize, usize)], chars: &[char]) -> bool {
    let t = line.trim();
    if t.starts_with('>') {
        return true;
    }
    if t.starts_with("On ") && t.ends_with("wrote:") {
        return true;
    }
    if t.contains("-----Original Message-----") || (t.len() >= 10 && t.chars().all(|c| c == '_')) {
        return true;
    }
    if t.starts_with("From:") {
        return rest.iter().take(4).any(|&l| {
            let next = text_of(chars, l);
            let next = next.trim();
            next.starts_with("Sent:") || next.starts_with("Date:")
        });
    }
    false
}

/// Closing lines that start a signature when only a few short lines follow.
const CLOSINGS: [&str; 16] = [
    "best regards", "kind regards", "warm regards", "warmest regards", "regards", "best", "best wishes",
    "all the best", "thanks", "thank you", "many thanks", "thanks so much", "sincerely", "yours truly",
    "cheers", "respectfully",
];

/// The longest signature taken for one: name, title, company, phone, address.
const SIGNATURE_LINES: usize = 6;
const SIGNATURE_LINE_CHARS: usize = 60;

/// A signature starts here: the "-- " delimiter, "Sent from my …", or a
/// closing ("Best regards,") followed by no more than a few short lines
/// before the end or the quoted part.
fn starts_signature(line: &str, rest: &[(usize, usize)], chars: &[char]) -> bool {
    let t = line.trim();
    if t == "--" || line == "-- " {
        return true;
    }
    if t.starts_with("Sent from my ") {
        return true;
    }
    let closing = t.trim_end_matches([',', '.', '!']).trim().to_lowercase();
    if !CLOSINGS.contains(&closing.as_str()) {
        return false;
    }
    let mut short = 0;
    for (i, &l) in rest.iter().enumerate() {
        let next = text_of(chars, l);
        if starts_quote(&next, &rest[i + 1..], chars) {
            break;
        }
        if next.trim().is_empty() {
            continue;
        }
        short += 1;
        if short > SIGNATURE_LINES || next.trim().chars().count() > SIGNATURE_LINE_CHARS {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(field: &str, marker: &str) -> Option<String> {
        let caret = field[..field.find(marker).unwrap()].chars().count();
        paragraph_to_check(field, Some(caret)).map(|p| p.text)
    }

    #[test]
    fn the_paragraph_around_the_caret_is_checked() {
        let field = "Hi Sam,\n\nThe heater in unit 4B dont work.\nCan you send someone?";
        assert_eq!(at(field, "dont").as_deref(), Some("The heater in unit 4B dont work."));
        let p = paragraph_to_check(field, Some(20)).unwrap();
        assert_eq!(p.start, 9);
        // Word and Outlook end paragraphs with \r.
        assert_eq!(at("One.\rTwo here.\rThree.", "here").as_deref(), Some("Two here."));
    }

    #[test]
    fn a_blank_paragraph_is_not_checked() {
        assert_eq!(paragraph_to_check("Hello\n\n\nBye", Some(6)), None);
        assert_eq!(paragraph_to_check("", None), None);
    }

    #[test]
    fn outlook_signature_and_reply_are_left_out() {
        let field = "Hi Maria,\r\rThe lease renewel is attached.\r\rBest regards,\rAnthony Garcia\rProperty Manager\rAudii Management LLC\r(555) 123-4567\r\rFrom: Maria Lopez\rSent: Friday, October 10, 2026 9:12 AM\rTo: Anthony\rSubject: Lease\r\rHi, can you send the renewel?";
        assert_eq!(at(field, "renewel is").as_deref(), Some("The lease renewel is attached."));
        assert_eq!(at(field, "Anthony Garcia"), None);
        assert_eq!(at(field, "Audii"), None);
        assert_eq!(at(field, "can you send"), None);
        // No caret (ValuePattern): the end of the user's own text.
        let own: String = field.chars().take(own_text_end(&field.chars().collect::<Vec<_>>())).collect();
        assert!(own.ends_with("attached.\r\r"), "{own:?}");
    }

    #[test]
    fn gmail_quotes_and_delimiters_are_left_out() {
        let field = "Sounds good, see you Tuesday.\n\nOn Fri, Oct 10, 2026 at 9:12 AM Maria Lopez <maria@example.com> wrote:\n> Can we meet Tuesday?\n> Thanks";
        assert_eq!(at(field, "Tuesday.").as_deref(), Some("Sounds good, see you Tuesday."));
        assert_eq!(at(field, "Can we"), None);
        let field = "Fixed it.\n-- \nAnthony\nAudii Management";
        assert_eq!(at(field, "Anthony"), None);
        assert_eq!(at("Fixed.\nSent from my iPhone", "iPhone"), None);
        let field = "See below.\n-----Original Message-----\nFrom: x\nold text";
        assert_eq!(at(field, "old text"), None);
        let field = "See below.\n________________________________\nFrom: x\nSent: y\nold";
        assert_eq!(at(field, "old"), None);
    }

    #[test]
    fn a_closing_word_in_the_middle_of_a_message_is_not_a_signature() {
        // "Thanks" followed by more paragraphs of writing: still the user's.
        let long = "Thanks,\nI looked at the unit today and the carpet needs replacing before the new tenant arrives next week, so please get three quotes.\nAlso check the blinds.";
        assert_eq!(at(long, "Also").as_deref(), Some("Also check the blinds."));
        // "Thanks for the update!" is a sentence, not a closing.
        assert_eq!(at("Thanks for the update!\nAnthony", "update").as_deref(), Some("Thanks for the update!"));
        // Typing at the very end of the own text, right before the signature.
        let field = "The rent is due.\nBest,\nAnthony";
        assert_eq!(paragraph_to_check(field, Some(16)).map(|p| p.text).as_deref(), Some("The rent is due."));
    }

    #[test]
    fn a_long_paragraph_is_cut_to_whole_words_around_the_caret() {
        let field = "word ".repeat(300);
        let p = paragraph_to_check(&field, Some(750)).unwrap();
        assert!(p.text.starts_with("word") && p.text.trim_end().ends_with("word"), "{:?}", &p.text[..20]);
        assert!(p.text.chars().count() <= BEFORE_CARET + AFTER_CARET);
    }

    #[test]
    fn formatting_a_paragraph_never_shows_its_text() {
        let p = Paragraph { text: "SENTINEL-paragraph-81c2".into(), start: 3 };
        assert!(!format!("{p:?} {p:#?}").contains("SENTINEL"));
    }
}
