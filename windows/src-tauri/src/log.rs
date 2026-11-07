// Small append-only log at %LOCALAPPDATA%\Glim\glim.log (Windows) or
// ~/.local/share/coucou/glim.log (Linux) — the equivalent of nbLog() in
// HookServer.swift. Nothing leaves the machine.

use std::io::Write;

use crate::{platform, settings};

/// Past this size the log moves to glim.log.1 (the one before goes) and a
/// fresh one starts: a day or two of history is always kept. It used to be
/// deleted outright, which wiped the days a "live revisions stopped" report
/// needed (2026-11-07).
pub const MAX_BYTES: u64 = 1_000_000;

/// Moves `path` to `<path>.1` once it is over `max` bytes.
pub fn rotate(path: &std::path::Path, max: u64) {
    if std::fs::metadata(path).map(|m| m.len() > max).unwrap_or(false) {
        let mut old = path.as_os_str().to_owned();
        old.push(".1");
        let old = std::path::PathBuf::from(old);
        let _ = std::fs::remove_file(&old);
        if std::fs::rename(path, &old).is_err() {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub fn line(message: impl AsRef<str>) {
    let t = platform::local_time();
    let stamp = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    );
    let dir = settings::local_dir();
    if platform::ensure_private_dir(&dir).is_err() {
        return;
    }
    let path = dir.join("glim.log");
    rotate(&path, MAX_BYTES);
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    // Readable by us only, like the macOS log.
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    if let Ok(mut file) = options.open(path) {
        let _ = writeln!(file, "{stamp} {}", message.as_ref());
    }
}

#[cfg(test)]
mod tests {
    use super::rotate;

    #[test]
    fn a_full_log_moves_to_dot_one_and_the_one_before_goes() {
        let dir = std::env::temp_dir().join(format!("glim-log-rotate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("glim.log");
        let old = dir.join("glim.log.1");
        std::fs::write(&old, "oldest").unwrap();
        std::fs::write(&log, "0123456789").unwrap();
        rotate(&log, 100);
        assert!(log.exists() && std::fs::read_to_string(&old).unwrap() == "oldest", "under the size: nothing moves");
        rotate(&log, 5);
        assert!(!log.exists());
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "0123456789", "kept, not deleted");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
