// Voice 1.5d: "draft an email to Maria saying …" (docs/voice-actions-plan.md
// §4, §5; decided 2026-10-16, Anthony): the island's "Draft email" card,
// then a draft saved in Outlook's Drafts and opened in Outlook. Glim never
// sends: Anthony sends it himself, in Outlook.
//
// This file holds the rules, pure and tested: how a spoken name becomes an
// address (`resolve`: "me", a written address, a remembered pick, Outlook's
// local address book: one match, several to pick from as chips, or none), the
// card's state (`Pending`), and what Outlook is asked to do (`Outlook`, with
// a fake for tests and a dry run that saves nothing).
//
// Names and addresses are the user's contacts: shown on the card, kept in
// memory while the card is up, remembered (a pick) only in settings.json,
// never logged (the log gets counts and codes).

use std::collections::BTreeMap;

use serde::Serialize;

/// One address-book entry: its display name and SMTP address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Contact {
    pub name: String,
    pub address: String,
}

/// Which line of the email a recipient is on (CC/BCC only when spoken).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Line {
    To,
    Cc,
    Bcc,
}

/// Where a spoken name stands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Found {
    /// One address: ready.
    One { contact: Contact },
    /// Several matches: the card shows them as chips to pick from.
    Pick { choices: Vec<Contact> },
    /// Nothing: "Change recipient" searches the address book.
    None,
}

/// One recipient on the card: what was said, its line, and where it stands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Recipient {
    pub said: String,
    pub line: Line,
    pub found: Found,
}

/// How Glim reaches Outlook (classic, through COM; outlook.rs), or a stand-in.
pub trait Outlook {
    /// Outlook's local address book: entries whose name matches `query`.
    fn search(&self, query: &str) -> Result<Vec<Contact>, DraftError>;
    /// Saves a draft in the Drafts of the account whose address is `from`
    /// (never another account) and opens it in Outlook when `open`. Never sends.
    fn save_draft(&self, from: &str, draft: &Draft, open: bool) -> Result<(), DraftError>;
}

/// Why there is no draft (a code for the log and the card).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftError {
    /// Classic Outlook isn't installed or didn't start.
    NoOutlook,
    /// No Outlook account has the sender's address (Settings → Email drafts).
    NoAccount,
    /// Settings → Email drafts → Your address is empty.
    NoSender,
    /// Outlook refused or failed.
    Failed,
}

impl DraftError {
    pub fn code(self) -> &'static str {
        match self {
            DraftError::NoOutlook => "no-outlook",
            DraftError::NoAccount => "no-account",
            DraftError::NoSender => "no-sender",
            DraftError::Failed => "outlook-failed",
        }
    }
}

/// What Outlook gets: addresses only, the subject and the plain-text body.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: String,
}

/// Words that mean the sender: "draft an email to me".
const ME: [&str; 3] = ["me", "myself", "my self"];

/// A written-out address ("maria@example.com"), loosely: one @, a dot after it,
/// no spaces.
pub fn looks_like_address(s: &str) -> bool {
    let s = s.trim();
    let Some((local, domain)) = s.split_once('@') else { return false };
    !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.') && !s.contains(char::is_whitespace) && !domain.contains('@')
}

/// The key a pick is remembered under: the spoken name, lowercased, spaces
/// tidied ("Maria  Lopez" → "maria lopez").
pub fn memory_key(said: &str) -> String {
    said.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// A spoken name → where it stands: "me" is the sender's own address, a
/// written address is taken as is, a remembered pick wins, then Outlook's
/// address book (one match, several, none).
pub fn resolve(said: &str, me: &str, remembered: &BTreeMap<String, String>, search: impl Fn(&str) -> Vec<Contact>) -> Found {
    let key = memory_key(said);
    if ME.contains(&key.as_str()) {
        return if looks_like_address(me) { Found::One { contact: Contact { name: me.into(), address: me.into() } } } else { Found::None };
    }
    if looks_like_address(said) {
        let a = said.trim().to_string();
        return Found::One { contact: Contact { name: a.clone(), address: a } };
    }
    if let Some(address) = remembered.get(&key).filter(|a| looks_like_address(a)) {
        // The name as the address book has it, when it's still there.
        let name = search(said).into_iter().find(|c| c.address.eq_ignore_ascii_case(address)).map_or_else(|| said.trim().to_string(), |c| c.name);
        return Found::One { contact: Contact { name, address: address.clone() } };
    }
    let mut found = search(said);
    found.retain(|c| looks_like_address(&c.address));
    found.dedup_by(|a, b| a.address.eq_ignore_ascii_case(&b.address));
    match found.len() {
        0 => Found::None,
        1 => Found::One { contact: found.remove(0) },
        _ => {
            found.truncate(MAX_CHOICES);
            Found::Pick { choices: found }
        }
    }
}

/// Chips shown for one name at most.
pub const MAX_CHOICES: usize = 6;

/// The card while it's up: Rust keeps it; the island sends only its id and
/// indexes (and a search's text).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pending {
    pub id: u64,
    pub recipients: Vec<Recipient>,
    pub subject: String,
    pub body: String,
    /// The recipient "Change recipient" is searching for, and what it found.
    pub changing: Option<usize>,
    pub results: Vec<Contact>,
    /// Picks made on this card, to remember once the draft is created:
    /// (spoken name, address).
    #[serde(skip)]
    pub picked: Vec<(String, String)>,
}

