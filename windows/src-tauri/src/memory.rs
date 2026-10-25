// Remembered rules and "about me" facts (docs/glim-learns-plan.md §3;
// decided 2026-10-24, §9 decision 2). "Remember: when I say the office I
// mean Outlook" on the Ask key → Claude's `remember` tool → Glim decides
// here, on the PC, whether to keep it: the card/SSN/IBAN scan plus a
// refusal list (health, money, IDs and numbers, tenant or customer details),
// facts of 8 words or fewer. Kept in %LOCALAPPDATA%\Glim\learn\memory.json.
//
// What goes to Claude: Ask-lane requests only (never dictation), every fact
// and only the rules relevant to the request (sharing a word with it, best
// first, at most 15, within 4,000 characters). That is a privacy change
// (CLAUDE.md, Settings → Memory). The log has kinds, codes and counts, never
// a rule or a fact.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// A rule's longest length, and how many are kept.
pub const MAX_RULE_CHARS: usize = 200;
pub const MAX_RULES: usize = 50;
/// A fact's most words, and how many are kept.
pub const MAX_FACT_WORDS: usize = 8;
pub const MAX_FACTS: usize = 30;
/// What a request may carry: at most this many rules, and characters in all.
pub const MAX_RULES_SENT: usize = 15;
pub const MAX_CHARS_SENT: usize = 4_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Rule,
    Fact,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Rule => "rule",
            Kind::Fact => "fact",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Memory {
    pub rules: Vec<String>,
    pub facts: Vec<String>,
}

/// Lower-case words: letters, digits, apostrophes.
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '\u{2019}')).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

const HEALTH: [&str; 30] = [
    "health", "doctor", "doctors", "medical", "medicine", "medication", "medications", "diagnosis", "diagnosed", "therapy",
    "therapist", "pregnant", "pregnancy", "illness", "disease", "cancer", "diabetes", "depression", "anxiety", "prescription",
    "hospital", "surgery", "disability", "allergy", "allergic", "symptom", "symptoms", "mental", "psychiatrist", "clinic",
];
const MONEY: [&str; 20] = [
    "salary", "salaries", "income", "paycheck", "wage", "wages", "debt", "debts", "owe", "owes", "owed", "loan", "loans",
    "mortgage", "bank", "account", "accounts", "credit", "dollars", "bucks",
];
const CUSTOMER: [&str; 10] = ["tenant", "tenants", "resident", "residents", "lease", "leases", "landlord", "customer", "customers", "renter"];
const UNIT_WORDS: [&str; 5] = ["unit", "apt", "apartment", "suite", "ste"];

