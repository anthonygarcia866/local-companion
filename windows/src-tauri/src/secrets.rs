// The secret vault (local agency v1, part 1; owner 2026-11-12).
//
// Settings → Secrets → "Add a secret": a name and a masked value. The value
// goes to Rust once, on Save, and into Windows Credential Manager as a
// generic credential `Glim/<name>` (DPAPI-protected, this user on this PC
// only: CRED_PERSIST_LOCAL_MACHINE, never roamed). It is never written to
// disk in plain text, never logged, never put in an event or an IPC answer,
// never sent to the AI or the network: the webview only ever gets back
// names and saved dates (`SecretInfo`).
//
// The only reader is a runbook's `env_secret` step (part 2), which writes it
// into a gitignored file the owner chose; `SecretValue::reveal` is called
// there and nowhere else (a test checks the source).
//
// Tests use `MemoryStore`; one test does a real round trip through
// Credential Manager under a test-only prefix, so the owner's are never
// touched.

use serde::Serialize;

use crate::i18n::{t, tf};

/// What every secret's credential name starts with.
pub const PREFIX: &str = "Glim/";

/// Credential Manager's limit for a generic credential's secret, in bytes.
pub const MAX_VALUE_BYTES: usize = 2560;

/// The longest name.
pub const MAX_NAME_CHARS: usize = 64;

/// Names Glim already uses for its own credentials (ai/key.rs, phone.rs),
/// never listed, replaced or deleted from here. Case doesn't matter.
pub const RESERVED: &[&str] = &["AnthropicApiKey", "PushoverKeys", "RunbookApprovalKey"];

/// A saved value, as only a runbook step may see it.
pub struct SecretValue(String);

impl SecretValue {
    /// A value for tests (runbooks' fakes).
    #[cfg(test)]
    pub fn for_test(v: &str) -> SecretValue {
        SecretValue(v.to_string())
    }

    /// The value itself: for a runbook's env_secret write, nothing else.
    pub fn reveal(&self) -> &str {
        &self.0
    }
}

impl Drop for SecretValue {
    fn drop(&mut self) {
        // Best effort: our copy doesn't linger in freed memory.
        // SAFETY: zeroes are valid UTF-8.
        unsafe { self.0.as_bytes_mut().iter_mut().for_each(|b| *b = 0) };
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretValue(hidden)")
    }
}

impl std::fmt::Display for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(hidden)")
    }
}

/// What Settings may know about a secret: its name and when it was saved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretInfo {
    pub name: String,
    /// Milliseconds since 1970 (UTC), from Credential Manager's LastWritten.
    pub saved_ms: u64,
}

/// Where secrets are kept: Credential Manager in the app, memory in tests.
pub trait Store {
    /// Every `Glim/` credential's name (without the prefix) and its date.
    fn list(&self) -> Result<Vec<SecretInfo>, String>;
    fn read(&self, name: &str) -> Result<Option<String>, String>;
    fn write(&self, name: &str, value: &str) -> Result<(), String>;
    fn delete(&self, name: &str) -> Result<(), String>;
}

/// A typed name, checked: 1-64 letters, digits, `_`, `-` or `.`, starting
/// with a letter or digit, and not one of Glim's own.
pub fn check_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(t("Give the secret a name first."));
    }
    let ok = name.chars().count() <= MAX_NAME_CHARS
        && name.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if !ok {
        return Err(t("A name can use letters, digits, - _ and . (up to 64), starting with a letter or digit."));
    }
    if is_reserved(name) {
        return Err(t("Glim uses that name for its own keys. Pick another."));
    }
    Ok(name.to_string())
}

fn is_reserved(name: &str) -> bool {
    RESERVED.iter().any(|r| r.eq_ignore_ascii_case(name)) || name.to_ascii_lowercase().starts_with("testonly")
}

/// A pasted value, cleaned up: surrounding spaces and line breaks go. The
/// error never quotes the value.
pub fn check_value(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(t("Paste the secret first."));
    }
    if value.len() > MAX_VALUE_BYTES {
        return Err(t("That secret is too long for Windows Credential Manager (2,560 bytes at most)."));
    }
    if value.chars().any(|c| c == '\n' || c == '\r' || c == '\0') {
        return Err(t("A secret must be one line."));
    }
    Ok(value.to_string())
}

