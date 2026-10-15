// "Pick up from yesterday" (Phase 1.5, docs/voice-actions-plan.md §4,
// `resume_work`): the data-gathering half only. Finds the owner's repos,
// reads each one's HANDOFF.md and pulls out where it stopped: Goal, Branch +
// last commit, In progress, Open questions.
//
// No hotkey, no network, nothing written. The island shows it as a card
// (`cards`); a click on a repo's row opens Claude Code there (`open`), never
// without that click (owner, 2026-10-13). The voice action (1.5c) will call
// the same.
//
// HANDOFF.md is the owner's own notes, so it is treated like captured text:
// - its contents are never logged (`summary_line` gives repo names and counts);
// - a file over MAX_BYTES is not read at all;
// - a file holding anything that looks like a secret (an API key, a token, a
//   private key, `password=…`) is skipped whole: none of it is kept;
// - a section holding a card / SSN / IBAN number or a long digit run (the
//   capture layer's `contains_sensitive_number`) is withheld: replaced by
//   WITHHELD and named in `Handoff::withheld`; the other sections are kept
//   (owner, 2026-10-13: a migration timestamp mustn't hide a whole repo).

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::capture::sensitive::contains_sensitive_number;

/// The file read in each repo.
pub const FILE_NAME: &str = "HANDOFF.md";
/// Larger files are skipped unread (the owner's are ~4 KB).
pub const MAX_BYTES: u64 = 64 * 1024;
/// Each section is cut to this many characters.
pub const MAX_SECTION_CHARS: usize = 2000;

/// Where one repo stopped, as its HANDOFF.md says. Every field is optional:
/// HANDOFFs are hand-written and many leave sections out.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Handoff {
    /// The repo's folder name.
    pub repo: String,
    pub path: PathBuf,
    pub goal: Option<String>,
    /// The "Branch + last commit" section as written.
    pub branch_section: Option<String>,
    /// Best effort from that section: a `Branch:` value or the first `code` span.
    pub branch: Option<String>,
    /// Best effort: the first 7–40 character hex word in that section.
    pub last_commit: Option<String>,
    pub in_progress: Option<String>,
    pub open_questions: Option<String>,
    /// The sections replaced by WITHHELD, in file order of the four fields.
    pub withheld: Vec<&'static str>,
}

/// What a withheld section reads as. Nothing of the original is kept.
pub const WITHHELD: &str = "[withheld: holds a card, ID or bank number]";

impl Handoff {
    /// None of the four sections was found.
    pub fn is_empty(&self) -> bool {
        self.goal.is_none() && self.branch_section.is_none() && self.in_progress.is_none() && self.open_questions.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    TooLarge,
    /// A key, token, private key or password: skipped whole.
    Secret,
    /// Not UTF-8, or it couldn't be opened.
    Unreadable,
}

impl Skip {
    fn label(self) -> &'static str {
        match self {
            Skip::TooLarge => "too large",
            Skip::Secret => "secret",
            Skip::Unreadable => "unreadable",
        }
    }
}

#[derive(Debug, Default)]
pub struct Scan {
    pub read: Vec<Handoff>,
    /// Repo folder name and why.
    pub skipped: Vec<(String, Skip)>,
}

/// The repos to read. A setting will hold an explicit list; until then (and
/// when it's empty) every folder directly under the home folder that holds a
/// HANDOFF.md.
pub fn repos(configured: &[PathBuf]) -> Vec<PathBuf> {
    if configured.is_empty() { discover(&crate::platform::home_dir()) } else { configured.to_vec() }
}

/// Folders directly under `home` holding a HANDOFF.md, by name. Read-only.
pub fn discover(home: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(home)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .filter(|p| p.join(FILE_NAME).is_file())
        .collect();
    found.sort();
    found
}

