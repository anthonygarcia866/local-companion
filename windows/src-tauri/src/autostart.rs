// Launch at startup (Settings → General): Glim's entry in
// HKCU\Software\Microsoft\Windows\CurrentVersion\Run.
//
// Replaces tauri-plugin-autostart, which only acted when the box was ticked
// or unticked, pointed the entry at whichever glim.exe was running (a copy in
// target\release included), wrote the path unquoted, and turned Task Manager's
// "Startup apps" switch back on every time it wrote. Here the entry is brought
// in line with the setting at every launch and on every change, it always
// points at the installed copy when there is one, and every outcome or
// failure goes to glim.log. Task Manager's switch is the user's: it is only
// turned back on when they tick the box in Glim's Settings.

use std::path::{Path, PathBuf};

/// The entry's name under Run (and StartupApproved\Run): the same one the
/// plugin used, so an entry it wrote is found and replaced.
pub const ENTRY: &str = "Glim";

/// What the Run entry needs.
#[derive(Debug, PartialEq)]
pub enum Action {
    /// Write (or rewrite) the entry with this command line.
    Write(String),
    Delete,
    Nothing,
}

/// The Run entry's command line for `exe`: quoted, so a path with spaces
/// starts the right program.
pub fn command_line(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

/// What to do so the entry matches the setting. `existing` is the entry's
/// current command line, if it has one.
pub fn plan(wanted: bool, exe: &Path, existing: Option<&str>) -> Action {
    match (wanted, existing) {
        (true, Some(cur)) if cur == command_line(exe) => Action::Nothing,
        (true, _) => Action::Write(command_line(exe)),
        (false, Some(_)) => Action::Delete,
        (false, None) => Action::Nothing,
    }
}

/// The installed glim.exe from the installer's uninstall key values
/// (`InstallLocation` is written quoted). None when either is missing.
pub fn installed_exe(install_location: Option<&str>, main_binary: Option<&str>) -> Option<PathBuf> {
    let dir = install_location?.trim().trim_matches('"');
    let exe = main_binary?.trim();
    (!dir.is_empty() && !exe.is_empty()).then(|| Path::new(dir).join(exe))
}

/// The program the entry starts: the installed copy when it exists, so a
/// build-folder or dev copy never becomes what Windows starts; otherwise
/// the running exe (a portable copy, never installed).
pub fn target(installed: Option<PathBuf>, current: PathBuf) -> PathBuf {
    installed.filter(|p| p.is_file()).unwrap_or(current)
}

/// Task Manager's "Startup apps" switch for the entry, from its
/// StartupApproved\Run value: Windows writes 02 (or 06) in the first byte for
/// enabled and 03 (or 07) for disabled. None when there is no value (enabled).
pub fn approved(value: &[u8]) -> Option<bool> {
    value.first().map(|b| b & 1 == 0)
}

/// Brings the entry in line with `wanted`. `ticked` is true when the user
/// just changed the setting in Glim (not at launch): only then is a "disabled"
/// left in Task Manager turned back on. Writes one line to glim.log.
#[cfg(windows)]
pub fn sync(wanted: bool, ticked: bool) {
    crate::log::line(format!("autostart: {}", imp::sync(wanted, ticked)));
}

#[cfg(not(windows))]
pub fn sync(_wanted: bool, _ticked: bool) {}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
    use windows::Win32::System::Registry::{
        RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_BINARY, REG_ROUTINE_FLAGS,
        REG_SZ, RRF_RT_REG_BINARY, RRF_RT_REG_SZ,
    };

    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
    const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Glim";
    /// What Windows writes in StartupApproved\Run for "enabled".
    const ENABLED: [u8; 12] = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

    pub fn sync(wanted: bool, ticked: bool) -> String {
        let current = match std::env::current_exe() {
            Ok(p) => p,
            Err(err) => return format!("FAILED: can't tell which glim.exe is running: {err}"),
        };
        let installed = super::installed_exe(
            read_string(UNINSTALL, "InstallLocation").ok().flatten().as_deref(),
            read_string(UNINSTALL, "MainBinaryName").ok().flatten().as_deref(),
        );
        let exe = super::target(installed, current);
        let existing = match read_string(RUN, ENTRY) {
            Ok(v) => v,
            Err(err) => return format!("FAILED to read the startup entry: {}", describe(err)),
        };
        let mut note = match super::plan(wanted, &exe, existing.as_deref()) {
            Action::Nothing if wanted => format!("on; the startup entry already starts {}", exe.display()),
            Action::Nothing => "off; no startup entry".to_string(),
            Action::Write(cmd) => match write_string(RUN, ENTRY, &cmd) {
                Ok(()) if existing.is_some() => format!("on; startup entry updated to start {}", exe.display()),
                Ok(()) => format!("on; startup entry added, starts {}", exe.display()),
                Err(err) => return format!("FAILED to write the startup entry: {}", describe(err)),
            },
            Action::Delete => match delete(RUN, ENTRY) {
                Ok(()) => "off; startup entry removed".to_string(),
                Err(err) => return format!("FAILED to remove the startup entry: {}", describe(err)),
            },
        };
        if wanted {
            let disabled = read_binary(APPROVED, ENTRY).ok().flatten().is_some_and(|v| super::approved(&v) == Some(false));
            if disabled && ticked {
                note += &match write_binary(APPROVED, ENTRY, &ENABLED) {
                    Ok(()) => "; turned back on in Task Manager's Startup apps".to_string(),
                    Err(err) => format!("; FAILED to turn it back on in Task Manager's Startup apps: {}", describe(err)),
                };
            } else if disabled {
                note += "; but it is turned off in Task Manager's Startup apps, so Windows won't start it (left as is)";
            }
        }
        note
    }

    fn describe(err: WIN32_ERROR) -> String {
        format!("{} (error {})", windows::core::Error::from(err.to_hresult()).message(), err.0)
    }

    fn get(key: &str, name: &str, flags: REG_ROUTINE_FLAGS) -> Result<Option<Vec<u8>>, WIN32_ERROR> {
        let (key, name) = (HSTRING::from(key), HSTRING::from(name));
        let mut len = 0u32;
        let rc = unsafe { RegGetValueW(HKEY_CURRENT_USER, &key, &name, flags, None, None, Some(&mut len)) };
        if rc == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if rc != ERROR_SUCCESS {
            return Err(rc);
        }
        let mut buf = vec![0u8; len as usize];
        let rc = unsafe {
            RegGetValueW(HKEY_CURRENT_USER, &key, &name, flags, None, Some(buf.as_mut_ptr().cast()), Some(&mut len))
        };
        match rc {
            ERROR_SUCCESS => {
                buf.truncate(len as usize);
                Ok(Some(buf))
            }
            ERROR_FILE_NOT_FOUND => Ok(None),
            rc => Err(rc),
        }
    }

    pub fn read_string(key: &str, name: &str) -> Result<Option<String>, WIN32_ERROR> {
        Ok(get(key, name, RRF_RT_REG_SZ)?.map(|bytes| {
            let wide: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).collect();
            let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
            String::from_utf16_lossy(&wide[..end])
        }))
    }

    fn read_binary(key: &str, name: &str) -> Result<Option<Vec<u8>>, WIN32_ERROR> {
        get(key, name, RRF_RT_REG_BINARY)
    }

    fn set(key: &str, name: &str, kind: u32, data: &[u8]) -> Result<(), WIN32_ERROR> {
        let rc = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(key),
                &HSTRING::from(name),
                kind,
                Some(data.as_ptr().cast()),
                data.len() as u32,
            )
        };
        if rc == ERROR_SUCCESS { Ok(()) } else { Err(rc) }
    }

    fn write_string(key: &str, name: &str, value: &str) -> Result<(), WIN32_ERROR> {
        let bytes: Vec<u8> = value.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect();
        set(key, name, REG_SZ.0, &bytes)
    }

    fn write_binary(key: &str, name: &str, value: &[u8]) -> Result<(), WIN32_ERROR> {
        set(key, name, REG_BINARY.0, value)
    }

    fn delete(key: &str, name: &str) -> Result<(), WIN32_ERROR> {
        let rc = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(key), &HSTRING::from(name)) };
        if rc == ERROR_SUCCESS || rc == ERROR_FILE_NOT_FOUND { Ok(()) } else { Err(rc) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exe() -> PathBuf {
        PathBuf::from(r"C:\Users\Some One\AppData\Local\Glim\glim.exe")
    }

    #[test]
    fn the_command_line_is_quoted_so_paths_with_spaces_work() {
        assert_eq!(command_line(&exe()), r#""C:\Users\Some One\AppData\Local\Glim\glim.exe""#);
    }

    #[test]
    fn on_writes_the_entry_when_it_is_missing_or_points_elsewhere() {
        let want = Action::Write(command_line(&exe()));
        assert_eq!(plan(true, &exe(), None), want);
        // The old plugin's form: unquoted, trailing space, the build folder.
        assert_eq!(plan(true, &exe(), Some(r"C:\src\windows\target\release\glim.exe ")), want);
        assert_eq!(plan(true, &exe(), Some(r"C:\Users\Some One\AppData\Local\Glim\glim.exe ")), want);
    }

    #[test]
    fn on_with_the_right_entry_does_nothing() {
        assert_eq!(plan(true, &exe(), Some(&command_line(&exe()))), Action::Nothing);
    }

    #[test]
    fn off_removes_any_entry_and_otherwise_does_nothing() {
        assert_eq!(plan(false, &exe(), Some("anything")), Action::Delete);
        assert_eq!(plan(false, &exe(), None), Action::Nothing);
    }

    #[test]
    fn the_installed_exe_comes_from_the_uninstall_key() {
        assert_eq!(
            installed_exe(Some(r#""C:\Users\a\AppData\Local\Glim""#), Some("glim.exe")),
            Some(PathBuf::from(r"C:\Users\a\AppData\Local\Glim\glim.exe"))
        );
        assert_eq!(installed_exe(Some(r"C:\Glim"), Some("glim.exe")), Some(PathBuf::from(r"C:\Glim\glim.exe")));
        assert_eq!(installed_exe(None, Some("glim.exe")), None);
        assert_eq!(installed_exe(Some(r#""""#), Some("glim.exe")), None);
        assert_eq!(installed_exe(Some(r"C:\Glim"), None), None);
    }

    #[test]
    fn the_installed_copy_wins_over_the_running_one_when_it_exists() {
        let dir = std::env::temp_dir().join(format!("glim-autostart-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let installed = dir.join("glim.exe");
        std::fs::write(&installed, b"").unwrap();
        let running = PathBuf::from(r"C:\src\windows\target\release\glim.exe");
        assert_eq!(target(Some(installed.clone()), running.clone()), installed);
        // Uninstalled since (the key is left behind, the file is gone), or never installed.
        assert_eq!(target(Some(dir.join("missing.exe")), running.clone()), running);
        assert_eq!(target(None, running.clone()), running);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn task_managers_switch_is_read_from_the_first_byte() {
        assert_eq!(approved(&[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), Some(true));
        assert_eq!(approved(&[6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), Some(true));
        assert_eq!(approved(&[3, 0, 0, 0, 0x9a, 0x1e, 0, 0, 0, 0, 0, 0]), Some(false));
        assert_eq!(approved(&[7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), Some(false));
        assert_eq!(approved(&[]), None);
    }

    /// Launch at startup is applied when Glim starts, not only when the
    /// setting changes; and nothing about it goes to stderr only.
    #[test]
    fn the_setting_is_applied_at_launch_and_failures_are_logged() {
        let lib = include_str!("lib.rs");
        let code = lib.split("#[cfg(test)]").next().unwrap();
        let setup = &code[code.find(".setup(move |app|").unwrap()..];
        assert!(setup.contains("autostart::sync("), "setup must sync the startup entry");
        let save = &code[code.find("fn save_settings").unwrap()..];
        let save = &save[..save.find("\n}\n").unwrap()];
        assert!(save.contains("autostart::sync("), "save_settings must sync on change");
        assert!(!code.contains("tauri_plugin_autostart"), "the plugin is replaced by this module");
        let this = include_str!("autostart.rs");
        let this = this.split("#[cfg(test)]").next().unwrap();
        assert!(!this.contains("eprintln!"), "failures go to glim.log");
    }
}
