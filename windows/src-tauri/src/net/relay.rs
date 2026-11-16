// The courier relay's door (owner 2026-11-14, docs/remote-courier.md): the
// one WebSocket Glim opens, and the only code that uses the WebSocket crate.
//
// Allowed: `wss://<name>.<subdomain>.workers.dev/v1/pc` on port 443, the
// host the owner typed in Settings → Remote → Remote tasks, and only while
// remote tasks are on, consented and not stopped (`set_relay_allowed`, kept
// in step by remote::courier). Nothing else, not even this computer (tests
// alone may use ws://127.0.0.1). The PC's relay token goes in the
// Authorization header of the upgrade request, never in the URL or a log.
// Certificates are checked against the webpki roots with rustls (ring), the
// TLS stack reqwest already uses.
//
// `session` runs one connection: it hands each text frame to the caller,
// sends what the caller queued, pings every 30 s (the relay answers without
// waking up) and gives up when nothing at all came back for 60 s (a
// half-open line after sleep or a network change sends no close frame), the
// socket closes, the wall clock jumped (the PC slept), or `kick` was called
// (Windows said it resumed or the network changed, remote/wake.rs): the
// caller reconnects at once and the relay hands over whatever is still
// pending (owner 2026-11-15).

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::Url;

pub type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// The relay's path for the PC.
pub const PC_PATH: &str = "/v1/pc";
/// The biggest frame read (a task envelope is at most 16 KB).
pub const MAX_FRAME: usize = 64 * 1024;
pub const PING: &str = r#"{"t":"ping"}"#;
pub const PONG: &str = r#"{"t":"pong"}"#;

static RELAY_ON: AtomicBool = AtomicBool::new(false);

/// Bumped by `kick`: a session that saw another number ends, and a courier
/// waiting out its backoff stops waiting. Per thread in tests (each test's
/// courier runs on its own thread), so one test's kick can't drop another's
/// line.
#[cfg(not(test))]
static KICKS: AtomicU64 = AtomicU64::new(0);
#[cfg(not(test))]
static KICK_WHY: AtomicU8 = AtomicU8::new(0);
#[cfg(test)]
thread_local! {
    static KICKS: AtomicU64 = const { AtomicU64::new(0) };
    static KICK_WHY: AtomicU8 = const { AtomicU8::new(0) };
}

#[cfg(not(test))]
fn with_kicks<R>(f: impl FnOnce(&AtomicU64, &AtomicU8) -> R) -> R {
    f(&KICKS, &KICK_WHY)
}

#[cfg(test)]
fn with_kicks<R>(f: impl FnOnce(&AtomicU64, &AtomicU8) -> R) -> R {
    KICKS.with(|k| KICK_WHY.with(|w| f(k, w)))
}

/// Why the line was kicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kick {
    /// Windows resumed from sleep.
    Resumed,
    /// An IP address changed (Wi-Fi, VPN, cable).
    Network,
}

impl Kick {
    pub fn code(self) -> &'static str {
        match self {
            Kick::Resumed => "resumed",
            Kick::Network => "network-changed",
        }
    }
}

/// Drop the current connection and reconnect now (remote/wake.rs).
pub fn kick(why: Kick) {
    with_kicks(|k, w| {
        w.store(why as u8, Ordering::SeqCst);
        k.fetch_add(1, Ordering::SeqCst);
    });
}

/// The kick count now (compare with a later one).
pub fn kicks() -> u64 {
    with_kicks(|k, _| k.load(Ordering::SeqCst))
}

fn last_kick() -> Kick {
    if with_kicks(|_, w| w.load(Ordering::SeqCst)) == Kick::Network as u8 { Kick::Network } else { Kick::Resumed }
}

/// Remote tasks on + consented + not stopped (remote::courier keeps it).
pub fn set_relay_allowed(on: bool) {
    RELAY_ON.store(on, Ordering::SeqCst);
}

pub fn relay_allowed() -> bool {
    RELAY_ON.load(Ordering::SeqCst)
}

fn host_ok(host: &str) -> bool {
    let Some(stem) = host.strip_suffix(".workers.dev") else { return false };
    let labels: Vec<&str> = stem.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|l| !l.is_empty() && l.len() <= 63 && !l.starts_with('-') && !l.ends_with('-') && l.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
}

/// The relay address as typed ("https://task-relay.example.workers.dev",
/// with or without a path): the PC's WebSocket URL, or why not.
pub fn endpoint(raw: &str) -> Result<Url, &'static str> {
    let raw = raw.trim().to_ascii_lowercase();
    let url = Url::parse(&raw).map_err(|_| "not-an-address")?;
    if !matches!(url.scheme(), "https" | "wss") || !url.username().is_empty() || url.password().is_some() || url.port().is_some_and(|p| p != 443) {
        return Err("not-a-relay");
    }
    let host = url.host_str().unwrap_or_default();
    if !host_ok(host) {
        return Err("not-a-relay");
    }
    Url::parse(&format!("wss://{host}{PC_PATH}")).map_err(|_| "not-a-relay")
}

