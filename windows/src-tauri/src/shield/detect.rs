// What the shield swaps out, found on this PC with plain rules (no model):
// emails (written, or spoken as "name at example dot com"), phone numbers,
// street addresses ("123 Main St", "PO Box 12"), unit numbers ("Unit 4B",
// "Apt 12", "Suite 200", "#204"; only the number goes) and people's names
// from the shield's name list. Each finder returns byte spans; `find` keeps
// the first of overlapping ones in the order email, address, phone, unit,
// person.

use super::{Kind, NameSet};

/// A value to swap: bytes `start..end` of the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
}

/// Everything to swap in `text`, in order, none overlapping.
pub fn find(text: &str, names: &NameSet) -> Vec<Span> {
    let words = word_spans(text);
    let mut all: Vec<Span> = Vec::new();
    for found in [emails(text, &words), addresses(text, &words), phones(text), units(text, &words), people(text, &words, names)] {
        all.extend(found);
    }
    // Earlier finders win: stable sort by start keeps their order on ties,
    // and a later span overlapping a kept one is dropped.
    let mut kept: Vec<Span> = Vec::new();
    for s in all {
        if s.start < s.end && !kept.iter().any(|k| s.start < k.end && k.start < s.end) {
            kept.push(s);
        }
    }
    kept.sort_by_key(|s| s.start);
    kept
}

// ── Words ─────────────────────────────────────────────────────────────────

/// Words: runs of letters and digits; an apostrophe before a capital
/// (O'Brien) and a hyphen between letters or digits (Mary-Jane) stay inside.
pub fn word_spans(text: &str) -> Vec<(usize, usize)> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (k, &(i, c)) in chars.iter().enumerate() {
        let prev_alnum = k > 0 && chars[k - 1].1.is_alphanumeric();
        let next = chars.get(k + 1).map(|&(_, n)| n);
        let joins = match c {
            '\'' | '\u{2019}' => prev_alnum && next.is_some_and(char::is_uppercase),
            '-' => prev_alnum && next.is_some_and(char::is_alphanumeric),
            _ => false,
        };
        if c.is_alphanumeric() || (joins && start.is_some()) {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(s) = start.take() {
            out.push((s, i));
        }
    }
    if let Some(s) = start {
        out.push((s, text.len()));
    }
    out
}

/// Only spaces (at most `max` characters) between two words.
fn spaced(text: &str, from: usize, to: usize, max: usize) -> bool {
    let gap = &text[from..to];
    !gap.is_empty() && gap.len() <= max && gap.chars().all(|c| c == ' ' || c == '\u{a0}')
}

fn lower(text: &str, w: (usize, usize)) -> String {
    text[w.0..w.1].to_lowercase()
}

fn capitalized(s: &str) -> bool {
    s.chars().next().is_some_and(char::is_uppercase)
}

// ── Emails ────────────────────────────────────────────────────────────────

const SPOKEN_TLDS: &[&str] = &["com", "org", "net", "edu", "gov", "io", "us", "co", "info", "biz", "me", "app", "uk", "ca"];

fn emails(text: &str, words: &[(usize, usize)]) -> Vec<Span> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let local_ok = |c: u8| c.is_ascii_alphanumeric() || b"._%+-".contains(&c);
    let domain_ok = |c: u8| c.is_ascii_alphanumeric() || c == b'.' || c == b'-';
    for (at, _) in text.match_indices('@') {
        let mut s = at;
        while s > 0 && local_ok(b[s - 1]) {
            s -= 1;
        }
        let mut e = at + 1;
        while e < b.len() && domain_ok(b[e]) {
            e += 1;
        }
        while e > at + 1 && (b[e - 1] == b'.' || b[e - 1] == b'-') {
            e -= 1;
        }
        let domain = &text[at + 1..e];
        let tld_ok = domain.rsplit('.').next().is_some_and(|t| t.len() >= 2 && t.chars().all(|c| c.is_ascii_alphabetic()));
        if s < at && domain.contains('.') && tld_ok {
            out.push(Span { start: s, end: e, kind: Kind::Email });
        }
    }
    // Spoken: "jordan at example dot com", "j dot lee at mail dot co dot uk".
    let lw: Vec<String> = words.iter().map(|&w| lower(text, w)).collect();
    for i in 1..words.len() {
        if lw[i] != "at" {
            continue;
        }
        let mut j = i + 1;
        let mut last = None;
        while j + 2 < words.len() && lw[j + 1] == "dot" {
            j += 2;
            if SPOKEN_TLDS.contains(&lw[j].as_str()) {
                last = Some(j);
            }
        }
        let Some(last) = last else { continue };
        let mut first = i - 1;
        while first >= 2 && lw[first - 1] == "dot" {
            first -= 2;
        }
        out.push(Span { start: words[first].0, end: words[last].1, kind: Kind::Email });
    }
    out
}