impl Pending {
    /// Every recipient has one address, and there is at least one To.
    pub fn ready(&self) -> bool {
        self.recipients.iter().any(|r| r.line == Line::To) && self.recipients.iter().all(|r| matches!(r.found, Found::One { .. }))
    }

    /// A chip was picked: choice `choice` of recipient `index`'s matches, or
    /// of the search results while that recipient is being changed.
    pub fn pick(&mut self, index: usize, choice: usize) -> bool {
        let Some(r) = self.recipients.get_mut(index) else { return false };
        let contact = if self.changing == Some(index) {
            self.results.get(choice).cloned()
        } else if let Found::Pick { choices } = &r.found {
            choices.get(choice).cloned()
        } else {
            None
        };
        let Some(contact) = contact else { return false };
        self.picked.retain(|(said, _)| memory_key(said) != memory_key(&r.said));
        self.picked.push((r.said.clone(), contact.address.clone()));
        r.found = Found::One { contact };
        self.changing = None;
        self.results.clear();
        true
    }

    /// "Change recipient" on recipient `index`: its search opens (empty).
    pub fn change(&mut self, index: usize) -> bool {
        if index >= self.recipients.len() {
            return false;
        }
        self.changing = Some(index);
        self.results.clear();
        true
    }

    /// What Outlook gets, once every recipient has an address.
    pub fn draft(&self) -> Option<Draft> {
        if !self.ready() {
            return None;
        }
        let mut d = Draft { subject: self.subject.clone(), body: self.body.clone(), ..Default::default() };
        for r in &self.recipients {
            if let Found::One { contact } = &r.found {
                match r.line {
                    Line::To => d.to.push(contact.address.clone()),
                    Line::Cc => d.cc.push(contact.address.clone()),
                    Line::Bcc => d.bcc.push(contact.address.clone()),
                }
            }
        }
        Some(d)
    }

    /// The picks to remember in settings ("Change recipient" or a chip): the
    /// spoken name → the address. "me" and written addresses aren't picks.
    pub fn remembered(&self) -> Vec<(String, String)> {
        self.picked.iter().map(|(said, a)| (memory_key(said), a.clone())).filter(|(k, _)| !ME.contains(&k.as_str()) && !looks_like_address(k)).collect()
    }
}

/// The card for a `draft_email` call: each spoken name resolved.
pub fn pending(id: u64, call: &Spoken, me: &str, remembered: &BTreeMap<String, String>, search: impl Fn(&str) -> Vec<Contact>) -> Pending {
    let mut recipients = Vec::new();
    for (line, names) in [(Line::To, &call.to), (Line::Cc, &call.cc), (Line::Bcc, &call.bcc)] {
        for said in names.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            recipients.push(Recipient { said: said.to_string(), line, found: resolve(said, me, remembered, &search) });
        }
    }
    Pending { id, recipients, subject: call.subject.trim().to_string(), body: call.body.trim().to_string(), changing: None, results: Vec::new(), picked: Vec::new() }
}

/// Claude's `draft_email` call, validated (ai::dictate): names as said.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Spoken {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: String,
}

/// Saves `pending`'s draft as `from` and opens it (dry run: nothing saved).
pub fn create(outlook: &dyn Outlook, from: &str, pending: &Pending, open: bool) -> Result<(), DraftError> {
    if !looks_like_address(from) {
        return Err(DraftError::NoSender);
    }
    let draft = pending.draft().ok_or(DraftError::Failed)?;
    outlook.save_draft(from, &draft, open)
}