/// Why `text` can't be kept as a `kind` (None: it can). Run by Glim, never
/// left to Claude (plan §3).
pub fn filter(kind: Kind, text: &str) -> Option<&'static str> {
    let text = text.trim();
    if text.is_empty() {
        return Some("empty");
    }
    if crate::capture::sensitive::contains_sensitive_number(text) {
        return Some("number");
    }
    let w = words(text);
    // IDs and numbers: a run of 4 or more digits, an email address, a phone
    // number (7 or more digits in all).
    let mut run = 0;
    let mut longest = 0;
    for c in text.chars() {
        run = if c.is_ascii_digit() { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let digits = text.chars().filter(char::is_ascii_digit).count();
    let email = text.split_whitespace().any(|t| t.find('@').is_some_and(|at| at > 0 && t[at + 1..].contains('.')));
    if longest >= 4 || digits >= 7 || email {
        return Some("number");
    }
    if w.iter().any(|x| HEALTH.contains(&x.as_str())) {
        return Some("health");
    }
    if text.chars().any(|c| matches!(c, '$' | '€' | '£' | '¥' | '₹')) || w.iter().any(|x| MONEY.contains(&x.as_str())) {
        return Some("money");
    }
    // A unit or apartment with a number next to it ("unit 7D", "apt 4").
    let unit = w.windows(2).any(|p| UNIT_WORDS.contains(&p[0].as_str()) && p[1].chars().any(|c| c.is_ascii_digit()));
    if unit || w.iter().any(|x| CUSTOMER.contains(&x.as_str())) {
        return Some("customer");
    }
    match kind {
        Kind::Fact if w.len() > MAX_FACT_WORDS => Some("fact-long"),
        Kind::Rule if text.chars().count() > MAX_RULE_CHARS => Some("rule-long"),
        _ => None,
    }
}

fn same(a: &str, b: &str) -> bool {
    words(a) == words(b)
}

impl Memory {
    /// Keeps `text` as a `kind`, or says why not.
    pub fn add(&mut self, kind: Kind, text: &str) -> Result<String, &'static str> {
        if let Some(why) = filter(kind, text) {
            return Err(why);
        }
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let (list, max, full) = match kind {
            Kind::Rule => (&mut self.rules, MAX_RULES, "rules-full"),
            Kind::Fact => (&mut self.facts, MAX_FACTS, "facts-full"),
        };
        if list.iter().any(|x| same(x, &text)) {
            return Err("duplicate");
        }
        if list.len() >= max {
            return Err(full);
        }
        list.push(text.clone());
        Ok(text)
    }

    pub fn remove(&mut self, kind: Kind, text: &str) -> bool {
        let list = match kind {
            Kind::Rule => &mut self.rules,
            Kind::Fact => &mut self.facts,
        };
        let before = list.len();
        list.retain(|x| x != text);
        list.len() != before
    }

    /// What one Ask request carries (plan §3, decision 2): every fact, and
    /// the rules sharing a word with `transcript`, most shared first (then
    /// the newest), at most 15, all within 4,000 characters. "" when there's
    /// nothing to send.
    pub fn for_request(&self, transcript: &str) -> String {
        const SMALL: [&str; 24] = [
            "a", "an", "the", "to", "my", "me", "i", "for", "of", "on", "in", "at", "and", "or", "is", "it", "be", "when", "say",
            "mean", "you", "glim", "please", "with",
        ];
        let said: Vec<String> = words(transcript).into_iter().filter(|w| !SMALL.contains(&w.as_str())).collect();
        let mut scored: Vec<(usize, usize, &String)> = self
            .rules
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let mut rw = words(r);
                rw.sort();
                rw.dedup();
                (rw.iter().filter(|w| said.contains(w)).count(), i, r)
            })
            .filter(|(n, _, _)| *n > 0)
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        let mut out = String::new();
        let mut budget = MAX_CHARS_SENT;
        let mut take = |line: &str, out: &mut String| {
            let line = format!("- {line}\n");
            if line.chars().count() > budget {
                return false;
            }
            budget -= line.chars().count();
            out.push_str(&line);
            true
        };
        let mut facts = String::new();
        for f in &self.facts {
            take(f, &mut facts);
        }
        let mut rules = String::new();
        for (_, _, r) in scored.into_iter().take(MAX_RULES_SENT) {
            take(r, &mut rules);
        }
        if !rules.is_empty() {
            out.push_str("The user's remembered rules (follow them when they apply):\n");
            out.push_str(&rules);
        }
        if !facts.is_empty() {
            out.push_str("About the user:\n");
            out.push_str(&facts);
        }
        out
    }

    /// The counts a request carried, for the log.
    pub fn counts(block: &str) -> (usize, usize) {
        let mut rules = 0;
        let mut facts = 0;
        let mut in_facts = false;
        for line in block.lines() {
            if line == "About the user:" {
                in_facts = true;
            } else if line.starts_with("- ") {
                if in_facts {
                    facts += 1;
                } else {
                    rules += 1;
                }
            }
        }
        (rules, facts)
    }
}

/// Whether the user is correcting Glim (plan §3's tip): the next Ask within
/// a minute starts with "no", "wrong", "not that" or "I meant".
pub fn is_correction(transcript: &str) -> bool {
    let w = words(transcript);
    let w: Vec<&str> = w.iter().map(String::as_str).skip_while(|x| matches!(*x, "um" | "uh" | "hey" | "glim" | "oh")).collect();
    matches!(w.first(), Some(&"no") | Some(&"nope") | Some(&"wrong")) || w.starts_with(&["not", "that"]) || w.starts_with(&["i", "meant"])
}

// ── The file ──────────────────────────────────────────────────────────────────

pub const FILE: &str = "memory.json";

/// %LOCALAPPDATA%\Glim\learn\memory.json
pub fn path() -> PathBuf {
    crate::platform::local_dir().join("learn").join(FILE)
}

