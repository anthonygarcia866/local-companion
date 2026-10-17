// Speech-to-text: whisper.cpp's server as a child process (voice plan §3.1,
// §6). whisper-cli can't transcribe from stdin, so it would need the audio in
// a temporary file; the server takes it in a POST from memory instead, and
// keeps the model loaded between utterances.
//
// - Files: put by hand in %LOCALAPPDATA%\Glim\whisper\ (whisper-server.exe
//   with its DLLs, and the model). Glim downloads nothing. Missing → the card,
//   nothing typed.
// - The child listens on 127.0.0.1 only, on a free port, and is started with
//   no proxy variables; requests go through net::request (localhost is on its
//   allowlist). It is killed when Glim exits, or when the model changes.
// - Models (Settings → Dictation → Speech model): "base" (default) or "small".

use std::path::PathBuf;
use std::time::Duration;

/// The model files Glim looks for.
pub fn model_file(size: &str) -> &'static str {
    if size == "small" { "ggml-small.en-q5_1.bin" } else { "ggml-base.en-q5_1.bin" }
}

/// "base" or "small"; anything else reads as "base".
pub fn model_size(setting: &str) -> &'static str {
    if setting == "small" { "small" } else { "base" }
}

pub const SERVER_EXE: &str = "whisper-server.exe";
/// The whole transcription, upload included (a 60 s clip on base takes ~10 s).
pub const TIMEOUT: Duration = Duration::from_secs(30);
/// The server's start, model load included.
pub const START_TIMEOUT: Duration = Duration::from_secs(20);

/// %LOCALAPPDATA%\Glim\whisper.
pub fn folder() -> PathBuf {
    crate::settings::local_dir().join("whisper")
}

/// The server and the model for `size`, when both are there.
pub fn files(size: &str) -> Option<(PathBuf, PathBuf)> {
    let dir = folder();
    let exe = dir.join(SERVER_EXE);
    let model = dir.join(model_file(size));
    (exe.is_file() && model.is_file()).then_some((exe, model))
}

