// The chat: the conversation, the system prompt, and which local model server
// a turn goes to. The wire format lives in local_chat.rs (Ollama, LM Studio).
// There are no cloud providers and no API keys: the only servers are on this
// machine, and net/ refuses anything else.
//
// File bytes never cross the IPC boundary: the island sends the question and
// gets the answer's text back.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::settings::Settings;
use crate::local_chat;

/// The provider a fresh install talks to.
pub const DEFAULT_PROVIDER: &str = "ollama";

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ChatContext {
    File { name: String, path: String },
    Window { app_name: String, title: String, url: Option<String> },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatReply {
    pub text: String,
}

/// A model a provider offers, for the picker in the chat view.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
}

// ── Conversation ──────────────────────────────────────────────────────────────

/// The conversation is kept twice: in the wire format of the provider that
/// answered last, and as plain text turns. Switching provider mid-conversation
/// rebuilds the history from the plain turns, so nothing in one provider's
/// format is ever sent to another.
#[derive(Default)]
pub struct Chat {
    inner: Mutex<Conversation>,
}

#[derive(Default)]
struct Conversation {
    /// Bumped by every reset, so an answer that lands after "New chat" is dropped.
    epoch: u64,
    /// The provider `native` belongs to.
    owner: Option<String>,
    native: Vec<Value>,
    /// `{"role", "content": text}` turns, the same whoever answered.
    plain: Vec<Value>,
}

/// What a provider needs to build one turn.
pub struct Turn {
    epoch: u64,
    provider: String,
    /// No earlier turn: the file or window context rides along with this one.
    pub first: bool,
    /// The earlier turns, in this provider's format.
    pub history: Vec<Value>,
}

impl Chat {
    pub fn reset(&self) {
        let mut c = self.inner.lock().unwrap();
        let epoch = c.epoch + 1;
        *c = Conversation { epoch, ..Default::default() };
    }

    /// Starts a turn with `provider`, converting the history if another
    /// provider answered the previous turns.
    pub fn begin(&self, provider: &str) -> Turn {
        let mut c = self.inner.lock().unwrap();
        if c.owner.as_deref() != Some(provider) {
            c.native = c.plain.clone();
            c.owner = Some(provider.to_string());
        }
        Turn {
            epoch: c.epoch,
            provider: provider.to_string(),
            first: c.plain.is_empty(),
            history: c.native.clone(),
        }
    }

    /// Records a finished turn: the user message and the answer in the
    /// provider's format, and their plain text. Only a successful turn is
    /// recorded, so the history always matches what the model saw.
    pub fn commit(&self, turn: &Turn, user: Value, assistant: Value, user_text: &str, answer: &str) {
        let mut c = self.inner.lock().unwrap();
        if c.epoch != turn.epoch || c.owner.as_deref() != Some(turn.provider.as_str()) {
            return;
        }
        c.native.push(user);
        c.native.push(assistant);
        c.plain.push(json!({ "role": "user", "content": user_text }));
        c.plain.push(json!({ "role": "assistant", "content": answer }));
    }
}

// ── System prompt ─────────────────────────────────────────────────────────────

/// Glim's instructions. Greets the user by their first name when the account
/// has one worth using (identity.rs).
pub fn system_prompt() -> String {
    system_prompt_for(crate::identity::first_name())
}

fn system_prompt_for(first_name: Option<&str>) -> String {
    let opening = match first_name {
        Some(name) => format!("You are Glim, {name}'s personal AI assistant living at the top of their screen."),
        None => "You are Glim, a personal AI assistant living at the top of the user's screen.".to_string(),
    };
    format!(
        "{opening} You run entirely on this computer and have no web access: say so when something needs current information. \
Respond in the user's language. Be thorough and complete — use as much detail as the task requires. \
Use light Markdown when it helps: short paragraphs, bullet lists, **bold**, `inline code` and fenced code blocks. Avoid tables and big headings: the chat window is small."
    )
}

/// The window context line, as ClaudeService.chat() writes it.
pub fn window_line(app_name: &str, title: &str, url: Option<&str>) -> String {
    let mut text = format!("Context — App: {app_name}, Window: {title}");
    if let Some(url) = url {
        text.push_str(&format!(", URL: {url}"));
    }
    text
}

