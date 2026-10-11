// Which apps the writing checker works in. Hardcoded for 1a; Phase 1d's
// settings make it editable and ask once about apps on neither list.

/// On by default: email, documents, browsers (Gmail, web forms), Notepad and
/// chat apps.
pub const ON: [&str; 15] = [
    "OUTLOOK.EXE", "olk.exe", "WINWORD.EXE", "chrome.exe", "msedge.exe", "Notepad.exe", "slack.exe",
    "ms-teams.exe", "Teams.exe", "Discord.exe", "WhatsApp.exe", "WhatsApp.Root.exe", "claude.exe",
    "ChatGPT.exe", "Grok Bot.exe",
];

/// Off by default, named so the choice is on record: editors and terminals
/// (code and commands, and terminals expose their whole scrollback). Password
/// managers are never read at all (capture::sensitive::PauseList). Read by
/// the tests now and by 1d's settings later.
#[allow(dead_code)]
pub const OFF: [&str; 9] = [
    "Code.exe", "Cursor.exe", "devenv.exe", "WindowsTerminal.exe", "cmd.exe", "powershell.exe", "pwsh.exe",
    "conhost.exe", "OpenConsole.exe",
];

/// Chat apps (a subset of ON): relaxed rules, see checker::Rules::Chat.
pub const CHAT: [&str; 9] = [
    "slack.exe", "ms-teams.exe", "Teams.exe", "Discord.exe", "WhatsApp.exe", "WhatsApp.Root.exe", "claude.exe",
    "ChatGPT.exe", "Grok Bot.exe",
];

/// The rules for this app: relaxed in chat apps, full everywhere else.
pub fn rules(exe: &str) -> super::Rules {
    if CHAT.iter().any(|a| a.eq_ignore_ascii_case(exe)) {
        super::Rules::Chat
    } else {
        super::Rules::Full
    }
}

/// Whether the checker works in this app. Apps on neither list are off.
pub fn enabled(exe: &str) -> bool {
    ON.iter().any(|a| a.eq_ignore_ascii_case(exe))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writing::Rules;

    #[test]
    fn chat_apps_get_relaxed_rules_and_the_rest_full() {
        for exe in CHAT {
            assert!(enabled(exe), "{exe} is on");
            assert_eq!(rules(exe), Rules::Chat, "{exe}");
        }
        assert_eq!(rules("SLACK.exe"), Rules::Chat);
        for exe in ["OUTLOOK.EXE", "olk.exe", "WINWORD.EXE", "chrome.exe", "msedge.exe", "Notepad.exe"] {
            assert_eq!(rules(exe), Rules::Full, "{exe}");
        }
    }

    #[test]
    fn email_documents_browsers_notepad_and_chat_are_on() {
        for exe in ["OUTLOOK.EXE", "outlook.exe", "olk.exe", "WINWORD.EXE", "chrome.exe", "msedge.exe", "Notepad.exe", "slack.exe", "ms-teams.exe", "claude.exe"] {
            assert!(enabled(exe), "{exe}");
        }
    }

    #[test]
    fn editors_terminals_password_managers_and_unknown_apps_are_off() {
        for exe in OFF.iter().copied().chain(["1Password.exe", "KeePass.exe", "glim.exe", "explorer.exe", "SomeNewApp.exe"]) {
            assert!(!enabled(exe), "{exe}");
        }
        assert!(OFF.iter().all(|o| !ON.iter().any(|a| a.eq_ignore_ascii_case(o))));
    }
}
