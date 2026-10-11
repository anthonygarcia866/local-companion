// Payment and ID data: what the capture layer never reads, or never passes on.
//
// Three checks, all inside the capture thread:
//
// 1. Before any text is read: a field whose labels look like payment or ID
//    data (card number, CVC, expiry, cardholder, bank account, routing number,
//    IBAN, SSN) is skipped, and so is anything in an app or on a site on the
//    pause list. The labels are what UI Automation exposes about a field, not
//    its text: Name (from <label>, aria-label, placeholder or title in a
//    browser), the LabeledBy element's name, HelpText, FullDescription
//    (aria-describedby) and AutomationId (the HTML id).
//    Browsers do not expose the HTML `autocomplete` hint (cc-number, cc-csc …)
//    or the `name` attribute through UI Automation: checked 2026-10-10 in
//    Chrome and Edge with examples/uia_probe.rs. A site whose id reads
//    "cc-number" is still caught through the AutomationId.
// 2. After the text is read and before anything else sees it: a reading that
//    contains a Luhn-valid card number, an SSN-shaped number or a valid IBAN
//    is dropped whole. The panel and the results log get the same "skipped
//    (sensitive)" a skipped field gets, with no length or caret.
// 3. The pause list (apps by executable, sites by host) is checked first.

/// What UI Automation says about a field, never its text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FieldLabels {
    pub name: String,
    pub labeled_by: String,
    pub help_text: String,
    pub description: String,
    pub automation_id: String,
}

/// Whether any of the field's labels looks like payment or ID data.
pub fn is_sensitive_field(l: &FieldLabels) -> bool {
    [&l.name, &l.labeled_by, &l.help_text, &l.description, &l.automation_id]
        .iter()
        .any(|s| looks_sensitive(s))
}

/// Whole words that mark a field as sensitive on their own.
const WORDS: [&str; 10] = ["cvc", "cvc2", "cvv", "cvv2", "csc", "cvn", "iban", "ssn", "cardholder", "mmyy"];

/// Phrases, matched with the spaces taken out ("Card number", "cardNumber",
/// "card_number" and "CARDNUMBER" are all "cardnumber").
const PHRASES: [&str; 33] = [
    "cardnumber", "cardnum", "cardno", "creditcard", "debitcard", "cardholder", "nameoncard",
    "securitycode", "cardverification", "cardcode", "cardexp", "expiry", "expiration", "expdate",
    "expmonth", "expyear", "mmyy", "bankaccount", "accountnumber", "accountno", "acctnumber", "acctno",
    "routingnumber", "routingno", "routingcode", "abanumber", "sortcode", "socialsecurity",
    "socialinsurance", "ccnumber", "ccnum", "ccexp", "cccsc",
];

/// After a separate word "cc", these mark a card field: the HTML
/// autocomplete names (cc-number, cc-csc, cc-exp, cc-name, cc-type …) used as
/// ids. "Cc" on its own is the email copy line and stays readable.
const AFTER_CC: [&str; 11] = ["number", "num", "no", "csc", "cvc", "cvv", "exp", "name", "type", "given", "family"];

/// Whether one label reads like payment or ID data.
pub fn looks_sensitive(label: &str) -> bool {
    let words = words_of(label);
    if words.iter().any(|w| WORDS.contains(&w.as_str())) {
        return true;
    }
    if words.windows(2).any(|p| p[0] == "cc" && AFTER_CC.contains(&p[1].as_str())) {
        return true;
    }
    // A phrase starts where a word starts ("Scorecard notes" holds "cardno"
    // but not at a word); one ending in "no" or "num" must also end where a
    // word ends ("Card notes", "Account numeral" are not account numbers).
    let joined: String = words.concat();
    let mut starts = Vec::new();
    let mut ends = std::collections::HashSet::new();
    let mut at = 0;
    for w in &words {
        starts.push(at);
        at += w.len();
        ends.insert(at);
    }
    PHRASES.iter().any(|p| {
        let whole_word_end = p.ends_with("no") || p.ends_with("num");
        starts
            .iter()
            .any(|&s| joined[s..].starts_with(p) && (!whole_word_end || ends.contains(&(s + p.len()))))
    })
}

