// Live checks for dictation on this PC (voice 1.5b), run by hand, never in
// CI: `cargo test --lib voice::live -- --ignored --nocapture --test-threads=1`.
//
// - `whisper_on_this_pc`: GLIM_LIVE_WHISPER = the real %LOCALAPPDATA%\Glim\whisper
//   folder (copied into the test's own scratch Glim folder: tests never touch
//   the real one), GLIM_LIVE_CLIPS = a folder of 16 kHz mono WAV clips. Runs
//   Glim's own code: files(), the server's start (loopback, no proxy
//   variables, tied to this process), transcribe() through net::request,
//   voice::decide's no-Claude rule on each transcript, and stop().
// - `claude_clean_up`: GLIM_LIVE_CLAUDE=1 reads the saved key from Windows
//   Credential Manager (never printed) and sends three short made-up
//   transcripts (a fraction of a cent).
//
// Only counts, timings, codes and the synthetic clips' transcripts are printed.

use std::path::{Path, PathBuf};
use std::time::Instant;

use super::whisper;

fn block_on<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
}

/// The PCM samples of a 16-bit WAV (any header length: the "data" chunk).
fn wav_pcm(path: &Path) -> Vec<i16> {
    let b = std::fs::read(path).unwrap();
    let mut i = 12;
    while i + 8 <= b.len() {
        let len = u32::from_le_bytes([b[i + 4], b[i + 5], b[i + 6], b[i + 7]]) as usize;
        if &b[i..i + 4] == b"data" {
            return b[i + 8..(i + 8 + len).min(b.len())].as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)).collect();
        }
        i += 8 + len + (len & 1);
    }
    panic!("no data chunk in {}", path.display());
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
    }
}

/// netstat's TCP rows for a pid: (local, remote, state).
fn tcp_rows(pid: u32) -> Vec<(String, String, String)> {
    let out = std::process::Command::new("netstat").args(["-ano", "-p", "TCP"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.len() == 5 && f[0] == "TCP" && f[4] == pid.to_string()).then(|| (f[1].to_string(), f[2].to_string(), f[3].to_string()))
        })
        .collect()
}

fn server_pids() -> Vec<u32> {
    let out = std::process::Command::new("tasklist").args(["/FI", "IMAGENAME eq whisper-server.exe", "/FO", "CSV", "/NH"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split("\",\"").nth(1).and_then(|p| p.parse().ok()))
        .collect()
}

#[test]
#[ignore = "live: needs GLIM_LIVE_WHISPER and GLIM_LIVE_CLIPS"]
fn whisper_on_this_pc() {
    let real = PathBuf::from(std::env::var("GLIM_LIVE_WHISPER").expect("GLIM_LIVE_WHISPER"));
    let clips = PathBuf::from(std::env::var("GLIM_LIVE_CLIPS").expect("GLIM_LIVE_CLIPS"));
    let before = server_pids();

    // Nothing there yet: "no-whisper", nothing started.
    assert!(whisper::files("base").is_none());
    let pcm = wav_pcm(&clips.join("d1.wav"));
    assert_eq!(block_on(whisper::transcribe(&pcm, "base", "")), Err("no-whisper"));
    println!("no files: no-whisper, nothing started");

    copy_dir(&real, &whisper::folder());
    let (exe, model) = whisper::files("base").expect("base found");
    println!("found: {} + {}", exe.file_name().unwrap().to_string_lossy(), model.file_name().unwrap().to_string_lossy());
    assert!(whisper::files("small").is_none(), "small isn't installed: its own no-whisper");

    let prompt = super::prompt_from("Glim, HOA, Okafor, Nguyen, Rodriguez Plumbing, dale-cuts, lease renewal, gate code, unit 4B");
    let mut names: Vec<_> = std::fs::read_dir(&clips).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "wav")).collect();
    names.sort();
    let mut server = None;
    for (n, path) in names.iter().enumerate() {
        let pcm = wav_pcm(path);
        let t0 = Instant::now();
        let text = block_on(whisper::transcribe(&pcm, "base", &prompt)).expect("transcribed");
        let secs = t0.elapsed().as_secs_f64();
        let id = path.file_stem().unwrap().to_string_lossy().into_owned();
        // §4.1 with no Claude, as if a field had the focus.
        let decision = match super::decide(super::Mode::Dictate, &text, Err(super::CleanupFailed::NoKey), true) {
            super::Decision::Type { .. } => "types raw",
            super::Decision::Heard { .. } => "heard card",
            super::Decision::Sensitive => "sensitive",
            super::Decision::Question { .. } => "question",
            super::Decision::Resume { .. } => "resume card",
            super::Decision::Open { .. } => "open app",
            super::Decision::Draft(_) => "draft card",
            super::Decision::EditDraft { .. } => "edit card",
        };
        println!("{id}: {:.1}s audio, {secs:.2}s{} → {decision}: {text}", pcm.len() as f64 / 16_000.0, if n == 0 { " (start + model load)" } else { "" });
        if id.starts_with('c') {
            assert_eq!(decision, "heard card", "{id}: a command is never typed raw");
        }
        if n == 0 {
            let started: Vec<u32> = server_pids().into_iter().filter(|p| !before.contains(p)).collect();
            assert_eq!(started.len(), 1, "one server");
            let pid = started[0];
            let rows = tcp_rows(pid);
            println!("server pid {pid}, TCP rows: {rows:?}");
            assert!(rows.iter().any(|(_, _, s)| s == "LISTENING"), "it listens");
            for (local, remote, _) in &rows {
                assert!(local.starts_with("127.0.0.1:"), "local {local}");
                assert!(remote.starts_with("127.0.0.1:") || remote == "0.0.0.0:0", "remote {remote}");
            }
            server = Some(pid);
        }
    }
    let after: Vec<u32> = server_pids().into_iter().filter(|p| !before.contains(p)).collect();
    assert_eq!(after, vec![server.unwrap()], "the same server for every clip");

    whisper::stop();
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(!server_pids().contains(&server.unwrap()), "stopped");
    println!("stop(): server gone");
}

