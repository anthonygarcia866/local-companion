// Windows: Win32 for the island window and the cursor, %APPDATA% for files.

use std::collections::HashMap;
use std::os::windows::io::RawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use tauri::{AppHandle, Manager, WebviewWindow};

use ::windows::core::{w, BOOL, PWSTR};
use ::windows::Win32::Foundation::{
    CloseHandle, HANDLE, HLOCAL, HWND, LPARAM, LocalFree, POINT, RPC_E_CHANGED_MODE,
};
use ::windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use ::windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
use ::windows::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use ::windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use ::windows::Win32::System::Ole::RevokeDragDrop;
use ::windows::Win32::System::Pipes::GetNamedPipeClientProcessId;
use ::windows::Win32::System::SystemInformation::GetLocalTime;
use ::windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use ::windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyboardLayoutList, MapVirtualKeyExW, ToUnicodeEx, HKL, MAPVK_VK_TO_VSC,
    VK_CONTROL, VK_LBUTTON, VK_MENU, VK_SHIFT,
};
use ::windows::Win32::UI::Shell::{
    ShellExecuteExW, SHELLEXECUTEINFOW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC,
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GetClassNameW, GetCursorPos, GetWindow, GetWindowLongPtrW,
    GetWindowDisplayAffinity, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    SetForegroundWindow, SetWindowDisplayAffinity, SetWindowLongPtrW, ShowWindow, GWL_EXSTYLE,
    GW_OWNER, SW_HIDE, SW_RESTORE, SW_SHOWNORMAL, WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

use super::LocalTime;
use crate::session_window::{self, Proc};

/// File name of the Claude Code relay.
pub const HOOK_EXE: &str = "glim-hook.exe";

/// Environment variable holding the home directory.
pub const HOME_VAR: &str = "USERPROFILE";

/// Keeps spawned helpers from flashing a console window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ── Files ─────────────────────────────────────────────────────────────────────

/// %APPDATA%\Glim — preferences.
pub fn config_dir() -> PathBuf {
    data_base("APPDATA").unwrap_or_else(|| PathBuf::from(".")).join(DATA_DIR_NAME)
}

/// %LOCALAPPDATA%\Glim — where glim-hook.exe, the inbox, the recap and the log live.
pub fn local_dir() -> PathBuf {
    data_base("LOCALAPPDATA").unwrap_or_else(|| PathBuf::from(".")).join(DATA_DIR_NAME)
}

/// The folder `%APPDATA%` or `%LOCALAPPDATA%` (`var`) names. Under `cargo
/// test` it is a scratch folder of the test's own instead, so no test can
/// create or change the real Glim folders; `tests_never_use_the_real_data_folders`
/// checks it, and CI checks the real folders are still absent afterwards.
fn data_base(var: &str) -> Option<PathBuf> {
    #[cfg(test)]
    return Some(test_data_root().join(var));
    #[cfg(not(test))]
    std::env::var_os(var).map(PathBuf::from)
}

/// The scratch stand-in for the user's profile folders, one per test thread
/// (libtest runs each test on a thread of its own).
#[cfg(test)]
pub fn test_data_root() -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    thread_local! {
        static ROOT: PathBuf = std::env::temp_dir()
            .join(format!("glim-test-data-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    }
    ROOT.with(Clone::clone)
}

/// The data folders' name, under %APPDATA% and %LOCALAPPDATA%.
const DATA_DIR_NAME: &str = "Glim";

/// What the data folders, the log and the relay were called before the rename
/// (upstream Coucou's names). `migrate_data_dirs` moves them over.
const LEGACY_DATA_DIR_NAME: &str = "Coucou";
const LEGACY_LOG: &str = "coucou.log";
const LEGACY_HOOK_EXE: &str = "coucou-hook.exe";

/// Moves what an older build left in %APPDATA%\Coucou and %LOCALAPPDATA%\Coucou
/// into the Glim folders, and returns what it did, for the log (which lives in
/// one of these folders, so it can only be written afterwards). Runs on every
/// launch before anything else touches the folders; once the old folders are
/// gone it does nothing.
///
/// The whole folder is renamed when the new one doesn't exist yet. Otherwise
/// (say a rename failed on an earlier launch because a relay was still running
/// from the old folder) each entry missing from the new folder is moved on its
/// own (folders on both sides are merged the same way), and the old folder goes
/// once it is empty. Nothing that already exists in the new folder is
/// overwritten.
pub fn migrate_data_dirs() -> Vec<String> {
    let mut notes = Vec::new();
    for var in ["APPDATA", "LOCALAPPDATA"] {
        let Some(base) = data_base(var) else { continue };
        migrate_dir(&base.join(LEGACY_DATA_DIR_NAME), &base.join(DATA_DIR_NAME), &mut notes);
    }
    let local = local_dir();
    // The log keeps its history under its new name.
    let (old_log, new_log) = (local.join(LEGACY_LOG), local.join("glim.log"));
    if old_log.exists() && !new_log.exists() {
        match std::fs::rename(&old_log, &new_log) {
            Ok(()) => notes.push(format!("renamed {} to {}", old_log.display(), new_log.display())),
            Err(e) => notes.push(format!("could not rename {}: {e}", old_log.display())),
        }
    }
    // The old relay: nothing can run it from here (the path hooks pointed at
    // went with the old folder), and launch stages glim-hook.exe beside it.
    let old_relay = local.join("bin").join(LEGACY_HOOK_EXE);
    if old_relay.exists() {
        match std::fs::remove_file(&old_relay) {
            Ok(()) => notes.push(format!("removed {}", old_relay.display())),
            Err(e) => notes.push(format!("could not remove {}: {e}", old_relay.display())),
        }
    }
    notes
}

fn migrate_dir(old: &Path, new: &Path, notes: &mut Vec<String>) {
    if !old.is_dir() {
        return;
    }
    if !new.exists() {
        match std::fs::rename(old, new) {
            Ok(()) => {
                notes.push(format!("moved {} to {}", old.display(), new.display()));
                return;
            }
            Err(e) => notes.push(format!("could not move {} as a whole ({e}); moving its entries", old.display())),
        }
    }
    if let Err(e) = std::fs::create_dir_all(new) {
        notes.push(format!("could not create {}: {e}", new.display()));
        return;
    }
    let Ok(entries) = std::fs::read_dir(old) else { return };
    for entry in entries.flatten() {
        let target = new.join(entry.file_name());
        if target.exists() {
            // Two folders of the same name (an inbox on both sides): merge them.
            if entry.path().is_dir() && target.is_dir() {
                migrate_dir(&entry.path(), &target, notes);
            } else {
                notes.push(format!("kept {} (already in {})", entry.path().display(), new.display()));
            }
            continue;
        }
        match std::fs::rename(entry.path(), &target) {
            Ok(()) => notes.push(format!("moved {} to {}", entry.path().display(), target.display())),
            Err(e) => notes.push(format!("could not move {}: {e}", entry.path().display())),
        }
    }
    // Only succeeds once everything made it across.
    if std::fs::remove_dir(old).is_ok() {
        notes.push(format!("removed the empty {}", old.display()));
    }
}

/// Where a saved image goes, best first: Pictures (also where OneDrive moves
/// it), then Downloads and the profile folder. The caller takes the first one
/// that exists. Read from the environment rather than the Known Folders API: a
/// relocated Pictures folder falls back to Downloads, which is fine for a PNG.
pub fn picture_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
    if let Some(home) = &home {
        dirs.push(home.join("Pictures"));
    }
    if let Some(onedrive) = std::env::var_os("OneDrive").map(PathBuf::from) {
        dirs.push(onedrive.join("Pictures"));
    }
    if let Some(home) = home {
        dirs.push(home.join("Downloads"));
        dirs.push(home);
    }
    dirs
}

/// %APPDATA% and %LOCALAPPDATA% are already private to the user.
pub fn ensure_private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

/// Nothing to set up before the webview starts.
pub fn prepare_environment() {}

pub fn local_time() -> LocalTime {
    let t = unsafe { GetLocalTime() };
    LocalTime {
        year: t.wYear.into(),
        month: t.wMonth.into(),
        day: t.wDay.into(),
        hour: t.wHour.into(),
        minute: t.wMinute.into(),
        second: t.wSecond.into(),
    }
}

// ── Processes ─────────────────────────────────────────────────────────────────

/// Spawned helpers must never flash a console window.
pub fn no_console(cmd: &mut Command) -> &mut Command {
    cmd.creation_flags(CREATE_NO_WINDOW)
}

pub fn open_url(url: &str) {
    let _ = no_console(Command::new("rundll32.exe").args(["url.dll,FileProtocolHandler", url]))
        .spawn();
}

/// Voice `open_app` (1.5c): opens an app Glim knows by its id, never a path
/// from anywhere else. False when Windows can't find or start it.
/// - "outlook": classic Outlook, through Windows' App Paths (`OUTLOOK.EXE`).
/// - "codex", "claude_code" (owner, 2026-10-16: the desktop apps): the Codex
///   app (package `OpenAI.Codex`, which Start lists as "ChatGPT") and the
///   Claude app (package `Claude`, where Claude Code lives), started by their
///   app ids, found on this PC (`package_family`).
pub fn open_known_app(app: &str) -> bool {
    use ::windows::core::w;
    match app {
        "outlook" => shell_open(w!("OUTLOOK.EXE")),
        "codex" => activate_package("OpenAI.Codex", "App"),
        "claude_code" => activate_package("Claude", "Claude"),
        // The Claude chat app (4d, owner 2026-10-16): the same desktop app.
        "claude" => activate_package("Claude", "Claude"),
        _ => false,
    }
}

/// Opens one of the user's own apps (Settings, voice::own_apps) at `path`:
/// checked again here (a full local path to an .exe or .lnk, on a drive
/// that isn't a network drive, the file there), then ShellExecute "open" on
/// it with no arguments; no shell. False when any check fails or Windows
/// refuses.
pub fn open_own_app(path: &str) -> bool {
    use ::windows::core::{w, PCWSTR};
    use ::windows::Win32::Storage::FileSystem::GetDriveTypeW;
    use ::windows::Win32::UI::Shell::ShellExecuteW;
    use ::windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    if crate::voice::own_apps::path_problem(path).is_some() {
        return false;
    }
    // A mapped network drive (Z:\) would reach the network: checked by its
    // root before the file is touched. DRIVE_REMOTE is 4.
    let root: Vec<u16> = format!("{}\\", &path[..2]).encode_utf16().chain(Some(0)).collect();
    if unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } == 4 {
        return false;
    }
    let file = std::path::Path::new(path);
    if !file.is_file() {
        return false;
    }
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(Some(0)).collect() };
    let target = wide(path);
    let dir = wide(&file.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default());
    let r = unsafe { ShellExecuteW(None, w!("open"), PCWSTR(target.as_ptr()), None, PCWSTR(dir.as_ptr()), SW_SHOWNORMAL) };
    r.0 as usize > 32
}