/// Settings' list: names and dates only, Glim's own keys left out, by name.
pub fn list(store: &dyn Store) -> Result<Vec<SecretInfo>, String> {
    let mut all: Vec<SecretInfo> = store.list()?.into_iter().filter(|s| check_name(&s.name).is_ok()).collect();
    all.sort_by_key(|s| s.name.to_ascii_lowercase());
    Ok(all)
}

/// Saves (or replaces) a secret and returns only what Settings may show.
pub fn save(store: &dyn Store, name: &str, raw: &str) -> Result<SecretInfo, String> {
    let name = check_name(name)?;
    let mut value = check_value(raw)?;
    let written = store.write(&name, &value);
    // SAFETY: zeroes are valid UTF-8.
    unsafe { value.as_bytes_mut().iter_mut().for_each(|b| *b = 0) };
    written?;
    list(store)?
        .into_iter()
        .find(|s| s.name.eq_ignore_ascii_case(&name))
        .ok_or_else(|| t("Windows didn't keep the secret. Try again."))
}

/// Settings → Secrets → Delete (after the page asked). Deleting one that
/// isn't there is fine.
pub fn delete(store: &dyn Store, name: &str) -> Result<(), String> {
    let name = check_name(name)?;
    store.delete(&name)
}

/// The value, for a runbook step only.
pub fn read(store: &dyn Store, name: &str) -> Result<Option<SecretValue>, String> {
    let name = check_name(name)?;
    Ok(store.read(&name)?.filter(|v| !v.is_empty()).map(SecretValue))
}

/// The store the app uses.
pub fn store() -> CredentialStore {
    CredentialStore { prefix: PREFIX }
}

/// Saves from Settings, logging only the outcome and the name.
pub fn save_logged(store: &dyn Store, name: &str, raw: &str) -> Result<SecretInfo, String> {
    let saved = save(store, name, raw);
    match &saved {
        Ok(info) => crate::log::line(format!("secrets: saved {}", info.name)),
        Err(_) => crate::log::line("secrets: not saved"),
    }
    saved
}

/// Deletes from Settings, logging only the outcome and the name.
pub fn delete_logged(store: &dyn Store, name: &str) -> Result<(), String> {
    let deleted = delete(store, name);
    match (&deleted, check_name(name)) {
        (Ok(()), Ok(n)) => crate::log::line(format!("secrets: deleted {n}")),
        _ => crate::log::line("secrets: not deleted"),
    }
    deleted
}

/// Windows Credential Manager, the credentials named `<prefix><name>`.
pub struct CredentialStore {
    pub prefix: &'static str,
}

impl CredentialStore {
    fn target(&self, name: &str) -> String {
        format!("{}{name}", self.prefix)
    }
}

/// FILETIME (100 ns since 1601) to milliseconds since 1970.
pub fn filetime_ms(high: u32, low: u32) -> u64 {
    let ticks = ((high as u64) << 32) | low as u64;
    ticks.saturating_sub(116_444_736_000_000_000) / 10_000
}

