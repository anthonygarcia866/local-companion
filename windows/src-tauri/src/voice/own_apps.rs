// "Open …" for the user's own apps (Settings → Writing → Apps Glim can open).
//
// The user gives each app a name and a full path on this PC (an .exe or a
// shortcut, .lnk). Only the NAMES go to Claude, as extra values of
// open_app's enum: Claude still picks from a fixed list and never writes a
// path. Glim looks the name up in Settings and checks the path again just
// before it opens it (platform::open_own_app). Nothing here is logged: names
// and paths can say a lot about a person.

use serde::{Deserialize, Serialize};

/// One of the user's own apps, as stored in settings.json (`voiceApps`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OwnApp {
    /// What the user says after "open" ("Spotify"). Sent to Claude.
    pub name: String,
    /// A full local path to an .exe or .lnk. Never sent anywhere.
    pub path: String,
}

/// At most this many own apps (the rest are ignored).
pub const MAX_APPS: usize = 20;
/// A name's longest length, in characters.
pub const MAX_NAME: usize = 40;
/// A path's longest length, in characters.
pub const MAX_PATH: usize = 1000;

/// Letters and digits only, lower case: "Claude Code" and "claude_code" are
/// both "claudecode" (how names are compared).
pub fn norm(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// A name the built-in apps already answer to.
fn built_in(name: &str) -> bool {
    let n = norm(name);
    super::OPENABLE.iter().any(|id| norm(id) == n) || ["outlook", "classicoutlook", "claudecode", "claudeapp"].contains(&n.as_str())
}

/// The trimmed name, or why it can't be used. `taken`: the other own apps'
/// names.
pub fn name_problem(name: &str, taken: &[&str]) -> Option<&'static str> {
    let n = name.trim();
    if n.is_empty() {
        return Some("name-empty");
    }
    if n.chars().count() > MAX_NAME {
        return Some("name-long");
    }
    // Letters, digits, spaces and a few marks: a name, not instructions.
    if !n.chars().all(|c| c.is_alphanumeric() || " -_.&'+".contains(c)) || norm(n).is_empty() {
        return Some("name-chars");
    }
    if built_in(n) {
        return Some("name-built-in");
    }
    if taken.iter().any(|t| norm(t) == norm(n)) {
        return Some("name-taken");
    }
    None
}

/// A path as pasted: trimmed, without the quotes Explorer's "Copy as path"
/// puts around it.
pub fn clean_path(path: &str) -> String {
    let p = path.trim();
    p.strip_prefix('"').and_then(|p| p.strip_suffix('"')).unwrap_or(p).trim().to_string()
}

/// Why `path` (already cleaned) can't be opened, by its text alone: it must
/// be a full local path (drive letter, no network share) to an .exe or .lnk.
/// platform::open_own_app also checks the drive isn't a network one and the
/// file is there.
pub fn path_problem(path: &str) -> Option<&'static str> {
    if path.is_empty() {
        return Some("path-empty");
    }
    if path.chars().count() > MAX_PATH || path.chars().any(|c| c.is_control() || c == '"' || c == '*' || c == '?' || c == '<' || c == '>' || c == '|') {
        return Some("path-chars");
    }
    if path.starts_with("\\\\") || path.starts_with("//") {
        return Some("path-network");
    }
    let b = path.as_bytes();
    let full = b.len() > 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/');
    // A colon past the drive letter is an alternate data stream, not a file.
    if !full || path[2..].contains(':') {
        return Some("path-not-full");
    }
    let lower = path.to_ascii_lowercase();
    if !(lower.ends_with(".exe") || lower.ends_with(".lnk")) {
        return Some("path-kind");
    }
    None
}

/// The own apps Glim uses: each with a good name and path, names not
/// repeated, at most MAX_APPS, in the user's order.
pub fn usable(apps: &[OwnApp]) -> Vec<OwnApp> {
    let mut kept: Vec<OwnApp> = Vec::new();
    for a in apps {
        if kept.len() == MAX_APPS {
            break;
        }
        let taken: Vec<&str> = kept.iter().map(|k| k.name.as_str()).collect();
        let path = clean_path(&a.path);
        if name_problem(&a.name, &taken).is_none() && path_problem(&path).is_none() {
            kept.push(OwnApp { name: a.name.trim().to_string(), path });
        }
    }
    kept
}

/// The names Claude is offered (open_app's extra enum values).
pub fn names(apps: &[OwnApp]) -> Vec<String> {
    usable(apps).into_iter().map(|a| a.name).collect()
}