/// ShellExecute "open" on a file Windows resolves itself (App Paths); no shell.
fn shell_open(file: ::windows::core::PCWSTR) -> bool {
    use ::windows::core::w;
    use ::windows::Win32::UI::Shell::ShellExecuteW;
    use ::windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    // ShellExecute returns a value above 32 when it started the app.
    let r = unsafe { ShellExecuteW(None, w!("open"), file, None, None, SW_SHOWNORMAL) };
    r.0 as usize > 32
}

/// The family name of the installed package called `name` among the
/// per-user package folders ("OpenAI.Codex" → "OpenAI.Codex_2p2nqsd0c76g0"):
/// the name, an underscore, a 13-character publisher id. None when it isn't
/// installed.
pub fn package_family(folders: &[String], name: &str) -> Option<String> {
    folders.iter().find(|f| {
        f.strip_prefix(name).and_then(|rest| rest.strip_prefix('_')).is_some_and(|id| id.len() == 13 && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
    }).cloned()
}

/// Starts packaged app `name`'s application `app_id` through Windows' own
/// activation (IApplicationActivationManager), on a thread of its own with
/// COM. False when the package isn't there or Windows refuses.
fn activate_package(name: &str, app_id: &str) -> bool {
    let Some(packages) = data_base("LOCALAPPDATA").map(|d| d.join("Packages")) else { return false };
    let folders: Vec<String> = std::fs::read_dir(packages).map(|r| r.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    let Some(family) = package_family(&folders, name) else { return false };
    let aumid: Vec<u16> = format!("{family}!{app_id}").encode_utf16().chain(Some(0)).collect();
    std::thread::spawn(move || unsafe {
        use ::windows::core::PCWSTR;
        use ::windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED};
        use ::windows::Win32::UI::Shell::{ApplicationActivationManager, IApplicationActivationManager, AO_NONE};
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let Ok(manager) = CoCreateInstance::<_, IApplicationActivationManager>(&ApplicationActivationManager, None, CLSCTX_LOCAL_SERVER) else { return false };
        manager.ActivateApplication(PCWSTR(aumid.as_ptr()), PCWSTR::null(), AO_NONE).is_ok()
    })
    .join()
    .unwrap_or(false)
}

pub fn reveal_folder(path: &str) {
    let _ = Command::new("explorer").arg(path).spawn();
}

/// Ties a child process's life to Glim's: it is put in a job object that
/// kills it when Glim's handle to the job closes (Glim exits or crashes).
/// The handle is kept for Glim's lifetime on purpose.
pub fn kill_with_glim(child: &std::process::Child) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    unsafe {
        let Ok(job) = CreateJobObjectW(None, None) else { return false };
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let set = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        set.is_ok() && AssignProcessToJobObject(job, windows::Win32::Foundation::HANDLE(child.as_raw_handle())).is_ok()
    }
}

