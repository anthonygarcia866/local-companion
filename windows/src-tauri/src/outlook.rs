// Classic Outlook through COM (voice 1.5d, draft_email; decided 2026-10-16).
// Local only: Outlook.Application is an out-of-process COM server on this PC;
// Glim makes no network call (Outlook syncs the saved draft itself, as it
// would any draft).
//
// What Glim asks of Outlook, and nothing else:
// - search its address book (AddressLists: the Contacts and the offline
//   global address list) for entries whose name matches;
// - save a draft in the Drafts of the account whose address is the sender
//   (Settings → Email drafts → Your address), never another account (the
//   shared leasing@ mailbox is never the sender, owner 2026-10-16), and open
//   it in Outlook. Glim never sends: there is no Send call anywhere.
//
// Outlook adds the user's default signature when a new message's window is
// created; Glim creates it (GetInspector) before putting its text in, and
// puts the text above whatever Outlook put there, so the signature stays.
//
// Late-bound (IDispatch), on the calling thread, which must be a COM STA
// (voice::draft_worker). Names and addresses are never logged.

#![cfg(windows)]

use std::mem::ManuallyDrop;

use windows::core::{GUID, PCWSTR};
use windows::Win32::Foundation::{VARIANT_FALSE, VARIANT_TRUE};
use windows::Win32::System::Com::{
    CLSIDFromProgID, CoCreateInstance, IDispatch, CLSCTX_LOCAL_SERVER, DISPATCH_FLAGS, DISPATCH_METHOD, DISPATCH_PROPERTYGET, DISPATCH_PROPERTYPUT,
    DISPATCH_PROPERTYPUTREF, DISPPARAMS,
};
use windows::Win32::System::Ole::DISPID_PROPERTYPUT;
use windows::Win32::System::Variant::{VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_BOOL, VT_BSTR, VT_I4};

use crate::voice::draft::{looks_like_address, Contact, Draft, DraftError, Outlook};

/// Outlook's folder number for Drafts (olFolderDrafts).
const DRAFTS: i32 = 16;
/// olMailItem.
const MAIL_ITEM: i32 = 0;
/// olExchangeUserAddressEntry / olExchangeRemoteUserAddressEntry.
const EXCHANGE_USERS: [i32; 2] = [0, 5];
/// Entries looked at per address list, at most (a large directory is cut).
const MAX_ENTRIES: i32 = 5000;

/// An integer argument (VT_I4).
pub(crate) fn int(n: i32) -> VARIANT {
    VARIANT { Anonymous: VARIANT_0 { Anonymous: ManuallyDrop::new(VARIANT_0_0 { vt: VT_I4, wReserved1: 0, wReserved2: 0, wReserved3: 0, Anonymous: VARIANT_0_0_0 { lVal: n } }) } }
}

/// A boolean argument (VT_BOOL).
fn boolean(b: bool) -> VARIANT {
    let v = if b { VARIANT_TRUE } else { VARIANT_FALSE };
    VARIANT { Anonymous: VARIANT_0 { Anonymous: ManuallyDrop::new(VARIANT_0_0 { vt: VT_BOOL, wReserved1: 0, wReserved2: 0, wReserved3: 0, Anonymous: VARIANT_0_0_0 { boolVal: v } }) } }
}

/// One COM object, by name.
#[derive(Clone)]
pub struct Disp(IDispatch);

impl Disp {
    fn id(&self, name: &str) -> windows::core::Result<i32> {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let names = [PCWSTR(wide.as_ptr())];
        let mut id = 0i32;
        unsafe { self.0.GetIDsOfNames(&GUID::zeroed(), names.as_ptr(), 1, 0x0800, &mut id)? };
        Ok(id)
    }

    fn invoke(&self, name: &str, flags: DISPATCH_FLAGS, args: &[VARIANT]) -> windows::core::Result<VARIANT> {
        let id = self.id(name)?;
        // IDispatch takes its arguments last first.
        let mut rev: Vec<VARIANT> = args.iter().rev().cloned().collect();
        let mut named = DISPID_PROPERTYPUT;
        let put = flags == DISPATCH_PROPERTYPUT || flags == DISPATCH_PROPERTYPUTREF;
        let params = DISPPARAMS {
            rgvarg: if rev.is_empty() { std::ptr::null_mut() } else { rev.as_mut_ptr() },
            rgdispidNamedArgs: if put { &mut named } else { std::ptr::null_mut() },
            cArgs: rev.len() as u32,
            cNamedArgs: u32::from(put),
        };
        let mut result = VARIANT::default();
        unsafe { self.0.Invoke(id, &GUID::zeroed(), 0x0800, flags, &params, Some(&mut result), None, None)? };
        Ok(result)
    }

    pub fn get(&self, name: &str) -> windows::core::Result<VARIANT> {
        self.invoke(name, DISPATCH_PROPERTYGET, &[])
    }

