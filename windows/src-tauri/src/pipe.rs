// Relay server for glim-hook.
//
// Windows: the named pipe `\\.\pipe\glim-<sid>`, one instance per connection.
// Linux: the Unix socket `$XDG_RUNTIME_DIR/coucou.sock`. Every hook event is
// forwarded to the island as a `hook` event. `PermissionRequest` is the only one
// that keeps its connection open: it waits for the island's decision and writes
// it back on the same connection, which is how approving from the island works.
//
// Claude Code is never blocked by us. Three things guarantee it:
//   * glim-hook gives the connection 300 ms and exits cleanly if we are closed;
//   * we only wait for a human once the island has *confirmed* the card is on
//     screen, so a paused island or a webview that is not listening costs a few
//     hundred milliseconds, not two minutes;
//   * whatever happens we drop the connection after the decision timeout, and
//     the terminal takes over.
//
// Answering in the terminal instead kills glim-hook (on "No" Claude Code
// aborts the hook with the turn, and sends no Stop): we notice the connection
// close and tell the island (`approval-gone`), so the card never outlives it.
//
// What we write back is the bare word `allow` or `deny` — or, when the request
// was Claude Code asking a question, `{"answers":{…}}` with what was picked on
// the island. Turning either into the documented hookSpecificOutput JSON is
// glim-hook's job, so the wire format Claude Code expects lives in exactly
// one place.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::task::Poll;
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
#[cfg(windows)]
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::sync::mpsc;

use crate::island::WINDOW_LABEL;
use crate::log;
use crate::session_window;

/// Slightly under glim-hook's own 110 s wait, so we always answer first.
const DECISION_TIMEOUT: Duration = Duration::from_secs(108);
/// How long the island gets to say "the card is up". This is the whole of B4:
/// without it, an island that is paused, hidden behind a crashed webview or
/// simply not listening would leave Claude Code staring at a prompt nobody can
/// see for nearly two minutes.
const ACK_TIMEOUT: Duration = Duration::from_millis(800);
const MAX_PAYLOAD: usize = 1 << 20;

/// What the island can say about a permission request.
pub enum Reply {
    /// The card is on screen and a human can act on it.
    Ack,
    /// A human clicked: `allow` or `deny`.
    Decision(String),
    /// Nobody can act on it — paused, or another request already holds the card.
    Decline,
}

/// Permission requests the island has been told about.
#[derive(Default)]
pub struct Pending(pub Mutex<HashMap<String, mpsc::Sender<Reply>>>);

static COUNTER: AtomicU64 = AtomicU64::new(1);

/// `\\.\pipe\glim-<sid>` — must match glim-hook's `pipe_path()` exactly.
#[cfg(windows)]
pub fn pipe_name() -> String {
    let key = crate::platform::current_user_sid()
        .unwrap_or_else(|| std::env::var("USERNAME").unwrap_or_else(|_| "user".into()));
    format!(r"\\.\pipe\glim-{key}")
}

#[cfg(windows)]
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let name = pipe_name();
        // first_pipe_instance also means we refuse to join a pipe somebody else
        // already owns under our name, rather than serving on top of it.
        let mut server = match ServerOptions::new().first_pipe_instance(true).create(&name) {
            Ok(s) => s,
            Err(err) => {
                log::line(format!("cannot open the relay pipe: {err}"));
                return;
            }
        };
        loop {
            if server.connect().await.is_err() {
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
            // Hand the connected instance to a task and listen on a fresh one.
            let next = match ServerOptions::new().create(&name) {
                Ok(s) => s,
                Err(err) => {
                    log::line(format!("cannot reopen the relay pipe: {err}"));
                    return;
                }
            };
            let connected = std::mem::replace(&mut server, next);
            let app = app.clone();
            tauri::async_runtime::spawn(async move { handle(app, connected).await });
        }
    });
}

