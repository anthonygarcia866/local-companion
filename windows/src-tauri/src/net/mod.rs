// THE network choke point. This module is the only code in the app allowed to
// make an outbound request: nothing else imports the HTTP client (a test below
// scans the sources and fails if anything does).
//
// The allowlist: http(s) to `127.0.0.1` or `localhost`, any port (Ollama and
// LM Studio, local_chat.rs), and since 1c `api.anthropic.com`, https on 443
// only (the Claude API, ai/claude.rs; docs/vision.md §3). Anything else
// (another host, a LAN address, a redirect that changes host, a system proxy)
// is refused before a socket is opened. Certificates are always checked.
//
// One more host has its own door: `api.pushover.net`, https on 443 only, only
// while Settings → Phone is on, and only through `push_request`, which only
// phone.rs calls (a test checks the source). `request` never reaches it, so no
// other part of Glim can, Phone on or off (docs/phone-plan.md §4).
//
// Every answer a server sends is read with a ceiling, so a misbehaving local
// server cannot make the app hold an unbounded amount of memory.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub use reqwest::{Method, RequestBuilder, Response, Url};

use crate::i18n::{t, tf};

/// Ceiling for a JSON answer: a chat reply or a model list.
pub const MAX_BODY: usize = 8 * 1024 * 1024;
/// Ceiling for an error body: only its message is shown.
pub const MAX_ERROR_BODY: usize = 64 * 1024;

/// True for an address that is this machine: `localhost`, 127.0.0.0/8, ::1,
/// and the unspecified addresses (0.0.0.0, ::), which connect to this machine.
pub fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ip.is_loopback() || ip.is_unspecified(),
        Ok(IpAddr::V6(ip)) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
        }
        Err(_) => false,
    }
}

pub fn is_loopback_url(url: &Url) -> bool {
    url.host_str().is_some_and(is_loopback_host)
}

/// The address of a model server as the user typed or pasted it, cleaned up:
/// a missing scheme becomes `http://`, trailing slashes and the documentation
/// sub-paths people paste (`/api`, `/v1`) go, and the loopback names become
/// 127.0.0.1 — on Windows `localhost` may resolve to ::1 first while Ollama
/// only listens on IPv4. Only http and https, no `user:password@`, and only
/// an address the allowlist accepts.
pub fn normalise_server_url(raw: &str) -> Result<Url, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(t("Enter the server address first."));
    }
    let with_scheme = if raw.contains("://") { raw.to_string() } else { format!("http://{raw}") };
    let mut url = Url::parse(&with_scheme).map_err(|_| tf("Not a valid address: {address}", &[("address", raw)]))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(t("The address must start with http:// or https://."));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(t("Leave the user name and password out of the address."));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(tf("Not a valid address: {address}", &[("address", raw)]));
    }
    if is_loopback_url(&url) {
        let _ = url.set_ip_host(IpAddr::V4(Ipv4Addr::LOCALHOST));
    } else {
        // A model server is on this machine: the Anthropic host isn't one.
        return Err(local_refusal(&host_for_log(&url)));
    }
    let mut path = url.path().trim_end_matches('/').to_string();
    for suffix in ["/api", "/v1"] {
        if let Some(rest) = path.strip_suffix(suffix) {
            path = rest.trim_end_matches('/').to_string();
        }
    }
    url.set_path(&path);
    url.set_query(None);
    url.set_fragment(None);
    // Only a server on this machine can be connected at all.
    check(&url)?;
    Ok(url)
}

/// `base` + `tail`, with exactly one slash between them.
pub fn join(base: &Url, tail: &str) -> String {
    format!("{}/{}", base.as_str().trim_end_matches('/'), tail.trim_start_matches('/'))
}

// ── The allowlist ─────────────────────────────────────────────────────────────

/// The local hosts a request may go to, over http or https on any port.
/// A free port on 127.0.0.1, for a local child server Glim starts (voice:
/// whisper.cpp's server). The probe listener is closed at once.
pub fn free_loopback_port() -> Option<u16> {
    let l = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).ok()?;
    l.local_addr().ok().map(|a| a.port())
}

/// Exact names: not `::1`, not other 127.x addresses, not `*.localhost`.
pub const ALLOWED_HOSTS: &[&str] = &["127.0.0.1", "localhost"];

