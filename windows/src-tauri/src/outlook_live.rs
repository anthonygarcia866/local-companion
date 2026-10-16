// Live checks for draft_email against the real classic Outlook (voice 1.5d,
// the testing plan decided 2026-10-16), run by hand, never in CI:
//
//   GLIM_LIVE_ME=<the owner's address> cargo test --lib outlook_live -- --ignored --nocapture --test-threads=1
//
// - `dry_run_probe` (read-only): Outlook starts in the background; the
//   sender's account is found by its address; the address book is counted;
//   a first name with two or more entries is found for test 2; whether a new
//   message gets a default signature when its window is created (not shown).
//   Nothing is saved.
// - `three_test_drafts` (GLIM_LIVE_DRAFTS=1 as well): three "[GLIM TEST]"
//   drafts addressed to the owner himself, through Glim's own code (resolve,
//   the card, create), checked in his own account's Drafts (never another
//   account), then deleted, from Deleted Items too. Never sent, never shown.
//
// Printed: counts, yes/no, and the one first name used for test 2. Never an
// address, never a body.

use std::collections::BTreeMap;

use crate::outlook::{int, ComOutlook, Disp};
use crate::voice::draft::{self, Contact, Found, Outlook, Spoken};

const PREFIX: &str = "[GLIM TEST]";
const DELETED: i32 = 3; // olFolderDeletedItems
const DRAFTS: i32 = 16;

fn me() -> String {
    std::env::var("GLIM_LIVE_ME").expect("GLIM_LIVE_ME")
}

fn connect() -> ComOutlook {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    ComOutlook::connect().expect("classic Outlook")
}

/// A folder of the account's own store.
fn folder(o: &ComOutlook, account: &Disp, which: i32) -> Disp {
    let _ = o;
    account.obj("DeliveryStore").unwrap().call_obj("GetDefaultFolder", &[int(which)]).unwrap()
}

/// The items in `f` whose subject starts with the test prefix.
fn test_items(f: &Disp) -> Vec<Disp> {
    let items = f.obj("Items").unwrap();
    (1..=items.int("Count")).filter_map(|i| items.item(i).ok()).filter(|m| m.text("Subject").starts_with(PREFIX)).collect()
}

#[test]
#[ignore = "live: starts classic Outlook in the background; needs GLIM_LIVE_ME"]
fn dry_run_probe() {
    let o = connect();
    let me = me();
    let accounts = o.ns.obj("Accounts").unwrap();
    println!("accounts: {}", accounts.int("Count"));
    let mine = o.account(&me);
    println!("an account with GLIM_LIVE_ME's address: {}", mine.is_some());
    assert!(mine.is_some());
    // The default account (Session's first), only whether it's the owner's.
    let first = accounts.item(1).map(|a| a.text("SmtpAddress").eq_ignore_ascii_case(&me)).unwrap_or(false);
    println!("the first account is the owner's: {first}");

    // The address book, and a first name with 2+ entries (test 2).
    let lists = o.ns.obj("AddressLists").unwrap();
    let mut firsts: BTreeMap<String, usize> = BTreeMap::new();
    let mut total = 0;
    for l in 1..=lists.int("Count") {
        let Ok(list) = lists.item(l) else { continue };
        let Ok(entries) = list.obj("AddressEntries") else { continue };
        let n = entries.int("Count");
        println!("address list {l}: {n} entries");
        for i in 1..=n.min(5000) {
            let Ok(e) = entries.item(i) else { continue };
            total += 1;
            let name = e.text("Name");
            let first = name.split(|c: char| c.is_whitespace() || c == ',').filter(|w| !w.is_empty()).next().unwrap_or("").to_string();
            if first.len() >= 3 && first.chars().all(char::is_alphabetic) {
                *firsts.entry(first).or_default() += 1;
            }
        }
    }
    println!("entries looked at: {total}");
    let t0 = std::time::Instant::now();
    let ambiguous = firsts.iter().filter(|(_, n)| **n >= 2).map(|(f, _)| f.clone()).find(|f| matches!(draft::resolve(f, &me, &BTreeMap::new(), |q| o.search(q).unwrap_or_default()), Found::Pick { .. }));
    println!("a first name with 2+ matches (Glim's resolve says Pick): {ambiguous:?} ({:.1}s)", t0.elapsed().as_secs_f64());

    // The signature: a new message in the owner's Drafts, its window created
    // (not shown); does Outlook put anything in it? Then thrown away unsaved.
    let account = mine.unwrap();
    let mail = folder(&o, &account, DRAFTS).obj("Items").unwrap().call_obj("Add", &[int(0)]).unwrap();
    let before = mail.text("Body");
    let _inspector = mail.obj("GetInspector");
    let after = mail.text("Body");
    println!("signature: empty before the window: {}, something after: {}", before.trim().is_empty(), crate::outlook::has_signature(&after));
    let _ = mail.call("Close", &[int(1)]); // olDiscard
}

