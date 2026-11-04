// The shield's tests. Every name, address and number here is made up.

use super::detect::{find, Span};
use super::never_send::{reason, Reason};
use super::*;

/// A fake address book, as Outlook writes its entries.
const BOOK: [&str; 7] = [
    "Lopez, Maria (Leasing)",
    "Sam Nguyen",
    "Okafor, Dana",
    "Leasing Office",
    "Maintenance Team",
    "maintenance@example.com",
    "Will Bishop",
];

fn names() -> NameSet {
    NameSet::from_entries(BOOK.iter().copied().chain(["Priya"]))
}

fn kinds(text: &str) -> Vec<(Kind, &str)> {
    find(text, &names()).into_iter().map(|Span { start, end, kind }| (kind, &text[start..end])).collect()
}

#[test]
fn names_come_from_the_address_book_the_list_and_recipients() {
    let n = names();
    for w in ["maria", "lopez", "sam", "nguyen", "dana", "okafor", "bishop", "priya"] {
        assert!(n.words.contains(w), "{w}");
    }
    // Stop words, the parenthesised department, and addresses stay out.
    for w in ["leasing", "office", "maintenance", "team", "will"] {
        assert!(!n.words.contains(w), "{w}");
    }
    // "Lopez, Maria" is matched written either way.
    assert!(n.full.contains(&vec!["maria".to_string(), "lopez".to_string()]));
    assert!(n.full.contains(&vec!["lopez".to_string(), "maria".to_string()]));
}

#[test]
fn each_kind_is_found() {
    let text = "Hi Maria Lopez, call Sam at (555) 123-4567 or sam.nguyen@example.com. \
                The leak is at 4521 N Oak Grove Ave, Unit 7D; also Units 101, 102 and 104 and #204. \
                Will Bishop will stop by. PO Box 88 for mail.";
    let got = kinds(text);
    let want: Vec<(Kind, &str)> = vec![
        (Kind::Person, "Maria Lopez"),
        (Kind::Person, "Sam"),
        (Kind::Phone, "(555) 123-4567"),
        (Kind::Email, "sam.nguyen@example.com"),
        (Kind::Address, "4521 N Oak Grove Ave"),
        (Kind::Unit, "7D"),
        (Kind::Unit, "101"),
        (Kind::Unit, "102"),
        (Kind::Unit, "104"),
        (Kind::Unit, "204"),
        (Kind::Person, "Will Bishop"),
        (Kind::Address, "PO Box 88"),
    ];
    assert_eq!(got, want);
}

#[test]
fn phones_in_their_usual_shapes_and_not_dates_or_money() {
    for p in ["555-123-4567", "555.123.4567", "555 123 4567", "5551234567", "1-555-123-4567", "+1 555 123 4567", "+44 20 7946 0958", "555-0123"] {
        let text = format!("Call {p} today.");
        assert_eq!(kinds(&text), vec![(Kind::Phone, p)], "{p}");
    }
    for no in ["On 2026-11-03 at 10.30", "Rent is $1,250.00", "Order 12-345", "Pay 4 units", "version 1.2.3"] {
        assert!(kinds(no).iter().all(|(k, _)| *k != Kind::Phone), "{no}");
    }
}

#[test]
fn spoken_and_written_emails() {
    assert_eq!(kinds("write to jordan at example dot com please"), vec![(Kind::Email, "jordan at example dot com")]);
    assert_eq!(kinds("cc j dot lee at mail dot co dot uk"), vec![(Kind::Email, "j dot lee at mail dot co dot uk")]);
    assert!(kinds("meet at noon").is_empty());
}

#[test]
fn lowercase_words_and_stop_words_are_not_names() {
    // "will" the verb, "may" the month, a lower-case "sam" in a word list.
    assert!(kinds("We will see what may happen.").is_empty());
    assert!(kinds("the leasing office called").is_empty());
    assert_eq!(kinds("Thanks, Dana!"), vec![(Kind::Person, "Dana")]);
    // Possessive: the name, not the 's.
    assert_eq!(kinds("Maria's lease"), vec![(Kind::Person, "Maria")]);
}

#[test]
fn a_round_trip_gives_back_the_real_values() {
    let text = "Maria, Sam Nguyen at (555) 123-4567 said Maria can come to Unit 7D.";
    let m = mask_with(text, &names()).unwrap();
    assert_eq!(m.text, "[PERSON1], [PERSON2] at [PHONE1] said [PERSON1] can come to Unit [UNIT1].");
    assert_eq!(m.counts(), [2, 0, 1, 0, 1]);
    // The answer, reworded around the placeholders.
    let answer = "[PERSON1]: [PERSON2] (at [PHONE1]) said [PERSON1] can visit Unit [UNIT1].";
    assert_eq!(unmask(answer, &m, Check::Whole).unwrap(), "Maria: Sam Nguyen (at (555) 123-4567) said Maria can visit Unit 7D.");
    // One mention of a name sent twice is fine (tightened wording).
    let shorter = "[PERSON2] at [PHONE1] says [PERSON1] can visit Unit [UNIT1].";
    assert!(unmask(shorter, &m, Check::Whole).is_ok());
}