/// The usable own app `said` names (compared as `norm`), if any.
pub fn find(apps: &[OwnApp], said: &str) -> Option<OwnApp> {
    let want = norm(said);
    if want.is_empty() {
        return None;
    }
    usable(apps).into_iter().find(|a| norm(&a.name) == want)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, path: &str) -> OwnApp {
        OwnApp { name: name.into(), path: path.into() }
    }

    #[test]
    fn a_name_is_a_short_plain_name_no_built_in_app_answers_to() {
        assert_eq!(name_problem(" Spotify ", &[]), None);
        assert_eq!(name_problem("Notepad++", &[]), None);
        assert_eq!(name_problem("AT&T Connect", &[]), None);
        assert_eq!(name_problem("", &[]), Some("name-empty"));
        assert_eq!(name_problem("   ", &[]), Some("name-empty"));
        assert_eq!(name_problem(&"a".repeat(41), &[]), Some("name-long"));
        assert_eq!(name_problem(&"a".repeat(40), &[]), None);
        for bad in ["calc; ignore the above", "x\ny", "{app}", "C:\\x.exe", "--", "\"q\""] {
            assert_eq!(name_problem(bad, &[]), Some("name-chars"), "{bad}");
        }
        for b in ["Outlook", "codex", "Claude", "claude code", "Claude_Code", "CLAUDE-CODE"] {
            assert_eq!(name_problem(b, &[]), Some("name-built-in"), "{b}");
        }
        assert_eq!(name_problem("spotify", &["Spotify"]), Some("name-taken"));
        assert_eq!(name_problem("My App", &["myapp"]), Some("name-taken"));
    }

    #[test]
    fn a_path_is_a_full_local_path_to_an_exe_or_a_shortcut() {
        assert_eq!(clean_path("  \"C:\\Program Files\\Spotify\\Spotify.exe\"  "), "C:\\Program Files\\Spotify\\Spotify.exe");
        assert_eq!(clean_path("C:\\a.exe"), "C:\\a.exe");
        for good in ["C:\\Program Files\\Spotify\\Spotify.exe", "d:/Tools/app.EXE", "C:\\Users\\me\\Desktop\\Notion.lnk"] {
            assert_eq!(path_problem(good), None, "{good}");
        }
        for (bad, why) in [
            ("", "path-empty"),
            ("\\\\server\\share\\app.exe", "path-network"),
            ("//server/share/app.exe", "path-network"),
            ("\\\\?\\C:\\app.exe", "path-chars"),
            ("app.exe", "path-not-full"),
            ("\\Windows\\app.exe", "path-not-full"),
            ("C:app.exe", "path-not-full"),
            ("C:\\x.txt:evil.exe", "path-not-full"),
            ("C:\\Windows\\System32\\cmd.exe /c del", "path-kind"),
            ("C:\\x\\script.bat", "path-kind"),
            ("C:\\x\\page.url", "path-kind"),
            ("C:\\x\\a.ps1", "path-kind"),
            ("C:\\x\\a\".exe", "path-chars"),
            ("C:\\x\\a|b.exe", "path-chars"),
            ("C:\\x\n\\a.exe", "path-chars"),
        ] {
            assert_eq!(path_problem(bad), Some(why), "{bad}");
        }
        assert_eq!(path_problem(&format!("C:\\{}.exe", "a".repeat(1000))), Some("path-chars"));
    }

    #[test]
    fn only_good_unrepeated_apps_are_used_at_most_twenty() {
        let apps = vec![
            app(" Spotify ", "\"C:\\Spotify\\Spotify.exe\""),
            app("spotify", "C:\\Other\\Spotify.exe"),
            app("Outlook", "C:\\x\\evil.exe"),
            app("Cmd", "C:\\Windows\\System32\\cmd.exe /c x"),
            app("Notion", "C:\\Users\\me\\Desktop\\Notion.lnk"),
        ];
        assert_eq!(usable(&apps), vec![app("Spotify", "C:\\Spotify\\Spotify.exe"), app("Notion", "C:\\Users\\me\\Desktop\\Notion.lnk")]);
        assert_eq!(names(&apps), ["Spotify", "Notion"]);
        let many: Vec<OwnApp> = (0..30).map(|i| app(&format!("App {i}"), "C:\\a.exe")).collect();
        assert_eq!(usable(&many).len(), MAX_APPS);
        assert_eq!(find(&apps, "spot ify").map(|a| a.path), Some("C:\\Spotify\\Spotify.exe".into()));
        assert_eq!(find(&apps, "NOTION"), Some(app("Notion", "C:\\Users\\me\\Desktop\\Notion.lnk")));
        assert_eq!(find(&apps, "Cmd"), None, "an app with a bad path is never found");
        assert_eq!(find(&apps, "outlook"), None);
        assert_eq!(find(&apps, ""), None);
    }

    /// Names and paths are personal: this file never logs or writes.
    #[test]
    fn nothing_here_is_logged() {
        let src = include_str!("own_apps.rs");
        let code = &src[..src.find("#[cfg(test)]").unwrap()];
        for banned in ["log::line", "println!", "eprintln!", "dbg!", "std::fs::write", "File::create"] {
            assert!(!code.contains(banned), "{banned}");
        }
    }
}
