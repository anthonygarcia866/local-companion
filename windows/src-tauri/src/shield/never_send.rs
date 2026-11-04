// The never-send filter: a request holding one of these is not sent at all
// (claude::send refuses the whole body; nothing is masked instead).
//
// - card, ID and bank numbers (capture::sensitive: Luhn-valid cards, numbers
//   written like an SSN, IBANs, long digit runs being typed), and LONG_RUN or
//   more digits said one at a time ("4 1 1 1 …", "four one one one …"), as
//   dictation hears a card read out;
// - an SSN written any other way after its label ("SSN 123456789",
//   "social security number: 123 45 6789", "ITIN …");
// - a date of birth: a label ("DOB", "date of birth", "birth date", "born
//   on" …) followed by a date. Stricter than "next to a name" on purpose: a
//   labelled date of birth is about a person whether or not their name is in
//   the same request (it may be a placeholder by then);
// - income or benefit amounts labelled as such: a money amount ($1,200,
//   1200 dollars) within a few words of "income", "salary", "wages", "SSI",
//   "SSDI", "SNAP", "child support", "pension" …
//
// Plain rules on this PC; nothing logged here.

use super::detect::word_spans;

/// Why a text must not be sent. Codes only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// A card, ID or bank number (capture::sensitive).
    Number,
    Ssn,
    BirthDate,
    Income,
}

impl Reason {
    pub fn code(self) -> &'static str {
        match self {
            Reason::Number => "number",
            Reason::Ssn => "ssn",
            Reason::BirthDate => "birth-date",
            Reason::Income => "income",
        }
    }
}

/// The first reason `text` must not be sent, if any.
pub fn reason(text: &str) -> Option<Reason> {
    if crate::capture::sensitive::contains_sensitive_number(text) {
        return Some(Reason::Number);
    }
    let words: Vec<(usize, usize)> = word_spans(text);
    let lw: Vec<String> = words.iter().map(|&(s, e)| text[s..e].to_lowercase()).collect();
    if said_digit_by_digit(&lw) {
        return Some(Reason::Number);
    }
    if labelled_ssn(text, &words, &lw) {
        return Some(Reason::Ssn);
    }
    if birth_date(text, &words, &lw) {
        return Some(Reason::BirthDate);
    }
    if income(text, &words, &lw) {
        return Some(Reason::Income);
    }
    None
}

/// Where each label (a sequence of lower-case words) ends, as word indices:
/// (first word, last word).
fn labels_at(lw: &[String], labels: &[&[&str]]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for i in 0..lw.len() {
        for label in labels {
            if i + label.len() <= lw.len() && label.iter().enumerate().all(|(k, w)| lw[i + k] == *w) {
                out.push((i, i + label.len() - 1));
            }
        }
    }
    out
}

// ── Digits said one at a time ─────────────────────────────────────────────

