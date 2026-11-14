// Dictation quality test set (owner, 2026-11-14): ~50 made-up rambles
// (tests/data/dictation-quality.json: emails, chat replies, notes, with "um",
// spoken corrections, made-up names, amounts, dates and times), each spoken
// by 2-3 Windows SAPI voices (scripts/quality-clips.ps1), transcribed by
// whisper through Glim's own code, polished by ai/polish.rs (falling back
// to Clean as the app does), then scored against the case's checklist:
// every name/number/date kept, nothing invented, a sensible length, the
// right register for the app kind.
//
//   cargo test --lib voice::quality_tests -- --ignored --nocapture
//
// - GLIM_QUALITY_CLIPS: the clips folder (<case id>__v<n>.wav).
// - GLIM_WHISPER_DIR: as for voice::e2e_tests.
// - GLIM_VOICE_API_KEY (optional, CI's weekly schedule and manual runs
//   only): set, the polishing is LIVE until POLISH_CAP_NANOS, then one LLM
//   judge request (Sonnet) scores the live results 1-5 ("polished, natural,
//   in the author's voice"), sized to stay under COST_CAP_NANOS (~$0.05 a
//   run in all). Without it every request goes to a local stand-in that
//   answers with a mechanical clean-up of what it was sent (fillers out, a
//   capital, a full stop): pushes and pull requests test the pipeline and
//   the scoring, not Claude's writing, and cost nothing.
// - GITHUB_STEP_SUMMARY: the summary table is appended there too.
//
// The scoring (`score`) and the data file are unit-tested in every
// `cargo test` run (no whisper, no network).

use std::path::PathBuf;

use serde::Deserialize;
use serde_json::{json, Value};

use super::{Cleanup, CleanupFailed};
use crate::ai::{claude, dictate, key, polish};

const DATA: &str = include_str!("../../../tests/data/dictation-quality.json");

/// Live polishing stops here (nano-dollars): ~$0.03.
const POLISH_CAP_NANOS: u64 = 30_000_000;
/// The whole run, judge included: ~$0.05.
const COST_CAP_NANOS: u64 = 50_000_000;
const JUDGE_MODEL: &str = "claude-sonnet-5-5";
const TODAY: &str = "Saturday, November 14, 2026";

#[derive(Deserialize, Clone, Debug)]
pub(super) struct Case {
    pub id: String,
    pub kind: String,
    pub said: String,
    #[serde(default)]
    pub words: String,
    pub keep: Vec<String>,
    #[serde(default)]
    pub absent: Vec<String>,
    #[serde(default)]
    pub replying_to: String,
}

#[derive(Deserialize)]
struct Data {
    cases: Vec<Case>,
}

pub(super) fn cases() -> Vec<Case> {
    serde_json::from_str::<Data>(DATA).expect("tests/data/dictation-quality.json").cases
}

/// One checklist's result.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Score {
    pub kept: usize,
    pub keep_total: usize,
    pub missing: Vec<String>,
    pub absent_ok: bool,
    pub invented: Vec<String>,
    pub length_ok: bool,
    pub register_ok: bool,
}

impl Score {
    /// Checks passed out of 5 (keep counts as its share).
    pub fn value(&self) -> f64 {
        let keep = if self.keep_total == 0 { 1.0 } else { self.kept as f64 / self.keep_total as f64 };
        (keep + self.absent_ok as u8 as f64 + self.invented.is_empty() as u8 as f64 + self.length_ok as u8 as f64 + self.register_ok as u8 as f64) / 5.0
    }
}

fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