/// The one remote host (1c, docs/vision.md §3): the Claude API, https on
/// port 443 only. Exact name: no other Anthropic host, no subdomain.
pub const ANTHROPIC_HOST: &str = "api.anthropic.com";

/// Phone notifications (docs/phone-plan.md §4): Pushover's API, https on
/// port 443 only, exact name. Reached only through `push_request`, and only
/// while `push_allowed()`; never through `request` or `check`.
pub const PUSH_HOST: &str = "api.pushover.net";

static PUSH_ON: AtomicBool = AtomicBool::new(false);

/// Settings → Phone switched on or off (phone::apply_settings).
pub fn set_push_allowed(on: bool) {
    PUSH_ON.store(on, Ordering::SeqCst);
}

pub fn push_allowed() -> bool {
    PUSH_ON.load(Ordering::SeqCst)
}

/// Why a request was refused, for the message and the log.
pub fn refusal(url: &str) -> String {
    tf("Blocked: Glim only connects to this computer and api.anthropic.com, not {url}.", &[("url", url)])
}

/// Why a model server's address was refused: those are on this machine only.
pub fn local_refusal(url: &str) -> String {
    tf("Blocked: Glim only connects to this computer (127.0.0.1 or localhost), not {url}.", &[("url", url)])
}

/// Ok when `url` may be requested: no credentials, and either http(s) to a
/// local host on the allowlist or https to the Anthropic host on port 443.
/// Every request and every redirect hop passes through here.
pub fn check(url: &Url) -> Result<(), String> {
    let host = url.host_str().map(|h| h.to_ascii_lowercase());
    let local = matches!(url.scheme(), "http" | "https") && host.as_deref().is_some_and(|h| ALLOWED_HOSTS.contains(&h));
    // `Url` drops a default port, so `port()` is None for https on 443.
    let anthropic = url.scheme() == "https" && host.as_deref() == Some(ANTHROPIC_HOST) && url.port().is_none_or(|p| p == 443);
    if url.username().is_empty() && url.password().is_none() && (local || anthropic) {
        Ok(())
    } else {
        Err(refusal(&host_for_log(url)))
    }
}

/// A redirect hop from `origin` to `next`: `next` must pass `check`, and a
/// request that started off this machine may not change host (the Anthropic
/// API can't send Glim anywhere else, not even to localhost).
pub fn hop_allowed(origin: &Url, next: &Url) -> Result<(), String> {
    check(next)?;
    if !is_loopback_url(origin) && origin.host_str() != next.host_str() {
        return Err(refusal(&host_for_log(next)));
    }
    Ok(())
}

/// `push_request`'s rule: Settings → Phone on, https to the Pushover host on
/// port 443, no credentials. Nothing else, not even this computer.
pub fn check_push(url: &Url) -> Result<(), String> {
    let host = url.host_str().map(|h| h.to_ascii_lowercase());
    let push = url.scheme() == "https" && host.as_deref() == Some(PUSH_HOST) && url.port().is_none_or(|p| p == 443);
    if push_allowed() && push && url.username().is_empty() && url.password().is_none() {
        Ok(())
    } else {
        Err(refusal(&host_for_log(url)))
    }
}

/// `check` for a URL as text.
pub fn check_str(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw.trim()).map_err(|_| refusal(raw.trim()))?;
    check(&url)?;
    Ok(url)
}

/// A request to `url`, or an error if the allowlist refuses it. The client:
/// * never uses a proxy (a system proxy would carry the request off the machine);
/// * resolves `localhost` to 127.0.0.1 itself, whatever the hosts file says;
/// * follows a redirect only to another allowed address (`hop_allowed`);
/// * checks certificates (rustls and its bundled roots; nothing here may turn
///   that off, a test checks the source).
///
/// The returned builder can take headers and a body, but not a new URL.
pub fn request(method: Method, url: &Url, timeout: Duration) -> Result<RequestBuilder, String> {
    check(url)?;
    // The Claude API's requests share one client, so the connection a voice
    // key's pre-warm opened (prewarm) is the one the request goes out on.
    if let Some(shared) = shared_client(url)? {
        return Ok(shared.request(method, url.clone()).timeout(timeout));
    }
    let origin = url.clone();
    let client = client(
        timeout,
        reqwest::redirect::Policy::custom(move |attempt| match hop_allowed(&origin, attempt.url()) {
            Ok(()) if attempt.previous().len() < 5 => attempt.follow(),
            Ok(()) => attempt.stop(),
            Err(e) => attempt.error(e),
        }),
    )?;
    Ok(client.request(method, url.clone()))
}