/// The phone numbers in `text` (byte spans), as the shield finds them.
pub fn phones_in(text: &str) -> Vec<(usize, usize)> {
    phones(text).into_iter().map(|s| (s.start, s.end)).collect()
}

// ── Phone numbers ─────────────────────────────────────────────────────────

/// (555) 123-4567, 555-123-4567, 555.123.4567, 555 123 4567, 5551234567,
/// 1-555-123-4567, +1 555 123 4567, +44 20 7946 0958, 555-0123.
fn phones(text: &str) -> Vec<Span> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = Vec::new();
    let mut k = 0;
    while k < chars.len() {
        let (i, c) = chars[k];
        let prev = k.checked_sub(1).map(|p| chars[p].1);
        let starts = (c.is_ascii_digit() || c == '(' || c == '+') && !prev.is_some_and(|p| p.is_alphanumeric() || p == '-' || p == '.' || p == '/' || p == '$');
        if !starts {
            k += 1;
            continue;
        }
        let plus = c == '+';
        let mut m = if plus { k + 1 } else { k };
        let mut groups: Vec<usize> = Vec::new();
        let mut dash_only = true;
        let mut end = i;
        loop {
            let paren = chars.get(m).is_some_and(|&(_, c)| c == '(');
            if paren {
                m += 1;
            }
            let digits = chars[m.min(chars.len())..].iter().take_while(|(_, c)| c.is_ascii_digit()).count();
            if digits == 0 {
                break;
            }
            m += digits;
            groups.push(digits);
            end = chars.get(m).map_or(text.len(), |&(j, _)| j);
            if paren {
                if chars.get(m).is_some_and(|&(_, c)| c == ')') {
                    m += 1;
                } else {
                    break;
                }
                // ") " or ")": the next group follows.
                if chars.get(m).is_some_and(|&(_, c)| c == ' ') {
                    m += 1;
                }
                dash_only = false;
                continue;
            }
            match chars.get(m).map(|&(_, c)| c) {
                Some(sep @ (' ' | '-' | '.')) if chars.get(m + 1).is_some_and(|&(_, n)| n.is_ascii_digit() || n == '(') => {
                    if sep != '-' {
                        dash_only = false;
                    }
                    m += 1;
                }
                _ => break,
            }
        }
        let next_ok = !text[end..].chars().next().is_some_and(char::is_alphanumeric);
        let total: usize = groups.iter().sum();
        let phone = next_ok
            && if plus {
                (8..=15).contains(&total)
            } else {
                matches!(groups.as_slice(), [3, 3, 4] | [1, 3, 3, 4] | [10])
                    || (groups.as_slice() == [11] && chars[k].1 == '1')
                    || (groups.as_slice() == [3, 4] && dash_only)
            };
        if phone {
            out.push(Span { start: i, end, kind: Kind::Phone });
            k = m.max(k + 1);
        } else {
            k += 1;
        }
    }
    out
}

// ── Street addresses ──────────────────────────────────────────────────────

const SUFFIXES: &[&str] = &[
    "street", "st", "avenue", "ave", "av", "road", "rd", "boulevard", "blvd", "drive", "dr", "lane", "ln", "court", "ct",
    "way", "place", "pl", "parkway", "pkwy", "highway", "hwy", "circle", "cir", "terrace", "ter", "trail", "trl", "square",
    "sq", "loop", "row", "path", "pike", "alley", "crescent", "plaza", "plz", "expressway", "freeway", "broadway", "point",
    "pt", "run", "walk", "close",
];
const DIRECTIONS: &[&str] = &["n", "s", "e", "w", "ne", "nw", "se", "sw", "north", "south", "east", "west"];

fn house_number(w: &str) -> bool {
    let digits = w.chars().take_while(char::is_ascii_digit).count();
    let rest = &w[digits..];
    (1..=6).contains(&digits) && (rest.is_empty() || (rest.len() == 1 && rest.chars().all(|c| c.is_ascii_alphabetic())))
}