#[cfg(windows)]
mod windows_store {
    use super::*;
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::ERROR_NOT_FOUND;
    use windows::Win32::Security::Credentials::{
        CredDeleteW, CredEnumerateW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn not_found(e: &windows::core::Error) -> bool {
        e.code() == ERROR_NOT_FOUND.to_hresult()
    }

    /// A NUL-terminated wide string, read.
    unsafe fn from_wide(p: PWSTR) -> String {
        if p.is_null() {
            return String::new();
        }
        unsafe { p.to_string().unwrap_or_default() }
    }

    impl Store for CredentialStore {
        fn list(&self) -> Result<Vec<SecretInfo>, String> {
            let filter = wide(&format!("{}*", self.prefix));
            let mut count = 0u32;
            let mut creds: *mut *mut CREDENTIALW = std::ptr::null_mut();
            match unsafe { CredEnumerateW(PCWSTR(filter.as_ptr()), None, &mut count, &mut creds) } {
                Err(e) if not_found(&e) => Ok(Vec::new()),
                Err(e) => Err(tf("Windows Credential Manager refused: {error}", &[("error", &e.message())])),
                Ok(()) => {
                    let mut out = Vec::new();
                    // SAFETY: CredEnumerateW succeeded: `creds` holds `count`
                    // pointers to CREDENTIALW, valid until CredFree. Only the
                    // name and date are read; the blob is never touched.
                    unsafe {
                        for i in 0..count as usize {
                            let c = &**creds.add(i);
                            if c.Type != CRED_TYPE_GENERIC {
                                continue;
                            }
                            let target = from_wide(c.TargetName);
                            if let Some(name) = target.strip_prefix(self.prefix) {
                                out.push(SecretInfo {
                                    name: name.to_string(),
                                    saved_ms: filetime_ms(c.LastWritten.dwHighDateTime, c.LastWritten.dwLowDateTime),
                                });
                            }
                        }
                        CredFree(creds as *const _);
                    }
                    Ok(out)
                }
            }
        }

        fn read(&self, name: &str) -> Result<Option<String>, String> {
            let target = wide(&self.target(name));
            let mut cred: *mut CREDENTIALW = std::ptr::null_mut();
            match unsafe { CredReadW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None, &mut cred) } {
                Err(e) if not_found(&e) => Ok(None),
                Err(e) => Err(tf("Windows Credential Manager refused: {error}", &[("error", &e.message())])),
                Ok(()) => {
                    // SAFETY: CredReadW succeeded, so `cred` points to a
                    // CREDENTIALW whose blob holds CredentialBlobSize bytes;
                    // both stay valid until CredFree.
                    let secret = unsafe {
                        let c = &*cred;
                        let bytes = if c.CredentialBlob.is_null() {
                            &[][..]
                        } else {
                            std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize)
                        };
                        let s = String::from_utf8(bytes.to_vec());
                        CredFree(cred as *const _);
                        s
                    };
                    secret.map(Some).map_err(|_| t("The saved secret is damaged. Replace it."))
                }
            }
        }

        fn write(&self, name: &str, value: &str) -> Result<(), String> {
            let mut target = wide(&self.target(name));
            let mut user = wide("Glim");
            let mut blob = value.as_bytes().to_vec();
            let cred = CREDENTIALW {
                Type: CRED_TYPE_GENERIC,
                TargetName: PWSTR(target.as_mut_ptr()),
                CredentialBlobSize: blob.len() as u32,
                CredentialBlob: blob.as_mut_ptr(),
                Persist: CRED_PERSIST_LOCAL_MACHINE,
                UserName: PWSTR(user.as_mut_ptr()),
                ..Default::default()
            };
            let result = unsafe { CredWriteW(&cred, 0) };
            blob.iter_mut().for_each(|b| *b = 0);
            result.map_err(|e| tf("Windows Credential Manager refused: {error}", &[("error", &e.message())]))
        }

        fn delete(&self, name: &str) -> Result<(), String> {
            let target = wide(&self.target(name));
            match unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) } {
                Ok(()) => Ok(()),
                Err(e) if not_found(&e) => Ok(()),
                Err(e) => Err(tf("Windows Credential Manager refused: {error}", &[("error", &e.message())])),
            }
        }
    }
}

#[cfg(not(windows))]
impl Store for CredentialStore {
    fn list(&self) -> Result<Vec<SecretInfo>, String> {
        Ok(Vec::new())
    }
    fn read(&self, _name: &str) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn write(&self, _name: &str, _value: &str) -> Result<(), String> {
        Err(t("Secrets are only available on Windows for now."))
    }
    fn delete(&self, _name: &str) -> Result<(), String> {
        Ok(())
    }
}

/// An in-memory store, for tests (here and in runbooks.rs).
#[cfg(test)]
#[derive(Default)]
pub struct MemoryStore(pub std::cell::RefCell<std::collections::BTreeMap<String, (String, u64)>>);