// ── Pre-warm (owner, 2026-10-28: faster answers) ──────────────────────────────

/// One client per shared origin: api.anthropic.com in the app (tests add
/// their own local server's origin). Its pool keeps the connection a
/// pre-warm opened for the request that follows (reqwest's idle timeout).
static SHARED: std::sync::Mutex<Option<std::collections::HashMap<String, reqwest::Client>>> = std::sync::Mutex::new(None);

/// Origins a test shares like the Claude API's (its own local server only:
/// a pooled connection can't outlive the test's runtime into another test).
#[cfg(test)]
static TEST_SHARED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn shared_origin(url: &Url) -> bool {
    let anthropic = url.host_str().is_some_and(|h| h.eq_ignore_ascii_case(ANTHROPIC_HOST));
    #[cfg(test)]
    if TEST_SHARED.lock().unwrap().contains(&url.origin().ascii_serialization()) {
        return true;
    }
    anthropic
}

fn shared_client(url: &Url) -> Result<Option<reqwest::Client>, String> {
    if !shared_origin(url) {
        return Ok(None);
    }
    let key = url.origin().ascii_serialization();
    let mut map = SHARED.lock().unwrap();
    let map = map.get_or_insert_with(Default::default);
    if let Some(c) = map.get(&key) {
        return Ok(Some(c.clone()));
    }
    let origin = url.clone();
    // The request's own timeout is set per request; this one only bounds a
    // client nobody gave one to.
    let c = client(
        Duration::from_secs(60),
        reqwest::redirect::Policy::custom(move |attempt| match hop_allowed(&origin, attempt.url()) {
            Ok(()) if attempt.previous().len() < 5 => attempt.follow(),
            Ok(()) => attempt.stop(),
            Err(e) => attempt.error(e),
        }),
    )?;
    map.insert(key, c.clone());
    Ok(Some(c))
}

/// While a voice key is held: the connection to the Claude API is opened
/// (DNS, TCP, TLS) so the clean-up request that follows doesn't wait for it.
/// What goes out is a bare `HEAD /` on that host: no body, no API key, none
/// of Glim's headers, nothing the user said (the transcript doesn't exist
/// yet). Only a shared origin (the allowlisted Claude API): anything else is
/// refused. The answer is dropped unread.
pub async fn prewarm(base: &Url) -> Result<(), String> {
    check(base)?;
    let mut root = base.clone();
    root.set_path("/");
    root.set_query(None);
    let Some(c) = shared_client(&root)? else {
        return Err(refusal(&host_for_log(&root)));
    };
    c.head(root).timeout(Duration::from_secs(5)).send().await.map(drop).map_err(|e| if e.is_timeout() { "timeout".into() } else { "network".into() })
}

/// Phone notifications' own door (phone.rs only): a request to Pushover's
/// API while Settings → Phone is on (`check_push`), with `request`'s client
/// rules, and no redirect followed at all.
pub fn push_request(method: Method, url: &Url, timeout: Duration) -> Result<RequestBuilder, String> {
    check_push(url)?;
    Ok(client(timeout, reqwest::redirect::Policy::none())?.request(method, url.clone()))
}

/// The client every request uses: no proxy, `localhost` pinned to 127.0.0.1,
/// certificates checked, and the caller's redirect rule.
fn client(timeout: Duration, redirect: reqwest::redirect::Policy) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10).min(timeout))
        .timeout(timeout)
        .no_proxy()
        .resolve("localhost", SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .redirect(redirect)
        .build()
        .map_err(|e| e.to_string())
}