/// Claude Code in its own console window, in `folder`, with `prompt` as its
/// first message (the resume card's click). No shell: the launcher is found
/// on PATH (or in ~/.local/bin, where Claude Code's installer puts it) and
/// the prompt is one argument.
pub fn open_claude_code_in(folder: &std::path::Path, prompt: &str) -> bool {
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    let claude = find_on_path("claude").or_else(|| {
        let local = super::home_dir().join(".local").join("bin").join("claude.exe");
        local.is_file().then_some(local)
    });
    let Some(claude) = claude else { return false };
    if !folder.is_dir() {
        return false;
    }
    Command::new(claude).current_dir(folder).arg(prompt).creation_flags(CREATE_NEW_CONSOLE).spawn().is_ok()
}

/// Our own `where`: walks %PATH% against %PATHEXT%, no shell involved.
/// Rust quotes arguments correctly for `.cmd`/`.bat` targets since 1.77, so
/// spawning `code.cmd` directly is safe.
pub fn find_on_path(stem: &str) -> Option<PathBuf> {
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    let dirs = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&dirs) {
        for ext in exts.split(';').filter(|e| !e.is_empty()) {
            let candidate = dir.join(format!("{stem}{}", ext.to_lowercase()));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

// ── Who we are ────────────────────────────────────────────────────────────────
//
// Named pipes share one machine-wide namespace, so the SID in the name is what
// keeps two accounts on the same machine from ever meeting on `glim-*`.
// glim-hook computes the same string (hook/src/win.rs) and additionally checks
// that the process serving the pipe really is us.

/// The SID of the account this process runs as, as `S-1-5-21-…`.
pub fn current_user_sid() -> Option<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;

        // First call sizes the buffer, second fills it.
        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
        if needed == 0 {
            let _ = CloseHandle(token);
            return None;
        }
        let mut buf = vec![0u8; needed as usize];
        let ok = GetTokenInformation(
            token,
            TokenUser,
            Some(buf.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )
        .is_ok();
        let _ = CloseHandle(token);
        if !ok {
            return None;
        }

        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut text = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut text).ok()?;
        let sid = text.to_string().ok();
        let _ = LocalFree(Some(HLOCAL(text.0 as *mut _)));
        sid
    }
}

// The display name only feeds the chat's greeting (identity.rs). The calls are
// declared here by hand, with their documented C signatures, so they need no
// extra `windows` crate features.
#[link(name = "netapi32")]
extern "system" {
    fn NetUserGetInfo(server: *const u16, user: *const u16, level: u32, buffer: *mut *mut u8) -> u32;
    fn NetApiBufferFree(buffer: *mut core::ffi::c_void) -> u32;
}

/// The local account's "Full name", else nothing. Only the local account
/// database is read (NetUserGetInfo with no server): the directory lookup of a
/// domain or Entra account (GetUserNameExW) was removed because it can query a
/// domain controller over the network.
pub fn user_full_name() -> Option<String> {
    local_full_name().filter(|n| !n.trim().is_empty())
}

fn local_full_name() -> Option<String> {
    let user = std::env::var("USERNAME").ok().filter(|u| !u.is_empty())?;
    let wide: Vec<u16> = user.encode_utf16().chain(std::iter::once(0)).collect();
    let mut info: *mut u8 = std::ptr::null_mut();
    // Level 10 is USER_INFO_10 { name, comment, usr_comment, full_name }, four
    // wide-string pointers; any user may read it about themselves.
    let status = unsafe { NetUserGetInfo(std::ptr::null(), wide.as_ptr(), 10, &mut info) };
    if status != 0 || info.is_null() {
        return None;
    }
    unsafe {
        let full = *(info as *const *const u16).add(3);
        let text = if full.is_null() {
            None
        } else {
            let mut len = 0usize;
            while len < 256 && *full.add(len) != 0 {
                len += 1;
            }
            String::from_utf16(std::slice::from_raw_parts(full, len)).ok()
        };
        NetApiBufferFree(info.cast());
        text
    }
}

// ── Cursor ────────────────────────────────────────────────────────────────────

/// The 60 Hz poll reads the cursor and flips click-through from it.
pub const CURSOR_POLL: bool = true;

/// Cursor position in physical screen pixels.
pub fn cursor_physical() -> Option<(f64, f64)> {
    let mut p = POINT::default();
    unsafe { GetCursorPos(&mut p).ok()? };
    Some((p.x as f64, p.y as f64))
}

/// The centre of the foreground window, in physical screen pixels, unless
/// there is none or it is one of Glim's own windows (the "active window's
/// display" setting keeps the island where it is then).
pub fn foreground_center() -> Option<(f64, f64)> {
    use ::windows::Win32::Foundation::RECT;
    use ::windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId};
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 || pid == std::process::id() {
            return None;
        }
        let mut r = RECT::default();
        GetWindowRect(hwnd, &mut r).ok()?;
        Some(((r.left + r.right) as f64 / 2.0, (r.top + r.bottom) as f64 / 2.0))
    }
}

