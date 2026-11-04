// The numbers-and-dates rule (owner, 2026-11-04): an answer that rewrites
// the user's text (a live revision, polished dictation, a rewrite, a "Check
// with AI" fix) must keep every number, amount, percentage, time and date
// the text had, and bring in none. Each side's facts are read here, on this
// PC; when the sets differ beyond writing the same value another way
// ("10" / "ten", "$1,850" / "$1850", "10:00" / "ten o'clock",
// "555-123-4567" / "(555) 123-4567"), the answer is discarded. The log gets
// the two counts, never the values.

use std::collections::BTreeSet;

use super::detect::word_spans;

const UNITS: [&str; 20] = [
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "thirteen", "fourteen",
    "fifteen", "sixteen", "seventeen", "eighteen", "nineteen",
];
const TENS: [&str; 10] = ["", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];
const ORDINALS: [&str; 20] = [
    "zeroth", "first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth", "tenth", "eleventh", "twelfth",
    "thirteenth", "fourteenth", "fifteenth", "sixteenth", "seventeenth", "eighteenth", "nineteenth",
];
const TENS_ORDINALS: [&str; 10] = ["", "", "twentieth", "thirtieth", "fortieth", "fiftieth", "sixtieth", "seventieth", "eightieth", "ninetieth"];
const DAYS: [&str; 7] = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];
const MONTHS: [&str; 12] = [
    "january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december",
];
/// Short forms that are no other word.
const MONTHS_SHORT: [&str; 8] = ["feb", "apr", "jun", "jul", "aug", "sept", "oct", "nov"];
/// Month names that are also everyday words: a fact only next to a number
/// or with a capital mid-text.
const MONTHS_AMBIGUOUS: [&str; 2] = ["may", "march"];
const RELATIVE: [&str; 6] = ["today", "tonight", "tomorrow", "yesterday", "noon", "midnight"];
/// After a number word, these make a bare "one" a fact.
const AFTER_ONE: [&str; 8] = ["o'clock", "percent", "dollar", "dollars", "am", "pm", "hundred", "thousand"];

/// One side's facts: each value as a canonical string ("1850", "2.5",
/// "tuesday", "pm"), and every number's digits in order (for a phone
/// written in other groups).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub set: BTreeSet<String>,
    digits: String,
}

impl Facts {
    pub fn len(&self) -> usize {
        self.set.len()
    }

    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    fn number(&mut self, n: String) {
        self.digits.push_str(&n.replace('.', ""));
        self.set.insert(n);
    }
}

fn unit(w: &str) -> Option<u64> {
    UNITS.iter().position(|u| *u == w).map(|v| v as u64)
}

fn tens(w: &str) -> Option<u64> {
    TENS.iter().position(|t| !t.is_empty() && *t == w).map(|v| v as u64 * 10)
}

fn ordinal_word(w: &str) -> Option<u64> {
    if let Some(v) = ORDINALS.iter().position(|o| *o == w) {
        return Some(v as u64);
    }
    TENS_ORDINALS.iter().position(|o| !o.is_empty() && *o == w).map(|v| v as u64 * 10)
}

fn month_of(w: &str) -> Option<&'static str> {
    if let Some(m) = MONTHS.iter().find(|m| **m == w) {
        return Some(m);
    }
    if MONTHS_SHORT.contains(&w) {
        return MONTHS.iter().find(|m| m.starts_with(&w[..3])).copied();
    }
    None
}

/// "1850.00" -> "1850", "2.50" -> "2.5", "007" -> "7".
fn canonical(int: &str, frac: &str) -> String {
    let int = int.trim_start_matches('0');
    let int = if int.is_empty() { "0" } else { int };
    let frac = frac.trim_end_matches('0');
    if frac.is_empty() {
        int.to_string()
    } else {
        format!("{int}.{frac}")
    }
}

/// The text with placeholders ([PHONE1]) blanked: their digits aren't the user's.
fn without_placeholders(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0usize;
    for c in text.chars() {
        match c {
            '[' => depth += 1,
            ']' if depth > 0 => {
                depth -= 1;
                out.push(' ');
            }
            _ if depth > 0 => {}
            _ => out.push(c),
        }
    }
    out
}

