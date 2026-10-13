// 1c's rewrite, between the island and the API (docs/phase1-plan.md §13).
//
// The capture thread runs it (capture/uia.rs): a tone click arrives as
// `Request::Rewrite { tone }` (an id, nothing else), the paragraph comes from
// the writing Session, the checks below run, and the request goes out on the
// async runtime (claude.rs). The answer comes back to the capture thread,
// which records its cost (spend.rs) and tells the island on EVENT.
//
// The island hears phases: "sending" (the lantern goes to s-delegate),
// "done" (the rewrite, shown as a suggestion), "failed" (a code the island
// words), "result" (what "Use this" or "Copy" did: a writing::Outcome code).

use std::sync::RwLock;

use serde::Serialize;

use super::claude::{self, Failure, Reply};
use super::spend::{self, Spend};
use crate::writing::{FixMode, Outcome, RewriteShown};

/// The event the island hears the rewrite on.
pub const EVENT: &str = "writing-rewrite";

/// Settings → AI → Brain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Brain {
    Claude,
    /// Ollama on this machine: not built yet (§13.3).
    Local,
}

impl Brain {
    /// "local" is the local model; anything else is the Claude API.
    pub fn from_setting(s: &str) -> Brain {
        if s == "local" {
            Brain::Local
        } else {
            Brain::Claude
        }
    }
}

/// What the rewrite needs from the settings, kept here so the capture thread
/// can read it without the app's state (lib.rs sets it at launch and on
/// every save).
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub brain: Brain,
    /// A model Glim lists (claude::MODELS); anything else is the default.
    pub model: &'static str,
    pub cap_usd: f64,
}

impl Default for Config {
    fn default() -> Self {
        Config { brain: Brain::Claude, model: claude::DEFAULT_MODEL, cap_usd: 0.0 }
    }
}

impl Config {
    pub fn from_settings(s: &crate::settings::Settings) -> Config {
        Config {
            brain: Brain::from_setting(&s.ai_brain),
            model: claude::model(&s.ai_model).map_or(claude::DEFAULT_MODEL, |m| m.id),
            cap_usd: s.ai_monthly_cap_usd,
        }
    }
}

static CONFIG: RwLock<Option<Config>> = RwLock::new(None);

pub fn set_config(c: Config) {
    *CONFIG.write().unwrap_or_else(|e| e.into_inner()) = Some(c);
}

pub fn config() -> Config {
    CONFIG.read().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}

/// Why nothing is sent, before the paragraph is even looked at. The order is
/// the one the user can act on: Pause, the brain, the key, the cap.
pub fn precheck(paused: bool, config: &Config, key: &Result<bool, String>, spent: &Spend) -> Result<(), &'static str> {
    if paused {
        return Err("paused");
    }
    if config.brain == Brain::Local {
        return Err("local-brain");
    }
    match key {
        Err(_) => return Err("key-error"),
        Ok(false) => return Err("no-key"),
        Ok(true) => {}
    }
    if spend::at_cap(spent, config.cap_usd) {
        return Err("cap");
    }
    Ok(())
}

/// The failure code the island words, for a reply that has no rewrite.
pub fn failure_code(f: &Failure) -> String {
    match f {
        Failure::Api { status: 401, .. } => "api-key".into(),
        Failure::Api { status: 402, .. } => "api-billing".into(),
        Failure::Api { status: 403, .. } => "api-permission".into(),
        Failure::Api { status: 429, .. } => "api-rate".into(),
        Failure::Api { status: 529, .. } => "api-busy".into(),
        Failure::Api { status, .. } if *status >= 500 => "api-server".into(),
        Failure::Api { .. } => "api-other".into(),
        other => other.code(),
    }
}

/// What the island is told. Text only in "done", the rewrite itself, which
/// the island shows (like a suggestion's words); never logged.
#[derive(Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RewriteEvent {
    pub phase: &'static str,
    pub id: u64,
    pub tone: String,
    pub text: String,
    pub mode: FixMode,
    pub app: String,
    /// Glim would type it ("Use this"); otherwise Copy only.
    pub offerable: bool,
    /// The model's name ("Claude Haiku 5.5").
    pub model: String,
    pub cost_usd: f64,
    /// "failed": why (precheck codes, Failure codes, failure_code).
    pub code: String,
    /// "failed" on a 429: seconds to wait, when the API said.
    pub retry_after: Option<u64>,
    /// "result": a writing::Outcome code.
    pub outcome: String,
}

impl std::fmt::Debug for RewriteEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RewriteEvent").field("phase", &self.phase).field("id", &self.id).field("code", &self.code).field("outcome", &self.outcome).finish()
    }
}

fn model_name(id: &str) -> String {
    claude::model(id).map_or_else(|| id.to_string(), |m| m.name.to_string())
}

impl RewriteEvent {
    pub fn sending(id: u64, tone: claude::Tone, model: &str) -> Self {
        RewriteEvent { phase: "sending", id, tone: tone.id().into(), model: model_name(model), ..Default::default() }
    }

    pub fn failed(id: u64, code: impl Into<String>) -> Self {
        RewriteEvent { phase: "failed", id, code: code.into(), ..Default::default() }
    }