/// A fullscreen app is in front on the display `(x, y, w, h)` (physical):
/// Windows says so (a Direct3D fullscreen app, presentation mode), or the
/// foreground window — not Glim's, not the desktop — covers the whole display.
/// A maximised window leaves the taskbar out, so it doesn't count.
pub fn foreground_fullscreen(display: (i32, i32, u32, u32)) -> bool {
    use ::windows::Win32::Foundation::RECT;
    use ::windows::Win32::UI::Shell::{
        SHQueryUserNotificationState, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
    };
    use ::windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId};
    unsafe {
        if let Ok(state) = SHQueryUserNotificationState() {
            if state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE {
                return true;
            }
        }
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == std::process::id() {
            return false;
        }
        let mut class = [0u16; 64];
        let n = GetClassNameW(hwnd, &mut class) as usize;
        let class = String::from_utf16_lossy(&class[..n.min(class.len())]);
        if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd") {
            return false;
        }
        let mut r = RECT::default();
        if GetWindowRect(hwnd, &mut r).is_err() {
            return false;
        }
        covers((r.left, r.top, r.right, r.bottom), display)
    }
}

/// Whether a window rectangle (left, top, right, bottom) covers the display.
fn covers(r: (i32, i32, i32, i32), d: (i32, i32, u32, u32)) -> bool {
    r.0 <= d.0 && r.1 <= d.1 && r.2 >= d.0 + d.2 as i32 && r.3 >= d.1 + d.3 as i32
}

#[cfg(test)]
mod fullscreen_tests {
    #[test]
    fn a_window_covering_the_display_is_fullscreen_a_maximised_one_is_not() {
        let display = (0, 0, 1920, 1080);
        assert!(super::covers((0, 0, 1920, 1080), display));
        // Maximised: borders past the edges, but the taskbar's 48 px left out.
        assert!(!super::covers((-8, -8, 1928, 1040), display));
        // Fullscreen on the display to the left.
        assert!(!super::covers((-1920, 0, 0, 1080), display));
    }
}

/// True while the left mouse button is held — the only signal we get that a
/// drag might be in flight before it reaches the window.
pub fn left_button_down() -> bool {
    unsafe { (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) != 0 }
}

// ── Island window ─────────────────────────────────────────────────────────────

fn hwnd_of(win: &WebviewWindow) -> Option<HWND> {
    let raw = win.hwnd().ok()?.0 as isize;
    if raw == 0 {
        return None;
    }
    Some(HWND(raw as *mut _))
}

/// Keeps `win` out of screen shares, recordings and screenshots
/// (WDA_EXCLUDEFROMCAPTURE, Windows 10 2004 or later), or lets them see it
/// again. False when Windows refused (an older Windows).
pub fn set_capture_excluded(win: &WebviewWindow, excluded: bool) -> bool {
    hwnd_of(win).is_some_and(|hwnd| set_hwnd_capture_excluded(hwnd, excluded))
}

fn set_hwnd_capture_excluded(hwnd: HWND, excluded: bool) -> bool {
    let affinity = if excluded { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
    unsafe {
        // Never on a hidden window. Seen 2026-10-27 with the hidden Settings
        // window: Windows went on drawing a white copy of it on top of
        // everything, which clicks went through and only ending Glim cleared,
        // while IsWindowVisible kept saying it was hidden. A window shown
        // later gets it then (screen_capture::after_show).
        if !IsWindowVisible(hwnd).as_bool() {
            return true;
        }
        // Nothing to change: no call at all.
        let mut now = 0u32;
        if GetWindowDisplayAffinity(hwnd, &mut now).is_ok() && now == affinity.0 {
            return true;
        }
        SetWindowDisplayAffinity(hwnd, affinity).is_ok()
    }
}

/// What Win32 itself says about `win` (not Tauri's flags), for
/// `glim.exe --selftest`: visible, and its display affinity.
pub fn window_state(win: &WebviewWindow) -> Option<(bool, u32)> {
    let hwnd = hwnd_of(win)?;
    unsafe {
        let mut affinity = 0u32;
        GetWindowDisplayAffinity(hwnd, &mut affinity).ok()?;
        Some((IsWindowVisible(hwnd).as_bool(), affinity))
    }
}

/// The affinity a window kept out of capture has.
pub const CAPTURE_EXCLUDED: u32 = WDA_EXCLUDEFROMCAPTURE.0;

/// Another glim.exe is running (same executable name, another process).
pub fn other_glim_running() -> bool {
    let me = std::process::id();
    let Some(name) = std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase())) else {
        return false;
    };
    process_table().iter().any(|(pid, p)| *pid != me && p.exe.to_lowercase() == name)
}

/// Hides `win` with Win32 itself, for a window Glim never showed.
pub fn hide_window(win: &WebviewWindow) {
    if let Some(hwnd) = hwnd_of(win) {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

/// Keeps the settings window from showing the "no drop" cursor.
///
/// wry installs its drop target by walking the webview's child windows **once**,
/// when the webview is created. WebView2 creates `Chrome_RenderWidgetHostHWND`
/// later and registers its own target on it; being the innermost window, that one
/// wins, and since the page has no HTML5 drop handler it refuses everything.
/// Revoking it makes OLE fall through to wry's target. The island is the other
/// way round: it takes drops through WebView2's own target (see webview_drop.rs),
/// so its windows are left alone.
///
/// Cheap and idempotent, so it is simply re-run whenever a drag might be starting.
pub fn unblock_webview_drops(app: &AppHandle) {
    let Some(win) = app.get_webview_window("settings") else { return };
    let Some(hwnd) = hwnd_of(&win) else { return };
    unsafe {
        let _ = EnumChildWindows(Some(hwnd), Some(revoke_render_widget), LPARAM(0));
    }
}

unsafe extern "system" fn revoke_render_widget(hwnd: HWND, _: LPARAM) -> BOOL {
    let mut name = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, &mut name) };
    if len > 0 {
        let class = String::from_utf16_lossy(&name[..len as usize]);
        if class == "Chrome_RenderWidgetHostHWND" {
            let _ = unsafe { RevokeDragDrop(hwnd) };
        }
    }
    true.into()
}

