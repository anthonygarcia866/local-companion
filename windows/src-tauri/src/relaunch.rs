// Glim starts itself again when WebView2's browser process exits.
//
// That process draws every Glim window; when it goes, each window is blank and
// a reload can't bring it back (webview_health.rs). So Glim starts a fresh
// glim.exe, which waits for this one to be gone (the single-instance plugin
// would otherwise hand it back to us), then exits.
//
// At most once per 10 minutes, tracked in %LOCALAPPDATA%\Glim\relaunch.txt
// ("<unix seconds of the last relaunch> <relaunches so far>"): a second stop
// within 10 minutes means relaunching isn't helping, so Glim shows one Windows
// notification instead and stays as it is. Log lines carry counts and the
// reason only.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::AppHandle;

/// Two relaunches at least this far apart.
pub const MIN_GAP_SECS: u64 = 10 * 60;
/// `glim.exe --relaunched-after <pid>`: wait for that Glim to exit first.
pub const AFTER_FLAG: &str = "--relaunched-after";
pub const NOTICE: &str = "Glim's window engine stopped; restart Glim from the Start menu";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Relaunch,
    Notify,
}

/// What relaunch.txt holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Record {
    pub last: Option<u64>,
    pub count: u32,
}

/// Relaunch unless the last relaunch was less than 10 minutes ago. A clock
/// set back (last in the future) doesn't block a relaunch.
pub fn decide(record: Record, now: u64) -> Decision {
    match record.last {
        Some(last) if now >= last && now - last < MIN_GAP_SECS => Decision::Notify,
        _ => Decision::Relaunch,
    }
}

fn parse(text: &str) -> Record {
    let mut parts = text.split_whitespace();
    let last = parts.next().and_then(|s| s.parse().ok());
    let count = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    Record { last, count }
}

fn format(record: Record) -> String {
    format!("{} {}\n", record.last.unwrap_or(0), record.count)
}

fn file_path() -> PathBuf {
    crate::settings::local_dir().join("relaunch.txt")
}

fn read(path: &Path) -> Record {
    std::fs::read_to_string(path).map(|t| parse(&t)).unwrap_or_default()
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// One answer per Glim: every window reports the same browser exit.
static HANDLED: AtomicBool = AtomicBool::new(false);

/// WebView2's browser process exited: relaunch, or notify (see the top).
pub fn on_browser_exited(app: &AppHandle) {
    if HANDLED.swap(true, Ordering::SeqCst) {
        return;
    }
    let path = file_path();
    let record = read(&path);
    match decide(record, now_secs()) {
        Decision::Relaunch => {
            let next = Record { last: Some(now_secs()), count: record.count.saturating_add(1) };
            let _ = std::fs::write(&path, format(next));
            match spawn_fresh() {
                Ok(()) => {
                    crate::log::line(format!("relaunch: window engine stopped; starting Glim again (relaunch {})", next.count));
                    app.exit(0);
                }
                Err(err) => {
                    crate::log::line(format!("relaunch: window engine stopped; could not start Glim again ({})", err.kind()));
                    notify(app);
                }
            }
        }
        Decision::Notify => {
            crate::log::line(format!(
                "relaunch: window engine stopped again within 10 min; not relaunching, notified ({} relaunches so far)",
                record.count
            ));
            notify(app);
        }
    }
}

fn spawn_fresh() -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    std::process::Command::new(exe).arg(AFTER_FLAG).arg(std::process::id().to_string()).spawn().map(|_| ())
}

/// The pid after `--relaunched-after`, if Glim was started that way.
fn previous_pid(argv: &[String]) -> Option<u32> {
    let at = argv.iter().position(|a| a == AFTER_FLAG)?;
    argv.get(at + 1)?.parse().ok()
}

/// First thing at launch: a relaunched Glim waits (up to 15 s) for the one
/// that started it to exit. Returns true when this is a relaunch.
pub fn wait_for_previous(argv: &[String]) -> bool {
    let Some(pid) = previous_pid(argv) else { return false };
    #[cfg(windows)]
    unsafe {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
        if let Ok(process) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            let _ = WaitForSingleObject(process, 15_000);
            let _ = CloseHandle(process);
        }
    }
    #[cfg(not(windows))]
    let _ = pid;
    true
}

