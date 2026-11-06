// The shield's names from classic Outlook's address book (its Contacts and
// the offline global address list), read through local COM only while
// Outlook is already running: Glim attaches (GetActiveObject) and never
// starts it, like the calendar glance. Also its Contacts folders (a contact
// without an email address is in no address list): their full, first and
// last names and company. Only names are read,
// kept in memory (shield::set_address_book), never stored, logged or sent;
// the log gets a count.
//
// Its own thread, a COM STA: a look every LOOK while the shield is on,
// a read when Outlook has just come up and then every REREAD.

use std::time::{Duration, Instant};

use windows::core::{Interface, IUnknown};
use windows::Win32::System::Com::{CLSIDFromProgID, CoInitializeEx, IDispatch, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Ole::GetActiveObject;
use windows::Win32::System::Variant::VARIANT;

use crate::outlook::Disp;

/// How often Outlook's process is looked for (the process list only).
pub const LOOK: Duration = Duration::from_secs(60);
/// How often the names are read again while Outlook runs.
pub const REREAD: Duration = Duration::from_secs(30 * 60);
/// At most this many entries per address list (a large global list).
pub const MAX_ENTRIES: i32 = 20_000;
const OUTLOOK_EXE: &str = "OUTLOOK.EXE";

/// The running Outlook's MAPI namespace, or None (not running, starting).
fn namespace() -> Option<Disp> {
    let clsid = unsafe { CLSIDFromProgID(windows::core::w!("Outlook.Application")) }.ok()?;
    let mut unknown: Option<IUnknown> = None;
    unsafe { GetActiveObject(&clsid, None, &mut unknown) }.ok()?;
    let app: IDispatch = unknown?.cast().ok()?;
    Disp::from(app).call_obj("GetNamespace", &[VARIANT::from("MAPI")]).ok()
}

/// The display name of the Outlook account sending as `address` (Settings
/// → Email drafts → Your address), to seed My names and businesses.
pub fn account_name(address: &str) -> Option<String> {
    let accounts = namespace()?.obj("Accounts").ok()?;
    for i in 1..=accounts.int("Count") {
        let Ok(a) = accounts.item(i) else { continue };
        if !a.text("SmtpAddress").trim().eq_ignore_ascii_case(address) {
            continue;
        }
        let name = a.obj("CurrentUser").map(|u| u.text("Name")).unwrap_or_default();
        let name = name.trim();
        if !name.is_empty() && !name.contains('@') {
            return Some(name.to_string());
        }
    }
    None
}

const OL_FOLDER_CONTACTS: i32 = 10;
const OL_CONTACT: i32 = 40;
/// How deep into the Contacts folder's subfolders.
const MAX_DEPTH: usize = 3;

/// The names in Outlook's Contacts folder and its subfolders: a contact with
/// no email address is in no address list (found by the live test,
/// 2026-11-05: such a surname went out unmasked), so its full name, first
/// and last name and company are read here.
fn contact_names(ns: &Disp, out: &mut Vec<String>) {
    let Ok(root) = ns.call_obj("GetDefaultFolder", &[crate::outlook::int(OL_FOLDER_CONTACTS)]) else { return };
    let mut folders = vec![(root, 0usize)];
    let mut read = 0;
    while let Some((folder, depth)) = folders.pop() {
        if let Ok(items) = folder.obj("Items") {
            for i in 1..=items.int("Count") {
                if read >= MAX_ENTRIES {
                    return;
                }
                let Ok(c) = items.item(i) else { continue };
                // Contacts only (not distribution lists).
                if c.int("Class") != OL_CONTACT {
                    continue;
                }
                read += 1;
                for field in ["FullName", "FirstName", "LastName", "CompanyName"] {
                    let v = c.text(field);
                    if !v.trim().is_empty() {
                        out.push(v);
                    }
                }
            }
        }
        if depth < MAX_DEPTH {
            if let Ok(subs) = folder.obj("Folders") {
                for k in 1..=subs.int("Count") {
                    if let Ok(f) = subs.item(k) {
                        folders.push((f, depth + 1));
                    }
                }
            }
        }
    }
}

/// The running Outlook's address-book names (its address lists and its
/// Contacts folders), or None when Glim couldn't attach (not running, still
/// starting).
pub fn read_names() -> Option<Vec<String>> {
    let ns = namespace()?;
    let lists = ns.obj("AddressLists").ok()?;
    let mut names: Vec<String> = Vec::new();
    contact_names(&ns, &mut names);
    for l in 1..=lists.int("Count") {
        let Ok(list) = lists.item(l) else { continue };
        let Ok(entries) = list.obj("AddressEntries") else { continue };
        for i in 1..=entries.int("Count").min(MAX_ENTRIES) {
            let Ok(e) = entries.item(i) else { continue };
            let name = e.text("Name");
            if !name.trim().is_empty() {
                names.push(name);
            }
        }
    }
    names.sort();
    names.dedup();
    Some(names)
}

/// Starts the thread (lib.rs, at launch). `seeded` gets the draft account's
/// name once, when My names has yet to be seeded (lib.rs saves it).
pub fn spawn(seeded: impl Fn(String) + Send + 'static) {
    std::thread::spawn(move || {
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        let mut last_read: Option<Instant> = None;
        let mut was_running = false;
        loop {
            let running = super::enabled() && crate::platform::process_running(OUTLOOK_EXE);
            let due = last_read.is_none_or(|t| t.elapsed() >= REREAD) || (running && !was_running);
            if running && due {
                if let Some(names) = read_names() {
                    crate::log::line(format!("shield: address book read, {} entries", names.len()));
                    super::set_address_book(names);
                    if let Some(name) = super::seed_from().and_then(|a| account_name(&a)) {
                        super::set_seed(&name);
                        seeded(name);
                    }
                    last_read = Some(Instant::now());
                    was_running = true;
                }
            } else if !running {
                was_running = false;
            }
            std::thread::sleep(LOOK);
        }
    });
}