/// A stand-in Outlook for tests: an address book, accounts (the default
/// first), and the drafts it was asked to save.
#[cfg(test)]
#[derive(Default)]
pub struct FakeOutlook {
    pub book: Vec<Contact>,
    /// The accounts' addresses, the default one first.
    pub accounts: Vec<String>,
    pub saved: std::sync::Mutex<Vec<(String, Draft, bool)>>,
}

#[cfg(test)]
impl Outlook for FakeOutlook {
    fn search(&self, query: &str) -> Result<Vec<Contact>, DraftError> {
        let q = memory_key(query);
        Ok(self.book.iter().filter(|c| memory_key(&c.name).split(' ').any(|w| w.starts_with(&q)) || memory_key(&c.name).starts_with(&q)).cloned().collect())
    }

    fn save_draft(&self, from: &str, draft: &Draft, open: bool) -> Result<(), DraftError> {
        let account = self.accounts.iter().find(|a| a.eq_ignore_ascii_case(from)).ok_or(DraftError::NoAccount)?;
        self.saved.lock().unwrap().push((account.clone(), draft.clone(), open));
        Ok(())
    }
}

/// The dry run (GLIM_DRAFT_DRY_RUN=1): searches the real address book, saves
/// nothing, opens nothing.
pub struct DryRun<'a>(pub &'a dyn Outlook);