#[test]
#[ignore = "live: GLIM_LIVE_CLAUDE=1 sends to the Claude API"]
fn claude_clean_up() {
    assert_eq!(std::env::var("GLIM_LIVE_CLAUDE").as_deref(), Ok("1"));
    use crate::ai::{claude, dictate, key};
    let key = key::load(&key::vault()).expect("vault").expect("a saved key");
    let base = crate::net::Url::parse(claude::API_BASE).unwrap();
    let mut nanos = 0;
    use super::Mode::{Ask, Dictate};
    let repos = ["dale-cuts".to_string(), "local-companion".to_string()];
    for (said, app, mode) in [
        ("um thanks for the quick turnaround on unit 4B we'll get the lease to you by friday", "OUTLOOK.EXE", Dictate),
        // Decision A's regression: the two sentences hijacked on 2026-10-16.
        ("this is a test of glim dictation in my own voice", "OUTLOOK.EXE", Dictate),
        ("please send the updated lease to the tenant before friday afternoon", "OUTLOOK.EXE", Dictate),
        ("open outlook", "notepad.exe", Ask),
        ("send it", "OUTLOOK.EXE", Ask),
        ("where was I on dale cuts", "notepad.exe", Ask),
        // 4c: the 2026-10-16 test's draft requests all came back as clarify.
        ("draft an email to me with the subject glim test one saying this is a test draft", "OUTLOOK.EXE", Ask),
        ("draft an email to myself saying the plumber comes tuesday", "notepad.exe", Ask),
        ("email jordan lee at example dot com saying the lease is ready", "OUTLOOK.EXE", Ask),
        // Fix b: the 2026-10-17 retest's "edit the email you just drafted" got "no earlier draft".
        ("edit the email you just drafted and add that the shift starts at nine", "OUTLOOK.EXE", Ask),
        ("change the email you just drafted to say wednesday instead of tuesday", "notepad.exe", Ask),
        ("add to that email that parking is in the back", "OUTLOOK.EXE", Ask),
    ] {
        let t0 = Instant::now();
        let cx = dictate::Context { mode, app, today: "2026-10-15", repos: &repos, asked: None };
        let (r, usage) = block_on(dictate::clean_up(&base, &key, "claude-haiku-5-5", said, &cx));
        nanos += usage.as_ref().map_or(0, |u| u.cost_nanos);
        println!("{mode:?} {said:?} → {r:?} in {:.2}s ({})", t0.elapsed().as_secs_f64(), usage.map_or("no usage".into(), |u| format!("{} in={} out={}", u.model, u.input_tokens, u.output_tokens)));
        assert!(r.is_ok(), "{said}: {r:?}");
        if mode == Dictate {
            assert!(matches!(r, Ok(super::Cleanup::Dictate(_))), "dictation is always typed: {said}");
        }
        if said.contains("draft an email to me") || said.contains("myself") {
            let Ok(super::Cleanup::Draft(d)) = &r else { panic!("a draft, not a question: {said}: {r:?}") };
            assert!(d.to.iter().any(|t| super::draft::means_me(t)), "'me' passed as said: {:?}", d.to);
        }
        if said.contains(" at example dot com saying") {
            let Ok(super::Cleanup::Draft(d)) = &r else { panic!("a draft: {r:?}") };
            let resolved: Vec<_> = d.to.iter().filter_map(|t| super::draft::spoken_address(t)).collect();
            assert_eq!(resolved, ["jordanlee@example.com"], "as said, then normalized: {:?}", d.to);
        }
        if said.contains("just drafted") || said.starts_with("add to that email") {
            assert!(matches!(r, Ok(super::Cleanup::EditDraft(_))), "a change to the last draft, not a question: {said}: {r:?}");
        }
    }
    // The second request of an edit, on a made-up draft.
    let current = super::draft::DraftText { subject: "Shift change".into(), body: "Hi Gabriela,\n\nThe shift moves to Tuesday.".into() };
    let t0 = Instant::now();
    let (r, usage) = block_on(dictate::revise(&base, &key, "claude-haiku-5-5", &current, "add that the shift starts at nine"));
    nanos += usage.as_ref().map_or(0, |u| u.cost_nanos);
    println!("revise → {r:?} in {:.2}s", t0.elapsed().as_secs_f64());
    let new = r.expect("revised");
    assert!(new.body.contains("Tuesday") && (new.body.contains('9') || new.body.to_lowercase().contains("nine")), "the change applied, the rest kept: {new:?}");
    assert!(new.body.starts_with("Hi Gabriela,"), "kept as it was: {new:?}");
    println!("total ${:.5}", nanos as f64 / 1e9);
}