const DIGIT_WORDS: &[&str] = &["zero", "oh", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"];

/// LONG_RUN (12) or more words in a row that are each one digit, written
/// ("4") or said ("four", "oh"): a card or account number read out. A phone
/// number (10 or 11) stays under it.
fn said_digit_by_digit(lw: &[String]) -> bool {
    let one = |w: &String| (w.len() == 1 && w.chars().all(|c| c.is_ascii_digit())) || DIGIT_WORDS.contains(&w.as_str());
    let mut run = 0;
    for w in lw {
        run = if one(w) { run + 1 } else { 0 };
        if run >= crate::capture::sensitive::LONG_RUN {
            return true;
        }
    }
    false
}

// ── SSN after its label ───────────────────────────────────────────────────

const SSN_LABELS: &[&[&str]] = &[
    &["ssn"],
    &["ssns"],
    &["ss"],
    &["itin"],
    &["social", "security", "number"],
    &["social", "security", "no"],
    &["social", "security", "num"],
    &["social", "security"],
    &["tax", "id"],
    &["taxpayer", "id"],
];

fn labelled_ssn(text: &str, words: &[(usize, usize)], lw: &[String]) -> bool {
    labels_at(lw, SSN_LABELS).into_iter().any(|(_, last)| {
        // Nine digits within the next few words, in up to three groups.
        let mut digits = 0;
        let mut groups = 0;
        for k in last + 1..(last + 6).min(words.len()) {
            let w = &text[words[k].0..words[k].1];
            if w.chars().all(|c| c.is_ascii_digit()) {
                digits += w.len();
                groups += 1;
                if digits == 9 && groups <= 3 {
                    let after = text[words[k].1..].chars().next();
                    return !after.is_some_and(|c| c.is_ascii_digit());
                }
                if digits > 9 {
                    return false;
                }
            } else if digits > 0 {
                return false;
            }
        }
        false
    })
}

// ── Dates of birth ────────────────────────────────────────────────────────

const BIRTH_LABELS: &[&[&str]] = &[
    &["dob"],
    &["d", "o", "b"],
    &["date", "of", "birth"],
    &["birth", "date"],
    &["birthdate"],
    &["born", "on"],
    &["born"],
    &["fecha", "de", "nacimiento"],
];

const MONTHS: &[&str] = &[
    "january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november",
    "december", "jan", "feb", "mar", "apr", "jun", "jul", "aug", "sep", "sept", "oct", "nov", "dec",
];

/// A date starting at word `k`: 3/4/1980, 03-04-80, 1980-03-04, March 4
/// (1980), 4 March 1980. Separators between its parts are checked in the text.
fn date_at(text: &str, words: &[(usize, usize)], lw: &[String], k: usize) -> bool {
    let num = |j: usize, len: std::ops::RangeInclusive<usize>| {
        j < words.len() && lw[j].chars().all(|c| c.is_ascii_digit()) && len.contains(&lw[j].len())
            || j < words.len() && {
                // "4th".
                let d = lw[j].chars().take_while(char::is_ascii_digit).count();
                d > 0 && d <= 2 && ["st", "nd", "rd", "th"].contains(&&lw[j][d..])
            }
    };
    let sep = |j: usize| j + 1 < words.len() && matches!(&text[words[j].1..words[j + 1].0], "/" | "-" | ".");
    // One word with dashes (word_spans keeps "1980-03-04" whole).
    if k < words.len() {
        let parts: Vec<&str> = lw[k].split('-').collect();
        if parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())) {
            let (a, b, c) = (parts[0].len(), parts[1].len(), parts[2].len());
            if (1..=4).contains(&a) && (1..=2).contains(&b) && (2..=4).contains(&c) {
                return true;
            }
        }
    }
    // Numbers: d/m/y or y.m.d.
    if num(k, 1..=4) && sep(k) && num(k + 1, 1..=2) && sep(k + 1) && num(k + 2, 2..=4) {
        return true;
    }
    // Month name and a day.
    if k < words.len() && MONTHS.contains(&lw[k].as_str()) && num(k + 1, 1..=2) {
        return true;
    }
    // Day and month name.
    if num(k, 1..=2) && k + 1 < words.len() && MONTHS.contains(&lw[k + 1].as_str()) {
        return true;
    }
    false
}

fn birth_date(text: &str, words: &[(usize, usize)], lw: &[String]) -> bool {
    labels_at(lw, BIRTH_LABELS).into_iter().any(|(_, last)| {
        // The date within the next few words ("DOB: 3/4/1980", "born on the 4th of March").
        (last + 1..(last + 5).min(words.len())).any(|k| date_at(text, words, lw, k))
    })
}

// ── Income and benefits ───────────────────────────────────────────────────

const INCOME_LABELS: &[&[&str]] = &[
    &["income"],
    &["incomes"],
    &["salary"],
    &["salaries"],
    &["wage"],
    &["wages"],
    &["earnings"],
    &["paycheck"],
    &["paychecks"],
    &["paystub"],
    &["pay", "stub"],
    &["gross", "pay"],
    &["net", "pay"],
    &["take", "home"],
    &["benefit"],
    &["benefits"],
    &["ssi"],
    &["ssdi"],
    &["snap"],
    &["tanf"],
    &["wic"],
    &["food", "stamps"],
    &["social", "security"],
    &["disability"],
    &["unemployment"],
    &["pension"],
    &["alimony"],
    &["child", "support"],
    &["welfare"],
    &["stipend"],
    &["ingresos"],
    &["salario"],
];

/// How many words apart a label and an amount may be.
const NEAR: usize = 6;

/// Word indices of money amounts: a number after "$", or before "dollars".
fn amounts(text: &str, words: &[(usize, usize)], lw: &[String]) -> Vec<usize> {
    let mut out = Vec::new();
    for (k, &(s, _)) in words.iter().enumerate() {
        if !lw[k].starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        let dollar = text[..s].trim_end_matches(' ').ends_with('$');
        // The amount's last word: "1,200.50" is three words.
        let mut last = k;
        while last + 1 < words.len() && matches!(&text[words[last].1..words[last + 1].0], "," | ".") && lw[last + 1].chars().all(|c| c.is_ascii_digit()) {
            last += 1;
        }
        let spelled = last + 1 < words.len() && ["dollars", "dollar", "usd", "bucks"].contains(&lw[last + 1].as_str());
        let thousands = lw[last].ends_with('k') && dollar;
        if dollar || spelled || thousands {
            out.push(k);
        }
    }
    out
}

fn income(text: &str, words: &[(usize, usize)], lw: &[String]) -> bool {
    let money = amounts(text, words, lw);
    if money.is_empty() {
        return false;
    }
    let labels = labels_at(lw, INCOME_LABELS);
    labels.iter().any(|&(first, last)| money.iter().any(|&m| (m > last && m - last <= NEAR) || (m < first && first - m <= NEAR)))
}