/// Lower-case words: split at anything that isn't a letter or digit and at
/// camelCase humps ("txtCardNumber" → txt, card, number).
fn words_of(label: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for c in label.chars() {
        if !c.is_alphanumeric() {
            if !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
            continue;
        }
        if c.is_uppercase() && prev_lower && !cur.is_empty() {
            words.push(std::mem::take(&mut cur));
        }
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
        cur.extend(c.to_lowercase());
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
}

/// Whether text contains a card number (13–19 digits passing the Luhn check,
/// optionally in groups split by single spaces or dashes), a number written
/// like an SSN (3-2-4 digits, split by spaces or dashes), a valid IBAN, or
/// any other run of LONG_RUN or more digits; international phone numbers
/// excepted (see `is_phone`).
pub fn contains_sensitive_number(text: &str) -> bool {
    digit_runs(text)
        .iter()
        .any(|run| !is_phone(run) && (has_card(&run.groups) || has_ssn(&run.groups) || run_len(run) >= LONG_RUN))
        || has_iban(text)
}

/// Readings with a run of this many digits are dropped even when the run is
/// no valid card: a card being typed passes through 12–15 digit states that
/// fail Luhn. The owner's choice (2026-10-10), accepting that long order,
/// tracking and account-style numbers drop the reading too.
pub const LONG_RUN: usize = 12;

/// The longest international phone number (E.164).
const MAX_PHONE_DIGITS: usize = 15;

fn run_len(run: &DigitRun) -> usize {
    run.groups.iter().map(|g| g.len()).sum()
}

/// An international phone number: a leading "+" and at most 15 digits
/// ("+44 20 7946 0958", "+4915112345678"). Exempt from every number check:
/// unbroken 13–15 digit phone numbers pass Luhn one time in ten. A card
/// typed after a "+" still drops the reading once it passes 15 digits, so
/// only a 15-digit (Amex) number written that way could get through.
fn is_phone(run: &DigitRun) -> bool {
    run.plus && run_len(run) <= MAX_PHONE_DIGITS
}

/// A run of digits, as its groups: "4111 1111-1111" is one run of three
/// groups.
struct DigitRun {
    groups: Vec<String>,
    /// Written right after a "+", as international phone numbers are.
    plus: bool,
}

/// Each run of digits in the text. A run ends at anything other than a digit
/// or one separator followed by a digit.
fn digit_runs(text: &str) -> Vec<DigitRun> {
    let chars: Vec<char> = text.chars().collect();
    let mut runs = Vec::new();
    let mut groups: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut plus = false;
    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_digit() {
            if cur.is_empty() && groups.is_empty() {
                plus = i > 0 && chars[i - 1] == '+';
            }
            cur.push(c);
        } else if is_separator(c) && !cur.is_empty() && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit()) {
            groups.push(std::mem::take(&mut cur));
        } else {
            if !cur.is_empty() {
                groups.push(std::mem::take(&mut cur));
            }
            if !groups.is_empty() {
                runs.push(DigitRun { groups: std::mem::take(&mut groups), plus });
            }
        }
    }
    if !cur.is_empty() {
        groups.push(cur);
    }
    if !groups.is_empty() {
        runs.push(DigitRun { groups, plus });
    }
    runs
}

fn is_separator(c: char) -> bool {
    c == ' ' || c == '-' || c == '\u{a0}'
}

