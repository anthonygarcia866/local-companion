// Voice 1.5d: "draft an email to Maria saying …" (docs/voice-actions-plan.md
// §4, §5; decided 2026-10-16, Anthony): the island's "Draft email" card,
// then a draft saved in Outlook's Drafts and opened in Outlook. Glim never
// sends: Anthony sends it himself, in Outlook.
//
// This file holds the rules, pure and tested: how a spoken name becomes an
// address (`resolve`: "me", a spoken or written address checked against the
// address book, a remembered pick, Outlook's local address book: one match,
// several to pick from as chips, or none), the card's state (`Pending`, which a
// voice answer can fill: Decision B), and what Outlook is asked to do
// (`Outlook`, with a fake for tests and a dry run that saves nothing).
//
// Never guessed (2026-10-16): an address that isn't in the address book is
// offered as a chip to confirm, never filled in by itself.
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
    /// (never another account) and opens it in Outlook when `open`. Never
    /// sends. Ok: the draft's Outlook id (EntryID), kept only in memory so
    /// "change the email you just drafted" can find it (fix b, 2026-10-17).
    fn save_draft(&self, from: &str, draft: &Draft, open: bool) -> Result<String, DraftError>;
    /// The subject of draft `id` and its text above the signature. Err(Gone)
    /// if it was deleted, moved or sent.
    fn read_draft(&self, id: &str) -> Result<DraftText, DraftError>;
    /// Puts a new subject and text (above the signature, which stays) into
    /// draft `id`, saves it and opens it when `open`. Never sends.
    fn update_draft(&self, id: &str, subject: &str, body: &str, open: bool) -> Result<(), DraftError>;
}

/// A saved draft's subject and Glim's part of its text (above the signature).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DraftText {
    pub subject: String,
    pub body: String,
}

/// Word's text (a message's editor) as plain text: a paragraph mark (\r) is
/// a paragraph, a blank line apart; a line break inside one (\v) is a newline.
/// Glim wrote "a\n\nb" as two paragraphs and "a\nb" as one with a break.
pub fn word_to_text(word: &str) -> String {
    let text = word.replace("\r\n", "\r").replace('\r', "\n\n").replace('\u{b}', "\n");
    let mut lines: Vec<&str> = Vec::new();
    for line in text.trim().split('\n').map(str::trim_end) {
        // Empty paragraphs don't pile up: at most one blank line.
        if line.is_empty() && lines.last().is_some_and(|l| l.is_empty()) {
            continue;
        }
        lines.push(line);
    }
    lines.join("\n")
}

/// How much of the old part above the signature is replaced, in Word's
/// character positions (UTF-16): everything but the paragraph marks just
/// before the signature. Those stay, so the new text never touches the
/// signature's bookmark (live, 2026-10-17: text put right against it went
/// into the bookmark, i.e. into the signature).
pub fn replaced_len(old: &str) -> usize {
    old.trim_end_matches(['\r', '\n', ' ', '\u{b}']).encode_utf16().count()
}

/// `body` as Word text: its paragraphs as Word paragraphs (\r), its line
/// breaks inside one as \v.
pub fn text_to_word(body: &str) -> String {
    let body = body.trim().replace("\r\n", "\n");
    let paragraphs: Vec<String> = body.split("\n\n").map(|p| p.trim().replace('\n', "\u{b}")).filter(|p| !p.is_empty()).collect();
    paragraphs.join("\r")
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
    /// The sender is the shared leasing mailbox: drafts are never made there
    /// (owner, 2026-10-16).
    SharedAccount,
    /// The draft to change was deleted, moved or sent, or none was made in
    /// the last few minutes.
    Gone,
}

