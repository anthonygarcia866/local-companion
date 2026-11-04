// Live revisions' engine: when a paragraph goes out, one at a time, stale
// and too-different answers dropped, a shown one hidden when typing resumes.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use super::*;
use crate::writing::{Observed, Rules};

const FIRST: &str = "We recieved the payment and will deposit it";
const TYPED: &str = "We recieved the payment and will deposit it tomorow.";

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn cfg() -> Config {
    Config::default()
}

fn see(s: &mut Session, same: bool, text: Option<&str>, at: Instant) {
    let r = Observed { same, text, caret: text.map(|t| t.chars().count()), rules: Rules::Full, mode: FixMode::Apply, app: "Notepad", paused: false, sensitive: false };
    s.observe_in(&r, at, |_| {});
}

/// A field typed into: a first reading, then the paragraph edited, then
/// readings every 300 ms while the user pauses, up to `pause_ms`.
fn typed(pause_ms: u64) -> (Session, Instant) {
    let mut s = Session::default();
    let t0 = Instant::now();
    see(&mut s, false, Some(FIRST), t0);
    see(&mut s, true, Some(TYPED), t0 + ms(100));
    let mut at = 100;
    while at < 100 + pause_ms {
        at = (at + 300).min(100 + pause_ms);
        see(&mut s, true, Some(TYPED), t0 + ms(at));
    }
    (s, t0 + ms(at))
}

#[test]
fn a_paused_edited_paragraph_goes_out_once() {
    let (mut s, now) = typed(1400);
    assert!(s.live_take(now, &cfg(), "Notepad.exe", false).is_none(), "1.4 s is not yet the pause");
    let (mut s, now) = typed(1500);
    let src = s.live_take(now, &cfg(), "Notepad.exe", false).expect("due at 1.5 s");
    assert_eq!(src.paragraph, TYPED);
    assert!(src.paused_for >= ms(1500));
    assert!(!format!("{src:?}").contains("payment"), "Debug shows no text");
    // That version doesn't go again, even after its answer.
    assert_eq!(s.live_answer(src.id, TYPED, now), Outcome::Unchanged);
    assert!(s.live_take(now + ms(5000), &cfg(), "Notepad.exe", false).is_none());
}

#[test]
fn the_pause_delay_is_the_settings() {
    let slow = Config { pause: ms(3000), ..cfg() };
    let (mut s, now) = typed(2000);
    assert!(s.live_take(now, &slow, "Notepad.exe", false).is_none());
    let mut settings = crate::settings::Settings::default();
    settings.live_pause_ms = 99_999;
    assert_eq!(Config::from_settings(&settings).pause, ms(MAX_PAUSE_MS), "clamped to 3 s");
    settings.live_pause_ms = 10;
    assert_eq!(Config::from_settings(&settings).pause, ms(MIN_PAUSE_MS), "clamped to 1 s");
}

#[test]
fn not_for_a_paragraph_that_wasnt_edited_or_is_short() {
    // Only read, never typed in: the caret just landed in old text.
    let mut s = Session::default();
    let t0 = Instant::now();
    for k in 0..10 {
        see(&mut s, k > 0, Some(TYPED), t0 + ms(300 * k));
    }
    assert!(s.live_take(t0 + ms(3000), &cfg(), "Notepad.exe", false).is_none(), "not edited");
    // Four words.
    let mut s = Session::default();
    see(&mut s, false, Some("Thanks for the"), t0);
    see(&mut s, true, Some("Thanks for the update"), t0 + ms(100));
    see(&mut s, true, Some("Thanks for the update"), t0 + ms(2000));
    assert!(s.live_take(t0 + ms(2000), &cfg(), "Notepad.exe", false).is_none(), "under 5 words");
}

#[test]
fn the_switches_master_and_per_app() {
    let (mut s, now) = typed(1600);
    let off = Config { on: false, ..cfg() };
    assert!(s.live_take(now, &off, "Notepad.exe", false).is_none());
    let mut apps = BTreeMap::new();
    apps.insert("notepad.exe".to_string(), false);
    let notepad_off = Config { apps, ..cfg() };
    assert!(s.live_take(now, &notepad_off, "Notepad.exe", false).is_none());
    assert!(notepad_off.on_in("slack.exe") && notepad_off.on_in("ms-teams.exe"), "every other app stays on");
    assert!(s.live_take(now, &cfg(), "Notepad.exe", false).is_some());
}