#[test]
#[ignore = "live: saves 3 [GLIM TEST] drafts to the owner and deletes them; needs GLIM_LIVE_ME and GLIM_LIVE_DRAFTS=1"]
fn three_test_drafts() {
    assert_eq!(std::env::var("GLIM_LIVE_DRAFTS").as_deref(), Ok("1"));
    let o = connect();
    let me = me();
    let account = o.account(&me).expect("the owner's account");
    let drafts = folder(&o, &account, DRAFTS);
    assert!(test_items(&drafts).is_empty(), "no leftover test drafts before");
    let search = |q: &str| o.search(q).unwrap_or_default();
    let none = BTreeMap::new();

    // 1. "Draft an email to me": the sender's own address.
    let p1 = draft::pending(1, &Spoken { to: vec!["me".into()], subject: format!("{PREFIX} 1 to me"), body: "Glim test 1: a draft to myself.\n\nNothing was sent.".into(), ..Default::default() }, &me, &none, search);
    assert!(p1.ready());
    draft::create(&o, &me, &p1, false).expect("draft 1");

    // 2. An ambiguous first name: the card offers chips; "Change recipient"
    //    then puts the owner's own address in (the draft only ever goes to him).
    let name = std::env::var("GLIM_LIVE_AMBIGUOUS").expect("GLIM_LIVE_AMBIGUOUS (from dry_run_probe)");
    let mut p2 = draft::pending(2, &Spoken { to: vec![name.clone()], subject: format!("{PREFIX} 2 ambiguous name changed to me"), body: "Glim test 2.".into(), ..Default::default() }, &me, &none, search);
    let Found::Pick { choices } = &p2.recipients[0].found else { panic!("{name}: not ambiguous") };
    println!("test 2: '{name}' offered {} chips", choices.len());
    assert!(!p2.ready(), "Create draft waits for a pick");
    assert!(p2.change(0));
    p2.results = vec![Contact { name: "me".into(), address: me.clone() }];
    assert!(p2.pick(0, 0));
    assert!(p2.ready());
    assert_eq!(p2.remembered().len(), 1, "the pick would be remembered (not saved by this test)");
    draft::create(&o, &me, &p2, false).expect("draft 2");

    // 3. CC only when spoken: to me, cc me.
    let p3 = draft::pending(3, &Spoken { to: vec!["me".into()], cc: vec!["me".into()], subject: format!("{PREFIX} 3 cc spoken"), body: "Glim test 3: with a CC.".into(), ..Default::default() }, &me, &none, search);
    draft::create(&o, &me, &p3, false).expect("draft 3");

    // All three in the owner's own Drafts, from his account, to him only.
    let saved = test_items(&drafts);
    println!("test drafts in the owner's Drafts: {}", saved.len());
    assert_eq!(saved.len(), 3);
    for m in &saved {
        let from = m.obj("SendUsingAccount").map(|a| a.text("SmtpAddress")).unwrap_or_default();
        assert!(from.eq_ignore_ascii_case(&me), "sent (by him) from his own account");
        assert!(!m.get("Sent").ok().and_then(|v| bool::try_from(&v).ok()).unwrap_or(true), "never sent");
        let recips = m.obj("Recipients").unwrap();
        let n = recips.int("Count");
        assert!(n >= 1);
        for i in 1..=n {
            let r = recips.item(i).unwrap();
            let a = r.text("Address");
            let smtp = r.obj("AddressEntry").ok().and_then(|e| e.call_obj("GetExchangeUser", &[]).ok()).map(|u| u.text("PrimarySmtpAddress")).filter(|s| !s.is_empty()).unwrap_or(a);
            assert!(smtp.eq_ignore_ascii_case(&me), "every recipient is the owner");
        }
        let cc = m.text("CC");
        println!("{}: {} recipient(s), cc: {}", m.text("Subject"), n, !cc.is_empty());
    }

    // Deleted afterward: from Drafts, then from Deleted Items.
    for m in saved {
        m.call("Delete", &[]).unwrap();
    }
    let deleted = folder(&o, &account, DELETED);
    for m in test_items(&deleted) {
        m.call("Delete", &[]).unwrap();
    }
    assert!(test_items(&drafts).is_empty() && test_items(&deleted).is_empty(), "nothing left");
    println!("all test drafts deleted (Drafts and Deleted Items)");
}