#[cfg(target_os = "linux")]
pub fn start(app: AppHandle) {
    use std::os::unix::fs::PermissionsExt;
    use tokio::net::UnixListener;

    tauri::async_runtime::spawn(async move {
        let Some(path) = crate::platform::relay_socket_path() else {
            log::line("no private runtime directory ($XDG_RUNTIME_DIR) — Claude Code hooks are inactive");
            return;
        };
        // A socket file left behind by a crash answers nothing and can go. One
        // that answers belongs to a Coucou that is still running: like
        // first_pipe_instance on Windows, we refuse to serve on top of it.
        if path.exists() {
            if std::os::unix::net::UnixStream::connect(&path).is_ok() {
                log::line("another Glim already serves the relay socket");
                return;
            }
            let _ = std::fs::remove_file(&path);
        }
        let listener = match UnixListener::bind(&path) {
            Ok(l) => l,
            Err(err) => {
                log::line(format!("cannot open the relay socket: {err}"));
                return;
            }
        };
        // The runtime directory is already 0700; this is belt and braces.
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        let uid = unsafe { libc::getuid() };
        loop {
            let stream = match listener.accept().await {
                Ok((stream, _)) => stream,
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    continue;
                }
            };
            // Only the relay run by our own user may drive the island.
            if !matches!(stream.peer_cred(), Ok(c) if c.uid() == uid) {
                log::line("refused a relay connection from another user");
                continue;
            }
            let app = app.clone();
            tauri::async_runtime::spawn(async move { handle(app, stream).await });
        }
    });
}

/// One accepted relay connection, whatever carries it.
trait Relay: AsyncRead + AsyncWrite + Unpin {
    /// Ends the conversation once everything has been written.
    fn finish(&mut self) {}
    /// The relay process on the other end, where the OS says.
    fn client_pid(&self) -> Option<u32> {
        None
    }
}

#[cfg(windows)]
impl Relay for NamedPipeServer {
    fn finish(&mut self) {
        let _ = self.disconnect();
    }
    fn client_pid(&self) -> Option<u32> {
        use std::os::windows::io::AsRawHandle;
        crate::platform::pipe_client_pid(self.as_raw_handle())
    }
}

/// Dropping the stream closes it; the relay reads up to our newline first.
#[cfg(target_os = "linux")]
impl Relay for tokio::net::UnixStream {}