impl DraftError {
    pub fn code(self) -> &'static str {
        match self {
            DraftError::NoOutlook => "no-outlook",
            DraftError::NoAccount => "no-account",
            DraftError::NoSender => "no-sender",
            DraftError::Failed => "outlook-failed",
            DraftError::SharedAccount => "shared-account",
            DraftError::Gone => "draft-gone",
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
const ME: [&str; 6] = ["me", "myself", "my self", "my email", "my address", "my own address"];

/// "me", "Me.", "myself!" → the sender (punctuation around it is dropped).
pub fn means_me(said: &str) -> bool {
    let key = memory_key(said.trim_matches(|c: char| !c.is_alphanumeric()));
    ME.contains(&key.as_str())
}

/// Mailboxes a draft is never saved in or sent from, whatever Settings says:
/// the shared leasing inbox (owner, 2026-10-16: "never from leasing@").
const NEVER_FROM: [&str; 1] = ["leasing"];

pub fn never_from(address: &str) -> bool {
    address.trim().split_once('@').is_some_and(|(local, _)| NEVER_FROM.iter().any(|n| local.eq_ignore_ascii_case(n)))
}

/// A spoken email address, as whisper writes it, → the address: "jordan lee
/// at example dot com" → "jordanlee@example.com".
/// "at" → @, "dot" → ., "underscore" → _, "dash"/"hyphen" → -, spaces gone,
/// lowercase. None unless the result is one address (a name stays a name:
/// "Maria at the office" has no dot after the @).
pub fn spoken_address(said: &str) -> Option<String> {
    let mut out = String::new();
    for w in said.trim().trim_end_matches(['.', ',', '!', '?']).split_whitespace() {
        let lw = w.to_lowercase();
        out.push_str(match lw.as_str() {
            "at" => "@",
            "dot" | "period" | "point" => ".",
            "underscore" => "_",
            "dash" | "hyphen" => "-",
            _ => lw.as_str(),
        });
    }
    let out = out.trim_end_matches('.').to_string();
    looks_like_address(&out).then_some(out)
}

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

/// A spoken name → where it stands: "me" is the sender's own address
/// (Settings → Email drafts → Your address); an address, written or spoken, is
/// looked up in the address book (found: filled in; not found: one chip to
/// confirm, never filled in by itself); a remembered pick wins; then Outlook's
/// address book by name (one match, several, none).
pub fn resolve(said: &str, me: &str, remembered: &BTreeMap<String, String>, search: impl Fn(&str) -> Vec<Contact>) -> Found {
    let key = memory_key(said);
    if means_me(said) {
        return if looks_like_address(me) { Found::One { contact: Contact { name: me.into(), address: me.into() } } } else { Found::None };
    }
    if let Some(address) = spoken_address(said) {
        return match search(&address).into_iter().find(|c| c.address.eq_ignore_ascii_case(&address)) {
            Some(contact) => Found::One { contact },
            None => Found::Pick { choices: vec![Contact { name: address.clone(), address }] },
        };
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
    /// A change to the draft Glim just saved (fix b, 2026-10-17): the card
    /// shows the new subject and text, and its button updates that draft.
    pub edit: bool,
    /// That draft's Outlook id: stays in Rust, never sent to the island.
    #[serde(skip)]
    pub entry: String,
}

impl Pending {
    /// Every recipient has one address, and there is at least one To.
    pub fn ready(&self) -> bool {
        // A change keeps the draft's recipients as they are in Outlook.
        if self.edit {
            return !self.entry.is_empty() && !self.body.trim().is_empty();
        }
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

    /// The recipient a voice answer is for: the one being changed, else the
    /// first that still needs an address. None: nothing is being asked.
    pub fn asking(&self) -> Option<usize> {
        self.changing.or_else(|| self.recipients.iter().position(|r| !matches!(r.found, Found::One { .. })))
    }

    /// A voice answer (Decision B) for the recipient `asking()` names: a chip
    /// by its name, address or place ("the second one"), else the words looked
    /// up as a new name or address. False when nothing matched; the card
    /// still asks.
    pub fn answer(&mut self, said: &str, me: &str, remembered: &BTreeMap<String, String>, search: impl Fn(&str) -> Vec<Contact>) -> bool {
        let Some(index) = self.asking() else { return false };
        let choices: Vec<Contact> = if self.changing == Some(index) {
            self.results.clone()
        } else if let Some(Found::Pick { choices }) = self.recipients.get(index).map(|r| &r.found) {
            choices.clone()
        } else {
            Vec::new()
        };
        if let Some(k) = choose(said, &choices) {
            return self.pick(index, k);
        }
        match resolve(said, me, remembered, &search) {
            Found::None => false,
            Found::One { contact } => {
                let r = &mut self.recipients[index];
                // A spoken name that found one entry is remembered like a pick.
                if !means_me(said) && spoken_address(said).is_none() {
                    let key = memory_key(&r.said);
                    self.picked.retain(|(s, _)| memory_key(s) != key);
                    self.picked.push((r.said.clone(), contact.address.clone()));
                }
                r.found = Found::One { contact };
                self.changing = None;
                self.results.clear();
                true
            }
            found @ Found::Pick { .. } => {
                // New chips for the same recipient; the next answer picks one.
                self.recipients[index].found = found;
                self.changing = None;
                self.results.clear();
                true
            }
        }
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
        self.picked.iter().map(|(said, a)| (memory_key(said), a.clone())).filter(|(k, _)| !means_me(k) && !looks_like_address(k) && spoken_address(k).is_none()).collect()
    }
}

/// Which chip an answer means: its place ("first", "the second one", "2",
/// "last"), its whole address (spoken or written), or the one chip whose name
/// holds every word said. None when it's unclear: never guessed.
pub fn choose(said: &str, choices: &[Contact]) -> Option<usize> {
    if choices.is_empty() {
        return None;
    }
    let plain = said.trim().trim_end_matches(['.', ',', '!', '?']);
    let words = memory_key(&plain.replace(|c: char| !c.is_alphanumeric() && c != '@' && c != '.', " "));
    const PLACES: [(&str, usize); 12] = [
        ("first", 0), ("1st", 0), ("one", 0), ("1", 0), ("second", 1), ("2nd", 1), ("two", 1), ("2", 1), ("third", 2), ("3rd", 2), ("three", 2), ("3", 2),
    ];
    let stripped = words.trim_start_matches("the ").trim_end_matches(" one");
    if let Some((_, k)) = PLACES.iter().find(|(w, _)| stripped == *w) {
        return (*k < choices.len()).then_some(*k);
    }
    if stripped == "last" {
        return Some(choices.len() - 1);
    }
    if let Some(address) = spoken_address(said) {
        return choices.iter().position(|c| c.address.eq_ignore_ascii_case(&address));
    }
    let said_words: Vec<&str> = words.split(' ').filter(|w| !w.is_empty()).collect();
    if said_words.is_empty() {
        return None;
    }
    let hits: Vec<usize> = choices
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            let name = memory_key(&c.name);
            said_words.iter().all(|w| name.split(' ').any(|n| n == *w))
        })
        .map(|(k, _)| k)
        .collect();
    (hits.len() == 1).then(|| hits[0])
}

/// The card for a `draft_email` call: each spoken name resolved.
pub fn pending(id: u64, call: &Spoken, me: &str, remembered: &BTreeMap<String, String>, search: impl Fn(&str) -> Vec<Contact>) -> Pending {
    let mut recipients = Vec::new();
    for (line, names) in [(Line::To, &call.to), (Line::Cc, &call.cc), (Line::Bcc, &call.bcc)] {
        for said in names.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            recipients.push(Recipient { said: said.to_string(), line, found: resolve(said, me, remembered, &search) });
        }
    }
    Pending { id, recipients, subject: call.subject.trim().to_string(), body: call.body.trim().to_string(), changing: None, results: Vec::new(), picked: Vec::new(), edit: false, entry: String::new() }
}

/// The card for a change to draft `entry`: the new subject and text, no
/// recipients (they stay as they are in Outlook).
pub fn edit_card(id: u64, entry: &str, new: &DraftText) -> Pending {
    Pending { id, recipients: Vec::new(), subject: new.subject.trim().to_string(), body: new.body.trim().to_string(), changing: None, results: Vec::new(), picked: Vec::new(), edit: true, entry: entry.to_string() }
}

/// How long "the email you just drafted" means the last draft Glim saved.
pub const EDIT_WINDOW: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// The draft Glim saved last, if it was saved (or changed) within
/// EDIT_WINDOW of `now`. Only its Outlook id is kept, in memory.
pub fn recent(last: &Option<(String, std::time::Instant)>, now: std::time::Instant) -> Option<&str> {
    last.as_ref().filter(|(id, at)| !id.is_empty() && now.saturating_duration_since(*at) < EDIT_WINDOW).map(|(id, _)| id.as_str())
}

/// Updates the card's draft in Outlook and opens it there. Never sends.
pub fn update(outlook: &dyn Outlook, card: &Pending) -> Result<(), DraftError> {
    if !card.ready() {
        return Err(DraftError::Failed);
    }
    outlook.update_draft(&card.entry, &card.subject, &card.body, true)
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
/// Ok: the draft's Outlook id.
pub fn create(outlook: &dyn Outlook, from: &str, pending: &Pending, open: bool) -> Result<String, DraftError> {
    if !looks_like_address(from) {
        return Err(DraftError::NoSender);
    }
    if never_from(from) {
        return Err(DraftError::SharedAccount);
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
    /// Changes asked of saved drafts: (id, subject, body, open).
    pub updated: std::sync::Mutex<Vec<(String, String, String, bool)>>,
}

#[cfg(test)]
impl Outlook for FakeOutlook {
    fn search(&self, query: &str) -> Result<Vec<Contact>, DraftError> {
        if looks_like_address(query) {
            return Ok(self.book.iter().filter(|c| c.address.eq_ignore_ascii_case(query.trim())).cloned().collect());
        }
        let q = memory_key(query);
        Ok(self.book.iter().filter(|c| memory_key(&c.name).split(' ').any(|w| w.starts_with(&q)) || memory_key(&c.name).starts_with(&q)).cloned().collect())
    }

    fn save_draft(&self, from: &str, draft: &Draft, open: bool) -> Result<String, DraftError> {
        let account = self.accounts.iter().find(|a| a.eq_ignore_ascii_case(from)).ok_or(DraftError::NoAccount)?;
        let mut saved = self.saved.lock().unwrap();
        saved.push((account.clone(), draft.clone(), open));
        Ok(format!("fake-{}", saved.len()))
    }

    fn read_draft(&self, id: &str) -> Result<DraftText, DraftError> {
        let n: usize = id.strip_prefix("fake-").and_then(|n| n.parse().ok()).ok_or(DraftError::Gone)?;
        let saved = self.saved.lock().unwrap();
        let (_, d, _) = saved.get(n.wrapping_sub(1)).ok_or(DraftError::Gone)?;
        Ok(DraftText { subject: d.subject.clone(), body: d.body.clone() })
    }

    fn update_draft(&self, id: &str, subject: &str, body: &str, open: bool) -> Result<(), DraftError> {
        self.read_draft(id)?;
        self.updated.lock().unwrap().push((id.to_string(), subject.to_string(), body.to_string(), open));
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

    fn save_draft(&self, from: &str, _draft: &Draft, _open: bool) -> Result<String, DraftError> {
        if !looks_like_address(from) {
            return Err(DraftError::NoSender);
        }
        // Nothing saved, so nothing to change later.
        Ok(String::new())
    }

    fn read_draft(&self, _id: &str) -> Result<DraftText, DraftError> {
        Err(DraftError::Gone)
    }

    fn update_draft(&self, _id: &str, _subject: &str, _body: &str, _open: bool) -> Result<(), DraftError> {
        Err(DraftError::Gone)
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
            updated: Default::default(),
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
        for said in ["Me.", "myself!", "my email", " MY ADDRESS "] {
            assert_eq!(resolve(said, ME_ADDR, &mem, search(&o)), Found::One { contact: c(ME_ADDR, ME_ADDR) }, "{said}");
        }
        assert_eq!(resolve("me", "", &mem, search(&o)), Found::None, "no address set: nothing to fill in");
        // A written address: filled in when the address book has it …
        assert_eq!(resolve(" sam@example.com ", ME_ADDR, &mem, search(&o)), Found::One { contact: c("Sam Nguyen", "sam@example.com") });
        // … else one chip to confirm, never filled in by itself.
        assert_eq!(resolve(" bob@example.com ", ME_ADDR, &mem, search(&o)), Found::Pick { choices: vec![c("bob@example.com", "bob@example.com")] });
        mem.insert("maria".into(), "mokafor@example.com".into());
        assert_eq!(resolve("Maria", ME_ADDR, &mem, search(&o)), Found::One { contact: c("Maria Okafor", "mokafor@example.com") });
    }

    #[test]
    fn a_spoken_address_is_normalized_then_checked_against_the_address_book() {
        assert_eq!(spoken_address("jordan lee at example dot com").as_deref(), Some("jordanlee@example.com"));
        assert_eq!(spoken_address("Jordan Lee at Example dot com.").as_deref(), Some("jordanlee@example.com"));
        assert_eq!(spoken_address("JordanLee@Example.com").as_deref(), Some("jordanlee@example.com"));
        assert_eq!(spoken_address("maria underscore lopez at example dot co dot uk").as_deref(), Some("maria_lopez@example.co.uk"));
        assert_eq!(spoken_address("sam dash n at example period com").as_deref(), Some("sam-n@example.com"));
        for name in ["Maria", "Maria at the office", "at dot", "Sam Nguyen", "", "me"] {
            assert_eq!(spoken_address(name), None, "{name}");
        }
        let mut o = outlook();
        o.book.push(c("Anthony Garcia", "anthonygarcia@example.com"));
        let none = BTreeMap::new();
        assert_eq!(resolve("anthony garcia at example dot com", ME_ADDR, &none, search(&o)), Found::One { contact: c("Anthony Garcia", "anthonygarcia@example.com") });
        // Not in the address book: a chip to confirm (whisper may have misheard).
        assert_eq!(
            resolve("tony at example dot com", ME_ADDR, &none, search(&o)),
            Found::Pick { choices: vec![c("tony@example.com", "tony@example.com")] }
        );
        // A spoken address is never remembered as a name.
        let call = Spoken { to: vec!["tony at example dot com".into()], body: "x".into(), ..Default::default() };
        let mut p = pending(1, &call, ME_ADDR, &none, search(&o));
        assert!(p.pick(0, 0));
        assert!(p.remembered().is_empty());
    }

    #[test]
    fn a_voice_answer_fills_the_card_and_never_guesses() {
        let o = outlook();
        let none = BTreeMap::new();
        let call = Spoken { to: vec!["Maria".into()], cc: vec!["Zed".into()], body: "x".into(), ..Default::default() };
        let mut p = pending(1, &call, ME_ADDR, &none, search(&o));
        assert_eq!(p.asking(), Some(0), "Which Maria? comes first");
        // Unclear: nothing changes.
        assert!(!p.answer("the fifth one", ME_ADDR, &none, search(&o)));
        assert!(!p.answer("Maria", ME_ADDR, &none, search(&o)) || matches!(p.recipients[0].found, Found::Pick { .. }), "both Marias: still a pick");
        assert_eq!(p.asking(), Some(0));
        // By name.
        assert!(p.answer("Okafor", ME_ADDR, &none, search(&o)));
        assert_eq!(p.recipients[0].found, Found::One { contact: c("Maria Okafor", "mokafor@example.com") });
        // The next one: Zed isn't in the book; an answer looks the words up.
        assert_eq!(p.asking(), Some(1));
        assert!(!p.answer("nobody", ME_ADDR, &none, search(&o)));
        assert!(p.answer("Sam", ME_ADDR, &none, search(&o)));
        assert_eq!(p.recipients[1].found, Found::One { contact: c("Sam Nguyen", "sam@example.com") });
        assert_eq!(p.asking(), None);
        assert!(p.ready());
        assert_eq!(p.remembered(), [("maria".to_string(), "mokafor@example.com".to_string()), ("zed".to_string(), "sam@example.com".to_string())]);
        // By place, and "me".
        let mut q = pending(2, &Spoken { to: vec!["Maria".into()], body: "x".into(), ..Default::default() }, ME_ADDR, &none, search(&o));
        assert!(q.answer("the second one", ME_ADDR, &none, search(&o)));
        assert_eq!(q.recipients[0].found, Found::One { contact: c("Maria Okafor", "mokafor@example.com") });
        let mut r = pending(3, &Spoken { to: vec!["Zed".into()], body: "x".into(), ..Default::default() }, ME_ADDR, &none, search(&o));
        assert!(r.answer("me", ME_ADDR, &none, search(&o)));
        assert!(r.ready());
        // A spoken address not in the book becomes a chip; "first" confirms it.
        let mut s2 = pending(4, &Spoken { to: vec!["Zed".into()], body: "x".into(), ..Default::default() }, ME_ADDR, &none, search(&o));
        assert!(s2.answer("zed at example dot com", ME_ADDR, &none, search(&o)));
        assert!(!s2.ready(), "never filled in by itself");
        assert!(s2.answer("first", ME_ADDR, &none, search(&o)));
        assert!(s2.ready());
    }

    #[test]
    fn choosing_a_chip_by_place_address_or_name() {
        let chips = vec![c("Maria Lopez", "maria.lopez@example.com"), c("Maria Okafor", "mokafor@example.com")];
        assert_eq!(choose("first", &chips), Some(0));
        assert_eq!(choose("The second one.", &chips), Some(1));
        assert_eq!(choose("2", &chips), Some(1));
        assert_eq!(choose("last", &chips), Some(1));
        assert_eq!(choose("third", &chips), None);
        assert_eq!(choose("Lopez", &chips), Some(0));
        assert_eq!(choose("maria okafor", &chips), Some(1));
        assert_eq!(choose("Maria", &chips), None, "both: unclear");
        assert_eq!(choose("maria at example dot com", &chips), None, "an address on no chip");
        assert_eq!(choose("mokafor at example dot com", &chips), Some(1));
        assert_eq!(choose("anything", &[]), None);
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
        assert_eq!(create(&o, ME_ADDR, &p, true), Ok("fake-1".to_string()), "its Outlook id comes back");
        let saved = o.saved.lock().unwrap().clone();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].0, ME_ADDR, "not the default (shared) account");
        assert!(saved[0].2, "opened in Outlook");
        // An address with no account of its own: nothing saved anywhere.
        assert_eq!(create(&o, "someone@else.com", &p, true), Err(DraftError::NoAccount));
        assert_eq!(create(&o, "", &p, true), Err(DraftError::NoSender));
        // Never from the shared leasing mailbox, even if Settings named it.
        assert_eq!(create(&o, "leasing@example.com", &p, true), Err(DraftError::SharedAccount));
        assert_eq!(create(&o, " Leasing@Example.com ", &p, true), Err(DraftError::SharedAccount));
        assert_eq!(DraftError::SharedAccount.code(), "shared-account");
        assert!(!never_from("leasingoffice@example.com") && !never_from("anthony@example.com"));
        assert_eq!(o.saved.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_drafts_text_goes_to_word_and_back_unchanged() {
        // Word holds paragraphs (\r) and in-paragraph breaks (\v).
        assert_eq!(word_to_text("First.\rSecond\u{b}line.\r\r"), "First.\n\nSecond\nline.");
        assert_eq!(word_to_text("A\r\r\r\rB"), "A\n\nB", "empty paragraphs: one blank line");
        // Only the text is replaced; the marks before the signature stay.
        assert_eq!(text_to_word("New one.\n\nSecond\nline."), "New one.\rSecond\u{b}line.");
        assert_eq!(replaced_len("Old.\r\r"), 4);
        assert_eq!(replaced_len("Été 4B.\r"), 7, "Word counts UTF-16 characters");
        for body in ["Glim edit test, CHANGED.\n\nSecond paragraph. And a third", "One line", "a\nb\n\nc"] {
            assert_eq!(word_to_text(&format!("{}\r\r", text_to_word(body))), body, "{body:?}");
        }
    }

    #[test]
    fn the_email_just_drafted_is_remembered_by_its_id_for_ten_minutes_then_changed_in_place() {
        use std::time::{Duration, Instant};
        let o = outlook();
        let p = pending(1, &Spoken { to: vec!["me".into()], subject: "Shift".into(), body: "The shift moves to Tuesday.".into(), ..Default::default() }, ME_ADDR, &BTreeMap::new(), search(&o));
        let id = create(&o, ME_ADDR, &p, true).unwrap();
        let t0 = Instant::now();
        let last = Some((id.clone(), t0));
        assert_eq!(recent(&last, t0 + Duration::from_secs(9 * 60)), Some(id.as_str()));
        assert_eq!(recent(&last, t0 + EDIT_WINDOW), None, "after ten minutes it's forgotten");
        assert_eq!(recent(&Some((String::new(), t0)), t0), None, "a dry run saved nothing to change");
        assert_eq!(recent(&None, t0), None);
        // Read back from Outlook (not from memory), changed on a card, updated in place.
        let now = o.read_draft(&id).unwrap();
        assert_eq!(now.body, "The shift moves to Tuesday.");
        let card = edit_card(2, &id, &DraftText { subject: "Shift".into(), body: "The shift moves to Wednesday.".into() });
        assert!(card.ready() && card.edit && card.recipients.is_empty());
        let shown = serde_json::to_value(&card).unwrap();
        assert!(shown.get("entry").is_none(), "the Outlook id never goes to the island");
        assert_eq!(update(&o, &card), Ok(()));
        assert_eq!(o.updated.lock().unwrap().clone(), vec![(id.clone(), "Shift".into(), "The shift moves to Wednesday.".into(), true)]);
        assert_eq!(o.saved.lock().unwrap().len(), 1, "no second draft");
        // A draft that's gone can't be changed.
        let gone = edit_card(3, "fake-9", &DraftText { subject: "x".into(), body: "y".into() });
        assert_eq!(update(&o, &gone), Err(DraftError::Gone));
        assert_eq!(DryRun(&o).update_draft(&id, "a", "b", true), Err(DraftError::Gone), "the dry run changes nothing");
    }

    #[test]
    fn the_dry_run_saves_nothing() {
        let o = outlook();
        let p = pending(1, &Spoken { to: vec!["Sam".into()], body: "x".into(), ..Default::default() }, ME_ADDR, &BTreeMap::new(), search(&o));
        assert_eq!(create(&DryRun(&o), ME_ADDR, &p, true), Ok(String::new()), "nothing saved: no id");
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