/// Reads and parses each repo's HANDOFF.md. A repo without one is left out.
pub fn read_all(repos: &[PathBuf]) -> Scan {
    let mut scan = Scan::default();
    for repo in repos {
        let name = repo.file_name().map_or_else(|| repo.to_string_lossy().to_string(), |n| n.to_string_lossy().to_string());
        let path = repo.join(FILE_NAME);
        if !path.is_file() {
            continue;
        }
        match read_one(&path) {
            Ok(text) => {
                let mut h = parse(&text);
                h.repo = name;
                h.path = path;
                scan.read.push(h);
            }
            Err(why) => scan.skipped.push((name, why)),
        }
    }
    scan
}

fn read_one(path: &Path) -> Result<String, Skip> {
    let len = std::fs::metadata(path).map_err(|_| Skip::Unreadable)?.len();
    if len > MAX_BYTES {
        return Err(Skip::TooLarge);
    }
    // Capped again while reading, in case the file grew since.
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|_| Skip::Unreadable)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(Skip::TooLarge);
    }
    let text = String::from_utf8(bytes).map_err(|_| Skip::Unreadable)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text).to_string();
    // Secrets anywhere in the file, kept sections or not: skip all of it.
    if looks_like_secret(&text) {
        return Err(Skip::Secret);
    }
    Ok(text)
}

/// One log line: repo names and counts, never anything from the files.
pub fn summary_line(scan: &Scan) -> String {
    let read: Vec<String> = scan
        .read
        .iter()
        .map(|h| match h.withheld.len() {
            0 => h.repo.clone(),
            n => format!("{} ({n} withheld)", h.repo),
        })
        .collect();
    let skipped: Vec<String> = scan.skipped.iter().map(|(n, why)| format!("{n} ({})", why.label())).collect();
    format!(
        "resume: {} read [{}], {} skipped [{}]",
        read.len(),
        read.join(", "),
        skipped.len(),
        skipped.join(", ")
    )
}

// ── Parsing ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Section {
    Goal,
    Branch,
    InProgress,
    OpenQuestions,
    Other,
}

/// Which section a heading or label names. Tolerant of the wordings in use:
/// "Branch + last commit", "In progress", "Open questions / decisions
/// waiting on me", any case.
fn section_of(label: &str) -> Section {
    let l = label.trim().trim_matches(|c: char| c == '*' || c == '_' || c == ':').trim().to_lowercase();
    if l.starts_with("goal") {
        Section::Goal
    } else if l.starts_with("branch") {
        Section::Branch
    } else if l.starts_with("in progress") || l.starts_with("in-progress") {
        Section::InProgress
    } else if l.starts_with("open question") || l.starts_with("questions") || l.starts_with("decisions waiting") {
        Section::OpenQuestions
    } else {
        Section::Other
    }
}

/// A line that starts a section: a Markdown heading (`## Goal`), or a label
/// at the start of a line or bullet (`Goal: …`, `- **In progress:** …`).
/// Returns the section and any text on the same line after the label.
fn section_start(line: &str) -> Option<(Section, String)> {
    let t = line.trim();
    if let Some(h) = t.strip_prefix('#') {
        let h = h.trim_start_matches('#');
        if h.starts_with(' ') {
            return Some((section_of(h), String::new()));
        }
        return None;
    }
    let body = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).unwrap_or(t);
    let plain = body.trim_start_matches('*');
    let colon = plain.find(':')?;
    let label = &plain[..colon];
    // A label is short and is one of ours; "Note: the goal is…" isn't.
    if label.len() > 60 {
        return None;
    }
    let section = section_of(label);
    if section == Section::Other {
        return None;
    }
    let rest = plain[colon + 1..].trim_start_matches('*').trim().to_string();
    Some((section, rest))
}

