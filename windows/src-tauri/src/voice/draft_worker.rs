// The "Draft email" card's Rust side (voice 1.5d; draft.rs holds the rules,
// outlook.rs talks to Outlook). One thread, a COM single-threaded apartment,
// owns Outlook and the card while it's up; the island sends ids, indexes and
// a search's text (draft_* commands in lib.rs) and hears "draft-email".
//
// Decision B (2026-10-16): while the card still needs a recipient (a pick,
// nothing found, or Change recipient open), WAITING is set and the next
// utterance from either voice key is its answer (voice::run → `answer`).
//
// GLIM_DRAFT_DRY_RUN=1 (decided 2026-10-16, the testing plan): the address
// book is searched for real, nothing is saved or opened.
//
// The log gets phases, counts and codes; never a name, an address, the
// subject or the body.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use super::draft::{self, DryRun, Outlook, Pending, Spoken};

pub const EVENT: &str = "draft-email";

/// What the island hears.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftEvent<'a> {
    /// "card" (show or redraw it), "created" (saved in Drafts and opened),
    /// "closed".
    pub phase: &'static str,
    pub card: Option<&'a Pending>,
    /// Why the last step didn't work ("no-outlook", "no-account",
    /// "no-sender", "outlook-failed"), or "".
    pub code: &'static str,
    /// The dry run: nothing is saved or opened.
    pub dry_run: bool,
    /// The card can be created (every name has one address).
    pub ready: bool,
}

enum Job {
    Begin(Spoken),
    Pick { id: u64, index: usize, choice: usize },
    Change { id: u64, index: usize },
    Search { id: u64, query: String },
    Create { id: u64 },
    Cancel { id: u64 },
    /// A voice answer for the recipient the card is asking about.
    Answer { text: String },
}

/// The card is up and still asking who an email goes to.
static WAITING: AtomicBool = AtomicBool::new(false);

/// Whether the next utterance answers the draft card (voice::run).
pub fn waiting() -> bool {
    WAITING.load(Ordering::SeqCst)
}

/// A voice answer for the card (Decision B): matched against its chips, or
/// looked up in the address book. Never logged.
pub fn answer(app: &AppHandle, text: String) {
    post(app, Job::Answer { text });
}

static JOBS: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();

fn dry_run() -> bool {
    std::env::var("GLIM_DRAFT_DRY_RUN").as_deref() == Ok("1")
}

fn post(app: &AppHandle, job: Job) {
    let tx = JOBS.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        let app = app.clone();
        std::thread::spawn(move || worker(app, rx));
        Mutex::new(tx)
    });
    let _ = tx.lock().unwrap_or_else(|e| e.into_inner()).send(job);
}

/// A `draft_email` call: resolve its names and show the card.
pub fn begin(app: &AppHandle, spoken: Spoken) {
    post(app, Job::Begin(spoken));
}
pub fn pick(app: &AppHandle, id: u64, index: usize, choice: usize) {
    post(app, Job::Pick { id, index, choice });
}
pub fn change(app: &AppHandle, id: u64, index: usize) {
    post(app, Job::Change { id, index });
}
pub fn search(app: &AppHandle, id: u64, query: String) {
    post(app, Job::Search { id, query });
}
pub fn create(app: &AppHandle, id: u64) {
    post(app, Job::Create { id });
}
pub fn cancel(app: &AppHandle, id: u64) {
    post(app, Job::Cancel { id });
}

/// The sender and the remembered picks, as settings have them now.
fn settings(app: &AppHandle) -> (String, BTreeMap<String, String>) {
    app.try_state::<crate::Shared>()
        .map(|s| {
            let s = s.settings.lock().unwrap();
            (s.draft_from.trim().to_string(), s.draft_recipients.clone())
        })
        .unwrap_or_default()
}