async fn handle(app: AppHandle, mut pipe: impl Relay) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match pipe.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.contains(&b'\n') || buf.len() > MAX_PAYLOAD {
                    break;
                }
            }
            Err(_) => return,
        }
    }
    let line = match buf.iter().position(|b| *b == b'\n') {
        Some(i) => &buf[..i],
        None => &buf[..],
    };
    let Ok(mut payload) = serde_json::from_slice::<Value>(line) else { return };
    if !payload.is_object() {
        return;
    }

    let event = payload
        .get("hook_event_name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    note_session_window(&pipe, &payload, &event);
    // Counts for the weekly recap — never the command, path or prompt itself.
    crate::recap::observe(&app, &payload);

    if event != "PermissionRequest" {
        // The status line relay calls in with every Claude Code update: not log-worthy.
        if event != "StatusLine" {
            log::line(format!("hook {event}"));
        }
        let _ = app.emit_to(WINDOW_LABEL, "hook", payload);
        pipe.finish();
        return;
    }

    let id = format!("{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
    let (tx, mut rx) = mpsc::channel::<Reply>(4);
    {
        let pending = app.state::<Pending>();
        pending.0.lock().unwrap().insert(id.clone(), tx);
    }
    crate::recap::note_request(&app, &id, &payload);
    payload["request_id"] = json!(id);
    log::line(format!("hook PermissionRequest id={id}"));
    let _ = app.emit_to(WINDOW_LABEL, "hook", payload);

    let waited = wait_for_decision(&id, &mut rx, until_closed(&mut pipe)).await;
    app.state::<Pending>().0.lock().unwrap().remove(&id);

    // No decision: say nothing at all. glim-hook then writes nothing to stdout
    // and Claude Code asks in the terminal, exactly as if Coucou were closed.
    match waited {
        Waited::Answered(d) => {
            let _ = pipe.write_all(format!("{d}\n").as_bytes()).await;
            let _ = pipe.flush().await;
        }
        Waited::Gone => {
            let _ = app.emit_to(WINDOW_LABEL, "approval-gone", json!({ "request_id": id }));
        }
        Waited::NoDecision => {}
    }
    pipe.finish();
}

/// Finds, once per session, the window it runs in — see session_window.rs.
///
/// Only while the session is unknown, so the process snapshot is not taken on
/// every event. The relay must still be running for its parents to be found:
/// a permission request always is (it waits for us), a quick event may already
/// have exited, and then a later event of the session tries again.
fn note_session_window(pipe: &impl Relay, payload: &Value, event: &str) {
    let Some(session) = payload.get("session_id").and_then(Value::as_str) else { return };
    if event == "SessionEnd" {
        session_window::forget(session);
        return;
    }
    if session_window::known(session) {
        return;
    }
    let Some(relay) = pipe.client_pid() else { return };
    let ancestors = crate::platform::process_ancestors(relay);
    // No ancestors: the relay was already gone, so try again next time. Some,
    // but none with a window (a classic console): settled, VS Code it is.
    if !ancestors.is_empty() {
        let owner = crate::platform::first_with_window(&ancestors);
        session_window::remember(session, owner.unwrap_or(session_window::NO_WINDOW));
    }
}

/// How a permission request's wait ended.
#[derive(Debug, PartialEq)]
enum Waited {
    /// The island's answer, to write back.
    Answered(String),
    /// No answer from Glim: the terminal asks.
    NoDecision,
    /// glim-hook went away first: answered in the terminal.
    Gone,
}

/// Resolves when the relay on the other end closes the connection: glim-hook
/// exited or was killed. It writes nothing after its one line, so anything
/// read here is ignored.
async fn until_closed(pipe: &mut (impl AsyncRead + Unpin)) {
    let mut chunk = [0u8; 256];
    while let Ok(n) = pipe.read(&mut chunk).await {
        if n == 0 {
            return;
        }
    }
}

/// The island's next reply; `None` once the relay has gone. A reply that is
/// already there wins over a relay that has just closed.
async fn recv_unless_gone(rx: &mut mpsc::Receiver<Reply>, mut gone: Pin<&mut impl Future<Output = ()>>) -> Option<Option<Reply>> {
    std::future::poll_fn(|cx| {
        if let Poll::Ready(reply) = rx.poll_recv(cx) {
            return Poll::Ready(Some(reply));
        }
        if gone.as_mut().poll(cx).is_ready() {
            return Poll::Ready(None);
        }
        Poll::Pending
    })
    .await
}

/// Two waits: a short one for "the card is up", then the long one for a human.
/// Either ends early if the relay goes (`gone`).
async fn wait_for_decision(id: &str, rx: &mut mpsc::Receiver<Reply>, gone: impl Future<Output = ()>) -> Waited {
    let mut gone = std::pin::pin!(gone);
    match tokio::time::timeout(ACK_TIMEOUT, recv_unless_gone(rx, gone.as_mut())).await {
        Ok(Some(Some(Reply::Ack))) => {}
        // A click that beats the ack is still a click.
        Ok(Some(Some(Reply::Decision(d)))) => {
            log::line(format!("hook id={id} answered {}", loggable(&d)));
            return Waited::Answered(d);
        }
        Ok(Some(Some(Reply::Decline))) => {
            log::line(format!("hook id={id} not shown — terminal takes over"));
            return Waited::NoDecision;
        }
        Ok(Some(None)) => return Waited::NoDecision,
        Ok(None) => {
            log::line(format!("hook id={id} relay gone — answered in the terminal"));
            return Waited::Gone;
        }
        Err(_) => {
            log::line(format!("hook id={id} island never acknowledged — terminal takes over"));
            return Waited::NoDecision;
        }
    }

    match tokio::time::timeout(DECISION_TIMEOUT, recv_unless_gone(rx, gone.as_mut())).await {
        Ok(Some(Some(Reply::Decision(d)))) => {
            log::line(format!("hook id={id} answered {}", loggable(&d)));
            Waited::Answered(d)
        }
        Ok(Some(Some(Reply::Decline))) => {
            log::line(format!("hook id={id} released without a decision"));
            Waited::NoDecision
        }
        Ok(None) => {
            log::line(format!("hook id={id} relay gone — answered in the terminal"));
            Waited::Gone
        }
        _ => {
            log::line(format!("hook id={id} timed out — terminal takes over"));
            Waited::NoDecision
        }
    }
}

/// The log says a question was answered, never with what.
fn loggable(decision: &str) -> &str {
    if decision.starts_with('{') { "a question" } else { decision }
}

fn send(app: &AppHandle, request_id: &str, reply: Reply, keep: bool) {
    let sender = {
        let pending = app.state::<Pending>();
        let mut map = pending.0.lock().unwrap();
        if keep { map.get(request_id).cloned() } else { map.remove(request_id) }
    };
    match sender {
        Some(tx) => {
            let _ = tx.try_send(reply);
        }
        None => log::line(format!("reply for id={request_id} — no pending request")),
    }
}

/// The island has the card on screen; the long wait may begin.
pub fn acknowledge(app: &AppHandle, request_id: &str) {
    send(app, request_id, Reply::Ack, true);
}

/// Nobody can act on this one — paused, or another card already holds the view.
pub fn decline(app: &AppHandle, request_id: &str) {
    log::line(format!("decline id={request_id}"));
    send(app, request_id, Reply::Decline, false);
}

/// Called by the island's Allow / Deny buttons. Only ever a bare word: turning
/// it into Claude Code's JSON is glim-hook's job.
pub fn answer(app: &AppHandle, request_id: &str, decision: &str) {
    let word = match decision {
        "allow" | "always" => "allow",
        _ => "deny",
    };
    log::line(format!("decision id={request_id} {word}"));
    send(app, request_id, Reply::Decision(word.to_string()), false);
}

/// Called when an option is picked for a question Claude Code asked. `answers`
/// maps each question's text to the chosen label, which is the shape
/// AskUserQuestion takes them in.
pub fn answer_question(app: &AppHandle, request_id: &str, answers: &HashMap<String, serde_json::Value>) {
    log::line(format!("decision id={request_id} answered a question"));
    // One line: the relay reads up to the first newline.
    let line = json!({ "answers": answers }).to_string();
    send(app, request_id, Reply::Decision(line), false);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block_on<T>(f: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
    }

    // Bug 1 (owner's retest 2026-10-18): "No" in the terminal kills glim-hook
    // and the card stayed up. Its connection closing ends the wait.
    #[test]
    fn a_relay_that_goes_ends_the_wait() {
        block_on(async {
            let (tx, mut rx) = mpsc::channel(4);
            tx.try_send(Reply::Ack).unwrap();
            assert_eq!(wait_for_decision("t-1", &mut rx, async {}).await, Waited::Gone, "on screen, then gone");
            let (_tx, mut rx) = mpsc::channel(4);
            assert_eq!(wait_for_decision("t-2", &mut rx, async {}).await, Waited::Gone, "gone before the ack");
        });
    }

    #[test]
    fn an_answer_already_given_wins_over_a_closing_relay() {
        block_on(async {
            let (tx, mut rx) = mpsc::channel(4);
            tx.try_send(Reply::Ack).unwrap();
            tx.try_send(Reply::Decision("allow".into())).unwrap();
            assert_eq!(wait_for_decision("t-3", &mut rx, async {}).await, Waited::Answered("allow".into()));
            // A relay that stays: as before.
            let (tx, mut rx) = mpsc::channel(4);
            tx.try_send(Reply::Ack).unwrap();
            tx.try_send(Reply::Decline).unwrap();
            assert_eq!(wait_for_decision("t-4", &mut rx, std::future::pending()).await, Waited::NoDecision);
        });
    }

    #[test]
    fn until_closed_waits_for_the_other_end_to_close() {
        block_on(async {
            let (mut ours, mut theirs) = tokio::io::duplex(1024);
            // Still connected, even writing: not closed.
            theirs.write_all(b"stray").await.unwrap();
            let open = tokio::time::timeout(Duration::from_millis(50), until_closed(&mut ours)).await;
            assert!(open.is_err(), "the relay is still there");
            drop(theirs);
            tokio::time::timeout(Duration::from_secs(1), until_closed(&mut ours))
                .await
                .expect("closed: the wait ends");
        });
    }

    // The real transport: glim-hook opens the pipe as a file, and its process
    // dying closes that handle.
    #[cfg(windows)]
    #[test]
    fn a_named_pipe_client_closing_is_noticed() {
        block_on(async {
            let name = format!(r"\\.\pipe\glim-test-gone-{}", std::process::id());
            let mut server = ServerOptions::new().first_pipe_instance(true).create(&name).unwrap();
            let client = std::fs::OpenOptions::new().read(true).write(true).open(&name).unwrap();
            server.connect().await.unwrap();
            let open = tokio::time::timeout(Duration::from_millis(50), until_closed(&mut server)).await;
            assert!(open.is_err(), "the client is still there");
            drop(client);
            tokio::time::timeout(Duration::from_secs(1), until_closed(&mut server))
                .await
                .expect("the client closed: the wait ends");
        });
    }
}
