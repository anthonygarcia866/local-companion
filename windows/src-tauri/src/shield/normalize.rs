// The dictation lane's neutral form (owner, 2026-11-03): before the shield
// masks a transcript, phone numbers and unit numbers are written the way
// they should be typed, on this PC. The placeholder then stands for the
// formatted value, so what comes back into the field is "unit 4B" and
// "555-123-4567", not whisper's "unit four B" or "555 123 4567". Claude
// never sees either value.
//
// - Phones: digits said as words ("five five five, one two three …") or
//   written in any grouping (detect::phones) become 555-123-4567,
//   1-555-123-4567 or 555-0123. "+…" international numbers stay as written.
// - Units: after "unit", "apt", "apartment", "suite", "ste" or "room", a
//   number said as words ("four", "two oh four", "twenty two", "one hundred
//   four") or in digits, and a letter said apart ("B"), become "4B", "204".

use super::detect::word_spans;

const DIGIT_WORDS: &[(&str, u32)] = &[
    ("zero", 0), ("oh", 0), ("o", 0), ("one", 1), ("two", 2), ("three", 3), ("four", 4), ("five", 5), ("six", 6), ("seven", 7),
    ("eight", 8), ("nine", 9),
];
const TEENS: &[(&str, u32)] = &[
    ("ten", 10), ("eleven", 11), ("twelve", 12), ("thirteen", 13), ("fourteen", 14), ("fifteen", 15), ("sixteen", 16),
    ("seventeen", 17), ("eighteen", 18), ("nineteen", 19),
];
const TENS: &[(&str, u32)] = &[("twenty", 20), ("thirty", 30), ("forty", 40), ("fifty", 50), ("sixty", 60), ("seventy", 70), ("eighty", 80), ("ninety", 90)];
const UNIT_WORDS: &[&str] = &["unit", "apt", "apartment", "suite", "ste", "room"];

fn lookup(table: &[(&str, u32)], w: &str) -> Option<u32> {
    table.iter().find(|(k, _)| *k == w).map(|(_, v)| *v)
}

/// A digit said as a word ("oh" and "o" are zero), not "o" on its own run.
fn digit_word(w: &str) -> Option<u32> {
    // A single digit written as a digit counts too ("2 oh 4").
    let mut chars = w.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_digit() => c.to_digit(10),
        _ => lookup(DIGIT_WORDS, w),
    }
}

/// The text with phones and units in their typed form.
pub fn dictation(text: &str) -> String {
    let text = spoken_phones(text);
    let text = written_phones(&text);
    units(&text)
}