/// One Windows notification (a balloon from a short-lived tray icon, which
/// Windows 10 and 11 show as a toast). It needs a window handle: any of
/// Glim's windows still has one even with its page gone.
fn notify(app: &AppHandle) {
    #[cfg(windows)]
    {
        use tauri::Manager;
        let Some(raw) = app.webview_windows().values().find_map(|w| w.hwnd().ok().map(|h| h.0 as isize)) else {
            crate::log::line("relaunch: no window to notify from");
            return;
        };
        let text = crate::i18n::t(NOTICE);
        std::thread::spawn(move || unsafe { balloon(raw, &text) });
    }
    #[cfg(not(windows))]
    let _ = app;
}

#[cfg(windows)]
unsafe fn balloon(raw_hwnd: isize, text: &str) {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
    };
    use windows::Win32::UI::WindowsAndMessaging::LoadIconW;

    fn copy(into: &mut [u16], text: &str) {
        let wide: Vec<u16> = text.encode_utf16().take(into.len() - 1).collect();
        into[..wide.len()].copy_from_slice(&wide);
    }
    let mut data: NOTIFYICONDATAW = std::mem::zeroed();
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = HWND(raw_hwnd as *mut _);
    data.uID = 0x6c69; // its own id: Glim's tray icon is untouched
    data.uFlags = NIF_ICON | NIF_TIP | NIF_INFO;
    // The app icon tauri-build embeds (WINDOWS_APP_ICON_RESOURCE_ID).
    if let Ok(module) = GetModuleHandleW(PCWSTR::null()) {
        data.hIcon = LoadIconW(Some(module.into()), PCWSTR(32512 as *const u16)).unwrap_or_default();
    }
    data.dwInfoFlags = NIIF_INFO;
    copy(&mut data.szTip, "Glim");
    copy(&mut data.szInfoTitle, "Glim");
    copy(&mut data.szInfo, text);
    if !Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
        crate::log::line("relaunch: the notification could not be shown");
        return;
    }
    std::thread::sleep(std::time::Duration::from_secs(30));
    let _ = Shell_NotifyIconW(NIM_DELETE, &data);
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_792_965_266;

    #[test]
    fn the_first_stop_relaunches() {
        assert_eq!(decide(Record::default(), T0), Decision::Relaunch);
    }

    #[test]
    fn a_second_stop_within_10_minutes_notifies_instead() {
        let after_one = Record { last: Some(T0), count: 1 };
        assert_eq!(decide(after_one, T0), Decision::Notify);
        assert_eq!(decide(after_one, T0 + 60), Decision::Notify);
        assert_eq!(decide(after_one, T0 + MIN_GAP_SECS - 1), Decision::Notify);
    }

    #[test]
    fn after_10_minutes_it_relaunches_again() {
        let after_one = Record { last: Some(T0), count: 1 };
        assert_eq!(decide(after_one, T0 + MIN_GAP_SECS), Decision::Relaunch);
        assert_eq!(decide(after_one, T0 + 3_600), Decision::Relaunch);
    }

    #[test]
    fn a_clock_set_back_does_not_block_a_relaunch() {
        assert_eq!(decide(Record { last: Some(T0), count: 1 }, T0 - 5), Decision::Relaunch);
    }

    #[test]
    fn the_file_round_trips_and_survives_junk() {
        let r = Record { last: Some(T0), count: 3 };
        assert_eq!(parse(&format(r)), r);
        assert_eq!(parse(""), Record::default());
        assert_eq!(parse("garbage"), Record::default());
        assert_eq!(parse(&format!("{T0}")), Record { last: Some(T0), count: 0 });
    }

    #[test]
    fn the_relaunch_flag_carries_the_old_pid() {
        let argv = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(previous_pid(&argv(&["glim.exe", AFTER_FLAG, "4242"])), Some(4242));
        assert_eq!(previous_pid(&argv(&["glim.exe"])), None);
        assert_eq!(previous_pid(&argv(&["glim.exe", AFTER_FLAG])), None);
        assert_eq!(previous_pid(&argv(&["glim.exe", AFTER_FLAG, "x"])), None);
        assert!(!wait_for_previous(&argv(&["glim.exe"])));
    }

    #[test]
    fn wiring() {
        let lib = include_str!("lib.rs").replace("\r\n", "\n");
        let run = &lib[lib.find("pub fn run()").unwrap()..];
        assert!(run.find("relaunch::wait_for_previous(").unwrap() < run.find("tauri_plugin_single_instance::init").unwrap());
        let health = include_str!("webview_health.rs").replace("\r\n", "\n");
        assert!(health.contains("crate::relaunch::on_browser_exited("));
    }
}