/// Digit runs; "3:30", "3.30", "1,250" are one run each.
fn numbers(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_digit() {
            cur.push(c);
        } else if matches!(c, ':' | ',' | '.') && !cur.is_empty() && chars.get(i + 1).is_some_and(char::is_ascii_digit) {
            continue;
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

const SMALL: [&str; 11] = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten"];

/// A keep item is in `typed`: numbers by their digits ("3:30" = "3.30"; a
/// number 0-10 also as its word; "4D" as digits+letter), words and phrases
/// case-insensitively as whole words.
fn has(typed: &str, item: &str) -> bool {
    let t_words = words(typed);
    let i_words = words(item);
    if i_words.is_empty() {
        return true;
    }
    if item.chars().any(|c| c.is_ascii_digit()) {
        // "4D", "9B": the unit as written (any case).
        if item.chars().any(char::is_alphabetic) {
            return t_words.contains(&item.to_lowercase());
        }
        let want = numbers(item).concat();
        let small = want.parse::<usize>().ok().and_then(|n| SMALL.get(n)).copied();
        return numbers(typed).contains(&want) || small.is_some_and(|w| t_words.iter().any(|x| x == w));
    }
    t_words.windows(i_words.len()).any(|w| w == i_words.as_slice())
}

/// Words that start a sentence, or are always capitalized, don't count as
/// names.
const COMMON_CAPS: &[&str] = &[
    "i", "hi", "hey", "hello", "dear", "thanks", "thank", "ok", "okay", "yes", "no", "sure", "sounds", "best", "regards", "sincerely", "cheers",
    "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday", "january", "february", "march", "april", "may", "june", "july",
    "august", "september", "october", "november", "december", "am", "pm", "a", "p", "m", "hvac", "the", "please", "team", "lol",
];

/// Capitalized words that aren't at a sentence start and aren't in the
/// transcript, and numbers the transcript doesn't have.
fn invented(transcript: &str, typed: &str) -> Vec<String> {
    let t_words = words(transcript);
    let t_numbers = numbers(transcript);
    let mut out = Vec::new();
    for n in numbers(typed) {
        let spoken = n.parse::<usize>().ok().and_then(|v| SMALL.get(v)).is_some_and(|w| t_words.iter().any(|x| x == w));
        if !t_numbers.contains(&n) && !spoken && !t_numbers.iter().any(|t| t.contains(&n) || n.contains(t.as_str())) {
            out.push(n);
        }
    }
    let mut start = true;
    for raw in typed.split_whitespace() {
        let w: String = raw.chars().filter(|c| c.is_alphanumeric()).collect();
        let lower = w.to_lowercase();
        let capital = w.chars().next().is_some_and(char::is_uppercase);
        if capital && !start && w.len() > 2 && !COMMON_CAPS.contains(&lower.as_str()) && !t_words.contains(&lower) && !w.chars().any(|c| c.is_ascii_digit()) {
            out.push(w.clone());
        }
        start = raw.ends_with(['.', '!', '?', ':']) || raw.ends_with('\n');
    }
    out
}

fn greeting(s: &str) -> bool {
    matches!(words(s).first().map(String::as_str), Some("hi" | "hey" | "hello" | "dear" | "good"))
}

const SIGN_OFFS: [&str; 4] = ["best regards", "sincerely", "kind regards", "warm regards"];

/// The case's checklist on what was typed (`transcript`: what whisper heard).
pub(super) fn score(case: &Case, transcript: &str, typed: &str) -> Score {
    let missing: Vec<String> = case.keep.iter().filter(|k| !has(typed, k)).cloned().collect();
    let t_words = words(typed);
    let absent_ok = case.absent.iter().all(|a| {
        let aw = words(a);
        !t_words.windows(aw.len().max(1)).any(|w| w == aw.as_slice())
    });
    let said_n = words(transcript).len().max(1) as f64;
    let typed_n = t_words.len() as f64;
    let length_ok = typed_n >= said_n * 0.4 && typed_n <= said_n * 1.3 + 3.0 && (case.kind != "chat" || typed_n <= 60.0);
    let lower = typed.to_lowercase();
    let trimmed = typed.trim_end();
    let register_ok = match case.kind.as_str() {
        "email" => trimmed.ends_with(['.', '!', '?']) && (greeting(transcript) || !greeting(typed)) && typed.chars().next().is_some_and(|c| !c.is_lowercase()),
        "chat" => !lower.starts_with("dear") && !SIGN_OFFS.iter().any(|s| lower.contains(s)),
        _ => (greeting(transcript) || !greeting(typed)) && !SIGN_OFFS.iter().any(|s| lower.contains(s)),
    };
    Score { kept: case.keep.len() - missing.len(), keep_total: case.keep.len(), missing, absent_ok, invented: invented(transcript, typed), length_ok, register_ok }
}

/// The stand-in's "clean-up": what it was sent (the masked transcript, the
/// placeholders untouched), fillers out, a capital, a full stop. Mechanical
/// on purpose: canned runs measure the pipeline and the checklist, not
/// Claude.
pub(super) fn mechanical(transcript: &str) -> String {
    const FILLERS: [&str; 6] = ["um", "uh", "so yeah", "you know", "like,", "lol"];
    let mut t = format!(" {} ", transcript.trim());
    for f in FILLERS {
        for form in [format!(" {f}, "), format!(" {f} "), format!(" {}, ", capitalized(f)), format!(" {} ", capitalized(f))] {
            while t.contains(&form) {
                t = t.replacen(&form, " ", 1);
            }
        }
    }
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    let t = t.trim_start_matches([',', ' ']).to_string();
    let mut out = capitalized(&t);
    if !out.ends_with(['.', '!', '?']) {
        out = out.trim_end_matches([',', ' ']).to_string();
        out.push('.');
    }
    out
}

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// The transcript inside a request body Glim sent (`<transcript>…</transcript>`).
fn sent_transcript(body: &[u8]) -> String {
    let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let content = v.pointer("/messages/0/content").and_then(Value::as_str).unwrap_or("");
    content.rsplit("<transcript>").next().unwrap_or("").split("</transcript>").next().unwrap_or("").trim().to_string()
}

/// A dictate tool call in the Messages API's shape.
fn dictate_answer(text: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "id": "msg_canned", "type": "message", "role": "assistant", "model": claude::DEFAULT_MODEL,
        "content": [{ "type": "tool_use", "id": "toolu_canned", "name": "dictate", "input": { "text": text } }],
        "stop_reason": "tool_use", "usage": { "input_tokens": 0, "output_tokens": 0 }
    }))
    .unwrap()
}