/// The four sections of a HANDOFF.md. Missing ones are None; a section given
/// twice keeps the first.
pub fn parse(text: &str) -> Handoff {
    let mut current = Section::Other;
    let mut parts: [(Section, Vec<String>, bool); 4] = [
        (Section::Goal, Vec::new(), false),
        (Section::Branch, Vec::new(), false),
        (Section::InProgress, Vec::new(), false),
        (Section::OpenQuestions, Vec::new(), false),
    ];
    // Inside a section opened by a heading, only another heading ends it:
    // "Next: …" or "Branch: …" lines there are its content. Labels open and
    // close sections everywhere else.
    let mut by_heading = false;
    for line in text.lines() {
        let is_heading = line.trim_start().starts_with('#');
        let labels_count = !(by_heading && current != Section::Other);
        let start = if is_heading || labels_count { section_start(line) } else { None };
        if let Some((section, rest)) = start {
            current = section;
            by_heading = is_heading;
            if let Some(slot) = parts.iter_mut().find(|p| p.0 == section) {
                if slot.2 {
                    current = Section::Other; // a repeat: ignored
                    continue;
                }
                slot.2 = true;
                if !rest.is_empty() {
                    slot.1.push(rest);
                }
            }
            continue;
        }
        // A heading we don't use ends the section; so does a top-level label
        // line we don't use ("Done:", "Gotchas:") when labels count here.
        if is_heading {
            current = Section::Other;
            by_heading = true;
            continue;
        }
        if labels_count && current != Section::Other && is_other_label(line) {
            current = Section::Other;
            continue;
        }
        if let Some(slot) = parts.iter_mut().find(|p| p.0 == current) {
            slot.1.push(line.trim_end().to_string());
        }
    }
    // A section with a card-like number is withheld whole, checked before
    // the length cut so a number straddling the cut is still caught.
    let mut withheld = Vec::new();
    let mut take = |s: Section, name: &'static str| -> Option<String> {
        let text = parts.iter().find(|p| p.0 == s).and_then(|p| joined(&p.1))?;
        if contains_sensitive_number(&text) {
            withheld.push(name);
            return Some(WITHHELD.to_string());
        }
        Some(cut(text))
    };
    let goal = take(Section::Goal, "goal");
    let branch_section = take(Section::Branch, "branch");
    let in_progress = take(Section::InProgress, "in progress");
    let open_questions = take(Section::OpenQuestions, "open questions");
    let readable_branch = branch_section.as_deref().filter(|s| *s != WITHHELD);
    Handoff {
        goal,
        branch: readable_branch.and_then(branch_name),
        last_commit: readable_branch.and_then(commit_hash),
        branch_section,
        in_progress,
        open_questions,
        withheld,
        ..Handoff::default()
    }
}

/// An unindented `Label:` line for a section we don't keep (Done, Gotchas).
fn is_other_label(line: &str) -> bool {
    if line.starts_with(' ') || line.starts_with('\t') {
        return false;
    }
    let t = line.trim().trim_start_matches("- ").trim_start_matches('*');
    match t.find(':') {
        Some(i) if i > 0 && i <= 30 => {
            let label = &t[..i];
            label.chars().all(|c| c.is_alphanumeric() || " +/-*".contains(c)) && section_of(label) == Section::Other
        }
        _ => false,
    }
}

fn joined(lines: &[String]) -> Option<String> {
    let text = lines.join("\n");
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn cut(text: String) -> String {
    match text.char_indices().nth(MAX_SECTION_CHARS) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text,
    }
}

fn branch_name(section: &str) -> Option<String> {
    for line in section.lines() {
        let t = line.trim().trim_start_matches("- ");
        if let Some(i) = t.to_lowercase().find("branch:") {
            let v = t[i + 7..].trim().trim_matches(|c: char| c == '`' || c == '*').split_whitespace().next()?;
            return Some(v.trim_matches('`').to_string());
        }
    }
    let start = section.find('`')? + 1;
    let len = section[start..].find('`')?;
    let v = section[start..start + len].trim();
    (!v.is_empty() && !v.contains(' ')).then(|| v.to_string())
}