/// The file's contents, or empty when it is missing; one that doesn't parse
/// is set aside as memory.json.bad (never logged) and counts as empty.
pub fn load(path: &Path) -> Memory {
    let Ok(bytes) = std::fs::read(path) else { return Memory::default() };
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Memory::default();
    }
    match serde_json::from_slice(&bytes) {
        Ok(m) => m,
        Err(_) => {
            let bad = path.with_extension("json.bad");
            let _ = std::fs::remove_file(&bad);
            let _ = std::fs::rename(path, &bad);
            crate::log::line("memory: the file didn't parse, set aside (counts as empty)");
            Memory::default()
        }
    }
}

/// A temp file beside it, then a rename: a crash never leaves half a file.
pub fn save(path: &Path, m: &Memory) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(m).map_err(std::io::Error::other)?;
    let temp = path.with_extension(format!("json.tmp-{}", std::process::id()));
    if let Err(err) = std::fs::write(&temp, bytes).and_then(|_| std::fs::rename(&temp, path)) {
        let _ = std::fs::remove_file(&temp);
        return Err(err);
    }
    Ok(())
}

static STATE: Mutex<Option<Memory>> = Mutex::new(None);

/// Runs `f` on what is remembered (loaded the first time); `changed` true
/// from `f` saves it.
pub fn with<R>(f: impl FnOnce(&mut Memory) -> (R, bool)) -> R {
    let mut state = STATE.lock().unwrap();
    let m = state.get_or_insert_with(|| {
        let m = load(&path());
        crate::log::line(format!("memory: {} rules, {} facts", m.rules.len(), m.facts.len()));
        m
    });
    let (r, changed) = f(m);
    if changed {
        if let Err(err) = save(&path(), m) {
            crate::log::line(format!("memory: could not save ({})", err.kind()));
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_and_short_facts_are_kept() {
        let mut m = Memory::default();
        assert_eq!(m.add(Kind::Rule, "  When I say the office   I mean Outlook "), Ok("When I say the office I mean Outlook".into()));
        assert_eq!(m.add(Kind::Fact, "Prefers short replies"), Ok("Prefers short replies".into()));
        assert_eq!(m.add(Kind::Fact, "Likes meetings after ten"), Ok("Likes meetings after ten".into()));
        assert_eq!(m.add(Kind::Rule, "when i say THE OFFICE i mean outlook"), Err("duplicate"));
        assert_eq!(m.rules.len(), 1);
        assert_eq!(m.facts.len(), 2);
        assert!(m.remove(Kind::Fact, "Prefers short replies"));
        assert!(!m.remove(Kind::Fact, "Prefers short replies"));
    }

    #[test]
    fn the_filter_refuses_each_category_and_says_which() {
        for (text, why) in [
            ("My card is 4111 1111 1111 1111", "number"),
            ("My gate code is 4286", "number"),
            ("Call me at 555 123 4567", "number"),
            ("Email jordan@example.com for invoices", "number"),
            ("I'm allergic to peanuts", "health"),
            ("My doctor is on Tuesdays", "health"),
            ("Takes medication at noon", "health"),
            ("My salary goes in on Fridays", "money"),
            ("I owe the plumber money", "money"),
            ("Rent is $1,850", "money"),
            ("The tenant in 7D is late", "customer"),
            ("Unit 9F gets the new lease", "customer"),
            ("Jordan lives in apt 4", "customer"),
            ("Residents get the newsletter", "customer"),
            ("", "empty"),
        ] {
            assert_eq!(filter(Kind::Rule, text), Some(why), "{text}");
        }
        assert_eq!(filter(Kind::Fact, "Prefers replies that are short and to the point"), Some("fact-long"), "9 words");
        assert_eq!(filter(Kind::Fact, "Prefers replies that are short, clear and direct"), None, "8 words");
        assert_eq!(filter(Kind::Rule, &"word ".repeat(41)), Some("rule-long"));
        // Everyday rules get through, numbers under four digits too.
        for ok in ["When I say the office I mean Outlook", "Open Codex after Outlook", "Meetings after 10 are fine", "Say my-repo for the main project"] {
            assert_eq!(filter(Kind::Rule, ok), None, "{ok}");
        }
    }

    #[test]
    fn the_lists_have_limits() {
        let mut m = Memory::default();
        for i in 0..MAX_RULES {
            m.add(Kind::Rule, &format!("rule number {}", "x".repeat(i + 1))).unwrap();
        }
        assert_eq!(m.add(Kind::Rule, "one more rule"), Err("rules-full"));
        for i in 0..MAX_FACTS {
            m.add(Kind::Fact, &format!("fact {}", "y".repeat(i + 1))).unwrap();
        }
        assert_eq!(m.add(Kind::Fact, "one more fact"), Err("facts-full"));
    }

    #[test]
    fn a_request_carries_every_fact_and_only_the_rules_it_shares_words_with() {
        let mut m = Memory::default();
        m.add(Kind::Rule, "When I say the office I mean Outlook").unwrap();
        m.add(Kind::Rule, "Open Codex after Outlook").unwrap();
        m.add(Kind::Rule, "Say my-repo for the main project").unwrap();
        m.add(Kind::Fact, "Prefers short replies").unwrap();
        let block = m.for_request("open the office");
        // Equally relevant: the newest first.
        assert_eq!(block, "The user's remembered rules (follow them when they apply):\n- Open Codex after Outlook\n- When I say the office I mean Outlook\nAbout the user:\n- Prefers short replies\n");
        assert_eq!(Memory::counts(&block), (2, 1));
        // Nothing shared: only the facts.
        assert_eq!(m.for_request("where was I"), "About the user:\n- Prefers short replies\n");
        // Words like "the", "say", "mean" don't make a rule relevant.
        assert!(!m.for_request("say the thing I mean").contains("rules"));
        assert_eq!(Memory::default().for_request("open the office"), "");
    }

    #[test]
    fn a_full_memory_stays_within_its_limits_so_every_ask_stays_cheap() {
        let mut m = Memory::default();
        for i in 0..MAX_RULES {
            let rule = format!("office rule {i} {}", "w".repeat(MAX_RULE_CHARS - 20));
            m.rules.push(rule.chars().take(MAX_RULE_CHARS).collect());
        }
        for i in 0..MAX_FACTS {
            m.facts.push(format!("fact {i} {}", "z".repeat(40)));
        }
        let block = m.for_request("open the office");
        let (rules, facts) = Memory::counts(&block);
        assert!(rules <= MAX_RULES_SENT, "{rules}");
        assert_eq!(facts, MAX_FACTS);
        assert!(block.chars().count() <= MAX_CHARS_SENT + 120, "{}", block.chars().count());
        // About 4 characters a token: well under 1,500 more input tokens
        // (an Ask is ~600 today; ai::budget flags a median over $0.001).
        assert!(block.chars().count() / 4 < 1_500);
        // The newest of the equally relevant rules come first.
        assert!(block.contains(&format!("office rule {} ", MAX_RULES - 1)));
    }

    #[test]
    fn a_correction_is_no_wrong_not_that_or_i_meant() {
        for yes in ["No, Codex", "nope, the other one", "Wrong app", "not that, Outlook", "I meant Codex", "um no I said Codex"] {
            assert!(is_correction(yes), "{yes}");
        }
        for no in ["notes app", "open Outlook", "know what, open Codex", "nothing", "I mean it", "wrongly named"] {
            assert!(!is_correction(no), "{no}");
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("glim-memory-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(FILE)
    }

    #[test]
    fn the_file_round_trips_and_a_corrupt_one_is_set_aside() {
        let file = scratch("file");
        let mut m = Memory::default();
        m.add(Kind::Rule, "Open Codex after Outlook").unwrap();
        m.add(Kind::Fact, "Prefers short replies").unwrap();
        save(&file, &m).unwrap();
        assert_eq!(load(&file), m);
        std::fs::write(&file, b"{\"rules\":[\"Open").unwrap();
        assert_eq!(load(&file), Memory::default());
        assert!(file.with_extension("json.bad").exists());
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn the_log_never_carries_a_rule_or_a_fact() {
        // Every "memory:" line in this file and in voice/run.rs, as written:
        // counts, kinds and codes only.
        let allowed = ["{}", "{rules}", "{facts}", "{kind}", "{code}"];
        for (file, src) in [("memory.rs", include_str!("memory.rs")), ("voice/run.rs", include_str!("voice/run.rs"))] {
            let src = src.split("#[cfg(test)]").next().unwrap();
            for (at, _) in src.match_indices("\"memory: ") {
                let line = &src[at..at + src[at + 1..].find('"').unwrap() + 2];
                let mut rest = line;
                while let Some(open) = rest.find('{') {
                    let close = rest[open..].find('}').map(|c| open + c + 1).unwrap_or(rest.len());
                    assert!(allowed.contains(&&rest[open..close]), "{file}: {line}");
                    rest = &rest[close..];
                }
            }
        }
    }
}
