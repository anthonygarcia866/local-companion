// Runbooks v1 tests: the file's checks, approval by hash, the runner, the
// env_secret write into a real temp git repo, the command line's refusals,
// and that no secret value reaches any log or progress line.

use super::env_file::{self, git_status, set_line, Git};
use super::*;
use crate::ai::key::MemoryVault;
use crate::secrets::{MemoryStore, SecretValue, Store};
use std::cell::RefCell;

const VALUE: &str = "placeholder-SENTINEL-4242";

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("glim-rb-{tag}-{}-{:?}", std::process::id(), std::thread::current().id()).replace(['(', ')'], ""));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn sh(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git").args(args).current_dir(dir).output().unwrap().status.success();
    assert!(ok, "git {args:?}");
}

/// A temp git repo with `.env.local` ignored and `tracked.env` tracked.
fn repo(tag: &str) -> PathBuf {
    let d = scratch(tag);
    sh(&d, &["init", "-q"]);
    std::fs::write(d.join(".gitignore"), ".env.local\n").unwrap();
    std::fs::write(d.join("tracked.env"), "A=1\n").unwrap();
    // Ignored by a pattern, but added anyway: tracked beats ignored.
    std::fs::write(d.join(".gitignore"), ".env.local\ntracked.env\n").unwrap();
    sh(&d, &["add", "-f", "tracked.env"]);
    d
}