fn commit_hash(section: &str) -> Option<String> {
    section
        .split(|c: char| !c.is_ascii_alphanumeric())
        .find(|w| (7..=40).contains(&w.len()) && w.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()) && w.chars().any(|c| c.is_ascii_digit()))
        .map(str::to_string)
}

// ── Secrets ───────────────────────────────────────────────────────────────────

/// Prefixes of API keys and tokens, each followed by at least
/// TOKEN_TAIL token characters.
const TOKEN_PREFIXES: &[&str] = &[
    "sk-ant-", "sk-proj-", "sk-", "sk_live_", "sk_test_", "rk_live_", "rk_test_", "whsec_", "ghp_", "gho_", "ghs_", "ghu_",
    "github_pat_", "glpat-", "xoxb-", "xoxp-", "xoxa-", "AKIA", "ASIA", "AIza", "sbp_", "re_", "eyJ",
];
const TOKEN_TAIL: usize = 16;
/// Words that, followed by `=` or `:` and a value, are a credential.
const SECRET_KEYS: &[&str] = &["password", "passwd", "secret", "api_key", "apikey", "api-key", "access_token", "auth_token", "token", "private_key"];

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Whether text holds something shaped like a credential: a PEM private key,
/// a known key or token prefix with a long tail, `password=…` style
/// assignments, or a long mixed-case, digit-bearing blob. Commit hashes
/// (lowercase hex) and UUIDs don't match.
pub fn looks_like_secret(text: &str) -> bool {
    if text.contains("-----BEGIN") && text.contains("PRIVATE KEY") {
        return true;
    }
    let words: Vec<&str> = text.split(|c: char| !is_token_char(c) && c != '.').filter(|w| !w.is_empty()).collect();
    for w in &words {
        for p in TOKEN_PREFIXES {
            if let Some(tail) = w.strip_prefix(p) {
                if tail.chars().filter(|c| is_token_char(*c) || *c == '.').count() >= TOKEN_TAIL {
                    return true;
                }
            }
        }
        if high_entropy(w) {
            return true;
        }
    }
    assignment(text)
}

/// 32+ token characters with upper, lower and digits all present.
fn high_entropy(w: &str) -> bool {
    w.len() >= 32
        && w.chars().all(|c| is_token_char(c) || c == '.')
        && w.chars().any(|c| c.is_ascii_uppercase())
        && w.chars().any(|c| c.is_ascii_lowercase())
        && w.chars().any(|c| c.is_ascii_digit())
}

/// `password = hunter2hunter`, `API_KEY: abcd…`, `SOME_TOKEN=…`: a secret word
/// ending a key, then `=` or `:`, then 8+ non-space characters.
fn assignment(text: &str) -> bool {
    let lower = text.to_lowercase();
    for key in SECRET_KEYS {
        let mut from = 0;
        while let Some(i) = lower[from..].find(key) {
            let end = from + i + key.len();
            from = end;
            // The key must end a word ("my_token" yes, "tokens" no).
            if lower[end..].chars().next().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                continue;
            }
            let rest = lower[end..].trim_start_matches([' ', '"', '\'']);
            let Some(rest) = rest.strip_prefix('=').or_else(|| rest.strip_prefix(':')) else { continue };
            let value: String = rest.trim_start_matches([' ', '"', '\'']).chars().take_while(|c| !c.is_whitespace() && *c != '"' && *c != '\'').collect();
            if value.chars().count() >= 8 {
                return true;
            }
        }
    }
    false
}

/// One repo's row on the island's card: where it stopped, short. Shown to
/// the user (it's their own notes); never logged.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub repo: String,
    pub goal: Option<String>,
    pub in_progress: Option<String>,
    pub branch: Option<String>,
    pub last_commit: Option<String>,
    pub has_questions: bool,
    /// Sections withheld for a card-like number (their names only).
    pub withheld: Vec<&'static str>,
    /// When HANDOFF.md last changed (seconds since 1970): newest first.
    pub updated: u64,
}