/// Every connection passes here: allowed now, and exactly the relay's shape.
pub fn check(url: &Url) -> Result<(), &'static str> {
    // Tests' stand-in relay (testing::server) only; never in a build.
    #[cfg(test)]
    if url.scheme() == "ws" && url.host_str() == Some("127.0.0.1") && url.path() == PC_PATH {
        return Ok(());
    }
    if !relay_allowed() {
        return Err("remote-off");
    }
    let ok = url.scheme() == "wss"
        && url.host_str().is_some_and(host_ok)
        && url.port().is_none()
        && url.path() == PC_PATH
        && url.query().is_none()
        && url.username().is_empty()
        && url.password().is_none();
    if ok { Ok(()) } else { Err("not-a-relay") }
}

fn tls() -> Result<Arc<rustls::ClientConfig>, &'static str> {
    let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|_| "tls")?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// Opens the relay connection with the PC token. Errors are fixed codes.
pub async fn connect(url: &Url, token: &str) -> Result<Socket, &'static str> {
    check(url)?;
    let mut req = url.as_str().into_client_request().map_err(|_| "not-a-relay")?;
    let auth = format!("Bearer {token}").parse().map_err(|_| "bad-token")?;
    req.headers_mut().insert("authorization", auth);
    let connector = if url.scheme() == "wss" { Some(tokio_tungstenite::Connector::Rustls(tls()?)) } else { None };
    let config = WebSocketConfig::default().max_message_size(Some(MAX_FRAME)).max_frame_size(Some(MAX_FRAME));
    match tokio::time::timeout(Duration::from_secs(15), tokio_tungstenite::connect_async_tls_with_config(req, Some(config), true, connector)).await {
        Err(_) => Err("timeout"),
        Ok(Err(tokio_tungstenite::tungstenite::Error::Http(r))) if r.status().as_u16() == 401 || r.status().as_u16() == 403 => Err("unauthorized"),
        Ok(Err(_)) => Err("unreachable"),
        Ok(Ok((ws, _))) => Ok(ws),
    }
}

/// How a session keeps time (short in tests).
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub tick: Duration,
    pub ping_every: Duration,
    /// Nothing heard for this long (no pong, no frame): the line is dead.
    pub silent_for: Duration,
    /// A wall-clock jump this much beyond the time that passed = woke up.
    pub sleep_jump: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing { tick: Duration::from_millis(250), ping_every: Duration::from_secs(30), silent_for: Duration::from_secs(60), sleep_jump: Duration::from_secs(15) }
    }
}

/// True when the PC was asleep between two checks: the wall clock moved
/// much more than the loop's own clock, or the loop itself ran far too late.
pub fn woke_from_sleep(wall_before: SystemTime, wall_now: SystemTime, mono_elapsed: Duration, t: &Timing) -> bool {
    let wall = wall_now.duration_since(wall_before).unwrap_or_default();
    wall > mono_elapsed + t.sleep_jump || mono_elapsed > t.tick + t.ping_every + t.sleep_jump
}

/// Why a session ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// The caller said stop (remote off or stopped): don't reconnect.
    Stopped,
    /// The connection is gone (closed, silent, slept, kicked): reconnect.
    Lost(&'static str),
}

/// What a session works with. Frames the relay didn't get before the line
/// dropped go back to `unsent`, sent first next time.
pub struct Io<'a> {
    pub outbox: &'a mut tokio::sync::mpsc::UnboundedReceiver<String>,
    pub unsent: &'a mut Vec<String>,
    pub on_frame: &'a mut dyn FnMut(&str),
    pub keep_going: &'a dyn Fn() -> bool,
}

pub async fn session(mut ws: Socket, io: Io<'_>, t: Timing) -> End {
    let Io { outbox, unsent, on_frame, keep_going } = io;
    let mut queue: Vec<String> = std::mem::take(unsent);
    let mut last_ping = Instant::now();
    let mut heard = Instant::now();
    let kicked_at = kicks();
    let mut wall = SystemTime::now();
    let mut mono = Instant::now();
    loop {
        while let Ok(f) = outbox.try_recv() {
            queue.push(f);
        }
        while !queue.is_empty() {
            let f = queue.remove(0);
            if ws.send(Message::Text(f.clone().into())).await.is_err() {
                queue.insert(0, f);
                *unsent = queue;
                return End::Lost("send");
            }
        }
        match tokio::time::timeout(t.tick, ws.next()).await {
            Err(_) => {}
            Ok(None) | Ok(Some(Err(_))) | Ok(Some(Ok(Message::Close(_)))) => return End::Lost("closed"),
            Ok(Some(Ok(Message::Text(text)))) => {
                heard = Instant::now();
                if text.as_str() != PONG {
                    on_frame(text.as_str());
                }
            }
            Ok(Some(Ok(_))) => heard = Instant::now(),
        }
        if !keep_going() {
            let _ = ws.close(None).await;
            return End::Stopped;
        }
        if kicks() != kicked_at {
            return End::Lost(last_kick().code());
        }
        let (wall_now, mono_elapsed) = (SystemTime::now(), mono.elapsed());
        if woke_from_sleep(wall, wall_now, mono_elapsed, &t) {
            return End::Lost("slept");
        }
        wall = wall_now;
        mono = Instant::now();
        if heard.elapsed() > t.silent_for {
            return End::Lost("silent");
        }
        if last_ping.elapsed() >= t.ping_every {
            if ws.send(Message::Text(PING.into())).await.is_err() {
                return End::Lost("send");
            }
            last_ping = Instant::now();
        }
    }
}