/// The plain-text record of what the user asked, context included.
pub fn plain_question(first: bool, context: Option<&ChatContext>, query: &str) -> String {
    match context.filter(|_| first) {
        Some(ChatContext::File { name, .. }) => format!("File: {name}\n\n{query}"),
        Some(ChatContext::Window { app_name, title, url }) => {
            format!("{}\n\n{query}", window_line(app_name, title, url.as_deref()))
        }
        None => query.to_string(),
    }
}

// ── Which provider ────────────────────────────────────────────────────────────

/// The model chosen for `provider`; empty until one is picked.
pub fn model_for(settings: &Settings, provider: &str) -> String {
    settings.chat_models.get(provider).map(|m| m.trim().to_string()).unwrap_or_default()
}

/// The provider in the settings, or the default when it is not a local server
/// (a settings.json from before the cloud providers were removed).
fn provider_of(settings: &Settings) -> &str {
    let p = settings.chat_provider.as_str();
    if local_chat::server(settings, p).is_some() { p } else { DEFAULT_PROVIDER }
}

/// A file rides along only if it is one of the app's own copies of a dropped
/// file (files.rs puts them in the inbox). The page names the path, so without
/// this any file the user can read could be sent to a chat provider.
fn checked_context(context: ChatContext) -> Result<ChatContext, String> {
    match context {
        ChatContext::File { name, path } => {
            let inbox = crate::files::inbox_dir();
            if !is_inside(&inbox, std::path::Path::new(&path)) {
                return Err(crate::i18n::t("Only a file dropped on the island can be sent with a question."));
            }
            Ok(ChatContext::File { name, path })
        }
        other => Ok(other),
    }
}

/// True when `path` is a regular file directly inside `dir`, both resolved
/// (no `..`, no symlink pointing out of it).
fn is_inside(dir: &std::path::Path, path: &std::path::Path) -> bool {
    let (Ok(dir), Ok(file)) = (dir.canonicalize(), path.canonicalize()) else { return false };
    file.parent() == Some(dir.as_path())
        && std::fs::symlink_metadata(&file).map(|m| m.is_file()).unwrap_or(false)
}

/// One chat turn with the local server chosen in the settings.
pub async fn send(
    app: &AppHandle,
    chat: &Chat,
    settings: &Settings,
    query: String,
    context: Option<ChatContext>,
) -> Result<ChatReply, String> {
    let context = context.map(checked_context).transpose()?;
    let provider = provider_of(settings);
    let model = model_for(settings, provider);
    let server = local_chat::server(settings, provider).ok_or_else(|| format!("Unknown chat provider: {provider}"))?;
    local_chat::send(app, chat, &server, &model, query, context).await
}

