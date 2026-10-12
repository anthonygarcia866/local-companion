// The Anthropic API key (1c, docs/phase1-plan.md §13.4).
//
// Stored in Windows Credential Manager as a generic credential, this user on
// this PC only (CRED_PERSIST_LOCAL_MACHINE, never roamed), through the
// `windows` crate: no new crate. Phase 0a removed credential storage with the
// `keyring` crate; this adds it back on purpose, for this one key.
//
// The key reaches Rust once, when Settings saves it. After that only Rust
// reads it, just before an API request (ai/claude.rs); Settings only ever
// gets `KeyStatus` (set or not, and a hint: "sk-ant-…" and the last four
// characters). The key is never logged, never in settings.json, never in an
// event: `ApiKey`'s Debug prints nothing of it, and a test checks the source.
//
// Tests never touch the real Credential Manager: they use `MemoryVault`. One
// #[ignore] test does a real write, read and delete for a manual run.

use serde::Serialize;

use crate::i18n::{t, tf};

/// The credential's name in Credential Manager ("Windows Credentials" →
/// "Generic Credentials" in Control Panel).
pub const TARGET: &str = "Glim/AnthropicApiKey";

/// What every Anthropic API key starts with.
pub const PREFIX: &str = "sk-ant-";

/// Credential Manager's limit for a generic credential's secret, in bytes.
pub const MAX_KEY_BYTES: usize = 2560;

/// The key, as only the request code may see it.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    /// The key itself: for the request's `x-api-key` header, nothing else.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiKey(hidden)")
    }
}

/// Where the key is kept: Credential Manager in the app, memory in tests.
pub trait Vault {
    fn read(&self) -> Result<Option<String>, String>;
    fn write(&self, secret: &str) -> Result<(), String>;
    fn delete(&self) -> Result<(), String>;
}

/// What Settings may know about the key.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyStatus {
    pub set: bool,
    /// "sk-ant-…abcd", or "" when there is no key.
    pub hint: String,
}

/// The masked form Settings shows: the prefix and the last four characters.
pub fn hint(key: &str) -> String {
    let tail: String = key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("{PREFIX}…{tail}")
}

/// A pasted key, cleaned up: surrounding spaces go; it must start with
/// `sk-ant-` and hold no spaces or control characters.
pub fn normalise(raw: &str) -> Result<String, String> {
    let key = raw.trim();
    if key.is_empty() {
        return Err(t("Paste the API key first."));
    }
    if !key.starts_with(PREFIX) || key.len() <= PREFIX.len() + 4 || key.len() > MAX_KEY_BYTES || key.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(t("That doesn't look like an Anthropic API key (they start with sk-ant-)."));
    }
    Ok(key.to_string())
}

pub fn status(vault: &dyn Vault) -> Result<KeyStatus, String> {
    Ok(match vault.read()? {
        Some(key) if !key.is_empty() => KeyStatus { set: true, hint: hint(&key) },
        _ => KeyStatus::default(),
    })
}

/// Saves a pasted key (Settings → AI → Save) and returns what Settings may show.
pub fn save(vault: &dyn Vault, raw: &str) -> Result<KeyStatus, String> {
    let key = normalise(raw)?;
    vault.write(&key)?;
    status(vault)
}

/// Settings → AI → Remove. Removing a key that isn't there is fine.
pub fn remove(vault: &dyn Vault) -> Result<KeyStatus, String> {
    vault.delete()?;
    status(vault)
}

/// The key for a request, if one is saved.
pub fn load(vault: &dyn Vault) -> Result<Option<ApiKey>, String> {
    Ok(vault.read()?.filter(|k| !k.is_empty()).map(ApiKey))
}

/// The vault the app uses.
pub fn vault() -> impl Vault {
    CredentialManager { target: TARGET }
}

/// Windows Credential Manager, one credential (`target`).
pub struct CredentialManager {
    pub target: &'static str,
}

#[cfg(windows)]
mod windows_vault {
    use super::*;
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::ERROR_NOT_FOUND;
    use windows::Win32::Security::Credentials::{
        CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn not_found(e: &windows::core::Error) -> bool {
        e.code() == ERROR_NOT_FOUND.to_hresult()
    }

    impl Vault for CredentialManager {
        fn read(&self) -> Result<Option<String>, String> {
            let target = wide(self.target);
            let mut cred: *mut CREDENTIALW = std::ptr::null_mut();
            match unsafe { CredReadW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None, &mut cred) } {
                Err(e) if not_found(&e) => Ok(None),
                Err(e) => Err(tf("Windows couldn't read the API key: {error}", &[("error", &e.message())])),
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
                    secret.map(Some).map_err(|_| t("The saved API key is damaged. Remove it and save it again."))
                }
            }
        }