/// A stand-in relay on 127.0.0.1 for the courier's tests (remote/tests.rs):
/// each accepted connection's Authorization header, the frames it sent, and
/// a way to send it frames or drop it without a goodbye.
#[cfg(test)]
pub mod testing {
    use super::*;
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

    pub struct Server {
        listener: TcpListener,
        pub url: String,
    }

    pub struct Conn {
        ws: WebSocketStream<TcpStream>,
        pub auth: String,
        /// Answer the client's pings (off = a relay gone quiet).
        pub answer_pings: bool,
    }

    pub async fn server() -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        Server { listener, url: format!("ws://127.0.0.1:{port}{PC_PATH}") }
    }

    impl Server {
        pub async fn accept(&self) -> Conn {
            let (tcp, _) = self.listener.accept().await.unwrap();
            let mut auth = String::new();
            let ws = tokio_tungstenite::accept_hdr_async(tcp, |req: &Request, resp: Response| {
                auth = req.headers().get("authorization").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
                Ok(resp)
            })
            .await
            .unwrap();
            Conn { ws, auth, answer_pings: true }
        }
    }

    impl Conn {
        pub async fn send(&mut self, text: &str) {
            self.ws.send(Message::Text(text.to_string().into())).await.unwrap();
        }

        /// The next non-ping text frame, or None after `within`.
        pub async fn recv(&mut self, within: Duration) -> Option<String> {
            let until = Instant::now() + within;
            loop {
                let left = until.checked_duration_since(Instant::now())?;
                match tokio::time::timeout(left, self.ws.next()).await {
                    Ok(Some(Ok(Message::Text(t)))) if t.as_str() == PING => {
                        if self.answer_pings {
                            let _ = self.ws.send(Message::Text(PONG.into())).await;
                        }
                    }
                    Ok(Some(Ok(Message::Text(t)))) => return Some(t.as_str().to_string()),
                    Ok(Some(Ok(_))) => {}
                    _ => return None,
                }
            }
        }

        /// Gone without a close frame, as a sleeping PC's line is.
        pub fn drop_abruptly(self) {
            // Dropping the stream sends no close frame.
            drop(self);
        }

        /// A half-open line: the socket stays open but nothing is read or
        /// answered, ever (no close frame, no pong).
        pub fn go_dead(self) -> Box<dyn Send> {
            Box::new(self.ws)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_workers_dev_relay_on_443() {
        assert_eq!(endpoint("https://task-relay.example.workers.dev").unwrap().as_str(), "wss://task-relay.example.workers.dev/v1/pc");
        assert_eq!(endpoint(" WSS://Task-Relay.Example.workers.dev/anything ").unwrap().as_str(), "wss://task-relay.example.workers.dev/v1/pc");
        for bad in [
            "http://task-relay.example.workers.dev",
            "https://example.workers.dev",
            "https://task-relay.example.workers.dev:8443",
            "https://user:pw@task-relay.example.workers.dev",
            "https://task-relay.example.workers.dev.evil.com",
            "https://api.anthropic.com",
            "https://127.0.0.1",
            "wss://localhost/v1/pc",
            "task-relay.example.workers.dev",
        ] {
            assert!(endpoint(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn nothing_connects_while_remote_tasks_are_off() {
        let url = endpoint("https://task-relay.example.workers.dev").unwrap();
        set_relay_allowed(false);
        assert_eq!(check(&url), Err("remote-off"));
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        assert_eq!(rt.block_on(connect(&url, "token")).err(), Some("remote-off"));
    }

    #[test]
    fn the_heartbeat_is_30_s_and_a_line_silent_for_60_s_is_dead() {
        let t = Timing::default();
        assert_eq!((t.ping_every, t.silent_for), (Duration::from_secs(30), Duration::from_secs(60)));
    }

    #[test]
    fn a_kick_is_counted_and_named() {
        let before = kicks();
        kick(Kick::Network);
        assert!(kicks() > before);
        assert_eq!(last_kick().code(), "network-changed");
        kick(Kick::Resumed);
        assert_eq!(last_kick().code(), "resumed");
    }

    #[test]
    fn a_wall_clock_jump_means_the_pc_slept() {
        let t = Timing::default();
        let before = SystemTime::now();
        assert!(!woke_from_sleep(before, before + Duration::from_millis(300), Duration::from_millis(300), &t));
        // Asleep 40 minutes: the wall clock moved, the loop's clock didn't.
        assert!(woke_from_sleep(before, before + Duration::from_secs(2400), Duration::from_millis(300), &t));
        // Or both moved and the loop ran 40 minutes late.
        assert!(woke_from_sleep(before, before + Duration::from_secs(2400), Duration::from_secs(2400), &t));
    }
}