    pub fn obj(&self, name: &str) -> windows::core::Result<Disp> {
        Ok(Disp(IDispatch::try_from(&self.get(name)?)?))
    }

    pub fn text(&self, name: &str) -> String {
        // A string property: read the BSTR in place (checked by its type first).
        self.get(name).ok().filter(|v| v.vt() == VT_BSTR).map(|v| unsafe { v.Anonymous.Anonymous.Anonymous.bstrVal.to_string() }).unwrap_or_default()
    }

    pub fn int(&self, name: &str) -> i32 {
        self.get(name).ok().and_then(|v| i32::try_from(&v).ok()).unwrap_or(0)
    }

    pub fn call(&self, name: &str, args: &[VARIANT]) -> windows::core::Result<VARIANT> {
        self.invoke(name, DISPATCH_METHOD | DISPATCH_PROPERTYGET, args)
    }

    pub fn call_obj(&self, name: &str, args: &[VARIANT]) -> windows::core::Result<Disp> {
        Ok(Disp(IDispatch::try_from(&self.call(name, args)?)?))
    }

    pub fn put(&self, name: &str, value: VARIANT) -> windows::core::Result<()> {
        self.invoke(name, DISPATCH_PROPERTYPUT, &[value]).map(drop)
    }

    fn put_ref(&self, name: &str, value: &Disp) -> windows::core::Result<()> {
        let v = VARIANT::from(value.0.clone());
        self.invoke(name, DISPATCH_PROPERTYPUTREF, std::slice::from_ref(&v)).or_else(|_| self.invoke(name, DISPATCH_PROPERTYPUT, &[v])).map(drop)
    }

    /// Item `i` (1-based) of a collection.
    pub fn item(&self, i: i32) -> windows::core::Result<Disp> {
        self.call_obj("Item", &[int(i)])
    }
}

/// Classic Outlook on this PC (it starts in the background if it isn't running).
pub struct ComOutlook {
    /// Held so Outlook stays up while Glim uses it.
    pub _app: Disp,
    pub ns: Disp,
}

impl ComOutlook {
    /// Outlook.Application, on a COM STA thread. Err(NoOutlook) when classic
    /// Outlook isn't installed or won't start.
    pub fn connect() -> Result<ComOutlook, DraftError> {
        let clsid = unsafe { CLSIDFromProgID(windows::core::w!("Outlook.Application")) }.map_err(|_| DraftError::NoOutlook)?;
        let app: IDispatch = unsafe { CoCreateInstance(&clsid, None, CLSCTX_LOCAL_SERVER) }.map_err(|_| DraftError::NoOutlook)?;
        let app = Disp(app);
        let ns = app.call_obj("GetNamespace", &[VARIANT::from("MAPI")]).map_err(|_| DraftError::NoOutlook)?;
        Ok(ComOutlook { _app: app, ns })
    }

    /// The account whose SMTP address is `from` (any case), else None.
    pub fn account(&self, from: &str) -> Option<Disp> {
        let accounts = self.ns.obj("Accounts").ok()?;
        (1..=accounts.int("Count")).filter_map(|i| accounts.item(i).ok()).find(|a| a.text("SmtpAddress").trim().eq_ignore_ascii_case(from.trim()))
    }

    /// An address-book entry's SMTP address (an Exchange user's primary one).
    fn smtp(entry: &Disp) -> String {
        if EXCHANGE_USERS.contains(&entry.int("AddressEntryUserType")) {
            if let Ok(user) = entry.call_obj("GetExchangeUser", &[]) {
                let a = user.text("PrimarySmtpAddress");
                if !a.is_empty() {
                    return a;
                }
            }
        }
        entry.text("Address")
    }
}

/// Every word of `query` starts a word of `name` (any case): "maria" finds
/// "Maria Lopez" and "Lopez, Maria"; "maria lo" finds the first.
pub fn name_matches(name: &str, query: &str) -> bool {
    let words: Vec<String> = name.split(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == ')').filter(|w| !w.is_empty()).map(str::to_lowercase).collect();
    let q: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    !q.is_empty() && q.iter().all(|qw| words.iter().any(|w| w.starts_with(qw.as_str())))
}

/// The body as HTML paragraphs (escaped), for a message that already holds
/// Outlook's signature.
pub fn body_html(text: &str) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    text.split("\n\n")
        .map(|p| format!("<p>{}</p>", p.lines().map(esc).collect::<Vec<_>>().join("<br>")))
        .collect::<String>()
}

/// `ours` inside `html` right after its <body …> tag (or in front of it).
pub fn insert_after_body_tag(html: &str, ours: &str) -> String {
    let lower = html.to_ascii_lowercase();
    match lower.find("<body").and_then(|i| lower[i..].find('>').map(|j| i + j + 1)) {
        Some(at) => format!("{}{}{}", &html[..at], ours, &html[at..]),
        None => format!("{ours}{html}"),
    }
}

/// Outlook put something in a brand-new message: its signature.
pub fn has_signature(body_text: &str) -> bool {
    !body_text.trim().is_empty()
}