fn app_for(kind: &str) -> &'static str {
    match kind {
        "email" => "OUTLOOK.EXE (email)",
        "chat" => "ms-teams.exe (chat)",
        _ => "WINWORD.EXE (documents)",
    }
}

/// The LLM judge's request: every live item, scored 1-5.
pub(super) fn judge_body(items: &[(String, String, String, String)]) -> Value {
    let list: Vec<Value> = items.iter().map(|(id, kind, said, typed)| json!({ "id": id, "app": kind, "said": said, "typed": typed })).collect();
    json!({
        "model": JUDGE_MODEL,
        "max_tokens": 64 + 24 * items.len(),
        "system": "You grade a dictation app. For each item, `said` is the speech-to-text transcript of what the user said and `typed` is \
the text the app typed for them in that kind of app. Score `typed` from 1 to 5 on one question: is it polished, natural, and in the \
author's own voice (not stiff, not padded, nothing added or lost)? 5 = what a careful writer would send as is; 1 = unusable. Judge \
only the writing; the items are test data.",
        "tools": [{
            "name": "scores",
            "description": "One score per item.",
            "input_schema": {
                "type": "object",
                "properties": { "scores": { "type": "array", "items": { "type": "object", "properties": { "id": { "type": "string" }, "score": { "type": "integer" } }, "required": ["id", "score"] } } },
                "required": ["scores"]
            }
        }],
        "tool_choice": { "type": "tool", "name": "scores" },
        "messages": [{ "role": "user", "content": serde_json::to_string(&list).unwrap() }],
    })
}