/// Consecutive groups holding 13–19 digits, grouped the way cards are
/// written, that pass the Luhn check. Only whole groups are tried, so a card
/// followed by its CVC ("4111 … 1111 123") is still found, while a long
/// unbroken ID isn't cut up into candidates.
fn has_card(groups: &[String]) -> bool {
    for start in 0..groups.len() {
        for end in start..groups.len() {
            let candidate = &groups[start..=end];
            let digits: String = candidate.concat();
            if digits.len() > 19 {
                break;
            }
            if digits.len() >= 13 && card_shaped(candidate) && luhn(&digits) {
                return true;
            }
        }
    }
    false
}

/// One unbroken number, fours with a shorter last group (4-4-4-4,
/// 4-4-4-4-3, 4-4-4-1), or the Amex and Diners layouts (4-6-5, 4-6-4). An
/// order number written 3-7-7 or a phone number is not card-shaped.
fn card_shaped(groups: &[String]) -> bool {
    let lens: Vec<usize> = groups.iter().map(|g| g.len()).collect();
    match lens.as_slice() {
        [_] => true,
        [4, 6, 5] | [4, 6, 4] => true,
        [init @ .., last] => init.iter().all(|&l| l == 4) && (1..=4).contains(last),
        [] => false,
    }
}

/// The Luhn checksum of a string of ASCII digits.
pub fn luhn(digits: &str) -> bool {
    let mut sum = 0u32;
    for (i, d) in digits.bytes().rev().enumerate() {
        let mut v = u32::from(d - b'0');
        if i % 2 == 1 {
            v *= 2;
            if v > 9 {
                v -= 9;
            }
        }
        sum += v;
    }
    sum.is_multiple_of(10)
}

/// Three consecutive groups of 3, 2 and 4 digits: 123-45-6789, 123 45 6789.
fn has_ssn(groups: &[String]) -> bool {
    groups.windows(3).any(|w| w[0].len() == 3 && w[1].len() == 2 && w[2].len() == 4)
}

/// An IBAN: two capital letters and two check digits, then letters and
/// digits (optionally in space-separated groups) to 15–34 characters in all,
/// passing the mod-97 check. Only lengths ending at a group boundary are
/// tried.
fn has_iban(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    for i in 0..chars.len() {
        let at = |k: usize| chars.get(i + k).copied().unwrap_or(' ');
        if i > 0 && chars[i - 1].is_alphanumeric() {
            continue;
        }
        if !(at(0).is_ascii_uppercase() && at(1).is_ascii_uppercase() && at(2).is_ascii_digit() && at(3).is_ascii_digit()) {
            continue;
        }
        let mut code = String::new();
        let mut j = i;
        while j < chars.len() && code.len() < 34 {
            let c = chars[j];
            if c.is_ascii_digit() || c.is_ascii_uppercase() {
                code.push(c);
                j += 1;
                let boundary = chars.get(j).is_none_or(|n| !(n.is_ascii_digit() || n.is_ascii_uppercase()));
                if boundary && code.len() >= 15 && iban_valid(&code) {
                    return true;
                }
            } else if c == ' ' && chars.get(j + 1).is_some_and(|n| n.is_ascii_digit() || n.is_ascii_uppercase()) {
                j += 1;
            } else {
                break;
            }
        }
    }
    false
}

fn iban_valid(code: &str) -> bool {
    let rearranged = code[4..].chars().chain(code[..4].chars());
    let mut rem = 0u32;
    for c in rearranged {
        let v = if c.is_ascii_digit() { c as u32 - '0' as u32 } else { c as u32 - 'A' as u32 + 10 };
        rem = if v >= 10 { (rem * 100 + v) % 97 } else { (rem * 10 + v) % 97 };
    }
    rem == 1
}

/// The executables that are web browsers: their fields are also checked
/// against the paused sites (by the URL of every page or frame they sit in).
pub const BROWSERS: [&str; 7] =
    ["chrome.exe", "msedge.exe", "firefox.exe", "brave.exe", "opera.exe", "vivaldi.exe", "arc.exe"];