        fn write(&self, secret: &str) -> Result<(), String> {
            let mut target = wide(self.target);
            let mut user = wide("Glim");
            let mut blob = secret.as_bytes().to_vec();
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
            // Our copy of the secret doesn't outlive the call.
            blob.iter_mut().for_each(|b| *b = 0);
            result.map_err(|e| tf("Windows couldn't save the API key: {error}", &[("error", &e.message())]))
        }

        fn delete(&self) -> Result<(), String> {
            let target = wide(self.target);
            match unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) } {
                Ok(()) => Ok(()),
                Err(e) if not_found(&e) => Ok(()),
                Err(e) => Err(tf("Windows couldn't remove the API key: {error}", &[("error", &e.message())])),
            }
        }
    }
}

#[cfg(not(windows))]
impl Vault for CredentialManager {
    fn read(&self) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn write(&self, _secret: &str) -> Result<(), String> {
        Err(t("Saving an API key is only available on Windows for now."))
    }
    fn delete(&self) -> Result<(), String> {
        Ok(())
    }
}

/// An in-memory vault, for tests.
#[cfg(test)]
#[derive(Default)]
pub struct MemoryVault(pub std::cell::RefCell<Option<String>>);

#[cfg(test)]
impl Vault for MemoryVault {
    fn read(&self) -> Result<Option<String>, String> {
        Ok(self.0.borrow().clone())
    }
    fn write(&self, secret: &str) -> Result<(), String> {
        *self.0.borrow_mut() = Some(secret.to_string());
        Ok(())
    }
    fn delete(&self) -> Result<(), String> {
        *self.0.borrow_mut() = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "sk-ant-api03-SENTINELabcdefghijklmnopqrstuvwxyz-WXYZ";

    #[test]
    fn a_pasted_key_is_trimmed_and_must_look_like_one() {
        assert_eq!(normalise(&format!("  {KEY}\n")).unwrap(), KEY);
        for bad in ["", "   ", "sk-ant-", "sk-ant-ab", "sk-proj-abcdefghijkl", "sk-ant-abc def ghij", "hello", &"sk-ant-x".repeat(400)] {
            assert!(normalise(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn settings_only_ever_sees_a_hint() {
        let v = MemoryVault::default();
        assert_eq!(status(&v).unwrap(), KeyStatus { set: false, hint: String::new() });
        let s = save(&v, KEY).unwrap();
        assert_eq!(s, KeyStatus { set: true, hint: "sk-ant-…WXYZ".into() });
        let shown = serde_json::to_string(&s).unwrap();
        assert!(!shown.contains("SENTINEL"), "{shown}");
        assert_eq!(load(&v).unwrap().unwrap().expose(), KEY);
        assert_eq!(remove(&v).unwrap(), KeyStatus::default());
        assert!(load(&v).unwrap().is_none());
        // Removing twice is fine.
        assert_eq!(remove(&v).unwrap(), KeyStatus::default());
    }

    #[test]
    fn a_bad_paste_keeps_the_saved_key() {
        let v = MemoryVault::default();
        save(&v, KEY).unwrap();
        assert!(save(&v, "not a key").is_err());
        assert_eq!(load(&v).unwrap().unwrap().expose(), KEY);
    }

    #[test]
    fn formatting_the_key_never_shows_it() {
        let k = load(&{
            let v = MemoryVault::default();
            v.write(KEY).unwrap();
            v
        })
        .unwrap()
        .unwrap();
        assert!(!format!("{k:?} {k:#?} {:?}", Some(&k)).contains("SENTINEL"));
    }

    /// Nothing in the AI code logs the key or passes it anywhere but the
    /// request header: `expose()` is called in one place.
    #[test]
    fn the_key_is_exposed_only_for_the_request_header() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut uses = Vec::new();
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "rs") && !p.ends_with("ai/key.rs") && !p.ends_with("ai\\key.rs") {
                    let text = std::fs::read_to_string(&p).unwrap();
                    for line in text.lines().filter(|l| l.contains(".expose()")) {
                        uses.push(format!("{}: {}", p.display(), line.trim()));
                    }
                }
            }
        }
        assert!(uses.len() <= 1, "{uses:#?}");
        for u in &uses {
            assert!(u.contains("x-api-key"), "{u}");
        }
    }

    /// A real round trip through Credential Manager, under a test name so the
    /// owner's key is never touched. Manual only:
    /// cargo test -p glim real_credential_manager -- --ignored
    #[cfg(windows)]
    #[test]
    #[ignore = "writes to the real Credential Manager"]
    fn real_credential_manager_round_trip() {
        let v = CredentialManager { target: "Glim/TestOnly-DeleteMe" };
        v.delete().unwrap();
        assert!(v.read().unwrap().is_none());
        assert_eq!(save(&v, KEY).unwrap().hint, "sk-ant-…WXYZ");
        assert_eq!(load(&v).unwrap().unwrap().expose(), KEY);
        assert!(!remove(&v).unwrap().set);
        assert!(v.read().unwrap().is_none());
    }
}