fn p(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn book(dir: &Path, name: &str, text: &str) {
    std::fs::write(dir.join(format!("{name}.toml")), text).unwrap();
}

struct Fake {
    store: MemoryStore,
    /// Stands in for %LOCALAPPDATA%\Glim\backups.
    backups: PathBuf,
    started: RefCell<Vec<String>>,
    exit: i32,
    port: bool,
    status: u16,
}

impl Default for Fake {
    fn default() -> Self {
        Fake { store: MemoryStore::default(), backups: scratch("backups"), started: RefCell::new(vec![]), exit: 0, port: true, status: 200 }
    }
}

impl World for Fake {
    fn secret(&self, name: &str) -> Result<Option<SecretValue>, String> {
        crate::secrets::read(&self.store, name)
    }
    fn backups(&self, runbook: &str) -> PathBuf {
        self.backups.join(runbook)
    }
    fn start_app(&self, app: &App) -> bool {
        self.started.borrow_mut().push(format!("{app:?}"));
        true
    }
    fn run(&self, _command: &str, _folder: &Path, _timeout: Duration) -> Result<i32, String> {
        Ok(self.exit)
    }
    fn port_answers(&self, _port: u16) -> bool {
        self.port
    }
    fn url_status(&self, _url: &str) -> Result<u16, String> {
        Ok(self.status)
    }
    fn sleep(&self, _d: Duration) {}
}

fn env_book(repo: &Path, file: &str) -> String {
    format!(
        "description = \"test\"\n[allow]\nfolders = ['{}']\n[[step]]\ntype = \"env_secret\"\nsecret = \"EXAMPLE_TOKEN\"\nfile = '{}'\nkey = \"EXAMPLE_KEY\"\n[[step]]\ntype = \"check\"\nfile_exists = '{}'\n",
        p(repo),
        p(&repo.join(file)),
        p(&repo.join(file)),
    )
}

// ── The file ──────────────────────────────────────────────────────────────────

#[test]
fn a_good_runbook_parses_and_describes_steps_with_names_only() {
    let d = repo("parse");
    let b = parse("example", env_book(&d, ".env.local").as_bytes()).unwrap();
    assert_eq!(b.steps.len(), 2);
    assert_eq!(b.steps[0].step.kind(), "env_secret");
    let text = b.steps[0].step.describe();
    assert!(text.contains("EXAMPLE_TOKEN") && text.contains("EXAMPLE_KEY") && text.contains(".env.local"), "{text}");
    assert_eq!(b.hash.len(), 64);
}

#[test]
fn bad_runbooks_are_refused_with_the_step_named() {
    let d = repo("bad");
    let f = p(&d.join(".env.local"));
    let folder = p(&d);
    let cases: Vec<(String, &str)> = vec![
        ("[[step]]\ntype = \"delete\"\n".into(), "unknown step type"),
        ("".into(), "no [[step]]"),
        (format!("[[step]]\ntype = \"env_secret\"\nsecret = \"X\"\nfile = '{f}'\nkey = \"K\"\n"), "outside the allowed folders"),
        (format!("[allow]\nfolders = ['{folder}']\n[[step]]\ntype = \"env_secret\"\nsecret = \"X\"\nfile = '{f}'\nkey = \"1K\"\n"), "key"),
        (format!("[allow]\nfolders = ['{folder}']\n[[step]]\ntype = \"env_secret\"\nsecret = \"X\"\nfile = '{f}'\nkey = \"K\"\ncommand = \"x\"\n"), "can't have command"),
        (format!("[allow]\nfolders = ['{folder}']\ncommands = [\"npm test\"]\n[[step]]\ntype = \"run\"\ncommand = \"npm run build\"\nfolder = '{folder}'\n"), "not exactly in [allow] commands"),
        (format!("[allow]\nfolders = ['{folder}']\ncommands = [\"npm test && del x\"]\n[[step]]\ntype = \"run\"\ncommand = \"npm test && del x\"\nfolder = '{folder}'\n"), "no shell operators"),
        ("[allow]\nfolders = ['C:\\']\n[[step]]\ntype = \"check\"\nport = 1\n".into(), "can't be allowed"),
        ("[[step]]\ntype = \"check\"\nurl = \"https://example.com/\"\n".into(), "on this PC"),
        ("[[step]]\ntype = \"check\"\nport = 80\nurl = \"http://127.0.0.1/\"\n".into(), "exactly one"),
        ("[[step]]\ntype = \"start_app\"\napp = \"example-app:\"\n".into(), "not in [allow] apps"),
        ("[allow]\napps = [\"https://example.com\"]\n[[step]]\ntype = \"check\"\nport = 1\n".into(), "can't be a runbook app"),
        ("[[step]]\ntype = \"check\"\nport = 1\nsurprise = 1\n".into(), "not valid runbook TOML"),
        ("[[step]]\ntype = \"check\"\nfile_exists = 'relative\\path'\n".into(), "full local path"),
    ];
    for (text, want) in cases {
        let e = parse("example", text.as_bytes()).unwrap_err();
        assert!(e.contains(want), "{want:?} not in {e:?} for {text}");
    }
    assert!(parse("has space", b"[[step]]\ntype = \"check\"\nport = 1\n").is_err());
}

#[test]
fn allowlisted_apps_commands_and_checks_parse() {
    let d = repo("apps");
    let folder = p(&d);
    let text = format!(
        "[allow]\nfolders = ['{folder}']\napps = [\"example-app://open\"]\ncommands = [\"npm run dev\"]\n[[step]]\ntype = \"start_app\"\napp = \"example-app://open\"\n[[step]]\ntype = \"run\"\ncommand = \"npm run dev\"\nfolder = '{folder}'\ntimeout_s = 30\nask = true\n[[step]]\ntype = \"check\"\nport = 5173\nwait_s = 20\n[[step]]\ntype = \"check\"\nurl = \"http://127.0.0.1:5173/\"\n"
    );
    let b = parse("example", text.as_bytes()).unwrap();
    assert_eq!(b.steps.iter().map(|s| s.step.kind()).collect::<Vec<_>>(), ["start_app", "run", "check", "check"]);
    assert!(b.steps[1].ask && !b.steps[0].ask);
    assert!(b.steps[1].step.describe().contains("npm run dev"));
}

// ── Approval ──────────────────────────────────────────────────────────────────

#[test]
fn hmac_matches_rfc_4231() {
    let mac: String = hmac_sha256(b"Jefe", b"what do ya want for nothing?").iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(mac, "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
}

#[test]
fn any_change_to_the_file_needs_approval_again() {
    let d = repo("approve");
    let books = scratch("approve-books");
    let approvals = books.join("approved.json");
    let vault = MemoryVault::default();
    book(&books, "example", &env_book(&d, ".env.local"));
    let (_, b) = load_from(&books, "example").unwrap();
    let b = b.unwrap();
    let key = || approval_key(&vault, false).unwrap();
    assert_eq!(state(&b, &read_approvals(&approvals), key().as_deref()), State::New);
    // Approving what the owner saw.
    assert!(approve(&b, "not-the-hash", &approvals, &vault, 1).is_err());
    approve(&b, &b.hash, &approvals, &vault, 1).unwrap();
    assert_eq!(state(&b, &read_approvals(&approvals), key().as_deref()), State::Approved);
    // One more comment line: changed.
    book(&books, "example", &format!("# edited\n{}", env_book(&d, ".env.local")));
    let (_, b2) = load_from(&books, "example").unwrap();
    let b2 = b2.unwrap();
    assert_ne!(b.hash, b2.hash);
    assert_eq!(state(&b2, &read_approvals(&approvals), key().as_deref()), State::Changed);
    // A hand-edited approvals file (the new hash, no valid MAC): still changed.
    let mut forged = read_approvals(&approvals);
    forged.get_mut("example").unwrap().hash = b2.hash.clone();
    assert_eq!(state(&b2, &forged, key().as_deref()), State::Changed);
    // Approving again works.
    approve(&b2, &b2.hash, &approvals, &vault, 2).unwrap();
    assert_eq!(state(&b2, &read_approvals(&approvals), key().as_deref()), State::Approved);
}

// ── Running ───────────────────────────────────────────────────────────────────

fn approved_dir(tag: &str, text: &str) -> (PathBuf, Approvals, Vec<u8>) {
    let books = scratch(tag);
    book(&books, "example", text);
    let vault = MemoryVault::default();
    let (_, b) = load_from(&books, "example").unwrap();
    let path = books.join("approved.json");
    approve(&b.unwrap(), &sha256_hex(text.as_bytes()), &path, &vault, 1).unwrap();
    (books, read_approvals(&path), approval_key(&vault, false).unwrap().unwrap())
}

#[test]
fn the_command_line_refuses_unapproved_and_changed_runbooks() {
    let d = repo("cli");
    let books = scratch("cli-books");
    book(&books, "example", &env_book(&d, ".env.local"));
    let fake = Fake::default();
    let mut audit = Vec::new();
    let r = run_named(&books, &Approvals::new(), None, "example", None, Trigger::Cli, &[], &fake, &mut |l| audit.push(l), &mut |_| {});
    assert_eq!(r, Err(Refusal::NotApproved));
    assert_eq!(cli::exit_code(&r), 3);
    assert!(audit.iter().any(|l| l == "example cli refused not-approved"), "{audit:?}");
    assert!(!d.join(".env.local").exists(), "nothing ran");

    let (books, approvals, key) = approved_dir("cli-changed", &env_book(&d, ".env.local"));
    book(&books, "example", &format!("{}\n", env_book(&d, ".env.local")));
    let r = run_named(&books, &approvals, Some(&key), "example", None, Trigger::Cli, &[], &fake, &mut |_| {}, &mut |_| {});
    assert_eq!(r, Err(Refusal::Changed));
    assert_eq!(cli::exit_code(&r), 3);

    let r = run_named(&books, &approvals, Some(&key), "missing", None, Trigger::Cli, &[], &fake, &mut |_| {}, &mut |_| {});
    assert!(matches!(r, Err(Refusal::NotFound(_))));
    assert_eq!(cli::exit_code(&r), 2);
    assert_eq!(cli::requested(&["glim.exe".into(), "--run".into(), "example".into()]), Some(Some("example".into())));
    assert_eq!(cli::requested(&["glim.exe".into(), "--run".into()]), Some(None));
    assert_eq!(cli::requested(&["glim.exe".into()]), None);
}

#[test]
fn ask_steps_ask_every_run_and_the_command_line_refuses_them() {
    let d = repo("ask");
    let folder = p(&d);
    let text = format!("[allow]\nfolders = ['{folder}']\ncommands = [\"npm test\"]\n[[step]]\ntype = \"run\"\ncommand = \"npm test\"\nfolder = '{folder}'\nask = true\n");
    let (books, approvals, key) = approved_dir("ask-books", &text);
    let fake = Fake::default();
    let r = run_named(&books, &approvals, Some(&key), "example", None, Trigger::Cli, &[], &fake, &mut |_| {}, &mut |_| {});
    assert_eq!(r, Err(Refusal::Asks(vec![1])));
    assert_eq!(cli::exit_code(&r), 4);
    let r = run_named(&books, &approvals, Some(&key), "example", None, Trigger::Settings, &[1], &fake, &mut |_| {}, &mut |_| {});
    assert!(r.unwrap().iter().all(|o| o.ok));
}

#[test]
fn steps_stop_at_the_first_failure() {
    let d = repo("stop");
    let folder = p(&d);
    let text = format!("[allow]\nfolders = ['{folder}']\ncommands = [\"npm test\"]\n[[step]]\ntype = \"run\"\ncommand = \"npm test\"\nfolder = '{folder}'\n[[step]]\ntype = \"check\"\nport = 5173\n");
    let (books, approvals, key) = approved_dir("stop-books", &text);
    let fake = Fake { exit: 2, ..Default::default() };
    let mut audit = Vec::new();
    let r = run_named(&books, &approvals, Some(&key), "example", None, Trigger::Settings, &[], &fake, &mut |l| audit.push(l), &mut |_| {}).unwrap();
    assert_eq!(r.len(), 1);
    assert!(!r[0].ok && r[0].detail == "exit code 2");
    assert_eq!(cli::exit_code(&Ok(r)), 1);
    assert!(audit.last().unwrap().ends_with("end FAIL 0/2"), "{audit:?}");
}

// ── env_secret in a real git repo ─────────────────────────────────────────────

#[test]
fn env_secret_writes_an_ignored_file_keeping_other_lines_with_a_backup() {
    let d = repo("env");
    let env = d.join(".env.local");
    std::fs::write(&env, "# keep me\r\nOTHER=1\r\nEXAMPLE_KEY=old\r\nexport EXAMPLE_KEY=older\r\n").unwrap();
    assert_eq!(git_status(&env), Git::Ignored);
    let (books, approvals, key) = approved_dir("env-books", &env_book(&d, ".env.local"));
    let fake = Fake::default();
    fake.store.write("EXAMPLE_TOKEN", VALUE).unwrap();
    let mut audit = Vec::new();
    let mut shown = Vec::new();
    let r = run_named(&books, &approvals, Some(&key), "example", None, Trigger::Cli, &[], &fake, &mut |l| audit.push(l), &mut |o| shown.push(cli::step_line(o))).unwrap();
    assert!(r.iter().all(|o| o.ok), "{r:?}");
    let text = std::fs::read_to_string(&env).unwrap();
    assert_eq!(text, format!("# keep me\r\nOTHER=1\r\nEXAMPLE_KEY={VALUE}\r\n"));
    // Nothing new in the repo: no backup or temp file beside it, no
    // info/exclude entry; `git status` shows nothing new.
    let beside: Vec<_> = std::fs::read_dir(&d).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with(".env.local.")).collect();
    assert!(beside.is_empty(), "{beside:?}");
    let exclude = std::fs::read_to_string(d.join(".git").join("info").join("exclude")).unwrap_or_default();
    assert!(!exclude.contains("glim"), "{exclude}");
    let porcelain = std::process::Command::new("git").args(["status", "--porcelain", "--ignored"]).current_dir(&d).output().unwrap();
    let porcelain = String::from_utf8_lossy(&porcelain.stdout);
    assert!(!porcelain.contains("bak") && !porcelain.contains("tmp"), "{porcelain}");
    // The dated backup is in the runbook's own backup folder, outside the repo.
    let folder = fake.backups.join("example");
    let backups: Vec<_> = std::fs::read_dir(&folder).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(backups.len(), 1, "{backups:?}");
    assert!(backups[0].starts_with(&env_file::backup_prefix(&std::fs::canonicalize(&env).unwrap())), "{backups:?}");
    assert!(backups[0].contains(".bak-2"), "{backups:?}");
    assert_eq!(std::fs::read_to_string(folder.join(&backups[0])).unwrap(), "# keep me\r\nOTHER=1\r\nEXAMPLE_KEY=old\r\nexport EXAMPLE_KEY=older\r\n");
    // The value is in no log line and no printed line.
    for l in audit.iter().chain(shown.iter()) {
        assert!(!l.contains("SENTINEL"), "{l}");
    }
}

#[test]
fn a_backup_folder_inside_a_repository_is_refused() {
    let d = repo("bkrepo");
    let env = d.join(".env.local");
    std::fs::write(&env, "A=1\n").unwrap();
    let (books, approvals, key) = approved_dir("bkrepo-books", &env_book(&d, ".env.local"));
    let fake = Fake { backups: repo("bkrepo-target"), ..Default::default() };
    fake.store.write("EXAMPLE_TOKEN", VALUE).unwrap();
    let r = run_named(&books, &approvals, Some(&key), "example", None, Trigger::Cli, &[], &fake, &mut |_| {}, &mut |_| {}).unwrap();
    assert!(!r[0].ok && r[0].detail.contains("inside a git repository"), "{r:?}");
    assert_eq!(std::fs::read_to_string(&env).unwrap(), "A=1\n", "untouched");
}

#[test]
fn backups_keep_the_newest_three_per_file() {
    let d = repo("prune");
    let env = d.join(".env.local");
    std::fs::write(&env, "A=1\n").unwrap();
    let fake = Fake::default();
    let folder = fake.backups.join("example");
    std::fs::create_dir_all(&folder).unwrap();
    let file = std::fs::canonicalize(&env).unwrap();
    let prefix = env_file::backup_prefix(&file);
    for stamp in ["20200101-000000", "20200102-000000", "20200103-000000", "20200104-000000"] {
        std::fs::write(folder.join(format!("{prefix}{stamp}")), "old").unwrap();
    }
    std::fs::write(folder.join("other-file.aaaaaaaa.bak-20200101-000000"), "kept").unwrap();
    env_file::write_secret(&file, "K", &crate::secrets::read(&{ let s = MemoryStore::default(); s.write("X", VALUE).unwrap(); s }, "X").unwrap().unwrap(), &folder).unwrap();
    let mut names: Vec<_> = std::fs::read_dir(&folder).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(names.iter().filter(|n| n.starts_with(&prefix)).count(), 3, "{names:?}");
    assert!(!names.iter().any(|n| n.ends_with("20200101-000000") && n.starts_with(&prefix)), "the oldest went: {names:?}");
    assert!(names.contains(&"other-file.aaaaaaaa.bak-20200101-000000".to_string()), "another file's backups stay");
}

#[test]
fn env_secret_refuses_tracked_and_not_ignored_files() {
    let d = repo("refuse");
    std::fs::write(d.join("plain.env"), "A=1\n").unwrap();
    let fake = Fake::default();
    fake.store.write("EXAMPLE_TOKEN", VALUE).unwrap();
    for (file, want) in [("tracked.env", "tracked by git"), ("plain.env", "not ignored by git")] {
        let before = std::fs::read_to_string(d.join(file)).unwrap();
        let (books, approvals, key) = approved_dir(&format!("refuse-{file}"), &env_book(&d, file));
        let r = run_named(&books, &approvals, Some(&key), "example", None, Trigger::Cli, &[], &fake, &mut |_| {}, &mut |_| {}).unwrap();
        assert!(!r[0].ok && r[0].detail.contains(want), "{file}: {r:?}");
        assert!(!r[0].detail.contains("SENTINEL"));
        assert_eq!(r.len(), 1, "stopped at the refusal");
        assert_eq!(std::fs::read_to_string(d.join(file)).unwrap(), before, "{file} untouched");
    }
    // Outside any repo.
    let bare = scratch("norepo");
    let (books, approvals, key) = approved_dir("refuse-norepo", &env_book(&bare, ".env.local"));
    let r = run_named(&books, &approvals, Some(&key), "example", None, Trigger::Cli, &[], &fake, &mut |_| {}, &mut |_| {}).unwrap();
    assert!(!r[0].ok && r[0].detail.contains("not in a git repository"), "{r:?}");
    assert!(!bare.join(".env.local").exists());
}

#[test]
fn a_missing_secret_fails_the_step_by_name() {
    let d = repo("nosecret");
    let (books, approvals, key) = approved_dir("nosecret-books", &env_book(&d, ".env.local"));
    let r = run_named(&books, &approvals, Some(&key), "example", None, Trigger::Cli, &[], &Fake::default(), &mut |_| {}, &mut |_| {}).unwrap();
    assert!(!r[0].ok && r[0].detail.contains("no secret called EXAMPLE_TOKEN"), "{r:?}");
}

#[test]
fn set_line_replaces_keeps_and_appends() {
    assert_eq!(set_line("", "K", "v"), "K=v\n");
    assert_eq!(set_line("A=1\n", "K", "v"), "A=1\nK=v\n");
    assert_eq!(set_line("A=1", "K", "v"), "A=1\nK=v\n");
    assert_eq!(set_line("K=old\nA=1\nK = dup\n", "K", "v"), "K=v\nA=1\n");
    assert_eq!(set_line("export K=old\n", "K", "v"), "export K=v\n");
    assert_eq!(set_line("KEY2=x\nK=old", "K", "v"), "KEY2=x\nK=v");
    assert_eq!(set_line("A=1\r\nK=old\r\n", "K", "v"), "A=1\r\nK=v\r\n");
    assert_eq!(set_line("A=1\n", "K", "has space \"q\""), "A=1\nK=\"has space \\\"q\\\"\"\n");
    assert_eq!(env_file::quoted("sb-abc_123.x/y+z="), "sb-abc_123.x/y+z=");
}

#[test]
fn nothing_logs_or_prints_a_value() {
    for src in [include_str!("mod.rs"), include_str!("env_file.rs"), include_str!("cli.rs")] {
        for line in src.lines().filter(|l| l.contains("audit(") || l.contains("audit_line(") || l.contains("log::line") || l.contains("writeln!")) {
            for banned in ["reveal", "value"] {
                assert!(!line.contains(banned), "{banned} in: {line}");
            }
        }
    }
}

// ── Audit and voice ───────────────────────────────────────────────────────────

#[test]
fn the_history_reads_the_last_runs_newest_first() {
    let log = "2026-11-12 10:00:00 a settings start 2 steps\n2026-11-12 10:00:01 a settings step 1/2 check OK (exists)\n2026-11-12 10:00:02 a settings end OK 2/2\n2026-11-12 11:00:00 b cli refused not-approved\n2026-11-12 12:00:00 a voice end FAIL 1/2\n";
    let h = history(log, 20);
    assert_eq!(h.len(), 3);
    assert_eq!((h[0].name.as_str(), h[0].trigger.as_str(), h[0].outcome.as_str(), h[0].done, h[0].total), ("a", "voice", "FAIL", 1, 2));
    assert_eq!(h[1].outcome, "refused not-approved");
    assert_eq!(h[2].time, "2026-11-12 10:00:02");
    assert_eq!(history(log, 1).len(), 1);
}

#[test]
fn the_audit_log_has_times_and_outcomes_only() {
    let d = scratch("audit");
    let path = d.join("runbooks.log");
    audit_line(&path, "example cli end OK 2/2");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.trim_end().ends_with(" example cli end OK 2/2"), "{text}");
    assert_eq!(history(&text, 20).len(), 1);
}