/// WS_EX_NOACTIVATE keeps clicks from stealing focus; WS_EX_TOOLWINDOW keeps the
/// island out of Alt-Tab.
pub fn make_non_activating(win: &WebviewWindow) {
    let Some(hwnd) = hwnd_of(win) else { return };
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let want = ex | WS_EX_NOACTIVATE.0 as isize | WS_EX_TOOLWINDOW.0 as isize;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, want);
    }
}

/// Temporarily allow activation so a text field inside the island can be typed in.
pub fn set_activating(win: &WebviewWindow, activating: bool) {
    let Some(hwnd) = hwnd_of(win) else { return };
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let want = if activating {
            ex & !(WS_EX_NOACTIVATE.0 as isize)
        } else {
            ex | WS_EX_NOACTIVATE.0 as isize
        };
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, want);
    }
}

/// The window in front now, unless it is `own` (the island itself): the one a
/// box on a card gives the keyboard back to (lend_keyboard in lib.rs).
pub fn foreground_other_than(own: &WebviewWindow) -> Option<isize> {
    use ::windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let own = hwnd_of(own)?.0 as isize;
    let front = unsafe { GetForegroundWindow() }.0 as isize;
    (front != 0 && front != own).then_some(front)
}

/// The program file of the visible window in front now ("OUTLOOK.EXE"):
/// Glim learns' check that an app it opened came forward. The process list
/// only; nothing is opened or attached.
pub fn foreground_exe() -> Option<String> {
    use ::windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let front = unsafe { GetForegroundWindow() };
    if front.0.is_null() || !unsafe { IsWindowVisible(front) }.as_bool() {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(front, Some(&mut pid)) };
    if pid == 0 {
        return None;
    }
    process_table().remove(&pid).map(|p| p.exe)
}

/// Gives the keyboard back to `to` if the island (`own`) still has it: a click
/// elsewhere since then already chose another window, and that one stays.
pub fn give_foreground_back(own: &WebviewWindow, to: isize) -> bool {
    use ::windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, IsWindow};
    let Some(own) = hwnd_of(own) else { return false };
    let front = unsafe { GetForegroundWindow() }.0 as isize;
    let alive = unsafe { IsWindow(Some(HWND(to as *mut _))) }.as_bool();
    match super::give_back_to(Some(to), front, own.0 as isize) {
        // Allowed: the island is the foreground window, so it may pass it on.
        Some(raw) if alive => bring_forward(raw),
        _ => false,
    }
}

/// Click-through here is the poll's WS_EX_TRANSPARENT toggle, not a region.
pub fn set_input_region(_win: &WebviewWindow, _rect: Option<(f64, f64, f64, f64)>) {}

/// The cursor poll already parks itself with the island (island.rs): nothing to do.
pub fn set_pointer_watch(_active: bool) {}

/// `set_position` already places a Win32 window on the right display.
pub fn pin_to_monitor(_win: &WebviewWindow, _x: i32, _y: i32) {}

// ── Session windows ("Open terminal") ─────────────────────────────────────────
//
// See session_window.rs: the relay's ancestors lead to the terminal or editor
// window a Claude Code session runs in.

/// The process on the other end of a relay connection.
pub fn pipe_client_pid(handle: RawHandle) -> Option<u32> {
    let mut pid = 0u32;
    unsafe { GetNamedPipeClientProcessId(HANDLE(handle), &mut pid).ok()? };
    (pid != 0).then_some(pid)
}

/// Every process, by ID: its parent and its executable's name.
fn process_table() -> HashMap<u32, Proc> {
    let mut out = HashMap::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
        while more {
            let len = entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(entry.szExeFile.len());
            out.insert(
                entry.th32ProcessID,
                Proc {
                    parent: entry.th32ParentProcessID,
                    exe: String::from_utf16_lossy(&entry.szExeFile[..len]),
                },
            );
            more = Process32NextW(snapshot, &mut entry).is_ok();
        }
        let _ = CloseHandle(snapshot);
    }
    out
}

/// Whether a process with this executable name (any case) is running. The
/// process list only: nothing is opened or attached.
pub fn process_running(exe: &str) -> bool {
    process_table().values().any(|p| p.exe.eq_ignore_ascii_case(exe))
}

/// The ancestors of a process, nearest first, below the desktop shell.
pub fn process_ancestors(pid: u32) -> Vec<u32> {
    session_window::ancestors(&process_table(), pid)
}

/// Visible top-level windows that are nobody's dialog and have a title:
/// (handle, process ID, title).
fn top_windows() -> Vec<(isize, u32, String)> {
    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let list = unsafe { &mut *(lparam.0 as *mut Vec<(isize, u32, String)>) };
        unsafe {
            if !IsWindowVisible(hwnd).as_bool() || GetWindow(hwnd, GW_OWNER).is_ok() {
                return true.into();
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid as *mut u32));
            let mut text = [0u16; 512];
            let len = GetWindowTextW(hwnd, &mut text);
            if len > 0 {
                list.push((hwnd.0 as isize, pid, String::from_utf16_lossy(&text[..len as usize])));
            }
        }
        true.into()
    }
    let mut list: Vec<(isize, u32, String)> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut list as *mut _ as isize));
    }
    list
}

// ── The `window` action (window_tool.rs) ─────────────────────────────────────

/// Every monitor's work area (the screen less the taskbar), in physical pixels.
fn work_areas() -> Vec<crate::window_tool::Rect> {
    use ::windows::Win32::Foundation::RECT;
    use ::windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO};
    unsafe extern "system" fn each(mon: HMONITOR, _dc: HDC, _r: *mut RECT, lparam: LPARAM) -> BOOL {
        let list = unsafe { &mut *(lparam.0 as *mut Vec<crate::window_tool::Rect>) };
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if unsafe { GetMonitorInfoW(mon, &mut info) }.as_bool() {
            let w = info.rcWork;
            list.push(crate::window_tool::Rect { x: w.left, y: w.top, w: w.right - w.left, h: w.bottom - w.top });
        }
        true.into()
    }
    let mut list = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut list as *mut _ as isize));
    }
    list
}