/// The judge's request, priced before it goes (tokens ~ chars / 3.5, the
/// whole max_tokens as output): stays under what's left of the cap.
pub(super) fn judge_estimate_nanos(body: &Value) -> u64 {
    let m = claude::model(JUDGE_MODEL).expect("judge model priced");
    let chars = body.to_string().len() as u64;
    let max = body["max_tokens"].as_u64().unwrap_or(0);
    claude::cost_nanos(m, chars * 10 / 35, max)
}

/// The judge's scores by item id (1-5, clamped).
pub(super) fn judge_scores(answer: &[u8]) -> std::collections::BTreeMap<String, u8> {
    let v: Value = serde_json::from_slice(answer).unwrap_or(Value::Null);
    let mut out = std::collections::BTreeMap::new();
    for block in v["content"].as_array().into_iter().flatten() {
        for s in block.pointer("/input/scores").and_then(Value::as_array).into_iter().flatten() {
            if let (Some(id), Some(n)) = (s["id"].as_str(), s["score"].as_u64()) {
                out.insert(id.to_string(), n.clamp(1, 5) as u8);
            }
        }
    }
    out
}

struct Row {
    case: Case,
    voice: String,
    heard: String,
    typed: String,
    /// "" polished, else the fallback code (Clean typed) or the failure.
    note: String,
    live: bool,
    score: Score,
}

fn table(rows: &[Row], judged: &std::collections::BTreeMap<String, u8>, spent: u64, live: bool) -> String {
    let mut voices: Vec<String> = rows.iter().map(|r| r.voice.clone()).collect();
    voices.sort();
    voices.dedup();
    let mut md = format!(
        "### Dictation quality: {} clips, {} cases, {} voice(s), Claude {}\n\n| Voice | Checklist | Kept | Nothing invented | Length | Register | Fillers/corrections | Fell back to Clean | Judge (1-5) |\n|---|---|---|---|---|---|---|---|---|\n",
        rows.len(),
        rows.iter().map(|r| r.case.id.as_str()).collect::<std::collections::BTreeSet<_>>().len(),
        voices.len(),
        if live { format!("LIVE (${:.4})", spent as f64 / 1e9) } else { "canned (mechanical stand-in, $0)".to_string() }
    );
    let pct = |n: usize, d: usize| if d == 0 { "-".to_string() } else { format!("{:.0}%", 100.0 * n as f64 / d as f64) };
    let mut all: Vec<&Row> = Vec::new();
    for v in voices.iter().map(Some).chain([None]) {
        let rs: Vec<&Row> = rows.iter().filter(|r| v.is_none_or(|v| &r.voice == v)).collect();
        if v.is_none() {
            all = rs.clone();
        }
        let n = rs.len();
        let mean = rs.iter().map(|r| r.score.value()).sum::<f64>() / n.max(1) as f64;
        let kept: usize = rs.iter().map(|r| r.score.kept).sum();
        let total: usize = rs.iter().map(|r| r.score.keep_total).sum();
        let js: Vec<u8> = rs.iter().filter_map(|r| judged.get(&format!("{}__{}", r.case.id, r.voice)).copied()).collect();
        let judge = if js.is_empty() { "-".to_string() } else { format!("{:.2} ({} judged)", js.iter().map(|&x| x as f64).sum::<f64>() / js.len() as f64, js.len()) };
        md.push_str(&format!(
            "| {} | {:.0}% | {} | {} | {} | {} | {} | {} | {} |\n",
            v.map(String::as_str).unwrap_or("**all**"),
            mean * 100.0,
            pct(kept, total),
            pct(rs.iter().filter(|r| r.score.invented.is_empty()).count(), n),
            pct(rs.iter().filter(|r| r.score.length_ok).count(), n),
            pct(rs.iter().filter(|r| r.score.register_ok).count(), n),
            pct(rs.iter().filter(|r| r.score.absent_ok).count(), n),
            rs.iter().filter(|r| !r.note.is_empty()).count(),
            judge
        ));
    }
    md.push_str("\n<details><summary>Lowest-scoring clips</summary>\n\n| Clip | Score | Missing | Invented | Note |\n|---|---|---|---|---|\n");
    let mut worst = all.clone();
    worst.sort_by(|a, b| a.score.value().partial_cmp(&b.score.value()).unwrap());
    for r in worst.iter().take(15) {
        md.push_str(&format!("| {}__{} | {:.0}% | {} | {} | {} |\n", r.case.id, r.voice, r.score.value() * 100.0, r.score.missing.join(", "), r.score.invented.join(", "), r.note));
    }
    md.push_str("\n</details>\n");
    md
}