/// The arguments the server is started with: loopback only.
pub fn server_args(model: &std::path::Path, port: u16, threads: usize) -> Vec<String> {
    vec![
        "-m".into(),
        model.to_string_lossy().into_owned(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        port.to_string(),
        "-t".into(),
        threads.to_string(),
        "-l".into(),
        "en".into(),
    ]
}

/// Proxy variables the child must not see (it never connects anywhere; this
/// keeps it from being pointed elsewhere).
pub const PROXY_VARS: [&str; 6] = ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"];

#[cfg(windows)]
mod server {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::process::{Child, Command, Stdio};
    use std::sync::Mutex;

    struct Running {
        child: Child,
        port: u16,
        size: &'static str,
    }

    static RUNNING: Mutex<Option<Running>> = Mutex::new(None);

    /// Stops the server (Glim exiting, or the model changed).
    pub fn stop() {
        if let Some(mut r) = RUNNING.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = r.child.kill();
            let _ = r.child.wait();
        }
    }

    /// The server's port, starting it (or restarting it on another model) if
    /// needed. Err("no-whisper") when the files aren't there.
    pub async fn ensure(size: &'static str) -> Result<u16, &'static str> {
        {
            let mut guard = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(r) = guard.as_mut() {
                let alive = matches!(r.child.try_wait(), Ok(None));
                if alive && r.size == size {
                    return Ok(r.port);
                }
                let _ = r.child.kill();
                let _ = r.child.wait();
                *guard = None;
            }
        }
        let (exe, model) = files(size).ok_or("no-whisper")?;
        let port = crate::net::free_loopback_port().ok_or("whisper-start")?;
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(8));
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut cmd = Command::new(&exe);
        cmd.args(server_args(&model, port, threads))
            .current_dir(folder())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW);
        for v in PROXY_VARS {
            cmd.env_remove(v);
        }
        let child = cmd.spawn().map_err(|_| "whisper-start")?;
        // It goes when Glim goes, even after a crash.
        if !crate::platform::kill_with_glim(&child) {
            crate::log::line("voice: whisper server not tied to Glim's lifetime");
        }
        *RUNNING.lock().unwrap_or_else(|e| e.into_inner()) = Some(Running { child, port, size });
        // Ready when it answers on its port (the model loads first).
        let started = std::time::Instant::now();
        let url = crate::net::Url::parse(&format!("http://127.0.0.1:{port}/")).map_err(|_| "whisper-start")?;
        while started.elapsed() < START_TIMEOUT {
            if let Ok(req) = crate::net::request(crate::net::Method::GET, &url, Duration::from_secs(1)) {
                if req.send().await.is_ok() {
                    return Ok(port);
                }
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        stop();
        Err("whisper-start")
    }
}

#[cfg(windows)]
pub use server::stop;

/// Transcribes 16 kHz mono PCM with the `size` model and the word-list
/// `prompt`. Err: "no-whisper" (files missing), "whisper-start", "whisper".
#[cfg(windows)]
pub async fn transcribe(pcm: &[i16], size: &'static str, prompt: &str) -> Result<String, &'static str> {
    let port = server::ensure(size).await?;
    let wav = super::wav_bytes(pcm);
    let mut fields = vec![("response_format", "text"), ("temperature", "0")];
    if !prompt.is_empty() {
        fields.push(("prompt", prompt));
    }
    let (ctype, body) = super::multipart(&fields, "file", "audio.wav", &wav);
    let url = crate::net::Url::parse(&format!("http://127.0.0.1:{port}/inference")).map_err(|_| "whisper")?;
    let req = crate::net::request(crate::net::Method::POST, &url, TIMEOUT).map_err(|_| "whisper")?;
    let resp = req.header("content-type", ctype).body(body).send().await.map_err(|_| "whisper")?;
    if !resp.status().is_success() {
        return Err("whisper");
    }
    let bytes = crate::net::read_capped(resp, crate::net::MAX_BODY).await.map_err(|_| "whisper")?;
    Ok(clean_transcript(&String::from_utf8_lossy(&bytes)))
}

/// whisper's text: one line, trimmed, its "[BLANK_AUDIO]"-style markers out.
pub fn clean_transcript(raw: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in raw.chars() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_is_the_default_model_and_small_the_other() {
        assert_eq!(model_size("small"), "small");
        for s in ["base", "", "medium", "large"] {
            assert_eq!(model_size(s), "base", "{s}");
        }
        assert_eq!(model_file("base"), "ggml-base.en-q5_1.bin");
        assert_eq!(model_file("small"), "ggml-small.en-q5_1.bin");
    }

    #[test]
    fn the_server_listens_on_loopback_only() {
        let args = server_args(std::path::Path::new("C:\\m.bin"), 51234, 8);
        let host = args.iter().position(|a| a == "--host").unwrap();
        assert_eq!(args[host + 1], "127.0.0.1");
        assert!(!args.iter().any(|a| a == "0.0.0.0" || a == "--public"));
        assert_eq!(args[args.iter().position(|a| a == "--port").unwrap() + 1], "51234");
    }

    #[test]
    fn markers_and_spacing_are_cleaned_from_the_transcript() {
        assert_eq!(clean_transcript(" Thanks for the update.\n"), "Thanks for the update.");
        assert_eq!(clean_transcript("[BLANK_AUDIO]"), "");
        assert_eq!(clean_transcript(" (wind blowing) Open Outlook. "), "Open Outlook.");
    }

    #[test]
    fn files_are_looked_for_in_glims_own_folder_and_nothing_is_downloaded() {
        assert!(folder().ends_with("whisper"));
        let src = include_str!("whisper.rs");
        let code = &src[..src.find("#[cfg(test)]").unwrap()];
        for banned in ["huggingface", "github.com", "https://"] {
            assert!(!code.contains(banned), "{banned}");
        }
    }
}