/// The window's rectangle, and how far it reaches past the drawn frame.
fn window_rect(hwnd: HWND) -> Option<(crate::window_tool::Rect, crate::window_tool::Margins)> {
    use ::windows::Win32::Foundation::RECT;
    use ::windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
    use ::windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut r) }.ok()?;
    let mut f = RECT::default();
    let framed = unsafe { DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut f as *mut _ as *mut _, std::mem::size_of::<RECT>() as u32) }.is_ok();
    let m = if framed {
        crate::window_tool::Margins { left: (f.left - r.left).max(0), top: (f.top - r.top).max(0), right: (r.right - f.right).max(0), bottom: (r.bottom - f.bottom).max(0) }
    } else {
        crate::window_tool::Margins::default()
    };
    Some((crate::window_tool::Rect { x: r.left, y: r.top, w: r.right - r.left, h: r.bottom - r.top }, m))
}

fn window_state(hwnd: HWND, works: &[crate::window_tool::Rect]) -> Option<crate::window_tool::State> {
    use ::windows::Win32::UI::WindowsAndMessaging::IsZoomed;
    let (rect, _) = window_rect(hwnd)?;
    Some(crate::window_tool::State {
        rect,
        maximized: unsafe { IsZoomed(hwnd) }.as_bool(),
        minimized: unsafe { IsIconic(hwnd) }.as_bool(),
        monitor: crate::window_tool::monitor_of(works, &rect),
    })
}

/// "Put Outlook on the left": the app's own top-level window (the one in
/// front if it is the app's, else its topmost), moved on its monitor or the
/// other one, then read back. Ok only when it ended up where it was asked to
/// (the check before the call is learned). Never a password manager's or
/// Glim's own window. Codes for the card: window-missing (no window of that
/// app), window-refused, window-no-other-display, window-failed.
pub fn arrange_window(exe: &str, at: crate::window_tool::Where, display: crate::window_tool::Display) -> Result<(), &'static str> {
    use crate::window_tool::{self as wt, Where};
    use ::windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER, SW_MAXIMIZE, SW_MINIMIZE};
    if let Some(code) = wt::refused(exe) {
        return Err(code);
    }
    let me = std::process::id();
    let table = process_table();
    let ours = |pid: u32| table.get(&pid).is_some_and(|p| p.exe.eq_ignore_ascii_case(exe)) && pid != me;
    let windows: Vec<isize> = top_windows().into_iter().filter(|(_, pid, _)| ours(*pid)).map(|(h, _, _)| h).collect();
    let front = unsafe { GetForegroundWindow() }.0 as isize;
    let Some(raw) = windows.iter().copied().find(|h| *h == front).or_else(|| windows.first().copied()) else {
        return Err("window-missing");
    };
    let hwnd = HWND(raw as *mut _);
    let works = work_areas();
    let (rect, _) = window_rect(hwnd).ok_or("window-failed")?;
    let Some(mon) = wt::pick(&works, &rect, display) else {
        return Err(if works.len() < 2 { "window-no-other-display" } else { "window-failed" });
    };
    let work = works[mon];
    let restore = |h: HWND| unsafe {
        if IsIconic(h).as_bool() || ::windows::Win32::UI::WindowsAndMessaging::IsZoomed(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
    };
    let set = |r: wt::Rect| unsafe { SetWindowPos(hwnd, None, r.x, r.y, r.w, r.h, SWP_NOZORDER | SWP_NOACTIVATE) }.is_ok();
    let mut wanted = None;
    match at {
        Where::Left | Where::Right | Where::Center => {
            restore(hwnd);
            let (now, m) = window_rect(hwnd).ok_or("window-failed")?;
            let t = wt::target(at, &work, &now, &m).ok_or("window-failed")?;
            // Across monitors with different scales the app resizes itself on
            // arrival (WM_DPICHANGED): set it again once it is there.
            set(t);
            if window_rect(hwnd).is_some_and(|(r, _)| r != t) {
                std::thread::sleep(std::time::Duration::from_millis(120));
                let (_, m2) = window_rect(hwnd).ok_or("window-failed")?;
                let t2 = wt::target(at, &work, &now, &m2).ok_or("window-failed")?;
                set(t2);
                wanted = Some(t2);
            } else {
                wanted = Some(t);
            }
        }
        Where::Maximize => {
            if wt::monitor_of(&works, &rect) != mon {
                restore(hwnd);
                let (now, _) = window_rect(hwnd).ok_or("window-failed")?;
                set(wt::onto(&work, &now));
            }
            unsafe {
                let _ = ShowWindow(hwnd, SW_MAXIMIZE);
            }
        }
        Where::Minimize => unsafe {
            let _ = ShowWindow(hwnd, SW_MINIMIZE);
        },
        Where::Restore => unsafe {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        },
    }
    // Read back (an app may take a moment, or refuse).
    let until = std::time::Instant::now() + std::time::Duration::from_millis(1500);
    loop {
        if window_state(hwnd, &works).is_some_and(|s| wt::placed(at, wanted, mon, &s)) {
            return Ok(());
        }
        if std::time::Instant::now() >= until {
            return Err("window-failed");
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// The first of `pids` that owns a window.
pub fn first_with_window(pids: &[u32]) -> Option<u32> {
    let windows = top_windows();
    pids.iter().copied().find(|pid| windows.iter().any(|(_, owner, _)| owner == pid))
}

fn bring_forward(raw: isize) -> bool {
    let hwnd = HWND(raw as *mut _);
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        // Allowed: the click on the island that asked for this was the last input.
        SetForegroundWindow(hwnd).as_bool()
    }
}

/// Brings `pid`'s window forward — the one titled after `folder` if there are several.
pub fn focus_process_window(pid: u32, folder: &str) -> bool {
    let candidates: Vec<(isize, String)> = top_windows()
        .into_iter()
        .filter(|(_, owner, _)| *owner == pid)
        .map(|(hwnd, _, title)| (hwnd, title))
        .collect();
    session_window::pick_window(&candidates, folder).is_some_and(|hwnd| bring_forward(*hwnd))
}

/// Brings forward the window of a running app, by executable name.
fn focus_app(exe: &str) -> bool {
    let pids: Vec<u32> = process_table()
        .into_iter()
        .filter(|(_, p)| p.exe.eq_ignore_ascii_case(exe))
        .map(|(pid, _)| pid)
        .collect();
    top_windows()
        .into_iter()
        .find(|(_, owner, _)| pids.contains(owner))
        .is_some_and(|(hwnd, _, _)| bring_forward(hwnd))
}

/// The Claude desktop app: brought forward when it runs (Claude Code's own
/// `claude.exe` has no window of its own, so it is never the one picked),
/// started from the Squirrel path or its registered MSIX protocol otherwise.
pub fn open_claude_desktop() -> bool {
    open_claude_with(
        || focus_app("claude.exe"),
        || {
            std::env::var_os("LOCALAPPDATA").is_some_and(|base| {
                let exe = PathBuf::from(base).join("AnthropicClaude").join("claude.exe");
                exe.is_file() && Command::new(exe).spawn().is_ok()
            })
        },
        activate_claude_protocol,
    )
}

fn open_claude_with(
    focus: impl FnOnce() -> bool,
    squirrel: impl FnOnce() -> bool,
    protocol: impl FnOnce() -> bool,
) -> bool {
    focus() || squirrel() || protocol()
}

/// The MSIX manifest registers `claude`; let Windows resolve its current install.
fn activate_claude_protocol() -> bool {
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        // A Tauri thread may already use MTA. Keep that apartment: the Shell can
        // delegate activation, and we must not uninitialize someone else's COM.
        if initialized.is_err() && initialized != RPC_E_CHANGED_MODE {
            return false;
        }
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC,
            lpVerb: w!("open"),
            lpFile: w!("claude://"),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        let opened = ShellExecuteExW(&mut info).is_ok();
        if initialized.is_ok() {
            CoUninitialize();
        }
        opened
    }
}

#[cfg(test)]
mod claude_launch_tests {
    use super::open_claude_with;
    use std::cell::RefCell;

    #[test]
    fn tries_launchers_in_order_and_stops_after_success() {
        for (focus, squirrel, protocol, expected, calls) in [
            (true, true, true, true, vec!["focus"]),
            (false, true, true, true, vec!["focus", "squirrel"]),
            (false, false, true, true, vec!["focus", "squirrel", "protocol"]),
            (false, false, false, false, vec!["focus", "squirrel", "protocol"]),
        ] {
            let seen = RefCell::new(Vec::new());
            let result = open_claude_with(
                || {
                    seen.borrow_mut().push("focus");
                    focus
                },
                || {
                    seen.borrow_mut().push("squirrel");
                    squirrel
                },
                || {
                    seen.borrow_mut().push("protocol");
                    protocol
                },
            );
            assert_eq!(result, expected);
            assert_eq!(seen.into_inner(), calls);
        }
    }
}

// ── Global shortcuts ──────────────────────────────────────────────────────────

/// RegisterHotKey works in every Windows session.
pub fn global_shortcuts_blocked() -> Option<&'static str> {
    None
}