fn ordinal(w: &str) -> bool {
    let digits = w.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && ["st", "nd", "rd", "th"].contains(&w[digits..].to_lowercase().as_str())
}

fn addresses(text: &str, words: &[(usize, usize)]) -> Vec<Span> {
    let mut out = Vec::new();
    let lw: Vec<String> = words.iter().map(|&w| lower(text, w)).collect();
    for i in 0..words.len() {
        // PO Box 12 / P.O. Box 12.
        if lw[i] == "box" && i + 1 < words.len() && words[i + 1].1 > words[i + 1].0 && text[words[i + 1].0..words[i + 1].1].chars().all(|c| c.is_ascii_digit()) {
            let first = if i >= 1 && lw[i - 1] == "po" {
                Some(i - 1)
            } else if i >= 2 && lw[i - 1] == "o" && lw[i - 2] == "p" {
                Some(i - 2)
            } else {
                None
            };
            if let Some(f) = first {
                out.push(Span { start: words[f].0, end: words[i + 1].1, kind: Kind::Address });
            }
            continue;
        }
        let w = &text[words[i].0..words[i].1];
        if !house_number(w) {
            continue;
        }
        // Not a unit's number ("Suite 200 Main …" is a unit, not an address).
        if i > 0 && UNIT_WORDS.contains(&lw[i - 1].as_str()) {
            continue;
        }
        let mut j = i + 1;
        while j < words.len() && j <= i + 5 {
            let gap = &text[words[j - 1].1..words[j].0];
            if gap.is_empty() || gap.len() > 3 || !gap.chars().all(|c| c == ' ' || c == '.') {
                break;
            }
            let word = &text[words[j].0..words[j].1];
            if j > i + 1 && SUFFIXES.contains(&lw[j].as_str()) {
                out.push(Span { start: words[i].0, end: words[j].1, kind: Kind::Address });
                break;
            }
            if capitalized(word) || ordinal(word) || DIRECTIONS.contains(&lw[j].as_str()) {
                j += 1;
            } else {
                break;
            }
        }
    }
    out
}

// ── Unit numbers ──────────────────────────────────────────────────────────

const UNIT_WORDS: &[&str] = &["unit", "units", "apt", "apts", "apartment", "apartments", "suite", "suites", "ste", "room"];

/// A unit's id: digits with at most two letters (204, 7D, B12), or one
/// capital letter other than I (Unit B).
fn unit_id(w: &str) -> bool {
    let digits = w.chars().filter(char::is_ascii_digit).count();
    let letters = w.chars().filter(char::is_ascii_alphabetic).count();
    let all_ascii = w.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    (all_ascii && digits >= 1 && letters <= 2 && w.len() <= 8) || (w.len() == 1 && w != "I" && w.chars().all(|c| c.is_ascii_uppercase()))
}

fn units(text: &str, words: &[(usize, usize)]) -> Vec<Span> {
    let mut out = Vec::new();
    let lw: Vec<String> = words.iter().map(|&w| lower(text, w)).collect();
    let word = |j: usize| &text[words[j].0..words[j].1];
    for i in 0..words.len() {
        // "#204", the # on its own (not "C#").
        let before = &text[..words[i].0];
        if before.ends_with('#') && !before[..before.len() - 1].ends_with(char::is_alphanumeric) && unit_id(word(i)) {
            out.push(Span { start: words[i].0, end: words[i].1, kind: Kind::Unit });
            continue;
        }
        if !UNIT_WORDS.contains(&lw[i].as_str()) {
            continue;
        }
        let mut j = i + 1;
        // "Unit No. 5".
        if j < words.len() && lw[j] == "no" {
            j += 1;
        }
        let mut first = true;
        while j < words.len() {
            let gap = &text[words[j - 1].1..words[j].0];
            let gap_ok = !gap.is_empty()
                && if first {
                    gap.len() <= 4 && gap.chars().all(|c| matches!(c, ' ' | '#' | '.' | ':'))
                } else {
                    gap.len() <= 3 && gap.chars().all(|c| matches!(c, ' ' | ',' | '/' | '&'))
                };
            if !gap_ok || !unit_id(word(j)) {
                break;
            }
            out.push(Span { start: words[j].0, end: words[j].1, kind: Kind::Unit });
            first = false;
            j += 1;
            // "101, 102 and 104".
            if j + 1 < words.len() && lw[j] == "and" && spaced(text, words[j - 1].1, words[j].0, 2) && unit_id(word(j + 1)) {
                j += 1;
            }
        }
    }
    out
}