#[test]
fn voice_matches_run_and_a_runbook_name_on_this_pc() {
    let names = vec!["example-site".to_string(), "deploy".to_string()];
    assert_eq!(voice_match("Run example site.", &names).as_deref(), Some("example-site"));
    assert_eq!(voice_match("um please run the deploy runbook", &names).as_deref(), Some("deploy"));
    assert_eq!(voice_match("run deploy now", &names).as_deref(), Some("deploy"));
    assert_eq!(voice_match("run something else", &names), None);
    assert_eq!(voice_match("deploy", &names), None);
    assert_eq!(voice_match("I want to run deploy", &names), None);
}

#[test]
fn names_are_file_stems_and_only_toml() {
    let d = scratch("names");
    std::fs::write(d.join("b.toml"), "").unwrap();
    std::fs::write(d.join("A.toml"), "").unwrap();
    std::fs::write(d.join("notes.txt"), "").unwrap();
    assert_eq!(names_in(&d), ["A", "b"]);
}

/// examples/runbook.example.toml (the repo's only runbook) is valid runbook
/// TOML, and its paths are placeholders only.
#[test]
fn the_example_runbook_is_valid_and_has_placeholders_only() {
    let text = include_str!("../../../../examples/runbook.example.toml");
    let raw: RawBook = toml::from_str(text).unwrap();
    assert_eq!(raw.step.iter().map(|s| s.kind.as_str()).collect::<Vec<_>>(), ["env_secret", "check"]);
    for line in text.lines().filter(|l| l.contains(":\\")) {
        assert!(line.contains(r"C:\path\to\"), "not a placeholder path: {line}");
    }
}
