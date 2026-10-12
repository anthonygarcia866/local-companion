// Glim's brain (1c, docs/phase1-plan.md §13, docs/vision.md §2): the
// Claude API by default. The key (key.rs) is in Windows Credential Manager;
// requests go only through net::request (the network checkpoint).

pub mod key;
