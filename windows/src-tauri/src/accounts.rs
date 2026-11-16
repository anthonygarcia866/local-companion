// Claude accounts on this PC (owner 2026-11-15): Settings → Claude Code &
// Agents → Accounts. Each is a name, a Claude Code config folder
// (CLAUDE_CONFIG_DIR; "" = Claude Code's default, ~/.claude) and the remote
// courier's cloud client that acts for it (remote/, "account-1" …).
// Prefilled with Claude (the default folder, account-1) and Claude2
// (~/.claude2, account-2) until the owner changes the list.
//
// What Glim does with it:
// - Keep-alive (keepalive.rs) runs one remote-control server per account,
//   each with its own CLAUDE_CONFIG_DIR.
// - The courier (remote/exec.rs) starts a session under the account mapped
//   to the task's cloud client, and reaches only that account's sessions
//   (its config folder's transcripts); anything else is "not-your-session".
//
// "Signed in" means only that the folder holds Claude Code's login file
// (.credentials.json): its existence is checked, it is never opened or read.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const CONFIG_DIR_VAR: &str = "CLAUDE_CONFIG_DIR";
pub const MAX_ACCOUNTS: usize = 4;
const LOGIN_FILE: &str = ".credentials.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ClaudeAccount {
    /// "Claude", "Claude2" …
    pub name: String,
    /// The config folder ("" = Claude Code's default, ~/.claude).
    pub config_dir: String,
    /// The remote courier's cloud client acting for it ("account-1" …).
    pub client: String,
}

/// The prefilled list: Claude and Claude2.
pub fn defaults(home: &Path) -> Vec<ClaudeAccount> {
    vec![
        ClaudeAccount { name: "Claude".into(), config_dir: String::new(), client: "account-1".into() },
        ClaudeAccount { name: "Claude2".into(), config_dir: home.join(".claude2").to_string_lossy().into_owned(), client: "account-2".into() },
    ]
}

/// The accounts: the saved list, or the prefilled one while none is saved.
pub fn list(saved: &[ClaudeAccount], home: &Path) -> Vec<ClaudeAccount> {
    if saved.is_empty() { defaults(home) } else { saved.to_vec() }
}

/// The folder Claude Code uses for this account.
pub fn config_dir(a: &ClaudeAccount, home: &Path) -> PathBuf {
    if a.config_dir.is_empty() { home.join(".claude") } else { PathBuf::from(&a.config_dir) }
}

/// The login file is there (never opened).
pub fn signed_in(a: &ClaudeAccount, home: &Path) -> bool {
    config_dir(a, home).join(LOGIN_FILE).is_file()
}

/// The account a cloud client acts for.
pub fn for_client<'a>(list: &'a [ClaudeAccount], client: &str) -> Option<&'a ClaudeAccount> {
    list.iter().find(|a| a.client == client)
}

/// What a process started for this account gets: its CLAUDE_CONFIG_DIR, or
/// for the default folder the variable REMOVED (an empty value: see
/// platform::start_in_console), so nothing inherited picks another account.
pub fn env(a: &ClaudeAccount) -> (&'static str, String) {
    (CONFIG_DIR_VAR, a.config_dir.clone())
}

/// The name as typed: 1-24 letters, digits, spaces, `-_.`.
pub fn parse_name(raw: &str) -> Result<String, &'static str> {
    let n = raw.trim();
    let ok = (1..=24).contains(&n.chars().count()) && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'));
    if ok { Ok(n.to_string()) } else { Err("bad-name") }
}

/// A config folder as typed: an existing folder inside the user's profile
/// (not the profile itself), or "" for the default.
pub fn parse_folder(raw: &str, home: &Path) -> Result<String, &'static str> {
    let raw = raw.trim().trim_matches('"');
    if raw.is_empty() {
        return Ok(String::new());
    }
    let p = PathBuf::from(raw);
    if !p.is_absolute() || !p.is_dir() {
        return Err("no-folder");
    }
    let canon = std::fs::canonicalize(&p).map_err(|_| "no-folder")?;
    let home = std::fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
    if !canon.starts_with(&home) || canon == home {
        return Err("outside-profile");
    }
    Ok(p.to_string_lossy().trim_end_matches(['\\', '/']).to_string())
}