/// The models a local server offers. Asked only when the user opens the
/// picker on that server, and only once it has an address.
pub async fn models(settings: &Settings, provider: &str) -> Result<Vec<ModelInfo>, String> {
    match local_chat::server(settings, provider) {
        Some(server) => local_chat::models(&server).await,
        None => Err(format!("Unknown chat provider: {provider}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn_texts(history: &[Value]) -> Vec<(String, Value)> {
        history
            .iter()
            .map(|m| (m["role"].as_str().unwrap().to_string(), m["content"].clone()))
            .collect()
    }

    #[test]
    fn a_turn_is_recorded_only_once_it_succeeds() {
        let chat = Chat::default();
        let t = chat.begin("ollama");
        assert!(t.first);
        assert!(t.history.is_empty());
        // A failed turn records nothing: the next one is still the first.
        let t = chat.begin("ollama");
        assert!(t.first);
        chat.commit(&t, json!({"role":"user","content":[{"type":"text","text":"hi"}]}), json!({"role":"assistant","content":[{"type":"text","text":"hello"}]}), "hi", "hello");
        let t = chat.begin("ollama");
        assert!(!t.first);
        assert_eq!(t.history.len(), 2);
        assert!(t.history[0]["content"].is_array());
    }

    #[test]
    fn switching_provider_rebuilds_the_history_from_plain_turns() {
        let chat = Chat::default();
        let t = chat.begin("ollama");
        chat.commit(&t, json!({"role":"user","content":[{"type":"text","text":"look"}]}), json!({"role":"assistant","content":[{"type":"text","text":"Found it."}]}), "look", "Found it.");

        let t = chat.begin("lmstudio");
        assert!(!t.first);
        assert_eq!(
            turn_texts(&t.history),
            vec![("user".into(), json!("look")), ("assistant".into(), json!("Found it."))]
        );
    }

    #[test]
    fn an_answer_that_lands_after_a_reset_or_a_switch_is_dropped() {
        let chat = Chat::default();
        let t = chat.begin("lmstudio");
        chat.reset();
        chat.commit(&t, json!({"role":"user","content":"q"}), json!({"role":"assistant","content":"a"}), "q", "a");
        assert!(chat.begin("lmstudio").first);

        let t = chat.begin("lmstudio");
        let _other = chat.begin("ollama");
        chat.commit(&t, json!({"role":"user","content":"q"}), json!({"role":"assistant","content":"a"}), "q", "a");
        assert!(chat.begin("ollama").first);
    }

    #[test]
    fn the_prompt_greets_by_first_name_and_says_it_is_offline() {
        let p = system_prompt_for(Some("Ada"));
        assert!(p.starts_with("You are Glim, Ada's personal AI assistant living at the top of their screen."));
        assert!(p.contains("no web access"));
        assert!(p.contains("light Markdown"));
        let p = system_prompt_for(None);
        assert!(p.starts_with("You are Glim, a personal AI assistant living at the top of the user's screen."));
    }

    #[test]
    fn context_goes_with_the_first_question_only() {
        let file = ChatContext::File { name: "a.txt".into(), path: "/x/a.txt".into() };
        assert_eq!(plain_question(true, Some(&file), "why?"), "File: a.txt\n\nwhy?");
        assert_eq!(plain_question(false, Some(&file), "why?"), "why?");
        let win = ChatContext::Window { app_name: "Code".into(), title: "main.rs".into(), url: None };
        assert_eq!(plain_question(true, Some(&win), "q"), "Context — App: Code, Window: main.rs\n\nq");
        assert_eq!(window_line("Edge", "Docs", Some("https://x.dev")), "Context — App: Edge, Window: Docs, URL: https://x.dev");
    }

    #[test]
    fn the_model_comes_from_the_settings_and_only_local_servers_are_used() {
        let mut s = Settings::default();
        assert_eq!(model_for(&s, "ollama"), "");
        s.chat_models.insert("ollama".into(), " llama3.2 ".into());
        assert_eq!(model_for(&s, "ollama"), "llama3.2");
        assert_eq!(provider_of(&s), "ollama");
        // A cloud provider left in an old settings.json falls back to Ollama.
        for old in ["anthropic", "openai", "google", "openrouter", "custom", ""] {
            s.chat_provider = old.into();
            assert_eq!(provider_of(&s), "ollama", "{old}");
        }
        s.chat_provider = "lmstudio".into();
        assert_eq!(provider_of(&s), "lmstudio");
    }

    #[test]
    fn only_files_in_the_inbox_ride_along() {
        let base = std::env::temp_dir().join(format!("glim-chat-ctx-{}", std::process::id()));
        let inbox = base.join("inbox");
        std::fs::create_dir_all(inbox.join("sub")).unwrap();
        std::fs::write(inbox.join("a.txt"), b"a").unwrap();
        std::fs::write(base.join("secret.txt"), b"s").unwrap();
        std::fs::write(inbox.join("sub").join("b.txt"), b"b").unwrap();
        assert!(is_inside(&inbox, &inbox.join("a.txt")));
        assert!(!is_inside(&inbox, &base.join("secret.txt")));
        assert!(!is_inside(&inbox, &inbox.join("..").join("secret.txt")));
        assert!(!is_inside(&inbox, &inbox.join("sub").join("b.txt")));
        assert!(!is_inside(&inbox, &inbox.join("missing.txt")));
        assert!(!is_inside(&inbox, &inbox));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(base.join("secret.txt"), inbox.join("link.txt")).unwrap();
            assert!(!is_inside(&inbox, &inbox.join("link.txt")));
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}