/// Replaces byte spans (sorted, not overlapping) with new text.
fn splice(text: &str, edits: &[(usize, usize, String)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (s, e, new) in edits {
        out.push_str(&text[at..*s]);
        out.push_str(new);
        at = *e;
    }
    out.push_str(&text[at..]);
    out
}

/// 10 digits → 555-123-4567; 11 starting with 1 → 1-555-123-4567; 7 →
/// 555-0123. None for any other count.
fn phone_form(digits: &str) -> Option<String> {
    match digits.len() {
        10 => Some(format!("{}-{}-{}", &digits[..3], &digits[3..6], &digits[6..])),
        11 if digits.starts_with('1') => Some(format!("1-{}-{}-{}", &digits[1..4], &digits[4..7], &digits[7..])),
        7 => Some(format!("{}-{}", &digits[..3], &digits[3..])),
        _ => None,
    }
}

/// Runs of digits said as words, between which only spaces, commas,
/// periods or dashes stand: 7, 10 or 11 of them are a phone number.
fn spoken_phones(text: &str) -> String {
    let words = word_spans(text);
    let lw: Vec<String> = words.iter().map(|&(s, e)| text[s..e].to_lowercase()).collect();
    let mut edits = Vec::new();
    let mut i = 0;
    while i < words.len() {
        // "o" alone starts nothing ("o" is a letter more often than a zero).
        if digit_word(&lw[i]).is_none() || lw[i] == "o" {
            i += 1;
            continue;
        }
        let mut j = i;
        let mut digits = String::new();
        while j < words.len() {
            let Some(d) = digit_word(&lw[j]) else { break };
            if j > i {
                let gap = &text[words[j - 1].1..words[j].0];
                if gap.len() > 3 || !gap.chars().all(|c| matches!(c, ' ' | ',' | '.' | '-')) {
                    break;
                }
            }
            digits.push(char::from_digit(d, 10).unwrap_or('0'));
            j += 1;
        }
        match phone_form(&digits) {
            Some(form) => {
                edits.push((words[i].0, words[j - 1].1, form));
                i = j;
            }
            None => i += 1,
        }
    }
    splice(text, &edits)
}

/// Phones written in digits (detect's shapes), regrouped.
fn written_phones(text: &str) -> String {
    let edits: Vec<(usize, usize, String)> = super::detect::phones_in(text)
        .into_iter()
        .filter(|&(s, _)| !text[s..].starts_with('+'))
        .filter_map(|(s, e)| {
            let digits: String = text[s..e].chars().filter(char::is_ascii_digit).collect();
            phone_form(&digits).filter(|f| *f != text[s..e]).map(|f| (s, e, f))
        })
        .collect();
    splice(text, &edits)
}

/// A number said as words, from word `i`: (its digits, the words used).
/// "two oh four" → 204 (digit by digit), "twenty two" → 22, "one hundred
/// four" → 104, "four" → 4.
fn spoken_number(lw: &[String], i: usize) -> Option<(String, usize)> {
    // Two or more digits said one by one.
    let run = lw[i..].iter().take_while(|w| digit_word(w).is_some()).count();
    if run >= 2 {
        let digits: String = lw[i..i + run].iter().filter_map(|w| digit_word(w)).filter_map(|d| char::from_digit(d, 10)).collect();
        if !lw.get(i + run).is_some_and(|w| w == "hundred") {
            return Some((digits, run));
        }
    }
    let small = |k: usize| -> Option<(u32, usize)> {
        let w = lw.get(k)?;
        if let Some(t) = lookup(TENS, w) {
            let ones = lw.get(k + 1).and_then(|n| digit_word(n)).filter(|d| *d > 0 && lw[k + 1] != "o" && lw[k + 1] != "oh");
            return Some(ones.map_or((t, 1), |o| (t + o, 2)));
        }
        lookup(TEENS, w).or_else(|| digit_word(w).filter(|_| w != "o" && w != "oh")).map(|v| (v, 1))
    };
    let (mut value, mut used) = small(i)?;
    if lw.get(i + used).is_some_and(|w| w == "hundred") && value < 10 {
        value *= 100;
        used += 1;
        if lw.get(i + used).is_some_and(|w| w == "and") && small(i + used + 1).is_some() {
            used += 1;
        }
        if let Some((rest, n)) = small(i + used).filter(|(v, _)| *v < 100) {
            value += rest;
            used += n;
        }
    }
    Some((value.to_string(), used))
}

/// A letter said on its own after a unit's number: "B", "b", "C." (not "a"
/// or "i", which are words).
fn unit_letter(word: &str) -> Option<char> {
    let mut chars = word.chars();
    let c = chars.next()?;
    (chars.next().is_none() && c.is_ascii_alphabetic() && !matches!(c, 'a' | 'i' | 'A' | 'I')).then(|| c.to_ascii_uppercase())
}

fn units(text: &str) -> String {
    let words = word_spans(text);
    let lw: Vec<String> = words.iter().map(|&(s, e)| text[s..e].to_lowercase()).collect();
    let mut edits = Vec::new();
    let mut i = 0;
    while i < words.len() {
        if !UNIT_WORDS.contains(&lw[i].as_str()) {
            i += 1;
            continue;
        }
        let mut k = i + 1;
        if lw.get(k).is_some_and(|w| w == "number" || w == "no") {
            k += 1;
        }
        if k >= words.len() {
            break;
        }
        let gap_ok = |a: usize, b: usize| {
            let gap = &text[words[a].1..words[b].0];
            !gap.is_empty() && gap.len() <= 3 && gap.chars().all(|c| matches!(c, ' ' | '#' | '.' | ':' | '-'))
        };
        if !gap_ok(k - 1, k) {
            i += 1;
            continue;
        }
        // The number: digits as written, or said as words; one word with
        // hyphens ("2-0-4", "two-oh-four") is read part by part.
        let parts: Vec<String> = lw[k].split('-').map(str::to_string).collect();
        let hyphened = parts.len() > 1
            && if parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())) {
                true
            } else {
                spoken_number(&parts, 0).is_some_and(|(_, n)| n == parts.len())
            };
        // Several digits said one by one win over the first one alone ("2 oh 4").
        let said_digits = spoken_number(&lw, k).filter(|(_, n)| *n >= 2);
        let (number, used) = if let Some(n) = said_digits {
            n
        } else if lw[k].chars().all(|c| c.is_ascii_digit()) {
            (lw[k].clone(), 1)
        } else if hyphened {
            let digits = if parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())) { parts.concat() } else { spoken_number(&parts, 0).map(|(d, _)| d).unwrap_or_default() };
            (digits, 1)
        } else if let Some(n) = spoken_number(&lw, k) {
            n
        } else {
            i += 1;
            continue;
        };
        let last = k + used - 1;
        // A letter said apart, right after it ("4 B", "four B").
        let letter = lw.get(last + 1).and_then(|_| unit_letter(&text[words[last + 1].0..words[last + 1].1])).filter(|_| {
            let gap = &text[words[last].1..words[last + 1].0];
            gap == " " || gap == "-"
        });
        let (end, form) = match letter {
            Some(c) => (words[last + 1].1, format!("{number}{c}")),
            None => (words[last].1, number),
        };
        if form != text[words[k].0..end] {
            edits.push((words[k].0, end, form));
        }
        i = last + 1 + usize::from(letter.is_some());
    }
    splice(text, &edits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_said_any_way_are_written_as_a_unit() {
        for (said, want) in [
            ("unit four B is ready", "unit 4B is ready"),
            ("Unit 4 B is ready", "Unit 4B is ready"),
            ("Unit 4B is ready", "Unit 4B is ready"),
            ("apartment two oh four", "apartment 204"),
            ("suite twenty two C.", "suite 22C."),
            ("unit one hundred four", "unit 104"),
            ("unit number seven", "unit 7"),
            ("Unit 2-0-4.", "Unit 204."),
            ("unit two-oh-four", "unit 204"),
            ("unit 2 oh 4 is next", "unit 204 is next"),
            ("unit 4 is next", "unit 4 is next"),
            ("unit 12 a lot of noise", "unit 12 a lot of noise"),
            ("the unit is ready", "the unit is ready"),
        ] {
            assert_eq!(dictation(said), want, "{said}");
        }
    }

    #[test]
    fn phones_said_or_written_any_way_are_grouped() {
        for (said, want) in [
            ("call me at five five five, one two three, four five six seven today", "call me at 555-123-4567 today"),
            ("call 555 123 4567 today", "call 555-123-4567 today"),
            ("call (555) 123-4567 today", "call 555-123-4567 today"),
            ("call 5551234567 today", "call 555-123-4567 today"),
            ("call 555-123-4567 today", "call 555-123-4567 today"),
            ("call one five five five one two three four five six seven", "call 1-555-123-4567"),
            ("dial five five five oh one two three", "dial 555-0123"),
            ("call +44 20 7946 0958", "call +44 20 7946 0958"),
            ("one two three go", "one two three go"),
        ] {
            assert_eq!(dictation(said), want, "{said}");
        }
    }
}