impl Outlook for DryRun<'_> {
    fn search(&self, query: &str) -> Result<Vec<Contact>, DraftError> {
        self.0.search(query)
    }

    fn save_draft(&self, from: &str, _draft: &Draft, _open: bool) -> Result<(), DraftError> {
        if !looks_like_address(from) {
            return Err(DraftError::NoSender);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME_ADDR: &str = "anthony@example.com";

    fn c(name: &str, address: &str) -> Contact {
        Contact { name: name.into(), address: address.into() }
    }

    fn outlook() -> FakeOutlook {
        FakeOutlook {
            book: vec![
                c("Maria Lopez", "maria.lopez@example.com"),
                c("Maria Okafor", "mokafor@example.com"),
                c("Sam Nguyen", "sam@example.com"),
                c("No Address", "not-an-address"),
            ],
            // The shared mailbox is the default account, on purpose.
            accounts: vec!["leasing@example.com".into(), ME_ADDR.into()],
            saved: Default::default(),
        }
    }

    fn search(o: &FakeOutlook) -> impl Fn(&str) -> Vec<Contact> + '_ {
        |q| o.search(q).unwrap()
    }

    #[test]
    fn a_spoken_name_resolves_one_several_or_none() {
        let o = outlook();
        let none = BTreeMap::new();
        assert_eq!(resolve("Sam", ME_ADDR, &none, search(&o)), Found::One { contact: c("Sam Nguyen", "sam@example.com") });
        assert_eq!(
            resolve("maria", ME_ADDR, &none, search(&o)),
            Found::Pick { choices: vec![c("Maria Lopez", "maria.lopez@example.com"), c("Maria Okafor", "mokafor@example.com")] }
        );
        assert_eq!(resolve("Zed", ME_ADDR, &none, search(&o)), Found::None);
        // An entry with no usable address is never offered.
        assert_eq!(resolve("No Address", ME_ADDR, &none, search(&o)), Found::None);
    }

    #[test]
    fn me_is_the_sender_a_written_address_is_taken_and_a_remembered_pick_wins() {
        let o = outlook();
        let mut mem = BTreeMap::new();
        for said in ["me", "Me", "myself"] {
            assert_eq!(resolve(said, ME_ADDR, &mem, search(&o)), Found::One { contact: c(ME_ADDR, ME_ADDR) }, "{said}");
        }
        assert_eq!(resolve("me", "", &mem, search(&o)), Found::None, "no address set: nothing to fill in");
        assert_eq!(resolve(" bob@example.com ", ME_ADDR, &mem, search(&o)), Found::One { contact: c("bob@example.com", "bob@example.com") });
        mem.insert("maria".into(), "mokafor@example.com".into());
        assert_eq!(resolve("Maria", ME_ADDR, &mem, search(&o)), Found::One { contact: c("Maria Okafor", "mokafor@example.com") });
    }

    #[test]
    fn the_card_is_ready_only_when_every_name_has_one_address() {
        let o = outlook();
        let call = Spoken { to: vec!["Maria".into()], cc: vec!["Sam".into()], subject: " Plumber Tuesday ".into(), body: "Hi Maria,\n\nThe plumber comes Tuesday at 10am.".into(), ..Default::default() };
        let mut p = pending(7, &call, ME_ADDR, &BTreeMap::new(), search(&o));
        assert_eq!(p.subject, "Plumber Tuesday");
        assert_eq!(p.recipients.iter().map(|r| r.line).collect::<Vec<_>>(), [Line::To, Line::Cc]);
        assert!(!p.ready());
        assert!(p.draft().is_none());
        assert!(!p.pick(0, 9), "no such chip");
        assert!(p.pick(0, 1));
        assert!(p.ready());
        let d = p.draft().unwrap();
        assert_eq!((d.to, d.cc, d.bcc), (vec!["mokafor@example.com".to_string()], vec!["sam@example.com".to_string()], vec![]));
        assert_eq!(p.remembered(), [("maria".to_string(), "mokafor@example.com".to_string())]);
        // CC alone is not enough.
        let only_cc = pending(8, &Spoken { cc: vec!["Sam".into()], body: "x".into(), ..Default::default() }, ME_ADDR, &BTreeMap::new(), search(&o));
        assert!(!only_cc.ready());
    }

    #[test]
    fn change_recipient_searches_and_its_pick_replaces_and_is_remembered() {
        let o = outlook();
        let call = Spoken { to: vec!["Sam".into()], body: "x".into(), ..Default::default() };
        let mut p = pending(1, &call, ME_ADDR, &BTreeMap::new(), search(&o));
        assert!(p.ready());
        assert!(p.change(0));
        assert!(!p.change(5));
        p.results = o.search("maria").unwrap();
        assert!(p.pick(0, 0));
        assert_eq!(p.recipients[0].found, Found::One { contact: c("Maria Lopez", "maria.lopez@example.com") });
        assert_eq!((p.changing, p.results.len()), (None, 0));
        assert_eq!(p.remembered(), [("sam".to_string(), "maria.lopez@example.com".to_string())]);
    }

    #[test]
    fn the_draft_is_saved_only_in_the_senders_own_account_never_the_shared_one() {
        let o = outlook();
        let call = Spoken { to: vec!["me".into()], subject: "[GLIM TEST] 1".into(), body: "x".into(), ..Default::default() };
        let p = pending(1, &call, ME_ADDR, &BTreeMap::new(), search(&o));
        assert_eq!(create(&o, ME_ADDR, &p, true), Ok(()));
        let saved = o.saved.lock().unwrap().clone();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].0, ME_ADDR, "not the default (shared) account");
        assert!(saved[0].2, "opened in Outlook");
        // An address with no account of its own: nothing saved anywhere.
        assert_eq!(create(&o, "someone@else.com", &p, true), Err(DraftError::NoAccount));
        assert_eq!(create(&o, "", &p, true), Err(DraftError::NoSender));
        assert_eq!(o.saved.lock().unwrap().len(), 1);
    }

    #[test]
    fn the_dry_run_saves_nothing() {
        let o = outlook();
        let p = pending(1, &Spoken { to: vec!["Sam".into()], body: "x".into(), ..Default::default() }, ME_ADDR, &BTreeMap::new(), search(&o));
        assert_eq!(create(&DryRun(&o), ME_ADDR, &p, true), Ok(()));
        assert!(o.saved.lock().unwrap().is_empty());
        assert_eq!(DryRun(&o).search("sam").unwrap().len(), 1);
    }

    #[test]
    fn glim_never_sends() {
        // No code path here or in the COM layer calls Send.
        for (name, src) in [("draft.rs", include_str!("draft.rs")), ("outlook.rs", include_str!("../outlook.rs"))] {
            let code = &src[..src.find("mod tests {").unwrap_or(src.len())];
            for banned in ["\"Send\"", ".Send(", "send_mail", "SendKeys"] {
                assert!(!code.contains(banned), "{name}: {banned}");
            }
        }
    }

    #[test]
    fn addresses_and_memory_keys() {
        for ok in ["a@b.co", "maria.lopez@example.com"] {
            assert!(looks_like_address(ok), "{ok}");
        }
        for bad in ["", "maria", "a@b", "@b.com", "a@.com", "a b@c.com", "a@b.com.", "a@b@c.com"] {
            assert!(!looks_like_address(bad), "{bad}");
        }
        assert_eq!(memory_key("  Maria   Lopez "), "maria lopez");
    }
}