/// The body of `response`, refused past `limit` bytes.
pub async fn read_capped(mut response: Response, limit: usize) -> Result<Vec<u8>, String> {
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err(t("The server's answer is too large."));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| tf("Network error: {error}", &[("error", &e.to_string())]))? {
        if body.len() + chunk.len() > limit {
            return Err(t("The server's answer is too large."));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// The text of an error body, cut down for a message: `error.message`
/// (OpenAI, Anthropic), a bare `error` string (Ollama), or the first characters.
pub fn error_detail(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
        if let Some(m) = v.pointer("/error/message").and_then(|m| m.as_str()) {
            return m.to_string();
        }
        if let Some(m) = v.get("error").and_then(|m| m.as_str()) {
            return m.to_string();
        }
        if let Some(m) = v.get("message").and_then(|m| m.as_str()) {
            return m.to_string();
        }
        // Some servers answer errors as a one-element array.
        if let Some(m) = v.pointer("/0/error/message").and_then(|m| m.as_str()) {
            return m.to_string();
        }
    }
    text.trim().chars().take(200).collect()
}

/// Only the host of an address, for the log: never a path, a query or a key.
pub fn host_for_log(url: &Url) -> String {
    match (url.host_str(), url.port()) {
        (Some(h), Some(p)) => format!("{h}:{p}"),
        (Some(h), None) => h.to_string(),
        _ => "?".into(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn loopback_hosts_are_recognised_and_others_are_not() {
        for host in ["localhost", "LOCALHOST", "app.localhost", "127.0.0.1", "127.8.9.1", "::1", "[::1]", "0.0.0.0", "[::]", "::ffff:127.0.0.1"] {
            assert!(is_loopback_host(host), "{host}");
        }
        for host in ["example.com", "192.168.1.10", "10.0.0.2", "localhost.example.com", "[2001:db8::1]", "128.0.0.1", ""] {
            assert!(!is_loopback_host(host), "{host}");
        }
    }

    #[test]
    fn a_pasted_server_address_is_cleaned_up() {
        let n = |s: &str| normalise_server_url(s).map(|u| u.to_string());
        assert_eq!(n("  http://localhost:11434/  ").unwrap(), "http://127.0.0.1:11434/");
        assert_eq!(n("http://localhost:11434/v1").unwrap(), "http://127.0.0.1:11434/");
        assert_eq!(n("http://127.0.0.1:1234/v1/").unwrap(), "http://127.0.0.1:1234/");
        assert_eq!(n("localhost:11434/api").unwrap(), "http://127.0.0.1:11434/");
        assert_eq!(n("0.0.0.0:11434").unwrap(), "http://127.0.0.1:11434/");
        assert_eq!(n("http://[::1]:8000").unwrap(), "http://127.0.0.1:8000/");
        assert_eq!(n("http://127.0.0.1:1234/proxy/v1?x=1#y").unwrap(), "http://127.0.0.1:1234/proxy");
        // Anything off this machine is refused at the door, Anthropic included:
        // a model server's address is a local one.
        assert!(n("https://llm.example.com/proxy/v1").unwrap_err().starts_with("Blocked"));
        assert!(n("https://api.anthropic.com").unwrap_err().contains("127.0.0.1 or localhost"));
        assert!(n("gpu-box.lan:8000").unwrap_err().starts_with("Blocked"));
        assert!(n("192.168.1.20:11434").unwrap_err().starts_with("Blocked"));
    }

    #[test]
    fn a_server_address_that_is_not_http_is_refused() {
        assert!(normalise_server_url("").is_err());
        assert!(normalise_server_url("   ").is_err());
        assert!(normalise_server_url("file:///etc/passwd").is_err());
        assert!(normalise_server_url("javascript://alert(1)").is_err());
        assert!(normalise_server_url("http://user:pw@example.com").is_err());
        assert!(normalise_server_url("http://").is_err());
    }

    #[test]
    fn join_puts_one_slash_between_base_and_path() {
        let base = normalise_server_url("http://127.0.0.1:11434").unwrap();
        assert_eq!(join(&base, "/v1/models"), "http://127.0.0.1:11434/v1/models");
        let base = Url::parse("http://localhost:1234/v1").unwrap();
        assert_eq!(join(&base, "chat/completions"), "http://localhost:1234/v1/chat/completions");
    }

    #[test]
    fn error_details_come_from_the_usual_places() {
        assert_eq!(error_detail(br#"{"error":{"message":"bad key"}}"#), "bad key");
        assert_eq!(error_detail(br#"{"error":"model 'x' not found"}"#), "model 'x' not found");
        assert_eq!(error_detail(br#"[{"error":{"message":"quota"}}]"#), "quota");
        assert_eq!(error_detail(b"  plain text  "), "plain text");
        assert_eq!(error_detail("x".repeat(500).as_bytes()).len(), 200);
    }

    #[test]
    fn only_the_host_goes_to_the_log() {
        let url = Url::parse("https://gw.example.com:8443/secret/path?key=abc").unwrap();
        assert_eq!(host_for_log(&url), "gw.example.com:8443");
        let url = Url::parse("https://gw.example.com/v1/messages").unwrap();
        assert_eq!(host_for_log(&url), "gw.example.com");
    }

    /// One-shot HTTP server on a free port answering the next request with `body`.
    pub(crate) fn serve_once(status: &str, headers: &str, body: Vec<u8>) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let head = format!("HTTP/1.1 {status}\r\n{headers}Connection: close\r\n\r\n");
        std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 8192];
            let _ = conn.read(&mut buf);
            let _ = conn.write_all(head.as_bytes());
            let _ = conn.write_all(&body);
        });
        url
    }

    /// Like `serve_once`, and hands back the request it got (head and body
    /// as raw bytes), for tests of what a client sends (ai/claude.rs).
    /// A server on 127.0.0.1 that accepts and never answers (timeouts).
    pub(crate) fn serve_silent(for_secs: u64) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let _held = listener.accept();
            std::thread::sleep(std::time::Duration::from_secs(for_secs));
        });
        url
    }

    pub(crate) fn serve_capture(status: &str, headers: &str, body: Vec<u8>) -> (String, std::sync::mpsc::Receiver<Vec<u8>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let head = format!("HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            // Read the head, then as much body as Content-Length says.
            let mut got = Vec::new();
            let mut buf = [0u8; 8192];
            loop {
                let n = conn.read(&mut buf).unwrap_or(0);
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&buf[..n]);
                if let Some(end) = got.windows(4).position(|w| w == b"\r\n\r\n") {
                    let text = String::from_utf8_lossy(&got[..end]).to_ascii_lowercase();
                    let len = text
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if got.len() >= end + 4 + len {
                        break;
                    }
                }
            }
            let _ = tx.send(got);
            let _ = conn.write_all(head.as_bytes());
            let _ = conn.write_all(&body);
        });
        (url, rx)
    }

    fn block_on<T>(f: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
    }

    /// A keep-alive HTTP/1.1 server on 127.0.0.1: every request as it came
    /// (head + body, raw), and how many connections were opened. Answers
    /// each with "ok". Shared like the Claude API's origin (TEST_SHARED).
    fn keep_alive_server() -> (Url, std::sync::Arc<std::sync::Mutex<(usize, Vec<String>)>>) {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = Url::parse(&format!("http://127.0.0.1:{}/", listener.local_addr().unwrap().port())).unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new((0usize, Vec::<String>::new())));
        let s = seen.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming().flatten() {
                s.lock().unwrap().0 += 1;
                let s = s.clone();
                std::thread::spawn(move || {
                    let mut out = conn.try_clone().unwrap();
                    let mut r = BufReader::new(conn);
                    loop {
                        let mut head = String::new();
                        loop {
                            let mut line = String::new();
                            if r.read_line(&mut line).unwrap_or(0) == 0 {
                                return;
                            }
                            head.push_str(&line);
                            if line == "\r\n" {
                                break;
                            }
                        }
                        let len = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                        let mut body = vec![0; len];
                        let _ = r.read_exact(&mut body);
                        s.lock().unwrap().1.push(format!("{head}{}", String::from_utf8_lossy(&body)));
                        // A HEAD's answer has no body (as any server's).
                        let answer: &[u8] = if head.starts_with("HEAD ") { b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n" } else { b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok" };
                        let _ = out.write_all(answer);
                    }
                });
            }
        });
        TEST_SHARED.lock().unwrap().push(url.origin().ascii_serialization());
        (url, seen)
    }

    /// Faster answers: the pre-warm is a bare HEAD / (no body, no key, none
    /// of Glim's headers), and the request after it goes out on the same
    /// connection, the transcript only in that request.
    #[test]
    fn a_prewarm_sends_nothing_and_the_request_reuses_its_connection() {
        let (base, seen) = keep_alive_server();
        let messages = base.join("v1/messages").unwrap();
        block_on(async {
            prewarm(&base).await.unwrap();
            let r = request(Method::POST, &messages, Duration::from_secs(5)).unwrap().header("x-api-key", "k").body("the transcript").send().await.unwrap();
            assert_eq!(read_capped(r, 100).await.unwrap(), b"ok");
        });
        let (connections, reqs) = seen.lock().unwrap().clone();
        assert_eq!(reqs.len(), 2, "{reqs:?}");
        let warm = reqs[0].to_ascii_lowercase();
        assert!(warm.starts_with("head / http/1.1\r\n"), "{warm}");
        for absent in ["x-api-key", "content-length", "content-type", "anthropic", "transcript", "user-agent"] {
            assert!(!warm.contains(absent), "{absent} in the pre-warm: {warm}");
        }
        assert!(warm.ends_with("\r\n\r\n"), "no body");
        assert!(reqs[1].starts_with("POST /v1/messages") && reqs[1].ends_with("the transcript"));
        assert_eq!(connections, 1, "the request reused the pre-warmed connection");
    }

    /// A key released before the pre-warm finished: both go out, nothing
    /// waits on the other, and the transcript is only in the request.
    #[test]
    fn a_key_released_fast_still_gets_its_answer() {
        let (base, seen) = keep_alive_server();
        let messages = base.join("v1/messages").unwrap();
        block_on(async {
            // The pre-warm starts in the background (as on key-down) and the
            // request goes out at once, without waiting for it.
            let b = base.clone();
            let warm = tokio::spawn(async move { prewarm(&b).await });
            let a = read_capped(request(Method::POST, &messages, Duration::from_secs(5)).unwrap().body("fast").send().await.unwrap(), 100).await;
            assert_eq!(a.unwrap(), b"ok");
            assert!(warm.await.unwrap().is_ok());
        });
        let reqs = seen.lock().unwrap().1.clone();
        assert_eq!(reqs.iter().filter(|r| r.contains("fast")).count(), 1);
        assert!(reqs.iter().filter(|r| r.starts_with("HEAD")).all(|r| !r.contains("fast")));
    }

    /// Only a shared origin (the Claude API) is ever pre-warmed: a local
    /// server nobody shared, another host, or plain http to the API: refused,
    /// nothing opened.
    #[test]
    fn only_the_claude_api_is_prewarmed() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let local = Url::parse(&format!("http://127.0.0.1:{}/", listener.local_addr().unwrap().port())).unwrap();
        listener.set_nonblocking(true).unwrap();
        for url in [local.as_str(), "https://example.com/", "http://api.anthropic.com/", "https://api.anthropic.com:8443/"] {
            assert!(block_on(prewarm(&Url::parse(url).unwrap())).is_err(), "{url}");
        }
        assert!(listener.accept().is_err(), "nothing connected to the unshared server");
    }

    #[test]
    fn a_body_past_the_ceiling_is_refused_with_or_without_a_length() {
        let get = |url: String| async move {
            let url = Url::parse(&url).unwrap();
            let response = request(Method::GET, &url, Duration::from_secs(5)).unwrap().send().await.unwrap();
            read_capped(response, 1000).await
        };
        // Declared length too large.
        let url = serve_once("200 OK", "Content-Length: 5000\r\n", vec![b'x'; 5000]);
        assert!(block_on(get(url)).is_err());
        // No length (read to the end of the connection): stopped while reading.
        let url = serve_once("200 OK", "", vec![b'x'; 5000]);
        assert!(block_on(get(url)).is_err());
        // Under the ceiling: read whole.
        let url = serve_once("200 OK", "Content-Length: 3\r\n", b"abc".to_vec());
        assert_eq!(block_on(get(url)).unwrap(), b"abc");
    }

    // ── The allowlist, tested by actually trying ──────────────────────────────

    #[test]
    fn only_127_0_0_1_and_localhost_are_allowed() {
        for ok in ["http://127.0.0.1:11434/v1/models", "http://localhost:1234", "https://LOCALHOST:8443/x", "http://127.0.0.1"] {
            assert!(check_str(ok).is_ok(), "{ok}");
        }
        for no in [
            "https://example.com",
            "http://example.com:11434",
            "http://192.168.1.10:11434",
            "http://10.0.0.2",
            "http://[::1]:11434",
            "http://127.0.0.2",
            "http://0.0.0.0:11434",
            "http://app.localhost",
            "http://localhost.example.com",
            "http://127.0.0.1.nip.io",
            "http://user:pw@127.0.0.1",
            "ftp://127.0.0.1",
            "file:///C:/Windows/win.ini",
            "not a url",
        ] {
            assert!(check_str(no).is_err(), "{no}");
        }
    }

    #[test]
    fn the_anthropic_api_is_allowed_only_over_https_on_443() {
        for ok in ["https://api.anthropic.com/v1/messages", "https://API.Anthropic.com/v1/messages", "https://api.anthropic.com:443/v1/messages"] {
            assert!(check_str(ok).is_ok(), "{ok}");
        }
        for no in [
            "http://api.anthropic.com/v1/messages",
            "https://api.anthropic.com:8443/v1/messages",
            "http://api.anthropic.com:443/v1/messages",
            "https://anthropic.com",
            "https://console.anthropic.com",
            "https://x.api.anthropic.com",
            "https://api.anthropic.com.example.com",
            "https://user:pw@api.anthropic.com",
            "wss://api.anthropic.com",
        ] {
            let err = check_str(no).expect_err(no);
            assert!(err.starts_with("Blocked"), "{err}");
        }
    }

    // Phone notifications (docs/phone-plan.md §4). The only test that turns
    // the switch on, so no other test sees it changed.
    #[test]
    fn pushover_is_reachable_only_through_push_request_while_phone_is_on() {
        let api = "https://api.pushover.net/1/messages.json";
        let url = Url::parse(api).unwrap();
        let wait = Duration::from_secs(5);
        assert!(!push_allowed(), "off by default");
        assert!(check_push(&url).expect_err("off: blocked").starts_with("Blocked"));
        assert!(push_request(Method::POST, &url, wait).is_err());
        set_push_allowed(true);
        let on = std::panic::catch_unwind(|| {
            for ok in [api, "https://API.Pushover.net/1/messages.json", "https://api.pushover.net:443/1/messages.json"] {
                let u = Url::parse(ok).unwrap();
                assert!(check_push(&u).is_ok(), "{ok}");
                assert!(push_request(Method::POST, &u, wait).is_ok(), "{ok}");
            }
            // Phone on, and still no other part of Glim can reach Pushover:
            // `request` (every other caller's door) and `check` refuse it.
            let err = request(Method::POST, &url, wait).expect_err("a non-phone request must be refused");
            assert!(err.starts_with("Blocked"), "{err}");
            assert!(check_str(api).is_err());
            // Nor can a redirect from anywhere land on it.
            assert!(hop_allowed(&Url::parse("http://127.0.0.1:9/").unwrap(), &url).is_err());
            assert!(hop_allowed(&Url::parse("https://api.anthropic.com/v1/messages").unwrap(), &url).is_err());
            // And the phone's door opens onto Pushover only.
            for no in [
                "http://api.pushover.net/1/messages.json",
                "https://api.pushover.net:8443/1/messages.json",
                "https://pushover.net/",
                "https://x.api.pushover.net/",
                "https://api.pushover.net.example.com/",
                "https://user:pw@api.pushover.net/",
                "https://api.anthropic.com/v1/messages",
                "http://127.0.0.1:9/",
                "http://localhost:9/",
            ] {
                let u = Url::parse(no).unwrap();
                assert!(check_push(&u).expect_err(no).starts_with("Blocked"), "{no}");
                assert!(push_request(Method::POST, &u, wait).is_err(), "{no}");
            }
        });
        set_push_allowed(false);
        on.unwrap();
        assert!(check_push(&url).is_err(), "off again: blocked again");
    }

    /// Phone notifications' door is phone.rs's alone: no other source file
    /// outside src/net/ calls `push_request` or flips the switch.
    #[test]
    fn only_phone_rs_uses_the_push_door() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let words = [concat!("push_", "request"), concat!("check_", "push"), concat!("set_push_", "allowed")];
        let mut users = Vec::new();
        let mut stack = vec![src.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    if path != src.join("net") {
                        stack.push(path);
                    }
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    for word in words {
                        if text.contains(word) {
                            users.push(format!("{} {word}", path.file_name().unwrap().to_string_lossy()));
                        }
                    }
                }
            }
        }
        users.sort();
        assert_eq!(users, ["phone.rs push_request", "phone.rs set_push_allowed"]);
    }

    #[test]
    fn a_redirect_from_anthropic_may_not_change_host() {
        let api = Url::parse("https://api.anthropic.com/v1/messages").unwrap();
        let hop = |s: &str| hop_allowed(&api, &Url::parse(s).unwrap());
        assert!(hop("https://api.anthropic.com/v1/other").is_ok());
        for no in ["http://127.0.0.1:11434/", "https://localhost/", "https://example.com/", "http://api.anthropic.com/v1/messages"] {
            assert!(hop(no).is_err(), "{no}");
        }
        // Local servers keep the old rule: any allowed local address.
        let local = Url::parse("http://127.0.0.1:11434/").unwrap();
        assert!(hop_allowed(&local, &Url::parse("http://localhost:11434/x").unwrap()).is_ok());
        assert!(hop_allowed(&local, &Url::parse("https://example.com/").unwrap()).is_err());
    }

    #[test]
    fn a_request_to_an_external_host_is_refused_before_anything_is_sent() {
        let url = Url::parse("https://example.com").unwrap();
        let err = request(Method::GET, &url, Duration::from_secs(5)).err().expect("refused");
        assert!(err.contains("example.com"), "{err}");
        assert!(err.starts_with("Blocked"), "{err}");
        // And through the same path a caller would use end to end.
        let result = block_on(async { request(Method::GET, &url, Duration::from_secs(5))?.send().await.map_err(|e| e.to_string()) });
        assert!(result.is_err());
    }

    #[test]
    fn a_redirect_off_this_machine_is_refused() {
        let url = serve_once("302 Found", "Location: https://example.com/\r\nContent-Length: 0\r\n", Vec::new());
        let url = Url::parse(&url).unwrap();
        let result = block_on(async { request(Method::GET, &url, Duration::from_secs(5)).unwrap().send().await });
        let err = result.expect_err("the redirect must not be followed");
        assert!(err.is_redirect(), "{err:?}");
    }

    #[test]
    fn a_system_proxy_is_never_used() {
        // A proxy that would answer for every host: requests must still go
        // straight to the local server.
        let proxy = serve_once("200 OK", "Content-Length: 5\r\n", b"proxy".to_vec());
        let server = serve_once("200 OK", "Content-Length: 6\r\n", b"direct".to_vec());
        std::env::set_var("HTTP_PROXY", &proxy);
        std::env::set_var("ALL_PROXY", &proxy);
        let url = Url::parse(&server).unwrap();
        let body = block_on(async {
            let r = request(Method::GET, &url, Duration::from_secs(5)).unwrap().send().await.unwrap();
            read_capped(r, 100).await.unwrap()
        });
        std::env::remove_var("HTTP_PROXY");
        std::env::remove_var("ALL_PROXY");
        assert_eq!(body, b"direct");
    }

    /// Certificates are always checked: no source file may switch that off.
    #[test]
    fn nothing_turns_certificate_checks_off() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        // Built with concat! so this file doesn't name them itself.
        let forbidden = [concat!("danger_", "accept_invalid_certs"), concat!("danger_", "accept_invalid_hostnames"), concat!("tls_built_in_root_certs", "(false)")];
        let mut stack = vec![src];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    for word in forbidden {
                        assert!(!text.contains(word), "{} uses {word}", path.display());
                    }
                }
            }
        }
    }

    /// The choke point is only a choke point if nothing goes around it: no
    /// source file outside src/net/ may name the HTTP client or open a socket.
    #[test]
    fn no_other_module_can_reach_the_network() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let forbidden = [
            "reqwest", "TcpStream", "TcpListener", "UdpSocket", "tokio::net::Tcp", "tokio::net::Udp",
            "ToSocketAddrs", "WinHttp", "WinInet", "URLDownloadToFile",
        ];
        let mut offenders = Vec::new();
        let mut stack = vec![src.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    if path != src.join("net") {
                        stack.push(path);
                    }
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    for word in forbidden {
                        if text.contains(word) {
                            offenders.push(format!("{} uses {word}", path.display()));
                        }
                    }
                }
            }
        }
        assert!(offenders.is_empty(), "network access outside src/net/: {offenders:#?}");
    }
}