/// Digits as written: "1,850.00", "2.5", "10:30", "10th", "3pm", "4B".
fn digit_facts(text: &str, out: &mut Facts) {
    let b = text.as_bytes();
    let digits_at = |i: usize| b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
    let mut i = 0;
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        let mut int = String::new();
        let mut n = digits_at(i);
        int.push_str(&text[i..i + n]);
        i += n;
        // Thousands: ",850".
        while i + 1 < b.len() && b[i] == b',' && digits_at(i + 1) == 3 && !(i + 4 < b.len() && b[i + 4].is_ascii_digit()) {
            int.push_str(&text[i + 1..i + 4]);
            i += 4;
        }
        let mut frac = String::new();
        if i + 1 < b.len() && b[i] == b'.' && digits_at(i + 1) > 0 {
            n = digits_at(i + 1);
            frac.push_str(&text[i + 1..i + 1 + n]);
            i += 1 + n;
        }
        // "3.30" is a time as often as a number (whisper writes times so):
        // an hour and two-digit minutes, with no currency sign before it,
        // reads as 3 and 30, like "3:30".
        let money = text[..start].trim_end().ends_with(['$', '€', '£']);
        let as_time = !money && frac.len() == 2 && int.len() <= 2 && int.parse::<u32>().is_ok_and(|h| h <= 23) && frac.parse::<u32>().is_ok_and(|m| m <= 59);
        if as_time {
            out.number(canonical(&int, ""));
            if frac != "00" {
                out.number(canonical(&frac, ""));
            }
        } else {
            out.number(canonical(&int, &frac));
        }
        // A time's minutes: "10:30" -> 10 and 30, "10:00" -> 10.
        if frac.is_empty() && i + 2 < b.len() && b[i] == b':' && digits_at(i + 1) == 2 {
            let minutes = &text[i + 1..i + 3];
            if minutes != "00" {
                out.number(canonical(minutes, ""));
            }
            i += 3;
        }
        // "3pm", "10am".
        let rest: Vec<u8> = b[i..].iter().take(3).map(u8::to_ascii_lowercase).collect();
        for s in [b"am", b"pm"] {
            if rest.starts_with(s) && !rest.get(2).is_some_and(u8::is_ascii_alphanumeric) {
                out.set.insert(String::from_utf8_lossy(s).into_owned());
            }
        }
    }
}