impl Outlook for ComOutlook {
    fn search(&self, query: &str) -> Result<Vec<Contact>, DraftError> {
        let lists = self.ns.obj("AddressLists").map_err(|_| DraftError::Failed)?;
        let by_address = looks_like_address(query);
        let mut out: Vec<Contact> = Vec::new();
        for l in 1..=lists.int("Count") {
            let Ok(list) = lists.item(l) else { continue };
            let Ok(entries) = list.obj("AddressEntries") else { continue };
            for i in 1..=entries.int("Count").min(MAX_ENTRIES) {
                let Ok(e) = entries.item(i) else { continue };
                let name = e.text("Name");
                // An address (spoken or written, voice 4c) matches the entry's
                // own address; anything else matches names.
                let address = if by_address {
                    let a = ComOutlook::smtp(&e);
                    if !a.trim().eq_ignore_ascii_case(query.trim()) {
                        continue;
                    }
                    a
                } else {
                    if !name_matches(&name, query) {
                        continue;
                    }
                    ComOutlook::smtp(&e)
                };
                if looks_like_address(&address) && !out.iter().any(|c| c.address.eq_ignore_ascii_case(&address)) {
                    out.push(Contact { name, address });
                }
            }
        }
        Ok(out)
    }

    fn save_draft(&self, from: &str, draft: &Draft, open: bool) -> Result<(), DraftError> {
        let account = self.account(from).ok_or(DraftError::NoAccount)?;
        let f = |_| DraftError::Failed;
        // In that account's own Drafts, sent (by the user) from that account.
        let store = account.obj("DeliveryStore").map_err(f)?;
        let drafts = store.call_obj("GetDefaultFolder", &[int(DRAFTS)]).map_err(f)?;
        let mail = drafts.obj("Items").map_err(f)?.call_obj("Add", &[int(MAIL_ITEM)]).map_err(f)?;
        mail.put_ref("SendUsingAccount", &account).map_err(f)?;
        mail.put("To", VARIANT::from(draft.to.join("; ").as_str())).map_err(f)?;
        if !draft.cc.is_empty() {
            mail.put("CC", VARIANT::from(draft.cc.join("; ").as_str())).map_err(f)?;
        }
        if !draft.bcc.is_empty() {
            mail.put("BCC", VARIANT::from(draft.bcc.join("; ").as_str())).map_err(f)?;
        }
        mail.put("Subject", VARIANT::from(draft.subject.as_str())).map_err(f)?;
        // The message's window, created but not shown: Outlook puts the
        // default signature in now, if there is one.
        let _inspector = mail.obj("GetInspector");
        if has_signature(&mail.text("Body")) {
            let html = mail.text("HTMLBody");
            mail.put("HTMLBody", VARIANT::from(insert_after_body_tag(&html, &body_html(&draft.body)).as_str())).map_err(f)?;
        } else {
            mail.put("Body", VARIANT::from(draft.body.as_str())).map_err(f)?;
        }
        mail.call("Save", &[]).map_err(f)?;
        if open {
            mail.call("Display", &[boolean(false)]).map_err(f)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_by_word_starts() {
        assert!(name_matches("Maria Lopez", "maria"));
        assert!(name_matches("Lopez, Maria", "Maria"));
        assert!(name_matches("Maria Lopez", "maria lo"));
        assert!(!name_matches("Maria Lopez", "aria"));
        assert!(!name_matches("Maria Lopez", "maria okafor"));
        assert!(!name_matches("Maria Lopez", ""));
    }

    #[test]
    fn the_body_goes_in_above_the_signature_escaped() {
        assert_eq!(body_html("Hi Maria,\n\nThe <plumber> & co\ncomes Tuesday."), "<p>Hi Maria,</p><p>The &lt;plumber&gt; &amp; co<br>comes Tuesday.</p>");
        assert_eq!(insert_after_body_tag("<html><BODY lang=en><div>-- sig</div></body></html>", "<p>x</p>"), "<html><BODY lang=en><p>x</p><div>-- sig</div></body></html>");
        assert_eq!(insert_after_body_tag("<div>sig</div>", "<p>x</p>"), "<p>x</p><div>sig</div>");
        assert!(has_signature("\r\nAnthony\r\n"));
        assert!(!has_signature(" \r\n "));
    }

    #[test]
    fn only_the_senders_own_account_and_never_a_send() {
        let src = include_str!("outlook.rs");
        let code = &src[..src.find("mod tests {").unwrap()];
        assert!(code.contains("let account = self.account(from).ok_or(DraftError::NoAccount)?;"));
        assert!(code.contains("mail.put_ref(\"SendUsingAccount\", &account)"));
        // The account is matched on its address, never "the default one".
        assert!(!code.contains("Session.CurrentUser") && !code.contains("DefaultAccount"));
        assert!(!code.contains("Attachments"), "no attachments (owner, 2026-10-16)");
    }
}