/// Each section is cut to this many characters on the card.
pub const CARD_CHARS: usize = 240;

fn short(s: &Option<String>) -> Option<String> {
    s.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(|t| {
        let cut: String = t.chars().take(CARD_CHARS).collect();
        if t.chars().count() > CARD_CHARS { format!("{}…", cut.trim_end()) } else { cut }
    })
}

/// The card's rows, the most recently updated HANDOFF.md first ("yesterday's"
/// repo on top). A HANDOFF.md with nothing in it is left out.
pub fn cards(scan: &Scan) -> Vec<Card> {
    let mut out: Vec<Card> = scan
        .read
        .iter()
        .filter(|h| !h.is_empty())
        .map(|h| Card {
            repo: h.repo.clone(),
            goal: short(&h.goal),
            in_progress: short(&h.in_progress),
            branch: h.branch.clone(),
            last_commit: h.last_commit.clone(),
            has_questions: h.open_questions.as_deref().is_some_and(|q| !q.trim().is_empty()),
            withheld: h.withheld.clone(),
            updated: std::fs::metadata(&h.path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs()),
        })
        .collect();
    out.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| a.repo.cmp(&b.repo)));
    out
}

/// What Claude Code is told when it opens in a repo (plan §4, `resume_work`).
pub const PROMPT: &str = "Read HANDOFF.md first, check it against git status, and continue from In progress.";