fn send(app: &AppHandle, phase: &'static str, card: Option<&Pending>, code: &'static str) {
    WAITING.store(phase == "card" && card.is_some_and(|c| c.asking().is_some()), Ordering::SeqCst);
    let e = DraftEvent { phase, card, code, dry_run: dry_run(), ready: card.is_some_and(Pending::ready) };
    let _ = app.emit_to(crate::island::WINDOW_LABEL, EVENT, &e);
}

fn worker(app: AppHandle, rx: std::sync::mpsc::Receiver<Job>) {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let mut outlook: Option<crate::outlook::ComOutlook> = None;
    let mut card: Option<Pending> = None;
    let mut next_id = 0u64;
    while let Ok(job) = rx.recv() {
        // Outlook is started (in the background) the first time it's needed.
        if outlook.is_none() {
            match crate::outlook::ComOutlook::connect() {
                Ok(o) => outlook = Some(o),
                Err(e) => {
                    crate::log::line(format!("draft: {}", e.code()));
                    if matches!(job, Job::Begin(_)) {
                        send(&app, "closed", None, e.code());
                    }
                    continue;
                }
            }
        }
        let Some(real) = outlook.as_ref() else { continue };
        let dry = DryRun(real);
        let o: &dyn Outlook = if dry_run() { &dry } else { real };
        let find = |q: &str| o.search(q).unwrap_or_default();
        let ours = |id: u64, card: &Option<Pending>| card.as_ref().is_some_and(|c| c.id == id);
        match job {
            Job::Begin(spoken) => {
                next_id += 1;
                let (me, remembered) = settings(&app);
                let p = draft::pending(next_id, &spoken, &me, &remembered, find);
                crate::log::line(format!(
                    "draft: card {} names, {} need a pick or a search{}",
                    p.recipients.len(),
                    p.recipients.iter().filter(|r| !matches!(r.found, draft::Found::One { .. })).count(),
                    if dry_run() { " (dry run)" } else { "" }
                ));
                send(&app, "card", Some(&p), "");
                card = Some(p);
            }
            Job::Pick { id, index, choice } if ours(id, &card) => {
                if let Some(c) = card.as_mut() {
                    c.pick(index, choice);
                }
                send(&app, "card", card.as_ref(), "");
            }
            Job::Change { id, index } if ours(id, &card) => {
                if let Some(c) = card.as_mut() {
                    c.change(index);
                }
                send(&app, "card", card.as_ref(), "");
            }
            Job::Search { id, query } if ours(id, &card) => {
                if let Some(c) = card.as_mut().filter(|c| c.changing.is_some()) {
                    let mut found = find(&query);
                    found.truncate(draft::MAX_CHOICES);
                    c.results = found;
                }
                send(&app, "card", card.as_ref(), "");
            }
            Job::Create { id } if ours(id, &card) => {
                let (from, _) = settings(&app);
                let Some(c) = card.as_ref() else { continue };
                match draft::create(o, &from, c, true) {
                    Ok(()) => {
                        let picks = c.remembered();
                        crate::log::line(format!("draft: saved in Drafts and opened{} ({} picks remembered)", if dry_run() { " (dry run: not really)" } else { "" }, picks.len()));
                        crate::remember_draft_picks(&app, &picks);
                        send(&app, "created", None, "");
                        card = None;
                    }
                    Err(e) => {
                        // The card stays open and says why.
                        crate::log::line(format!("draft: not saved ({})", e.code()));
                        send(&app, "card", card.as_ref(), e.code());
                    }
                }
            }
            Job::Answer { text } if card.is_some() => {
                let (me, remembered) = settings(&app);
                let filled = card.as_mut().is_some_and(|c| c.answer(&text, &me, &remembered, find));
                crate::log::line(format!("draft: voice answer {}", if filled { "used" } else { "matched nothing" }));
                // Nothing matched: the card says so, and still asks.
                send(&app, "card", card.as_ref(), if filled { "" } else { "no-match" });
            }
            Job::Cancel { id } if ours(id, &card) => {
                crate::log::line("draft: cancelled");
                card = None;
                send(&app, "closed", None, "");
            }
            // A click on a card that's gone: nothing.
            _ => {}
        }
    }
}