#[cfg(test)]
impl Store for MemoryStore {
    fn list(&self) -> Result<Vec<SecretInfo>, String> {
        Ok(self.0.borrow().iter().map(|(k, (_, ms))| SecretInfo { name: k.clone(), saved_ms: *ms }).collect())
    }
    fn read(&self, name: &str) -> Result<Option<String>, String> {
        Ok(self.0.borrow().get(name).map(|(v, _)| v.clone()))
    }
    fn write(&self, name: &str, value: &str) -> Result<(), String> {
        let n = self.0.borrow().len() as u64;
        self.0.borrow_mut().insert(name.to_string(), (value.to_string(), 1_700_000_000_000 + n));
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<(), String> {
        self.0.borrow_mut().remove(name);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALUE: &str = "placeholder-SENTINEL-0123456789abcdef";

    #[test]
    fn names_are_checked_and_glims_own_are_refused() {
        assert_eq!(check_name("  EXAMPLE_TOKEN ").unwrap(), "EXAMPLE_TOKEN");
        for ok in ["a", "Z9", "example.token-2", &"x".repeat(64)] {
            assert!(check_name(ok).is_ok(), "{ok:?}");
        }
        for bad in ["", " ", "-x", "_x", ".x", "a b", "a/b", "a\\b", "a*", "naïve", &"x".repeat(65)] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
        for own in ["AnthropicApiKey", "anthropicapikey", "PushoverKeys", "RunbookApprovalKey", "TestOnly-DeleteMe", "testonlyx"] {
            assert!(check_name(own).is_err(), "{own:?}");
        }
    }

    #[test]
    fn values_are_trimmed_one_line_and_errors_never_quote_them() {
        assert_eq!(check_value(&format!("  {VALUE}\r\n")).unwrap(), VALUE);
        for bad in [String::new(), "   ".into(), format!("{VALUE}\n{VALUE}"), "x".repeat(MAX_VALUE_BYTES + 1), format!("a\0{VALUE}")] {
            let e = check_value(&bad).unwrap_err();
            assert!(!e.contains("SENTINEL"), "{e}");
        }
    }

    #[test]
    fn settings_only_ever_sees_names_and_dates() {
        let s = MemoryStore::default();
        assert!(list(&s).unwrap().is_empty());
        let info = save(&s, "EXAMPLE_TOKEN", VALUE).unwrap();
        assert_eq!(info.name, "EXAMPLE_TOKEN");
        assert!(info.saved_ms > 0);
        // What goes back over IPC after a save, and the list.
        let shown = format!("{} {}", serde_json::to_string(&info).unwrap(), serde_json::to_string(&list(&s).unwrap()).unwrap());
        assert!(!shown.contains("SENTINEL"), "{shown}");
        assert!(shown.contains("savedMs"), "{shown}");
        assert_eq!(read(&s, "EXAMPLE_TOKEN").unwrap().unwrap().reveal(), VALUE);
    }

    #[test]
    fn replace_and_delete() {
        let s = MemoryStore::default();
        save(&s, "EXAMPLE_TOKEN", VALUE).unwrap();
        save(&s, "EXAMPLE_TOKEN", "placeholder-new").unwrap();
        assert_eq!(read(&s, "EXAMPLE_TOKEN").unwrap().unwrap().reveal(), "placeholder-new");
        assert_eq!(list(&s).unwrap().len(), 1);
        // A bad paste keeps the saved one.
        assert!(save(&s, "EXAMPLE_TOKEN", "  ").is_err());
        assert_eq!(read(&s, "EXAMPLE_TOKEN").unwrap().unwrap().reveal(), "placeholder-new");
        delete(&s, "EXAMPLE_TOKEN").unwrap();
        assert!(read(&s, "EXAMPLE_TOKEN").unwrap().is_none());
        delete(&s, "EXAMPLE_TOKEN").unwrap();
        assert!(list(&s).unwrap().is_empty());
    }

    #[test]
    fn glims_own_credentials_are_never_listed() {
        let s = MemoryStore::default();
        s.write("AnthropicApiKey", VALUE).unwrap();
        s.write("PushoverKeys", VALUE).unwrap();
        s.write("b", VALUE).unwrap();
        s.write("A", VALUE).unwrap();
        let names: Vec<String> = list(&s).unwrap().into_iter().map(|i| i.name).collect();
        assert_eq!(names, ["A", "b"]);
        assert!(delete(&s, "AnthropicApiKey").is_err());
        assert!(s.read("AnthropicApiKey").unwrap().is_some());
    }

    #[test]
    fn formatting_or_panicking_with_a_value_never_shows_it() {
        let s = MemoryStore::default();
        save(&s, "EXAMPLE_TOKEN", VALUE).unwrap();
        let v = read(&s, "EXAMPLE_TOKEN").unwrap().unwrap();
        assert!(!format!("{v:?} {v:#?} {v} {:?}", Some(&v)).contains("SENTINEL"));
        // Crash output: a panic message built from the value.
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| panic!("step failed with {v} / {v:?}"))).unwrap_err();
        let msg = caught.downcast_ref::<String>().cloned().unwrap_or_default();
        assert!(msg.contains("hidden") && !msg.contains("SENTINEL"), "{msg}");
    }