pub fn is_browser(exe: &str) -> bool {
    BROWSERS.iter().any(|b| b.eq_ignore_ascii_case(exe))
}

/// Apps and sites Glim never captures in. Built in for now; Phase 1d's
/// settings make both lists editable.
#[derive(Clone, Debug, PartialEq)]
pub struct PauseList {
    /// Executable names, any case.
    pub apps: Vec<String>,
    /// "example.com" pauses the host and its subdomains; "example.com/path"
    /// only under that path; "*/segment" any page with that path segment.
    pub sites: Vec<String>,
}

/// Password managers.
const PAUSED_APPS: [&str; 12] = [
    "1Password.exe", "Bitwarden.exe", "KeePass.exe", "KeePassXC.exe", "Dashlane.exe", "LastPass.exe",
    "NordPass.exe", "Enpass.exe", "RoboForm.exe", "Proton Pass.exe", "KeeperPasswordManager.exe",
    "CredentialUIBroker.exe",
];

/// Checkout, payment and banking sites, and checkout pages anywhere.
const PAUSED_SITES: [&str; 33] = [
    "paypal.com", "checkout.stripe.com", "js.stripe.com", "pay.google.com", "payments.google.com",
    "pay.amazon.com", "amazon.com/gp/buy", "squareup.com/checkout", "venmo.com", "cash.app", "wise.com",
    "zelle.com", "chase.com", "bankofamerica.com", "wellsfargo.com", "citi.com", "capitalone.com",
    "usbank.com", "americanexpress.com", "discover.com", "schwab.com", "fidelity.com", "ally.com",
    "pnc.com", "truist.com", "navyfederal.org", "1password.com", "bitwarden.com", "lastpass.com",
    "*/checkout", "*/checkouts", "*/payment", "*/payments",
];

