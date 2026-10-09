// THE network choke point. This module is the only code in the app allowed to
// make an outbound request: nothing else imports the HTTP client (a test below
// scans the sources and fails if anything does).
//
// Phase 0 allowlist: http(s) to `127.0.0.1` or `localhost`, any port. Anything
// else (another host, a LAN address, a redirect that leaves this machine, a
// system proxy) is refused before a socket is opened. Ollama and LM Studio are
// the only callers (local_chat.rs).
//
// Every answer a server sends is read with a ceiling, so a misbehaving local
// server cannot make the app hold an unbounded amount of memory.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
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

/// The only hosts a request may go to. Exact names: not `::1`, not other
/// 127.x addresses, not `*.localhost`.
pub const ALLOWED_HOSTS: &[&str] = &["127.0.0.1", "localhost"];

/// Why a request was refused, for the message and the log.
pub fn refusal(url: &str) -> String {
    tf("Blocked: Glim only connects to this computer (127.0.0.1 or localhost), not {url}.", &[("url", url)])
}

/// Ok when `url` may be requested: http or https, no credentials, and a host
/// on the allowlist. Every request and every redirect hop passes through here.
pub fn check(url: &Url) -> Result<(), String> {
    let allowed = matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none()
        && url
            .host_str()
            .map(|h| h.to_ascii_lowercase())
            .is_some_and(|h| ALLOWED_HOSTS.contains(&h.as_str()));
    if allowed {
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
/// * follows a redirect only to another allowed address.
///
/// The returned builder can take headers and a body, but not a new URL.
pub fn request(method: Method, url: &Url, timeout: Duration) -> Result<RequestBuilder, String> {
    check(url)?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10).min(timeout))
        .timeout(timeout)
        .no_proxy()
        .resolve("localhost", SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .redirect(reqwest::redirect::Policy::custom(|attempt| match check(attempt.url()) {
            Ok(()) if attempt.previous().len() < 5 => attempt.follow(),
            Ok(()) => attempt.stop(),
            Err(e) => attempt.error(e),
        }))
        .build()
        .map_err(|e| e.to_string())?;
    Ok(client.request(method, url.clone()))
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
        // Anything off this machine is refused at the door.
        assert!(n("https://llm.example.com/proxy/v1").unwrap_err().starts_with("Blocked"));
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

    fn block_on<T>(f: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
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