    #[test]
    fn the_value_never_reaches_the_log() {
        let s = MemoryStore::default();
        let log = crate::settings::local_dir().join("glim.log");
        save_logged(&s, "EXAMPLE_TOKEN", VALUE).unwrap();
        let _ = save_logged(&s, "bad name", VALUE);
        let _ = save_logged(&s, "EXAMPLE_TOKEN", &format!("{VALUE}\n{VALUE}"));
        delete_logged(&s, "EXAMPLE_TOKEN").unwrap();
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(text.contains("secrets: saved EXAMPLE_TOKEN"), "logged at all");
        assert!(!text.contains("SENTINEL"), "{text}");
        // And no log line in this file's code mentions the value.
        let src = include_str!("secrets.rs");
        let code = &src[..src.find("/// An in-memory store").unwrap()];
        for line in code.lines().filter(|l| l.contains("log::line")) {
            for banned in ["raw", "value", "reveal"] {
                assert!(!line.contains(banned), "{banned} in: {line}");
            }
        }
    }

    /// Only a runbook's env_secret step reads a value out.
    #[test]
    fn the_value_is_revealed_only_by_a_runbook_step() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut uses = Vec::new();
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "rs") && !p.ends_with("secrets.rs") {
                    let text = std::fs::read_to_string(&p).unwrap().replace("\r\n", "\n");
                    let code = text.split("#[cfg(test)]\nmod tests").next().unwrap_or("");
                    for line in code.lines().filter(|l| l.contains(".reveal()") || l.contains("secrets::read(")) {
                        uses.push(format!("{}: {}", p.display(), line.trim()));
                    }
                }
            }
        }
        for u in &uses {
            assert!(u.contains("runbooks"), "{u}");
        }
    }

    #[test]
    fn filetime_converts_to_unix_ms() {
        // 2026-01-01T00:00:00Z = 1767225600000 ms.
        let ticks: u64 = 1_767_225_600_000 * 10_000 + 116_444_736_000_000_000;
        assert_eq!(filetime_ms((ticks >> 32) as u32, ticks as u32), 1_767_225_600_000);
        assert_eq!(filetime_ms(0, 0), 0);
    }

    /// A real round trip through Credential Manager (CI runs it), under a
    /// test-only prefix so the owner's secrets are never touched.
    #[cfg(windows)]
    #[test]
    fn real_credential_manager_round_trip() {
        let prefix: &'static str = Box::leak(format!("GlimTest-{}/", std::process::id()).into_boxed_str());
        let s = CredentialStore { prefix };
        s.delete("EXAMPLE_TOKEN").unwrap();
        assert!(read(&s, "EXAMPLE_TOKEN").unwrap().is_none());
        let info = save(&s, "EXAMPLE_TOKEN", VALUE).unwrap();
        assert_eq!(info.name, "EXAMPLE_TOKEN");
        // Saved just now (within a day, whatever the runner's clock).
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
        assert!(info.saved_ms.abs_diff(now) < 86_400_000, "{} vs {now}", info.saved_ms);
        assert_eq!(read(&s, "EXAMPLE_TOKEN").unwrap().unwrap().reveal(), VALUE);
        save(&s, "EXAMPLE_TOKEN", "placeholder-new").unwrap();
        assert_eq!(read(&s, "EXAMPLE_TOKEN").unwrap().unwrap().reveal(), "placeholder-new");
        assert_eq!(list(&s).unwrap().len(), 1);
        delete(&s, "EXAMPLE_TOKEN").unwrap();
        assert!(s.read("EXAMPLE_TOKEN").unwrap().is_none());
        assert!(list(&s).unwrap().is_empty());
    }
}