// ── People's names ────────────────────────────────────────────────────────

/// Common words that are also names, or are in address-book entries that
/// aren't people ("Leasing Office"): never swapped on their own. A whole
/// name holding one ("Will Smith") still is.
const STOP_WORDS: &[&str] = &[
    "a", "about", "after", "again", "all", "am", "an", "and", "any", "april", "are", "art", "as", "at", "august", "be",
    "best", "bill", "both", "but", "by", "can", "carter", "center", "chase", "cole", "conference", "court", "dash",
    "day", "dean", "dear", "desk", "do", "east", "faith", "for", "frank", "friday", "from", "front", "glim", "go",
    "grace", "grant", "group", "hall", "he", "help", "hello", "her", "hi", "him", "his", "hope", "hr", "hunter", "i",
    "if", "in", "info", "is", "it", "jack", "january", "joy", "july", "june", "just", "lane", "leasing", "main",
    "maintenance", "management", "manager", "march", "mark", "max", "may", "me", "monday", "my", "new", "no", "north",
    "not", "of", "office", "ok", "on", "or", "our", "outlook", "page", "park", "pat", "price", "property", "ray", "regards",
    "rich", "room", "rose", "sales", "saturday", "see", "she", "so", "south", "staff", "summer", "sunday", "sunny",
    "support", "team", "teams", "thank", "thanks", "the", "they", "this", "thursday", "to", "today", "tuesday", "unit",
    "up", "us", "we", "wednesday", "west", "will", "with", "word", "yes", "you", "your", "admin", "accounting", "service",
    "services", "account", "accounts", "billing", "payments", "payment", "reception", "security", "it's", "shop",
    "store", "home", "work", "mobile", "cell", "fax", "phone", "email", "mail", "office's", "contact", "contacts",
    "everyone", "all-staff", "board", "owner", "owners", "tenant", "tenants", "resident", "residents", "residence",
    "community", "apartments", "homes", "realty", "properties", "llc", "inc", "corp", "co", "dept",
];

pub fn is_stop_word(w: &str) -> bool {
    STOP_WORDS.contains(&w)
}

/// A name entry's words, lower case: what's in parentheses goes, and an
/// address is no name. "Lopez, Maria" also gives "maria lopez" (second).
pub fn name_words(entry: &str) -> (Vec<String>, Option<Vec<String>>) {
    if entry.contains('@') {
        return (Vec::new(), None);
    }
    let mut plain = String::new();
    let mut depth = 0i32;
    for c in entry.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => plain.push(c),
            _ => {}
        }
    }
    let comma = plain.find(',');
    let words: Vec<String> = word_spans(&plain)
        .into_iter()
        .map(|(s, e)| plain[s..e].to_lowercase())
        .filter(|w| w.chars().count() >= 2 && w.chars().all(|c| c.is_alphabetic() || c == '\'' || c == '\u{2019}' || c == '-'))
        .collect();
    let reversed = comma.and_then(|at| {
        let before = word_spans(&plain[..at]).len();
        (before >= 1 && before < words.len()).then(|| {
            let mut r = words[before..].to_vec();
            r.extend_from_slice(&words[..before]);
            r
        })
    });
    (words, reversed)
}

fn people(text: &str, words: &[(usize, usize)], names: &NameSet) -> Vec<Span> {
    let mut out = Vec::new();
    if names.is_empty() && names.full.is_empty() {
        return out;
    }
    let lw: Vec<String> = words.iter().map(|&w| lower(text, w)).collect();
    let mut i = 0;
    while i < words.len() {
        // A whole name, any case, words one or two spaces apart.
        let whole = names.full.iter().find(|full| {
            i + full.len() <= words.len()
                && full.iter().enumerate().all(|(k, w)| lw[i + k] == *w)
                && (1..full.len()).all(|k| spaced(text, words[i + k - 1].1, words[i + k].0, 2))
        });
        if let Some(full) = whole {
            out.push(Span { start: words[i].0, end: words[i + full.len() - 1].1, kind: Kind::Person });
            i += full.len();
            continue;
        }
        // One word of a name, written with a capital.
        let w = &text[words[i].0..words[i].1];
        if capitalized(w) && names.words.contains(&lw[i]) && !is_stop_word(&lw[i]) {
            out.push(Span { start: words[i].0, end: words[i].1, kind: Kind::Person });
        }
        i += 1;
    }
    out
}