    /// The event for an answered request: the rewrite (when `shown`, i.e.
    /// still the one asked for) or why there is none.
    pub fn answered(id: u64, reply: &Reply, shown: Option<&RewriteShown>) -> Self {
        let model = reply.usage.as_ref().map(|u| model_name(&u.model)).unwrap_or_default();
        let cost_usd = reply.usage.as_ref().map_or(0.0, |u| u.cost_nanos as f64 / 1e9);
        match (&reply.result, shown) {
            (Ok(text), Some(s)) => RewriteEvent {
                phase: "done",
                id,
                text: text.clone(),
                mode: s.mode,
                app: s.app.clone(),
                offerable: s.offerable,
                model,
                cost_usd,
                ..Default::default()
            },
            (Ok(_), None) => RewriteEvent { phase: "stale", id, ..Default::default() },
            (Err(f), _) => {
                let retry_after = match f {
                    Failure::Api { retry_after, .. } => *retry_after,
                    _ => None,
                };
                RewriteEvent { phase: "failed", id, code: failure_code(f), retry_after, model, cost_usd, ..Default::default() }
            }
        }
    }

    pub fn result(id: u64, outcome: Outcome, app: String) -> Self {
        RewriteEvent { phase: "result", id, outcome: outcome.code().into(), app, ..Default::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::claude::Usage;

    fn spent(usd: f64) -> Spend {
        Spend { month: "2026-10".into(), nanos: (usd * 1e9) as u64, ..Spend::default() }
    }

    #[test]
    fn the_checks_run_in_the_order_the_user_can_act_on() {
        let claude = Config { cap_usd: 5.0, ..Config::default() };
        let local = Config { brain: Brain::Local, ..claude.clone() };
        assert_eq!(precheck(true, &local, &Ok(false), &spent(9.0)), Err("paused"));
        assert_eq!(precheck(false, &local, &Ok(false), &spent(9.0)), Err("local-brain"));
        assert_eq!(precheck(false, &claude, &Err("locked".into()), &spent(9.0)), Err("key-error"));
        assert_eq!(precheck(false, &claude, &Ok(false), &spent(9.0)), Err("no-key"));
        assert_eq!(precheck(false, &claude, &Ok(true), &spent(5.0)), Err("cap"));
        assert_eq!(precheck(false, &claude, &Ok(true), &spent(4.99)), Ok(()));
        // No cap: never stopped by spend.
        assert_eq!(precheck(false, &Config::default(), &Ok(true), &spent(1_000.0)), Ok(()));
    }

    #[test]
    fn the_settings_become_a_config_with_safe_defaults() {
        let mut s = crate::settings::Settings::default();
        assert_eq!(Config::from_settings(&s), Config::default());
        s.ai_brain = "local".into();
        s.ai_model = "claude-opus-5-5".into();
        s.ai_monthly_cap_usd = 3.0;
        assert_eq!(Config::from_settings(&s), Config { brain: Brain::Local, model: "claude-opus-5-5", cap_usd: 3.0 });
        s.ai_brain = "gpt".into();
        s.ai_model = "claude-unknown-9".into();
        let c = Config::from_settings(&s);
        assert_eq!((c.brain, c.model), (Brain::Claude, "claude-haiku-5-5"));
    }

    fn usage() -> Option<Usage> {
        Some(Usage { model: "claude-haiku-5-5".into(), input_tokens: 200, output_tokens: 150, cost_nanos: 95_000 })
    }

    #[test]
    fn the_island_gets_the_rewrite_or_a_code() {
        let shown = RewriteShown { mode: FixMode::Apply, app: "Word".into(), offerable: true };
        let ok = Reply { result: Ok("SENTINEL rewrite".into()), usage: usage() };
        let e = RewriteEvent::answered(7, &ok, Some(&shown));
        assert_eq!((e.phase, e.id, e.text.as_str(), e.model.as_str(), e.cost_usd), ("done", 7, "SENTINEL rewrite", "Claude Haiku 5.5", 0.000095));
        assert!(!format!("{e:?}").contains("SENTINEL"));
        // Dismissed meanwhile: the island only learns the request is over.
        let e = RewriteEvent::answered(7, &ok, None);
        assert_eq!((e.phase, e.text.as_str()), ("stale", ""));
        let busy = Reply { result: Err(Failure::Api { status: 529, kind: "overloaded_error".into(), retry_after: None }), usage: None };
        assert_eq!(RewriteEvent::answered(8, &busy, Some(&shown)).code, "api-busy");
        let rate = Reply { result: Err(Failure::Api { status: 429, kind: "rate_limit_error".into(), retry_after: Some(20) }), usage: None };
        let e = RewriteEvent::answered(9, &rate, None);
        assert_eq!((e.phase, e.code.as_str(), e.retry_after), ("failed", "api-rate", Some(20)));
        // A refusal is billed: its cost still shows.
        let refused = Reply { result: Err(Failure::Refused), usage: usage() };
        let e = RewriteEvent::answered(10, &refused, Some(&shown));
        assert_eq!((e.code.as_str(), e.cost_usd), ("refused", 0.000095));
        for (status, code) in [(401, "api-key"), (402, "api-billing"), (403, "api-permission"), (500, "api-server"), (504, "api-server"), (413, "api-other")] {
            assert_eq!(failure_code(&Failure::Api { status, kind: "x".into(), retry_after: None }), code);
        }
    }

    #[test]
    fn the_event_uses_the_islands_names() {
        let e = RewriteEvent::sending(3, claude::Tone::Formal, "claude-sonnet-5-5");
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["phase"], "sending");
        assert_eq!(v["tone"], "formal");
        assert_eq!(v["model"], "Claude Sonnet 5.5");
        for key in ["costUsd", "retryAfter", "offerable", "mode", "outcome"] {
            assert!(v.get(key).is_some(), "{key}");
        }
        let r = RewriteEvent::result(3, Outcome::Applied, "Word".into());
        assert_eq!((r.phase, r.outcome.as_str()), ("result", "applied"));
    }
}