/// The character Ctrl+Alt(+Shift)+`vk` types on one of the installed keyboard
/// layouts, if any. Windows reads Ctrl+Alt as AltGr, so registering such a
/// combination as a global shortcut would stop the user typing that character
/// (AltGr+E is € on most European layouts, AltGr+A is ą in Polish).
pub fn ctrl_alt_types(vk: u16, shift: bool) -> Option<String> {
    let mut layouts = [HKL::default(); 32];
    let count = unsafe { GetKeyboardLayoutList(Some(&mut layouts[..])) };
    let count = usize::try_from(count).unwrap_or(0).min(layouts.len());

    let mut state = [0u8; 256];
    state[VK_CONTROL.0 as usize] = 0x80;
    state[VK_MENU.0 as usize] = 0x80;
    if shift {
        state[VK_SHIFT.0 as usize] = 0x80;
    }
    for &layout in &layouts[..count] {
        let scan = unsafe { MapVirtualKeyExW(u32::from(vk), MAPVK_VK_TO_VSC, Some(layout)) };
        let mut buf = [0u16; 8];
        // Flag 0x4: leave the keyboard state alone, so a dead key met here
        // doesn't change what the user types next (Windows 10 1607 and later).
        let n = unsafe { ToUnicodeEx(u32::from(vk), scan, &state, &mut buf, 0x4, Some(layout)) };
        let typed = match n {
            // A dead key (´ ^ ¨…) is still something the user types with it.
            n if n < 0 => String::from_utf16_lossy(&buf[..1]),
            0 => continue,
            n => String::from_utf16_lossy(&buf[..(n as usize).min(buf.len())]),
        };
        if typed.chars().any(|c| !c.is_control()) {
            return Some(typed);
        }
    }
    None
}

// ── Desktop Mochi window ──────────────────────────────────────────────────────

/// Windows places a window wherever it is asked, and the cursor poll is there.
pub fn desktop_mode() -> super::DesktopMode {
    super::DesktopMode::Poll
}

/// Same window styles as the island: never takes focus, never in Alt-Tab (it
/// is created without a taskbar button).
pub fn prepare_desktop_window(win: &WebviewWindow, _mode: super::DesktopMode) -> bool {
    make_non_activating(win);
    true
}

/// The cursor poll toggles WS_EX_TRANSPARENT from the body hit test instead.
pub fn set_desktop_shape(_win: &WebviewWindow, _shape: super::MouseShape) {}

/// Layer-shell only (Linux).
pub fn set_layer_margins(_win: &WebviewWindow, _x: f64, _y: f64) {}

/// Layer-shell only (Linux).
pub fn set_layer_overlay(_win: &WebviewWindow, _on: bool) {}

/// Layer-shell only (Linux): the logical size of the display the island is on.
pub fn layer_display(_island: &WebviewWindow, _mochi: &WebviewWindow) -> Option<(f64, f64)> {
    None
}

#[cfg(test)]
mod package_tests {
    use super::package_family;

    #[test]
    fn a_package_is_found_by_its_name_and_publisher_id_only() {
        let folders: Vec<String> = ["Claude_pzs8sxrjxfjjc", "ClaudeHelper_pzs8sxrjxfjjc", "OpenAI.Codex_2p2nqsd0c76g0", "OpenAI.Codex.Beta_2p2nqsd0c76g0", "Claude_short"].map(String::from).to_vec();
        assert_eq!(package_family(&folders, "Claude").as_deref(), Some("Claude_pzs8sxrjxfjjc"));
        assert_eq!(package_family(&folders, "OpenAI.Codex").as_deref(), Some("OpenAI.Codex_2p2nqsd0c76g0"));
        assert_eq!(package_family(&folders, "Outlook"), None);
        assert_eq!(package_family(&["Claude_short".to_string()], "Claude"), None);
    }
}