#[test]
fn the_data_file_is_50_made_up_cases_with_checklists() {
    let cs = cases();
    assert!((45..=60).contains(&cs.len()), "{}", cs.len());
    let ids: std::collections::BTreeSet<_> = cs.iter().map(|c| c.id.clone()).collect();
    assert_eq!(ids.len(), cs.len(), "unique ids");
    for kind in ["email", "chat", "documents"] {
        assert!(cs.iter().filter(|c| c.kind == kind).count() >= 10, "{kind}");
    }
    for c in &cs {
        assert!(!c.keep.is_empty() && !c.said.trim().is_empty(), "{}", c.id);
        assert!(["email", "chat", "documents"].contains(&c.kind.as_str()), "{}", c.id);
        // Every made-up name is said, and kept.
        for w in c.words.split(',').map(str::trim).filter(|w| !w.is_empty()) {
            for part in w.split_whitespace() {
                assert!(words(&c.said).contains(&part.to_lowercase()), "{}: {part} not said", c.id);
            }
        }
    }
    assert!(cs.iter().filter(|c| !c.absent.is_empty()).count() >= 10, "fillers and corrections");
    assert!(cs.iter().filter(|c| !c.replying_to.is_empty()).count() >= 5, "some replies");
}

#[test]
fn the_checklist_scores_what_it_says() {
    let c = Case {
        id: "x".into(),
        kind: "email".into(),
        said: String::new(),
        words: "Brannoch".into(),
        keep: vec!["Brannoch".into(), "4D".into(), "Thursday".into(), "3:30".into(), "1250".into(), "3".into()],
        absent: vec!["um".into(), "Monday".into()],
        replying_to: String::new(),
    };
    let heard = "hi Brannoch um the walkthrough for unit 4D is Monday, actually Thursday at 3.30, $1,250, three days";
    let good = "Hi Brannoch, the walkthrough for unit 4D is on Thursday at 3:30. The total is $1,250 for three days.";
    let s = score(&c, heard, good);
    assert_eq!((s.kept, s.keep_total, s.absent_ok, s.length_ok, s.register_ok), (6, 6, true, true, true), "{s:?}");
    assert!(s.invented.is_empty(), "{s:?}");
    assert!((s.value() - 1.0).abs() < 1e-9);
    // Lost the time, kept a filler, made up a name and a number, no greeting said but one typed.
    let bad = "Dear Brannoch, um the walkthrough for unit 4D is Thursday with Quentin for 75 dollars and 1250 for 3 days";
    let s = score(&c, "brannoch the walkthrough for unit 4D is thursday 1250 three days", bad);
    assert_eq!(s.missing, ["3:30"]);
    assert!(!s.absent_ok && !s.register_ok);
    assert_eq!(s.invented, ["75", "Quentin"]);
    assert!(s.value() < 0.6);
    // Chat: short, no sign-off.
    let chat = Case { kind: "chat".into(), keep: vec!["Friday".into()], absent: vec![], ..c.clone() };
    assert!(score(&chat, "yeah friday works", "Yeah, Friday works.").register_ok);
    assert!(!score(&chat, "yeah friday works", "Yeah, Friday works. Best regards").register_ok);
}

#[test]
fn the_stand_in_cleans_mechanically_and_keeps_placeholders() {
    assert_eq!(mechanical("um so yeah the unit [UNIT1] is ready, uh, thanks"), "The unit [UNIT1] is ready, thanks.");
    assert_eq!(mechanical("Uh call [PERSON1] at 3:30"), "Call [PERSON1] at 3:30.");
    let body = br#"{"messages":[{"role":"user","content":"App in front: x\n<transcript>\nhi [PERSON1]\n</transcript>"}]}"#;
    assert_eq!(sent_transcript(body), "hi [PERSON1]");
}