impl Default for PauseList {
    fn default() -> Self {
        PauseList {
            apps: PAUSED_APPS.iter().map(|s| s.to_string()).collect(),
            sites: PAUSED_SITES.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl PauseList {
    pub fn pauses_app(&self, exe: &str) -> bool {
        self.apps.iter().any(|a| a.eq_ignore_ascii_case(exe))
    }

    /// Whether a page or frame URL is on a paused site. Only http(s) URLs
    /// have a site; anything else (file:, about:) is not paused.
    pub fn pauses_url(&self, url: &str) -> bool {
        let Some((host, path)) = host_and_path(url) else { return false };
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        self.sites.iter().any(|site| {
            let site = site.to_ascii_lowercase();
            let (site_host, site_path) = site.split_once('/').unwrap_or((&site, ""));
            if site_host == "*" {
                return !site_path.is_empty() && segments.contains(&site_path);
            }
            let host_ok = host == site_host || host.ends_with(&format!(".{site_host}"));
            host_ok && (site_path.is_empty() || path.trim_start_matches('/').starts_with(site_path))
        })
    }
}

/// (lower-case host, path) of an http or https URL.
fn host_and_path(url: &str) -> Option<(String, String)> {
    let lower = url.trim().to_ascii_lowercase();
    let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"))?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = host.split(':').next().unwrap_or(host).trim_end_matches('.');
    let path = rest[end..].split(['?', '#']).next().unwrap_or("");
    (!host.is_empty()).then(|| (host.to_string(), path.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test card numbers published by card networks and payment processors,
    // and SSNs from never-issued ranges: none belongs to anyone.
    const FAKE_CARDS: [&str; 10] = [
        "4111111111111111",
        "4111 1111 1111 1111",
        "4111-1111-1111-1111",
        "5555 5555 5555 4444",
        "5105-1051-0510-5100",
        "3782 822463 10005",
        "371449635398431",
        "6011 1111 1111 1117",
        "3566002020360505",
        "4222222222222",
    ];
    const FAKE_SSNS: [&str; 4] = ["123-45-6789", "078-05-1120", "219 09 9999", "987-65-4320"];
    const FAKE_IBANS: [&str; 3] = ["DE89 3704 0044 0532 0130 00", "GB82WEST12345698765432", "FR14 2004 1010 0505 0001 3M02 606"];

    #[test]
    fn fake_card_numbers_are_found_in_any_text() {
        for card in FAKE_CARDS {
            for text in [
                card.to_string(),
                format!("Hi, my card is {card}, thanks"),
                format!("card: {card} exp 12/27 cvc 123"),
                format!("{card} 123"),
                format!("first line\n\t{card}\nlast"),
            ] {
                assert!(contains_sensitive_number(&text), "missed: {text}");
            }
        }
    }

    #[test]
    fn fake_ssns_and_ibans_are_found() {
        for n in FAKE_SSNS.iter().chain(FAKE_IBANS.iter()) {
            assert!(contains_sensitive_number(n), "missed: {n}");
            assert!(contains_sensitive_number(&format!("Mine is {n}.")), "missed in a sentence: {n}");
        }
    }

    #[test]
    fn ordinary_text_with_numbers_gets_through() {
        for text in [
            "Call me at (555) 123-4567 or 555-123-4567, or +1 555 123 4567.",
            "UK office: +44 20 7946 0958",
            "Meet at 1600 Amphitheatre Parkway, Mountain View, CA 94043-1351.",
            "Unit 4B, 221 Baker St, Apt 12, ZIP 90210",
            "Invoice #2026-0042 total: $1,234.56 due 2026-10-31, PO 450012345.",
            "Rent of $2,450.00 for 10/2026; balance $13,005.75 after 3 payments.",
            "Version 0.2.3, build 17:06, 123-456-7890",
            "The 4111 units in 1111 buildings",
            "Card ending in 1111, exp 12/27.",
            "DE is Germany; GB12 is not an IBAN.",
        ] {
            assert!(!contains_sensitive_number(text), "dropped ordinary text: {text}");
        }
    }

    #[test]
    fn phone_numbers_get_through_even_when_long() {
        for text in [
            "Call (555) 123-4567, 555-123-4567, 555 123 4567 or +1 555 123 4567.",
            "+1-555-123-4567 ext. 89",
            "UK office: +44 20 7946 0958",
            "Berlin: +49 30 12345678, mobile +4915112345678",
            "Beijing: +86 138 0013 8000",
            "Paris: +33 1 23 45 67 89",
        ] {
            assert!(!contains_sensitive_number(text), "dropped a phone number: {text}");
        }
    }

    #[test]
    fn a_card_being_typed_is_dropped_from_12_digits() {
        // Each state a typed card passes through, though most fail Luhn.
        let card = "4111 1111 1111 1111";
        for len in 1..=card.len() {
            let typed = &card[..len];
            let digits = typed.chars().filter(|c| c.is_ascii_digit()).count();
            assert_eq!(contains_sensitive_number(&format!("my card is {typed}")), digits >= LONG_RUN, "{typed}");
        }
        for typed in ["411111111111", "4111-1111-1111-1", "3782 822463 10", "6011 1111 1111 11"] {
            assert!(contains_sensitive_number(typed), "{typed}");
        }
        // A "+" doesn't hide a 16-digit card, valid or not.
        assert!(contains_sensitive_number("+4111 1111 1111 1111"));
        assert!(contains_sensitive_number("+4111 1111 1111 1112"));
    }

    #[test]
    fn long_numbers_drop_the_reading_the_tradeoff() {
        // The stricter rule's cost: these hold no card, but 12+ digits.
        for text in [
            "Order 112-4567890-1234567 shipped",
            "Tracking 9400111899223397846523",
            "Account ref 123456789012",
            "Dial 0044 20 7946 0958",
            "Rooms 101 102 103 104",
        ] {
            assert!(contains_sensitive_number(text), "{text}");
        }
        assert!(!contains_sensitive_number("Ref 12345678901 (11 digits)"));
    }

    #[test]
    fn a_number_that_fails_luhn_is_not_a_card() {
        assert!(!luhn("4111111111111112"));
        assert!(luhn("79927398713"));
        assert!(!has_card(&["4111".into(), "1111".into(), "1111".into(), "1112".into()]));
    }

    #[test]
    fn payment_and_id_labels_are_sensitive() {
        for label in [
            "Card number", "cardNumber", "card_number", "CARD NUMBER", "txtCardNumber", "cc-number", "cc-csc",
            "cc-exp", "cc-exp-month", "cc-name", "ccNumber", "Credit card", "Debit card number", "CVC", "CVV2",
            "Security code", "Expiry date", "Expiration (MM/YY)", "MM / YY", "Cardholder name", "Name on card",
            "Bank account", "Account number", "Routing number", "routingNo", "IBAN", "Sort code", "SSN",
            "Social Security Number", "socialSecurityNumber",
        ] {
            assert!(looks_sensitive(label), "not caught: {label}");
        }
    }

    #[test]
    fn everyday_labels_are_not_sensitive() {
        for label in [
            "Cc", "Bcc", "To", "Subject", "Message Body", "Text editor", "Page 1 content", "Notes", "Search",
            "Address and search bar", "Phone number", "Email address", "Street address", "Comment", "Description",
            "Card title", "Discard", "Scorecard notes", "Card notes", "Account notes", "Account name", "Caribbean trip", "Lesson plan", "Recipient",
        ] {
            assert!(!looks_sensitive(label), "wrongly caught: {label}");
        }
    }

    #[test]
    fn any_label_of_a_field_can_mark_it() {
        let blank = FieldLabels::default();
        assert!(!is_sensitive_field(&blank));
        for l in [
            FieldLabels { name: "Card number".into(), ..Default::default() },
            FieldLabels { labeled_by: "Expiry date".into(), ..Default::default() },
            FieldLabels { help_text: "Security code".into(), ..Default::default() },
            FieldLabels { description: "Routing number".into(), ..Default::default() },
            FieldLabels { automation_id: "cardNumber".into(), ..Default::default() },
        ] {
            assert!(is_sensitive_field(&l), "{l:?}");
        }
    }

    #[test]
    fn the_pause_list_covers_password_managers_and_payment_sites() {
        let p = PauseList::default();
        assert!(p.pauses_app("1Password.exe") && p.pauses_app("keepassxc.exe") && p.pauses_app("BITWARDEN.EXE"));
        assert!(!p.pauses_app("notepad.exe") && !p.pauses_app("WINWORD.EXE") && !p.pauses_app("chrome.exe"));
        for url in [
            "https://www.paypal.com/checkoutnow?token=x",
            "https://secure.chase.com/web/auth/dashboard",
            "https://js.stripe.com/v3/elements-inner-card.html",
            "https://www.amazon.com/gp/buy/spc/handlers/display.html",
            "https://shop.example.com/checkout/payment",
            "https://store.example.com/checkouts/abc123",
            "HTTPS://WWW.WELLSFARGO.COM:443/",
        ] {
            assert!(p.pauses_url(url), "not paused: {url}");
        }
        for url in [
            "https://mail.google.com/mail/u/0/#inbox",
            "https://www.amazon.com/dp/B000",
            "https://example.appfolio.com/notes",
            "https://notchase.com/",
            "https://example.com/blog/checkout-tips",
            "file:///C:/page.html",
            "about:srcdoc",
            "",
        ] {
            assert!(!p.pauses_url(url), "wrongly paused: {url}");
        }
    }

    #[test]
    fn browsers_are_known_by_executable() {
        assert!(is_browser("chrome.exe") && is_browser("MSEDGE.EXE") && is_browser("firefox.exe"));
        assert!(!is_browser("msedgewebview2.exe") && !is_browser("notepad.exe"));
    }
}