/// Edit distance, for short words.
fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push((prev[j] + usize::from(ca != cb)).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// A fact word as meant: itself, or the one it misspells by a letter
/// ("tomorow", "wenesday", "febuary", "fourty"), so that fixing the
/// spelling is not a changed date. Words under 5 letters only exactly.
fn as_meant(w: &str) -> String {
    // Real words a letter away from one ("weight" / "eight", "forth" / "forty").
    const WORDS: &[&str] = &["weight", "weights", "height", "heights", "weighty", "forth", "seconds", "thirds", "fourths", "fifths", "tenths"];
    let vocab = UNITS.iter().chain(&TENS).chain(&ORDINALS).chain(&TENS_ORDINALS).chain(&DAYS).chain(&MONTHS).chain(&RELATIVE);
    let vocab: Vec<&str> = vocab.copied().filter(|v| !v.is_empty()).collect();
    if w.chars().count() < 5 || vocab.contains(&w) || WORDS.contains(&w) {
        return w.to_string();
    }
    match vocab.iter().find(|v| v.chars().count() >= 5 && distance(w, v) == 1) {
        Some(v) => v.to_string(),
        None => w.to_string(),
    }
}

/// Number words, ordinals, days, months, today/tomorrow, am/pm.
fn word_facts(text: &str, out: &mut Facts) {
    // (lower-case word, written with a capital, has a digit)
    let mut words: Vec<(String, bool, bool)> = Vec::new();
    for (s, e) in word_spans(text) {
        let w = &text[s..e];
        let capital = w.chars().next().is_some_and(char::is_uppercase);
        let digit = w.chars().any(|c| c.is_ascii_digit());
        for part in w.split('-').filter(|p| !p.is_empty()) {
            words.push((if digit { part.to_lowercase() } else { as_meant(&part.to_lowercase()) }, capital, digit));
        }
    }
    // "o'clock" and "a.m." are split by word_spans: join them back.
    let mut joined: Vec<(String, bool, bool)> = Vec::with_capacity(words.len());
    let mut k = 0;
    while k < words.len() {
        let next = words.get(k + 1).map(|w| w.0.as_str());
        match (words[k].0.as_str(), next) {
            ("o", Some("clock")) => {
                joined.push(("o'clock".into(), false, false));
                k += 2;
            }
            ("a" | "p", Some("m")) if k > 0 && (words[k - 1].2 || unit(&words[k - 1].0).is_some() || tens(&words[k - 1].0).is_some()) => {
                joined.push((format!("{}m", words[k].0), false, false));
                k += 2;
            }
            _ => {
                joined.push(words[k].clone());
                k += 1;
            }
        }
    }
    let words = joined;
    let is_number = |w: &(String, bool, bool)| w.2 || unit(&w.0).is_some() || tens(&w.0).is_some() || ordinal_word(&w.0).is_some();
    // A month nearby; "may" and "march" only with a capital.
    let near_month = |k: usize| {
        (k.saturating_sub(3)..(k + 4).min(words.len()))
            .any(|j| j != k && month_of(&words[j].0).is_some() && (!MONTHS_AMBIGUOUS.contains(&words[j].0.as_str()) || words[j].1))
    };
    let mut k = 0;
    while k < words.len() {
        let w = words[k].0.as_str();
        if words[k].2 {
            k += 1;
            continue;
        }
        // "two hundred fifty", "twenty five", "a hundred and five", "ten".
        let starts_number = unit(w).is_some() || tens(w).is_some() || (w == "a" && matches!(words.get(k + 1).map(|x| x.0.as_str()), Some("hundred" | "thousand")));
        if starts_number {
            let start = k;
            let (mut total, mut current, mut n_words) = (0u64, 0u64, 0usize);
            while k < words.len() {
                let x = words[k].0.as_str();
                if let Some(v) = unit(x).or_else(|| tens(x)) {
                    current += v;
                } else if x == "a" && n_words == 0 {
                    current += 1;
                } else if x == "hundred" && n_words > 0 {
                    current = current.max(1) * 100;
                } else if x == "thousand" && n_words > 0 {
                    total += current.max(1) * 1000;
                    current = 0;
                } else if !(x == "and" && n_words > 0 && words.get(k + 1).is_some_and(|y| unit(&y.0).is_some() || tens(&y.0).is_some())) {
                    break;
                }
                n_words += 1;
                k += 1;
            }
            let after = words.get(k).map(|x| x.0.as_str()).unwrap_or("");
            // A bare "one" ("the one you sent") is no fact.
            let bare = n_words == 1 && words[start].0 == "one";
            if !bare || AFTER_ONE.contains(&after) || near_month(start) {
                out.number((total + current).to_string());
            }
            continue;
        }
        if let Some(v) = ordinal_word(w) {
            // "first", "second", "third" only next to a month ("the third of May").
            if v > 3 || near_month(k) {
                out.number(v.to_string());
            }
        } else if DAYS.contains(&w) || RELATIVE.contains(&w) {
            out.set.insert(w.to_string());
        } else if let Some(m) = month_of(w) {
            let number_near = (k > 0 && is_number(&words[k - 1])) || words.get(k + 1).is_some_and(is_number);
            if !MONTHS_AMBIGUOUS.contains(&w) || number_near || (words[k].1 && k > 0) {
                out.set.insert(m.to_string());
            }
        } else if matches!(w, "am" | "pm") && k > 0 && is_number(&words[k - 1]) {
            out.set.insert(w.to_string());
        }
        k += 1;
    }
}

/// The facts in `text`.
pub fn facts(text: &str) -> Facts {
    let plain = without_placeholders(text);
    let mut out = Facts::default();
    digit_facts(&plain, &mut out);
    word_facts(&plain, &mut out);
    out
}

/// Whether `after` keeps exactly the facts of `before`.
pub fn same(before: &str, after: &str) -> bool {
    let (a, b) = (facts(before), facts(after));
    if a.set == b.set {
        return true;
    }
    // The same digits in the same order, regrouped (a phone: "5551234567"
    // and "555-123-4567"), and the same words (days, months, am/pm).
    let words = |f: &Facts| f.set.iter().filter(|v| !v.starts_with(|c: char| c.is_ascii_digit())).cloned().collect::<BTreeSet<_>>();
    !a.digits.is_empty() && a.digits == b.digits && words(&a) == words(&b)
}

/// The log's note for a discarded answer: counts only.
pub fn log_counts(what: &str, before: &str, after: &str) -> String {
    format!("{what}: numbers/dates changed, discarded (before={} after={})", facts(before).len(), facts(after).len())
}

/// `same`, and the log's note (counts only) when not: for the callers that
/// keep no log of their own (ai/claude.rs logs nothing itself).
pub fn check(what: &str, before: &str, after: &str) -> bool {
    let ok = same(before, after);
    if !ok {
        crate::log::line(log_counts(what, before, after));
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(text: &str) -> Vec<String> {
        facts(text).set.into_iter().collect()
    }

    #[test]
    fn numbers_amounts_percentages_times_and_dates_are_read() {
        assert_eq!(set("Rent is $1,850.00 due on the 1st."), ["1", "1850"]);
        assert_eq!(set("A 15% fee"), ["15"]);
        assert_eq!(set("fifteen percent"), ["15"]);
        assert_eq!(set("Meet Tuesday at 10:30 am"), ["10", "30", "am", "tuesday"]);
        assert_eq!(set("ten o'clock on the tenth"), ["10"]);
        assert_eq!(set("two hundred fifty dollars"), ["250"]);
        assert_eq!(set("twenty-five units"), ["25"]);
        assert_eq!(set("the third of May"), ["3", "may"]);
        assert_eq!(set("Call [PHONE1] tomorrow"), ["tomorrow"]);
        // Not facts: a bare "one", "first" in prose, "may" the verb.
        assert!(set("the one you sent first, you may keep it").is_empty());
    }

    #[test]
    fn the_same_value_written_another_way_is_the_same() {
        for (a, b) in [
            ("ten", "10"),
            ("$1,850", "$1850"),
            ("$1,850.00", "1850 dollars"),
            ("ten o'clock", "10:00"),
            ("the tenth", "the 10th"),
            ("15 percent", "15%"),
            ("555-123-4567", "(555) 123-4567"),
            ("5551234567", "555-123-4567"),
            ("Tuesday at 3pm", "Tuesday at 3 p.m."),
            ("See you on Tuesday.", "See you Tuesday!"),
            ("unit four B", "unit 4B"),
            // Whisper writes a time as "3.30" (Stage 4 on CI, 2026-11-04).
            ("on Thursday at 3.30", "on Thursday at 3:30"),
            ("$2.50 each", "$2.50 each"),
            // A spelling fix is not a changed date.
            ("we will deposit it tomorow", "we will deposit it tomorrow"),
            ("see you wenesday", "see you Wednesday"),
            ("fourty dollars", "forty dollars"),
        ] {
            assert!(same(a, b), "{a} / {b}");
        }
    }

    #[test]
    fn a_changed_dropped_or_added_value_is_not() {
        for (a, b) in [
            ("$1,850", "$1,580"),
            ("Tuesday", "Wednesday"),
            ("at 10 am", "at 10 pm"),
            ("on the tenth", "on the eleventh"),
            ("15%", "50%"),
            ("Rent is $1,850.", "Rent is due."),
            ("See you soon.", "See you on Friday."),
            ("in 3 days", "in three weeks or 4"),
            ("May 5", "June 5"),
            ("call me at 10:30", "call me at 10:45"),
        ] {
            assert!(!same(a, b), "{a} / {b}");
        }
    }

    #[test]
    fn the_log_gets_counts_only() {
        let line = log_counts("live", "Pay $1,850 on Tuesday", "Pay on Wednesday");
        assert_eq!(line, "live: numbers/dates changed, discarded (before=2 after=1)");
        assert!(!line.contains("1850") && !line.contains("uesday"));
    }
}