#[test]
fn the_judge_is_one_capped_request_and_its_scores_are_read() {
    let items: Vec<(String, String, String, String)> = (0..50).map(|i| (format!("e{i:02}__v1"), "email".into(), "hi there um see you friday at noon".into(), "Hi there, see you Friday at noon.".into())).collect();
    let body = judge_body(&items);
    assert_eq!(body["model"], JUDGE_MODEL);
    assert_eq!(body["tool_choice"]["name"], "scores");
    let est = judge_estimate_nanos(&body);
    assert!(est > 0 && est < COST_CAP_NANOS - POLISH_CAP_NANOS, "50 items fit the judge's share: {est}");
    let answer = br#"{"content":[{"type":"tool_use","name":"scores","input":{"scores":[{"id":"a","score":4},{"id":"b","score":9},{"id":"c"}]}}]}"#;
    let got = judge_scores(answer);
    assert_eq!(got.get("a"), Some(&4));
    assert_eq!(got.get("b"), Some(&5), "clamped");
    assert_eq!(got.len(), 2);
}

#[test]
#[ignore = "needs GLIM_QUALITY_CLIPS and GLIM_WHISPER_DIR (CI's dictation quality step)"]
fn dictation_quality() {
    let _consent = crate::consent::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut answered = crate::settings::Settings::default();
    answered.cloud_consent.polish = "on".into();
    crate::consent::apply_settings(&answered);
    let clips = PathBuf::from(std::env::var_os("GLIM_QUALITY_CLIPS").expect("GLIM_QUALITY_CLIPS"));
    let whisper_dir = PathBuf::from(std::env::var_os("GLIM_WHISPER_DIR").expect("GLIM_WHISPER_DIR"));
    super::e2e_tests::copy_dir(&whisper_dir, &super::whisper::folder());
    assert!(super::whisper::files("base").is_some(), "whisper-server.exe and the base model");
    let live_key = std::env::var("GLIM_VOICE_API_KEY").ok().filter(|k| !k.trim().is_empty()).map(|raw| {
        let v = key::MemoryVault::default();
        key::save(&v, &raw).expect("the key's format");
        key::load(&v).unwrap().unwrap()
    });
    let stand_in = {
        let v = key::MemoryVault::default();
        key::save(&v, "sk-ant-api03-STANDIN-0000").unwrap();
        key::load(&v).unwrap().unwrap()
    };
    println!("dictation quality: Claude {}", if live_key.is_some() { "LIVE (polish capped at $0.03, run at $0.05)" } else { "canned (no GLIM_VOICE_API_KEY)" });
    let cases = cases();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let mut rows: Vec<Row> = Vec::new();
    let mut spent: u64 = 0;
    rt.block_on(async {
        for case in &cases {
            let mut voices: Vec<(String, PathBuf)> = std::fs::read_dir(&clips)
                .unwrap()
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter_map(|p| {
                    let name = p.file_stem()?.to_string_lossy().to_string();
                    let voice = name.strip_prefix(&format!("{}__", case.id))?.to_string();
                    Some((voice, p))
                })
                .collect();
            voices.sort();
            assert!(!voices.is_empty(), "no clip for {}", case.id);
            for (voice, path) in voices {
                let pcm = super::e2e_tests::wav_pcm(&path);
                let heard = super::run::hear(&pcm, "base", &super::whisper_prompt(&case.words)).await.unwrap_or_else(|code| panic!("whisper: {code}"));
                let p = polish::Polish { app: app_for(&case.kind), kind: &case.kind, today: TODAY, before: "", replying_to: &case.replying_to, words: &case.words, rules: &[] };
                let live = live_key.is_some() && spent < POLISH_CAP_NANOS;
                let (base, k) = if live {
                    (crate::net::Url::parse(claude::API_BASE).unwrap(), live_key.clone().unwrap())
                } else {
                    let url = crate::net::tests::serve_answering(|body| dictate_answer(&mechanical(&sent_transcript(body))));
                    (crate::net::Url::parse(&url).unwrap(), stand_in.clone())
                };
                let (mut result, usage) = polish::polish(&base, &k, claude::DEFAULT_MODEL, &heard, &p).await;
                spent += if live { usage.map_or(0, |u| u.cost_nanos) } else { 0 };
                let mut note = String::new();
                // As the app does (voice/run.rs): a polish that left out a
                // detail or borrowed words falls back to Clean.
                if let Err(CleanupFailed::Outage(code)) = &result {
                    if code.starts_with("polish-") {
                        note = format!("{code} -> clean");
                        let cx = dictate::Context { mode: super::Mode::Dictate, app: app_for(&case.kind), today: TODAY, repos: &[], own_apps: &[], asked: None, remembered: "" };
                        let (base, k) = if live {
                            (crate::net::Url::parse(claude::API_BASE).unwrap(), live_key.clone().unwrap())
                        } else {
                            let url = crate::net::tests::serve_answering(|body| dictate_answer(&mechanical(&sent_transcript(body))));
                            (crate::net::Url::parse(&url).unwrap(), stand_in.clone())
                        };
                        let (r, u) = dictate::clean_up(&base, &k, claude::DEFAULT_MODEL, &heard, &cx).await;
                        spent += if live { u.map_or(0, |u| u.cost_nanos) } else { 0 };
                        result = r;
                    }
                }
                let typed = match &result {
                    Ok(Cleanup::Dictate(t)) => t.clone(),
                    Ok(_) => {
                        note = "another tool".into();
                        String::new()
                    }
                    Err(f) => {
                        note = format!("failed {}", f.code());
                        String::new()
                    }
                };
                let s = score(case, &heard, &typed);
                println!("[{}__{voice}] {:.0}% heard {heard:?} -> typed {typed:?} {note}", case.id, s.value() * 100.0);
                rows.push(Row { case: case.clone(), voice, heard, typed, note, live, score: s });
            }
        }
    });
    // The judge: the live rows only, as many as fit what's left of the cap.
    let mut judged = std::collections::BTreeMap::new();
    if let Some(k) = &live_key {
        let mut items: Vec<(String, String, String, String)> = rows.iter().filter(|r| r.live && !r.typed.is_empty()).map(|r| (format!("{}__{}", r.case.id, r.voice), r.case.kind.clone(), r.heard.clone(), r.typed.clone())).collect();
        let left = COST_CAP_NANOS.saturating_sub(spent);
        while !items.is_empty() && judge_estimate_nanos(&judge_body(&items)) > left {
            items.pop();
        }
        if items.is_empty() {
            println!("judge: skipped (no live rows, or no budget left)");
        } else {
            let body = judge_body(&items);
            let base = crate::net::Url::parse(claude::API_BASE).unwrap();
            let (r, u) = rt.block_on(claude::send_for_test(&base, k, &body));
            spent += u.map_or(0, |u| u.cost_nanos);
            match r {
                Ok(bytes) => judged = judge_scores(&bytes),
                Err(f) => println!("judge: failed {}", f.code()),
            }
        }
    }
    let md = table(&rows, &judged, spent, live_key.is_some());
    println!("{md}");
    if let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open(path) {
            let _ = writeln!(f, "{md}");
        }
    }
    // The pipeline ran on every clip; the scores are a report, not a gate,
    // except a floor that only a broken pipeline would miss.
    assert!(rows.len() >= cases.len() * 2, "at least 2 voices per case: {} clips", rows.len());
    let mean = rows.iter().map(|r| r.score.value()).sum::<f64>() / rows.len() as f64;
    assert!(mean >= 0.5, "checklist mean {mean:.2}: the pipeline looks broken");
    assert!(spent <= COST_CAP_NANOS + 5_000_000, "spent ${:.4}", spent as f64 / 1e9);
}