#[test]
fn one_request_at_a_time_and_a_lost_one_is_given_up() {
    let (mut s, now) = typed(1600);
    let first = s.live_take(now, &cfg(), "Notepad.exe", false).unwrap();
    // The user types on and pauses again while the first is on its way.
    let more = format!("{TYPED} Thanks again");
    see(&mut s, true, Some(&more), now + ms(100));
    see(&mut s, true, Some(&more), now + ms(1700));
    assert!(s.live_take(now + ms(1700), &cfg(), "Notepad.exe", false).is_none(), "one in flight");
    // Its answer is stale now: the paragraph changed.
    assert_eq!(s.live_answer(first.id, "We received the payment and will deposit it tomorrow.", now + ms(1800)), Outcome::Stale);
    let second = s.live_take(now + ms(1800), &cfg(), "Notepad.exe", false).expect("the new version goes");
    assert_eq!(second.paragraph, more);
    // A request that never answers is given up after GIVE_UP.
    see(&mut s, true, Some(&format!("{more} today")), now + ms(1900));
    see(&mut s, true, Some(&format!("{more} today")), now + ms(1900) + GIVE_UP);
    assert!(s.live_take(now + ms(1900) + GIVE_UP, &cfg(), "Notepad.exe", false).is_some());
    // An unknown id is stale.
    assert_eq!(s.live_answer(999, "x", now), Outcome::Stale);
}

#[test]
fn an_edit_is_shown_as_a_word_diff_and_hidden_when_typing_resumes() {
    let (mut s, now) = typed(1600);
    let src = s.live_take(now, &cfg(), "Notepad.exe", false).unwrap();
    let fixed = "We received the payment and will deposit it tomorrow.";
    let Outcome::Shown(event) = s.live_answer(src.id, fixed, now) else { panic!("not shown") };
    assert_eq!(event.phase, "show");
    let kinds: Vec<(&str, &str)> = event.ops.iter().map(|o| (o.kind, o.text.as_str())).collect();
    assert_eq!(kinds, [("same", "We "), ("del", "recieved "), ("add", "received "), ("same", "the payment and will deposit it "), ("del", "tomorow."), ("add", "tomorrow.")]);
    assert!(!format!("{event:?}").contains("payment"), "Debug shows no text");
    let shown = s.live_shown().unwrap();
    assert_eq!((shown.paragraph.as_str(), shown.revised.as_str(), shown.start), (TYPED, fixed, 0));
    assert!(s.live_hidden().is_none());
    // Typing again: it goes, the island is told once.
    see(&mut s, true, Some(&format!("{TYPED} A")), now + ms(200));
    let hidden = s.live_hidden().unwrap();
    assert_eq!((hidden.phase, hidden.id, hidden.why), ("hide", src.id, "typing"));
    assert!(s.live_hidden().is_none());
    assert!(s.live_shown().is_none());
}

#[test]
fn a_field_change_or_pause_hides_it_too() {
    let (mut s, now) = typed(1600);
    let src = s.live_take(now, &cfg(), "Notepad.exe", false).unwrap();
    assert!(matches!(s.live_answer(src.id, "We received the payment and will deposit it tomorrow.", now), Outcome::Shown(_)));
    see(&mut s, false, Some("Another field entirely here"), now + ms(300));
    assert_eq!(s.live_hidden().unwrap().why, "field");
}

#[test]
fn a_revision_changing_most_words_is_dropped() {
    let (mut s, now) = typed(1600);
    let src = s.live_take(now, &cfg(), "Notepad.exe", false).unwrap();
    let rewritten = "Payment arrived; deposit scheduled for tomorrow morning at the bank.";
    assert!(changed_share(TYPED, rewritten) > MAX_CHANGED);
    assert_eq!(s.live_answer(src.id, rewritten, now), Outcome::TooDifferent);
    assert!(s.live_shown().is_none());
}

#[test]
fn changed_share_and_diff() {
    assert_eq!(changed_share("a b c d", "a b c d"), 0.0);
    assert_eq!(changed_share("a b c d", "a b x d"), 0.25);
    assert_eq!(changed_share("a b", "c d e f"), 1.0);
    assert_eq!(changed_share("", ""), 0.0);
    let ops = word_diff("Hi  there", "Hi there friend");
    let text: String = ops.iter().filter(|o| o.kind != "del").map(|o| o.text.as_str()).collect();
    assert_eq!(text, "Hi there friend", "the kept and added pieces spell the new text");
    let old: String = word_diff(TYPED, "We received it.").iter().filter(|o| o.kind != "add").map(|o| o.text.as_str()).collect();
    assert_eq!(old, TYPED, "the kept and removed pieces spell the old text");
}

#[test]
fn the_context_is_the_paragraph_before() {
    let field = "Hi Dana,\n\nThanks for the update.\n\nWe recieved the payment and will deposit it tomorow.";
    let mut s = Session::default();
    let t0 = Instant::now();
    see(&mut s, false, Some(&field[..field.len() - 9]), t0);
    see(&mut s, true, Some(field), t0 + ms(100));
    see(&mut s, true, Some(field), t0 + ms(1700));
    let src = s.live_take(t0 + ms(1700), &cfg(), "Notepad.exe", false).unwrap();
    assert_eq!(src.before, "Thanks for the update.");
    assert_eq!(crate::writing::text::paragraph_before("only one", 0), "");
}