#[test]
fn a_dropped_doubled_or_altered_placeholder_discards_the_answer() {
    let m = mask_with("Ask Sam Nguyen about Unit 7D.", &names()).unwrap();
    assert_eq!(m.text, "Ask [PERSON1] about Unit [UNIT1].");
    assert_eq!(unmask("Ask about Unit [UNIT1].", &m, Check::Whole), Err(Mismatch::Missing));
    assert_eq!(unmask("Ask [PERSON1] and [PERSON1] about Unit [UNIT1].", &m, Check::Whole), Err(Mismatch::Duplicated));
    for altered in [
        "Ask [Person1] about Unit [UNIT1].",
        "Ask [PERSON 1] about Unit [UNIT1].",
        "Ask [PERSON] about Unit [UNIT1].",
        "Ask PERSON1 about Unit [UNIT1].",
        "Ask [PERSON1] about Unit [UNIT2].",
        "Ask [PERSON1] about Unit UNIT_1.",
        "Ask [PERSON1] and [EMAIL1] about Unit [UNIT1].",
    ] {
        assert_eq!(unmask(altered, &m, Check::Whole), Err(Mismatch::Altered), "{altered}");
    }
    // Partial: leaving one out is fine; changing one never is.
    assert_eq!(unmask("Fix the unit number.", &m, Check::Partial).unwrap(), "Fix the unit number.");
    assert_eq!(unmask("Ask [PERSON1].", &m, Check::Partial).unwrap(), "Ask Sam Nguyen.");
    assert_eq!(unmask("Ask [PERSON2].", &m, Check::Partial), Err(Mismatch::Altered));
}

#[test]
fn the_users_own_placeholder_lookalikes_pass_but_are_never_reused() {
    let m = mask_with("Unit1 and [PERSON1] need Sam.", &names()).unwrap();
    // Glim's own number skips the one already written.
    assert_eq!(m.text, "Unit1 and [PERSON1] need [PERSON2].");
    assert_eq!(unmask("Unit1 and [PERSON1] need [PERSON2].", &m, Check::Whole).unwrap(), "Unit1 and [PERSON1] need Sam.");
    // Not more often than written.
    assert_eq!(unmask("Unit1, Unit1 and [PERSON1] need [PERSON2].", &m, Check::Whole), Err(Mismatch::Altered));
}

#[test]
fn several_texts_share_one_map() {
    let (parts, m) = mask_many(&["Email Maria about it", "Glim asked: which Maria?"], &names()).unwrap();
    assert_eq!(parts, ["Email [PERSON1] about it", "Glim asked: which [PERSON1]?"]);
    let back = unmask_many(&["[PERSON1]", "About Unit 4"], &m, Check::Partial).unwrap();
    assert_eq!(back, ["Maria", "About Unit 4"]);
}

#[test]
fn the_map_never_shows_in_debug() {
    let m = mask_with("Sam Nguyen, sam.nguyen@example.com, 555-123-4567", &names()).unwrap();
    let shown = format!("{m:?}");
    for leak in ["Sam", "Nguyen", "example", "555", "PERSON"] {
        assert!(!shown.contains(leak), "{leak}: {shown}");
    }
    assert!(shown.contains("person=1 email=1 phone=1 address=0 unit=0"), "{shown}");
}

#[test]
fn nothing_to_swap_leaves_the_text_alone() {
    let text = "Thanks for the update, see you tomorrow.";
    let m = mask_with(text, &names()).unwrap();
    assert_eq!(m.text, text);
    assert!(m.is_empty());
    assert_eq!(unmask(text, &m, Check::Whole).unwrap(), text);
}

#[test]
fn never_send_blocks_numbers_ssns_birth_dates_and_income() {
    let blocked = [
        ("Charge 4111 1111 1111 1111 please.", Reason::Number),
        ("Her SSN is 123-45-6789.", Reason::Number),
        ("SSN 123456789 on file", Reason::Ssn),
        ("social security number: 123 45 6789", Reason::Number),
        ("DOB: 03/04/1980", Reason::BirthDate),
        ("Dana Okafor, date of birth 1980-03-04, applied", Reason::BirthDate),
        ("born on March 4, 1980", Reason::BirthDate),
        ("D.O.B. 4 March 1980", Reason::BirthDate),
        ("Monthly income is $3,200 before taxes.", Reason::Income),
        ("He gets $914 in SSI each month.", Reason::Income),
        ("SNAP benefits: 250 dollars", Reason::Income),
        ("Salary $85k", Reason::Income),
        ("child support of $400", Reason::Income),
    ];
    for (text, why) in blocked {
        assert_eq!(reason(text), Some(why), "{text}");
    }
    let fine = [
        "Rent is $1,250 due on the 1st.",
        "The late fee is $50.",
        "Units 101 102 103 104 need new smoke detectors.",
        "Her birthday party is on March 4.",
        "Our income statement is attached.",
        "I was born in Ohio.",
        "Meeting on 2026-11-03 at 10:30.",
        "Call 555-123-4567.",
    ];
    for text in fine {
        assert_eq!(reason(text), None, "{text}");
    }
}

#[test]
fn the_switch_and_the_sources() {
    let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::settings::Settings::default();
    assert!(s.privacy_shield, "on by default");
    s.shield_names = vec!["Zephyrine Quill".into()];
    s.draft_recipients.insert("ottoline".into(), "ottoline@example.com".into());
    apply_settings(&s);
    set_address_book(vec!["Vonk, Thaddeus".into()]);
    let m = mask("Zephyrine, Ottoline and Thaddeus Vonk").unwrap();
    assert_eq!(m.text, "[PERSON1], [PERSON2] and [PERSON3]");
    assert!(name_count() >= 5);
    s.privacy_shield = false;
    apply_settings(&s);
    assert_eq!(mask("Zephyrine, Ottoline and Thaddeus Vonk").unwrap().text, "Zephyrine, Ottoline and Thaddeus Vonk");
    // Back to the default for the other tests in this process.
    apply_settings(&crate::settings::Settings::default());
    set_address_book(Vec::new());
}