/// Adds an account; its cloud client is the first "account-N" no other uses.
pub fn add(list: &mut Vec<ClaudeAccount>, name: String, config_dir: String, home: &Path) -> Result<(), &'static str> {
    if list.len() >= MAX_ACCOUNTS {
        return Err("too-many");
    }
    if list.iter().any(|a| a.name.eq_ignore_ascii_case(&name)) {
        return Err("name-taken");
    }
    let dir = |d: &str| config_dir_key(d, home);
    if list.iter().any(|a| dir(&a.config_dir) == dir(&config_dir)) {
        return Err("folder-taken");
    }
    let client = (1..).map(|n| format!("account-{n}")).find(|c| !list.iter().any(|a| &a.client == c)).unwrap_or_default();
    list.push(ClaudeAccount { name, config_dir, client });
    Ok(())
}

fn config_dir_key(d: &str, home: &Path) -> String {
    let p = if d.is_empty() { home.join(".claude") } else { PathBuf::from(d) };
    p.to_string_lossy().trim_end_matches(['\\', '/']).to_ascii_lowercase()
}

/// One account for Settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    pub name: String,
    /// The folder as Claude Code uses it (the default spelled out).
    pub config_dir: String,
    pub signed_in: bool,
    pub client: String,
}

pub fn views(list: &[ClaudeAccount], home: &Path) -> Vec<AccountView> {
    list.iter()
        .map(|a| AccountView { name: a.name.clone(), config_dir: config_dir(a, home).to_string_lossy().into_owned(), signed_in: signed_in(a, home), client: a.client.clone() })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("glim-accounts-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(d.join(".claude")).unwrap();
        std::fs::create_dir_all(d.join(".claude2")).unwrap();
        d
    }

    #[test]
    fn prefilled_with_claude_and_claude2_mapped_to_account_1_and_2() {
        let h = home("prefilled");
        let l = list(&[], &h);
        assert_eq!(l.iter().map(|a| (a.name.as_str(), a.client.as_str())).collect::<Vec<_>>(), [("Claude", "account-1"), ("Claude2", "account-2")]);
        assert_eq!(config_dir(&l[0], &h), h.join(".claude"));
        assert_eq!(config_dir(&l[1], &h), h.join(".claude2"));
        assert_eq!(for_client(&l, "account-2").unwrap().name, "Claude2");
        assert!(for_client(&l, "account-3").is_none());
        // The default account's variable is removed, the other's set.
        assert_eq!(env(&l[0]), ("CLAUDE_CONFIG_DIR", String::new()));
        assert_eq!(env(&l[1]).1, h.join(".claude2").to_string_lossy());
        let _ = std::fs::remove_dir_all(&h);
    }

    #[test]
    fn signed_in_is_only_whether_the_login_file_exists() {
        let h = home("signed-in");
        let l = list(&[], &h);
        assert!(!signed_in(&l[1], &h));
        std::fs::write(h.join(".claude2").join(".credentials.json"), "not read").unwrap();
        assert!(signed_in(&l[1], &h));
        assert_eq!(views(&l, &h).iter().map(|v| v.signed_in).collect::<Vec<_>>(), [false, true]);
        let _ = std::fs::remove_dir_all(&h);
    }

    #[test]
    fn adding_checks_the_name_the_folder_and_picks_the_next_client() {
        let h = home("adding");
        let mut l = list(&[], &h);
        std::fs::create_dir_all(h.join(".claude3")).unwrap();
        let f = parse_folder(&h.join(".claude3").to_string_lossy(), &h).unwrap();
        add(&mut l, parse_name(" Claude3 ").unwrap(), f.clone(), &h).unwrap();
        assert_eq!(l[2].client, "account-3");
        assert_eq!(add(&mut l, "claude3".into(), String::new(), &h), Err("name-taken"));
        assert_eq!(add(&mut l, "Other".into(), f.to_ascii_uppercase(), &h), Err("folder-taken"));
        assert_eq!(add(&mut l, "Other".into(), h.join(".claude").to_string_lossy().into_owned(), &h), Err("folder-taken"), "the default folder spelled out");
        assert_eq!(parse_folder(&h.to_string_lossy(), &h), Err("outside-profile"));
        assert_eq!(parse_folder("relative\\path", &h), Err("no-folder"));
        assert_eq!(parse_folder(&h.join("missing").to_string_lossy(), &h), Err("no-folder"));
        assert_eq!(parse_folder("", &h), Ok(String::new()));
        assert!(parse_name("bad/name").is_err());
        // Removing one frees its client name for the next.
        l.retain(|a| a.name != "Claude2");
        std::fs::create_dir_all(h.join(".claude4")).unwrap();
        add(&mut l, "Claude4".into(), h.join(".claude4").to_string_lossy().into_owned(), &h).unwrap();
        assert_eq!(l.last().unwrap().client, "account-2");
        let _ = std::fs::remove_dir_all(&h);
    }
}
