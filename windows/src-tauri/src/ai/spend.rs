// This month's Claude API spend, and the optional monthly cap
// (docs/phase1-plan.md §13.5, docs/vision.md §2).
//
// Glim's own estimate: the token counts of each answer times the model's
// price (claude.rs). Anthropic's Console bill is the real number, and
// Settings says so. Kept in spend.json beside settings.json: numbers only,
// never text. A new month (local time) starts again at zero.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::claude::Usage;

/// What spend.json holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Spend {
    /// "2026-10", local time.
    pub month: String,
    pub nanos: u64,
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl Spend {
    /// This record as of `month`: a record of another month counts as zero.
    pub fn in_month(self, month: &str) -> Spend {
        if self.month == month {
            self
        } else {
            Spend { month: month.to_string(), ..Spend::default() }
        }
    }

    pub fn add(&mut self, u: &Usage) {
        self.nanos = self.nanos.saturating_add(u.cost_nanos);
        self.requests += 1;
        self.input_tokens = self.input_tokens.saturating_add(u.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(u.output_tokens);
    }
}

/// The current month, local time: "2026-10".
pub fn month_now() -> String {
    let t = crate::platform::local_time();
    format!("{:04}-{:02}", t.year, t.month)
}

/// The cap in nano-dollars, or None when there is none (0, negative, or not
/// a number in settings.json).
pub fn cap_nanos(cap_usd: f64) -> Option<u64> {
    (cap_usd.is_finite() && cap_usd > 0.0).then(|| (cap_usd * 1e9).round() as u64)
}

/// True when this month's spend has reached the cap: nothing more is sent.
pub fn at_cap(spend: &Spend, cap_usd: f64) -> bool {
    cap_nanos(cap_usd).is_some_and(|cap| spend.nanos >= cap)
}

/// What Settings and the island show.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendView {
    pub month: String,
    pub usd: f64,
    pub requests: u64,
    /// 0 when there is no cap.
    pub cap_usd: f64,
    pub at_cap: bool,
}

pub fn view(spend: &Spend, cap_usd: f64) -> SpendView {
    SpendView {
        month: spend.month.clone(),
        usd: spend.nanos as f64 / 1e9,
        requests: spend.requests,
        cap_usd: cap_nanos(cap_usd).map_or(0.0, |_| cap_usd),
        at_cap: at_cap(spend, cap_usd),
    }
}

pub fn path() -> PathBuf {
    crate::platform::config_dir().join("spend.json")
}

/// The record at `path`; a missing or unreadable file counts as nothing spent.
pub fn load_from(path: &Path) -> Spend {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Writes beside the file and renames over it, so a crash never leaves half
/// a record.
pub fn save_to(path: &Path, spend: &Spend) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        crate::platform::ensure_private_dir(dir)?;
    }
    let json = serde_json::to_vec_pretty(spend).map_err(std::io::Error::other)?;
    let temp = path.with_extension(format!("json.glim-{}", std::process::id()));
    let written = std::fs::write(&temp, json).and_then(|()| std::fs::rename(&temp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    written
}

/// One read-add-write at a time within the app.
static LOCK: Mutex<()> = Mutex::new(());

/// This month's spend at `path`.
pub fn current(path: &Path, month: &str) -> Spend {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    load_from(path).in_month(month)
}

/// Adds one answered request to `month`'s spend at `path`, and returns the
/// new total.
pub fn record(path: &Path, month: &str, usage: &Usage) -> std::io::Result<Spend> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut spend = load_from(path).in_month(month);
    spend.add(usage);
    save_to(path, &spend)?;
    Ok(spend)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(cost_nanos: u64) -> Usage {
        Usage { model: "claude-haiku-5-5".into(), input_tokens: 200, output_tokens: 180, cost_nanos }
    }

    /// A fresh file path in the system temp folder, never Glim's own.
    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("glim-spend-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("spend.json")
    }

    #[test]
    fn requests_add_up_within_a_month_and_a_new_month_starts_at_zero() {
        let p = temp_file("months");
        assert_eq!(current(&p, "2026-10"), Spend { month: "2026-10".into(), ..Spend::default() });
        record(&p, "2026-10", &usage(114_700)).unwrap();
        let s = record(&p, "2026-10", &usage(100_000)).unwrap();
        assert_eq!((s.nanos, s.requests, s.input_tokens, s.output_tokens), (214_700, 2, 400, 360));
        assert_eq!(current(&p, "2026-10"), s);
        // November: nothing spent yet, and the next request starts the count again.
        assert_eq!(current(&p, "2026-11").nanos, 0);
        let s = record(&p, "2026-11", &usage(5)).unwrap();
        assert_eq!((s.month.as_str(), s.nanos, s.requests), ("2026-11", 5, 1));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn the_file_holds_numbers_only() {
        let p = temp_file("numbers");
        record(&p, "2026-10", &usage(42)).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["month", "nanos", "requests", "inputTokens", "outputTokens"]);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn a_damaged_or_missing_file_counts_as_nothing_spent() {
        let p = temp_file("damaged");
        assert_eq!(load_from(&p), Spend::default());
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"{not json").unwrap();
        assert_eq!(load_from(&p), Spend::default());
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn the_cap_stops_requests_once_reached() {
        let spend = |usd: f64| Spend { month: "2026-10".into(), nanos: (usd * 1e9) as u64, ..Spend::default() };
        // No cap: 0, negative or not a number.
        for cap in [0.0, -5.0, f64::NAN, f64::INFINITY] {
            assert!(!at_cap(&spend(1_000.0), cap), "{cap}");
            assert_eq!(view(&spend(1.0), cap).cap_usd, 0.0);
        }
        assert!(!at_cap(&spend(4.99), 5.0));
        assert!(at_cap(&spend(5.0), 5.0));
        assert!(at_cap(&spend(5.01), 5.0));
        let v = view(&spend(5.0), 5.0);
        assert_eq!((v.usd, v.cap_usd, v.at_cap), (5.0, 5.0, true));
    }

    #[test]
    fn the_month_is_year_dash_month() {
        let m = month_now();
        assert_eq!(m.len(), 7, "{m}");
        assert_eq!(&m[4..5], "-");
        assert!(m[..4].parse::<u32>().unwrap() >= 2026 && (1..=12).contains(&m[5..].parse::<u32>().unwrap()));
    }
}