/// The repo folder for a name the island sent: only one of `found` (the
/// repos Glim reads), matched by its folder name exactly. Never a path from
/// the island.
pub fn folder_for(name: &str, found: &[PathBuf]) -> Option<PathBuf> {
    if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
        return None;
    }
    found.iter().find(|p| p.file_name().is_some_and(|n| n == name) && p.join(FILE_NAME).is_file()).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMPLETE: &str = include_str!("fixtures/complete.md");
    const PARTIAL: &str = include_str!("fixtures/partial.md");
    const LABELS: &str = include_str!("fixtures/labels.md");
    const SENSITIVE_CARD: &str = include_str!("fixtures/sensitive-card.md");

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("glim-resume-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn repo(home: &Path, name: &str, handoff: Option<&[u8]>) -> PathBuf {
        let dir = home.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(bytes) = handoff {
            std::fs::write(dir.join(FILE_NAME), bytes).unwrap();
        }
        dir
    }

    #[test]
    fn a_complete_handoff_gives_every_section() {
        let h = parse(COMPLETE);
        assert_eq!(h.goal.as_deref(), Some("Ship the tenant portal's maintenance-request form."));
        assert_eq!(h.branch.as_deref(), Some("feature/maintenance-form"));
        assert_eq!(h.last_commit.as_deref(), Some("3f9c2ab"));
        assert!(h.branch_section.as_deref().unwrap().contains("Add photo upload"));
        let p = h.in_progress.unwrap();
        assert!(p.starts_with("Wiring the photo upload"), "{p}");
        assert!(p.contains("Next: run the e2e test"), "multi-line kept: {p}");
        assert!(!p.contains("Gotchas") && !p.contains("Done"), "stops at the next heading: {p}");
        assert!(h.open_questions.unwrap().contains("Max photo size"));
    }

    #[test]
    fn missing_sections_are_none_and_other_headings_are_ignored() {
        let h = parse(PARTIAL);
        assert_eq!(h.goal.as_deref(), Some("Two PRs open, both waiting for the owner's live test."));
        assert_eq!(h.branch_section, None);
        assert_eq!(h.branch, None);
        assert_eq!(h.last_commit, None);
        assert_eq!(h.in_progress, None);
        assert_eq!(h.open_questions, None);
        assert!(!h.is_empty());
    }

    #[test]
    fn the_global_template_written_as_labels_parses_too() {
        let h = parse(LABELS);
        assert_eq!(h.goal.as_deref(), Some("Fix the SMS reminder timing."));
        assert_eq!(h.branch.as_deref(), Some("fix/sms-timing"));
        assert_eq!(h.last_commit.as_deref(), Some("a1b2c3d4e5"));
        assert_eq!(h.in_progress.as_deref(), Some("Writing the migration; next: apply it to staging."));
        assert_eq!(h.open_questions.as_deref(), Some("Keep the 2h reminder for free shops?"));
    }

    #[test]
    fn empty_and_heading_only_files_parse_to_nothing() {
        assert!(parse("").is_empty());
        assert!(parse("# HANDOFF\n\n## Goal\n\n## In progress\n   \n").is_empty());
        assert!(parse("\u{feff}just some notes\nwith no sections\n").is_empty());
    }

    #[test]
    fn a_long_section_is_cut() {
        let text = format!("## Goal\n{}\n", "x".repeat(MAX_SECTION_CHARS + 500));
        let g = parse(&text).goal.unwrap();
        assert_eq!(g.chars().count(), MAX_SECTION_CHARS + 1);
        assert!(g.ends_with('…'));
        // Cut on a character boundary, not a byte.
        let text = format!("## Goal\n{}\n", "é".repeat(MAX_SECTION_CHARS + 5));
        assert!(parse(&text).goal.unwrap().ends_with('…'));
    }

    #[test]
    fn a_repeated_section_keeps_the_first() {
        let h = parse("## Goal\nfirst\n## Goal\nsecond\n");
        assert_eq!(h.goal.as_deref(), Some("first"));
    }

    #[test]
    fn reading_skips_oversized_secret_and_unreadable_files_and_keeps_the_rest() {
        let home = scratch("read");
        let big = format!("## Goal\nbig\n{}", "a".repeat(MAX_BYTES as usize + 1));
        // Secret-shaped fixtures are built here, not committed, so the repo
        // holds no string a secret scanner would flag.
        let key = format!("## Goal\nship\n## Gotchas\nkey is {}{}\n", "sk-ant-api03-", "Zx9".repeat(10));
        let password = "## Goal\nship\nadmin password: correct-horse-battery\n";
        let pem = format!("## Goal\nx\n-----BEGIN {} KEY-----\nabc\n", "RSA PRIVATE");
        // A card number AND a secret: the secret wins, nothing is kept.
        let both = format!("{SENSITIVE_CARD}\n## Open questions\ntoken={}{}\n", "ghp_", "a1B2".repeat(9));
        let repos = vec![
            repo(&home, "complete", Some(COMPLETE.as_bytes())),
            repo(&home, "empty", Some(b"")),
            repo(&home, "card", Some(SENSITIVE_CARD.as_bytes())),
            repo(&home, "oversized", Some(big.as_bytes())),
            repo(&home, "key", Some(key.as_bytes())),
            repo(&home, "password", Some(password.as_bytes())),
            repo(&home, "pem", Some(pem.as_bytes())),
            repo(&home, "card-and-secret", Some(both.as_bytes())),
            repo(&home, "latin1", Some(b"## Goal\ncaf\xe9\n")),
            repo(&home, "no-handoff", None),
        ];
        let scan = read_all(&repos);
        assert_eq!(scan.read.iter().map(|h| h.repo.as_str()).collect::<Vec<_>>(), ["complete", "empty", "card"]);
        assert!(scan.read[1].is_empty());
        assert_eq!(scan.read[0].path, home.join("complete").join(FILE_NAME));
        assert_eq!(
            scan.skipped,
            [
                ("oversized".to_string(), Skip::TooLarge),
                ("key".to_string(), Skip::Secret),
                ("password".to_string(), Skip::Secret),
                ("pem".to_string(), Skip::Secret),
                ("card-and-secret".to_string(), Skip::Secret),
                ("latin1".to_string(), Skip::Unreadable),
            ]
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_card_number_withholds_only_its_section() {
        let h = parse(SENSITIVE_CARD);
        assert_eq!(h.goal.as_deref(), Some("Refund test for unit 4B."), "the other sections are kept");
        assert_eq!(h.in_progress.as_deref(), Some(WITHHELD));
        assert_eq!(h.withheld, ["in progress"]);
        assert!(!format!("{h:?}").contains("4111"), "nothing of the number is kept anywhere");
        assert!(!format!("{h:?}").contains("Visa"), "nor the rest of its section");
        assert!(!h.is_empty());
    }

    #[test]
    fn a_migration_timestamp_withholds_its_section_not_the_repo() {
        // The false positive that decided it (owner, 2026-10-13): a 14-digit
        // migration name reads as a card being typed.
        let h = parse(include_str!("fixtures/migration.md"));
        assert_eq!(h.goal.as_deref(), Some("Referral tracking for the VA."));
        assert_eq!(h.branch.as_deref(), Some("feat/referral-codes"));
        assert_eq!(h.last_commit.as_deref(), Some("9e1d0c4"));
        assert_eq!(h.in_progress.as_deref(), Some(WITHHELD));
        assert!(h.open_questions.unwrap().contains("JANE code"));
        assert_eq!(h.withheld, ["in progress"]);
    }

    #[test]
    fn a_withheld_branch_section_yields_no_branch_or_commit() {
        let h = parse("## Branch + last commit\n`fix/x` at 3f9c2ab, card 4111 1111 1111 1111\n## Goal\nok\n");
        assert_eq!(h.branch_section.as_deref(), Some(WITHHELD));
        assert_eq!((h.branch, h.last_commit), (None, None));
        assert_eq!(h.withheld, ["branch"]);
        assert_eq!(h.goal.as_deref(), Some("ok"));
    }

    #[test]
    fn a_number_past_the_length_cut_still_withholds_the_section() {
        let text = format!("## In progress\n{} 4111 1111 1111 1111\n", "x".repeat(MAX_SECTION_CHARS + 10));
        assert_eq!(parse(&text).in_progress.as_deref(), Some(WITHHELD));
    }

    #[test]
    fn numbers_outside_the_four_sections_are_simply_not_kept() {
        let h = parse("## Goal\nship\n## Gotchas\ntest card 4111 1111 1111 1111\n");
        assert_eq!(h.goal.as_deref(), Some("ship"));
        assert!(h.withheld.is_empty());
        assert!(!format!("{h:?}").contains("4111"));
    }

    #[test]
    fn the_log_line_names_repos_and_counts_and_nothing_from_the_files() {
        let home = scratch("log");
        let key = format!("## Goal\n{}{}\n", "sk-ant-api03-", "Zx9".repeat(10));
        let repos = vec![
            repo(&home, "complete", Some(COMPLETE.as_bytes())),
            repo(&home, "card", Some(SENSITIVE_CARD.as_bytes())),
            repo(&home, "key", Some(key.as_bytes())),
        ];
        let scan = read_all(&repos);
        let line = summary_line(&scan);
        assert_eq!(line, "resume: 2 read [complete, card (1 withheld)], 1 skipped [key (secret)]");
        for leaked in ["maintenance", "3f9c2ab", "photo", "4111", "Max photo size", "Refund", "sk-ant", "in progress"] {
            assert!(!line.contains(leaked), "{leaked} in {line}");
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn discovery_finds_folders_holding_a_handoff_only() {
        let home = scratch("discover");
        repo(&home, "b-repo", Some(b"## Goal\nb\n"));
        repo(&home, "a-repo", Some(b"## Goal\na\n"));
        repo(&home, "no-handoff", None);
        std::fs::write(home.join(FILE_NAME), b"## Goal\nhome itself\n").unwrap(); // not a repo
        std::fs::create_dir_all(home.join("dir-named").join(FILE_NAME)).unwrap(); // a folder, not a file
        assert_eq!(discover(&home), [home.join("a-repo"), home.join("b-repo")]);
        // An explicit list wins over discovery.
        assert_eq!(repos(&[home.join("b-repo")]), [home.join("b-repo")]);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn secrets_are_recognised_and_ordinary_handoff_text_is_not() {
        for s in [
            format!("{}{}", "ghp_", "a1B2".repeat(9)),
            format!("{}{}", "AKIA", "IOSFODNN7EXAMPLE1"),
            format!("STRIPE_SECRET_KEY={}{}", "sk_live_", "4eC39HqLyjWDarjtT1zdp7dc"),
            format!("{}.{}.{}", "eyJhbGciOiJIUzI1NiJ9", "eyJzdWIiOiIxMjM0NTY3ODkwIn0", "dozjgNryP4J3jVmNHl0w5N"),
            "SUPABASE_ACCESS_TOKEN = abcdef0123456789".to_string(),
            "api_key: \"Qw3rTy-Uiop-Asdf\"".to_string(),
            format!("blob {}", "Ab3".repeat(12)),
        ] {
            assert!(looks_like_secret(&s), "{s}");
        }
        for s in [
            include_str!("fixtures/complete.md"),
            "Stripe key in .dev.vars (never here)",
            "commit 3f9c2ab1d4e5f60718293a4b5c6d7e8f90a1b2c3, uuid 123e4567-e89b-12d3-a456-426614174000",
            "rotate the token: see the runbook",
            "password handoff to the user if logged out",
            "tokens: 218 in, 14 out",
            "Worker f5f725a7, sk-tiny, re_try later",
            "C:\\Users\\antho\\AppData\\Local\\Glim\\glim.exe 12,073,472 bytes",
        ] {
            assert!(!looks_like_secret(s), "{s}");
        }
    }

    #[test]
    fn the_real_handoff_shape_from_this_repo_parses() {
        // The headings this repo's own HANDOFF.md uses (2026-10-13): Goal,
        // then sections the template doesn't name.
        let h = parse("# HANDOFF (updated 2026-10-13)\n\n## Goal\nTwo PRs open.\n\n## Owner's constraints (2026-10-13)\n- no merges\n\n## Open PRs\n- #18\n\n## Gotchas\n- x\n");
        assert_eq!(h.goal.as_deref(), Some("Two PRs open."));
        assert_eq!((h.branch_section, h.in_progress, h.open_questions), (None, None, None));
    }

    #[test]
    fn cards_put_the_most_recent_handoff_first_and_cut_long_sections() {
        let dir = std::env::temp_dir().join(format!("glim-resume-cards-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (name, body) in [("older", "## Goal\nOld work\n## In progress\nSomething\n"), ("newer", "## Goal\nNew work\n## In progress\nThe next step\n## Open questions\n- Ignore?\n"), ("empty", "nothing here\n")] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
            std::fs::write(dir.join(name).join(FILE_NAME), body).unwrap();
        }
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(86_400);
        std::fs::File::options().write(true).open(dir.join("older").join(FILE_NAME)).unwrap().set_modified(old).unwrap();
        let found = discover(&dir);
        let cards = cards(&read_all(&found));
        assert_eq!(cards.iter().map(|c| c.repo.as_str()).collect::<Vec<_>>(), ["newer", "older"], "newest first; an empty one is left out");
        assert_eq!(cards[0].goal.as_deref(), Some("New work"));
        assert!(cards[0].has_questions && !cards[1].has_questions);
        let long = Some("x".repeat(CARD_CHARS + 50));
        assert_eq!(short(&long).unwrap().chars().count(), CARD_CHARS + 1, "cut, with an ellipsis");
        // Only a repo Glim found, by its exact folder name.
        assert_eq!(folder_for("newer", &found), Some(dir.join("newer")));
        for bad in ["", "..", "newer/..", "C:\\Windows", "missing"] {
            assert_eq!(folder_for(bad, &found), None, "{bad}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