#[cfg(test)]
mod migration_tests {
    use super::migrate_dir;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("glim-migrate-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_old_folder_moves_whole_when_the_new_one_does_not_exist() {
        let base = scratch("whole");
        let (old, new) = (base.join("Coucou"), base.join("Glim"));
        std::fs::create_dir_all(old.join("inbox")).unwrap();
        std::fs::write(old.join("recap.json"), "{}").unwrap();
        let mut notes = Vec::new();
        migrate_dir(&old, &new, &mut notes);
        assert!(!old.exists(), "{notes:?}");
        assert!(new.join("inbox").is_dir() && new.join("recap.json").is_file(), "{notes:?}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn when_both_exist_missing_entries_move_and_nothing_is_overwritten() {
        let base = scratch("merge");
        let (old, new) = (base.join("Coucou"), base.join("Glim"));
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(old.join("recap.json"), "old").unwrap();
        std::fs::write(old.join("only-old.txt"), "moved").unwrap();
        std::fs::write(new.join("recap.json"), "new").unwrap();
        let mut notes = Vec::new();
        migrate_dir(&old, &new, &mut notes);
        assert_eq!(std::fs::read_to_string(new.join("recap.json")).unwrap(), "new", "never overwritten");
        assert_eq!(std::fs::read_to_string(new.join("only-old.txt")).unwrap(), "moved");
        assert!(old.join("recap.json").exists(), "what could not move stays where it was");
        // Run again: nothing changes, nothing is lost.
        migrate_dir(&old, &new, &mut notes);
        assert_eq!(std::fs::read_to_string(old.join("recap.json")).unwrap(), "old");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_old_folder_goes_once_everything_has_moved() {
        let base = scratch("empty");
        let (old, new) = (base.join("Coucou"), base.join("Glim"));
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(old.join("a.txt"), "a").unwrap();
        let mut notes = Vec::new();
        migrate_dir(&old, &new, &mut notes);
        assert!(!old.exists() && new.join("a.txt").exists(), "{notes:?}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_folder_on_both_sides_is_merged_and_the_old_one_goes() {
        let base = scratch("nested");
        let (old, new) = (base.join("Coucou"), base.join("Glim"));
        std::fs::create_dir_all(old.join("inbox")).unwrap();
        std::fs::create_dir_all(new.join("inbox")).unwrap();
        std::fs::write(old.join("inbox").join("dropped.png"), "png").unwrap();
        std::fs::write(old.join("glim-or-not.log"), "log").unwrap();
        let mut notes = Vec::new();
        migrate_dir(&old, &new, &mut notes);
        assert!(new.join("inbox").join("dropped.png").is_file(), "{notes:?}");
        assert!(new.join("glim-or-not.log").is_file(), "{notes:?}");
        assert!(!old.exists(), "{notes:?}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn tests_never_use_the_real_data_folders() {
        use super::{config_dir, local_dir, test_data_root};
        for (dir, var) in [(config_dir(), "APPDATA"), (local_dir(), "LOCALAPPDATA")] {
            assert!(dir.starts_with(test_data_root()), "{} is not a test folder", dir.display());
            if let Some(real) = std::env::var_os(var).map(PathBuf::from) {
                assert!(!dir.starts_with(real.join("Glim")), "{} is the real data folder", dir.display());
            }
        }
    }

    #[test]
    fn migration_only_ever_looks_in_the_test_folders() {
        // Runs the real entry point: with an old folder in this test's stand-in
        // profile it must move that one, and nothing outside it.
        let root = super::test_data_root();
        let old = root.join("LOCALAPPDATA").join("Coucou");
        std::fs::create_dir_all(old.join("inbox")).unwrap();
        std::fs::write(old.join("coucou.log"), "history").unwrap();
        let notes = super::migrate_data_dirs();
        assert!(notes.iter().all(|n| n.contains(&*root.to_string_lossy())), "{notes:?}");
        assert_eq!(std::fs::read_to_string(super::local_dir().join("glim.log")).unwrap(), "history");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn no_old_folder_means_nothing_happens() {
        let base = scratch("none");
        let mut notes = Vec::new();
        migrate_dir(&base.join("Coucou"), &base.join("Glim"), &mut notes);
        assert!(notes.is_empty() && !base.join("Glim").exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}

#[cfg(test)]
mod capture_tests {
    use super::*;
    use ::windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_POPUP, WS_VISIBLE,
    };

    /// The affinity Windows reports for `hwnd`.
    fn affinity(hwnd: HWND) -> u32 {
        let mut got = u32::MAX;
        unsafe { GetWindowDisplayAffinity(hwnd, &mut got) }.unwrap();
        got
    }

    #[test]
    fn a_window_of_our_own_is_excluded_from_capture_and_let_back_in() {
        // A popup of the test's own, far off every screen and never
        // activated: nothing is seen, nothing is captured
        // (docs/glim-learns-plan.md §10).
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("STATIC"),
                w!("glim-capture-test"),
                WS_POPUP | WS_VISIBLE,
                -30000,
                -30000,
                10,
                10,
                None,
                None,
                None,
                None,
            )
        }
        .expect("a test window");
        assert_eq!(affinity(hwnd), WDA_NONE.0);
        assert!(set_hwnd_capture_excluded(hwnd, true));
        assert_eq!(affinity(hwnd), WDA_EXCLUDEFROMCAPTURE.0);
        assert!(set_hwnd_capture_excluded(hwnd, false));
        assert_eq!(affinity(hwnd), WDA_NONE.0);
        let _ = unsafe { DestroyWindow(hwnd) };
    }

    /// A hidden window is left alone: on the hidden Settings window the call
    /// left a white copy of it on screen (2026-10-27).
    #[test]
    fn a_hidden_window_is_left_alone() {
        let hwnd = unsafe {
            CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!("glim-capture-test-hidden"), WS_POPUP, 0, 0, 10, 10, None, None, None, None)
        }
        .expect("a hidden test window");
        assert!(set_hwnd_capture_excluded(hwnd, true), "not a refusal");
        assert_eq!(affinity(hwnd), WDA_NONE.0, "untouched while hidden");
        let _ = unsafe { DestroyWindow(hwnd) };
    }
}
