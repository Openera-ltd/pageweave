//! PageWeave downloads (decisions D-023, D-024): one careful HTTPS download used for translation
//! models and app updates. HTTPS uses the operating system's certificate store and proxy settings;
//! a file is written to `<file>.part` (an interrupted download resumes where it stopped), checked
//! against its expected size and SHA-256, and only then renamed into place.

use pw_core::{ErrorCategory, PwError, PwResult};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The app's data folder name (the same as the desktop app's identifier).
pub const APP_DIR: &str = "io.github.kuyamcliff.pageweave";

/// The per-user data folder: `%LOCALAPPDATA%\io.github.kuyamcliff.pageweave` on Windows,
/// `$XDG_DATA_HOME/io.github.kuyamcliff.pageweave` elsewhere.
pub fn data_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
    }?;
    Some(base.join(APP_DIR))
}

/// How a download names itself in errors.
pub struct What {
    /// Error code for network failures; `<code>_corrupt` for checksum failures.
    pub code: &'static str,
    /// Subject of the message, e.g. "The translation model".
    pub noun: &'static str,
    /// Where files are written, for file errors.
    pub folder: &'static str,
    pub user_agent: &'static str,
}

fn net_err(w: &What, why: impl std::fmt::Display) -> PwError {
    PwError::new(ErrorCategory::Network, w.code, format!("{} could not be downloaded ({why}).", w.noun))
        .with_action("Check the internet connection and try again; the download continues where it stopped.")
}

/// An HTTPS client using the operating system's certificates and the system proxy.
pub fn agent(user_agent: &str) -> ureq::Agent {
    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::Rustls)
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .unversioned_rustls_crypto_provider(provider)
        .build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .proxy(ureq::Proxy::try_from_env())
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .http_status_as_error(false)
        .user_agent(user_agent)
        .build()
        .new_agent()
}

/// Lowercase hex SHA-256 of a file.
pub fn sha256_file(p: &Path, label: &str) -> PwResult<String> {
    use sha2::{Digest, Sha256};
    let mut f = std::fs::File::open(p).map_err(|e| PwError::from_io(&e, label))?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h).map_err(|e| PwError::from_io(&e, label))?;
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Fetch a small document (at most `max_bytes`). `None` when the server says it does not exist
/// (404). Network and server errors are retried like `fetch`.
pub fn get_small(w: &What, url: &str, max_bytes: u64) -> PwResult<Option<Vec<u8>>> {
    let mut last = String::new();
    for attempt in 1..=4u32 {
        if attempt > 1 {
            std::thread::sleep(Duration::from_secs(2u64.pow(attempt - 1)));
        }
        let resp = match agent(w.user_agent).get(url).call() {
            Ok(r) => r,
            Err(e) => {
                last = e.to_string();
                continue;
            }
        };
        match resp.status().as_u16() {
            200 => {}
            404 => return Ok(None),
            s @ 500..=599 => {
                last = format!("server answered {s}");
                continue;
            }
            other => return Err(net_err(w, format!("server answered {other}"))),
        }
        let mut body = resp.into_body();
        let mut out = Vec::new();
        match body.as_reader().take(max_bytes + 1).read_to_end(&mut out) {
            Ok(_) if out.len() as u64 > max_bytes => return Err(net_err(w, "the answer was too large")),
            Ok(_) => return Ok(Some(out)),
            Err(e) => last = e.to_string(),
        }
    }
    Err(net_err(w, last))
}

/// Download one file to `target`, resuming `target.part`. `progress(bytes)` reports bytes written
/// for this file so far. Retries network and server errors up to four times.
pub fn fetch(
    w: &What,
    url: &str,
    target: &Path,
    size: u64,
    sha: &str,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64),
) -> PwResult<()> {
    let part = target.with_extension(format!("{}.part", target.extension().and_then(|e| e.to_str()).unwrap_or("")));
    for attempt in 1..=4u32 {
        let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if have > size {
            let _ = std::fs::remove_file(&part);
            continue;
        }
        if have < size {
            let mut req = agent(w.user_agent).get(url);
            if have > 0 {
                req = req.header("Range", format!("bytes={have}-"));
            }
            let resp = match req.call() {
                Ok(r) => r,
                Err(e) if attempt < 4 => {
                    std::thread::sleep(Duration::from_secs(2u64.pow(attempt)));
                    let _ = e;
                    continue;
                }
                Err(e) => return Err(net_err(w, e)),
            };
            let status = resp.status().as_u16();
            let append = match status {
                206 => true,
                200 => false,
                500..=599 if attempt < 4 => {
                    std::thread::sleep(Duration::from_secs(2u64.pow(attempt)));
                    continue;
                }
                other => return Err(net_err(w, format!("server answered {other}"))),
            };
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .append(append)
                .truncate(!append)
                .open(&part)
                .map_err(|e| PwError::from_io(&e, w.folder))?;
            let mut written = if append { have } else { 0 };
            let mut body = resp.into_body();
            let mut reader = body.as_reader();
            let mut buf = vec![0u8; 1 << 16];
            let mut failed = None;
            loop {
                if cancelled() {
                    return Err(PwError::cancelled());
                }
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        written += n as u64;
                        if written > size {
                            failed = Some("more data than expected".to_string());
                            break;
                        }
                        file.write_all(&buf[..n]).map_err(|e| PwError::from_io(&e, w.folder))?;
                        progress(written);
                    }
                    Err(e) => {
                        failed = Some(e.to_string());
                        break;
                    }
                }
            }
            file.flush().map_err(|e| PwError::from_io(&e, w.folder))?;
            drop(file);
            if let Some(why) = failed {
                if attempt < 4 {
                    std::thread::sleep(Duration::from_secs(2u64.pow(attempt)));
                    continue;
                }
                return Err(net_err(w, why));
            }
        }
        if std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0) != size {
            continue;
        }
        if sha256_file(&part, w.folder)? != sha {
            // A damaged download is never kept: start this file again.
            let _ = std::fs::remove_file(&part);
            if attempt < 4 {
                continue;
            }
            return Err(PwError::new(
                ErrorCategory::CorruptResource,
                format!("{}_corrupt", w.code),
                format!("{} was downloaded damaged (its checksum did not match) and was deleted.", w.noun),
            )
            .with_action("Try again."));
        }
        std::fs::rename(&part, target).map_err(|e| PwError::from_io(&e, w.folder))?;
        return Ok(());
    }
    Err(net_err(w, "too many attempts"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    const W: What = What { code: "test.download", noun: "The test file", folder: "test folder", user_agent: "test" };

    /// A tiny HTTP server answering every request with `body` (honouring `Range: bytes=N-`),
    /// cutting the first answer short after `cut` bytes, and failing the first `errors` requests
    /// with 503. Returns the URL and a request counter.
    fn serve(body: Vec<u8>, cut: Option<usize>, errors: usize) -> (String, Arc<AtomicUsize>) {
        let l = TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!("http://{}/m.bin", l.local_addr().expect("addr"));
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        std::thread::spawn(move || {
            for stream in l.incoming() {
                let Ok(mut s) = stream else { continue };
                let n = c.fetch_add(1, Ordering::SeqCst);
                let mut reader = std::io::BufReader::new(s.try_clone().expect("clone"));
                let mut from = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        from = v.trim().trim_end_matches('-').parse().unwrap_or(0);
                    }
                }
                if n < errors {
                    let _ = s.write_all(b"HTTP/1.1 503 Busy\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    continue;
                }
                let rest = &body[from.min(body.len())..];
                let status = if from > 0 { "206 Partial Content" } else { "200 OK" };
                let head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", rest.len());
                let _ = s.write_all(head.as_bytes());
                let send = match cut {
                    Some(k) if n == errors => &rest[..k.min(rest.len())],
                    _ => rest,
                };
                let _ = s.write_all(send);
            }
        });
        (url, count)
    }

    fn sha(b: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn interrupted_downloads_resume_and_bad_data_is_never_kept() {
        let dir = tempfile::tempdir().expect("tmp");
        let body: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        // Cut short after 70 kB, then resumed with a Range request; a 503 first is retried.
        let (url, count) = serve(body.clone(), Some(70_000), 1);
        let target = dir.path().join("m.bin");
        let seen = std::sync::Mutex::new(Vec::new());
        fetch(&W, &url, &target, body.len() as u64, &sha(&body), &|| false, &|b| seen.lock().expect("lock").push(b))
            .expect("fetch");
        assert_eq!(std::fs::read(&target).expect("read"), body);
        assert!(count.load(Ordering::SeqCst) >= 3, "503, cut answer, resumed answer");
        assert!(!dir.path().join("m.bin.part").exists());
        let seen = seen.into_inner().expect("lock");
        assert!(seen.windows(2).all(|w| w[0] <= w[1]) && *seen.last().expect("progress") == body.len() as u64);

        // Data that does not match the checksum is deleted and fetched again; never kept.
        let (url, _) = serve(body.clone(), None, 0);
        let bad = dir.path().join("bad.bin");
        let e =
            fetch(&W, &url, &bad, body.len() as u64, &sha(b"something else"), &|| false, &|_| {}).expect_err("corrupt");
        assert_eq!(e.code, "test.download_corrupt");
        assert!(!bad.exists() && !dir.path().join("bad.bin.part").exists());

        // A wrong size from the server and a cancelled download.
        let (url, _) = serve(body.clone(), None, 0);
        let e = fetch(&W, &url, &dir.path().join("s.bin"), 10, &sha(&body), &|| false, &|_| {}).expect_err("size");
        assert!(e.code.starts_with("test.download"), "{}", e.code);
        let (url, _) = serve(body.clone(), None, 0);
        let e = fetch(&W, &url, &dir.path().join("c.bin"), body.len() as u64, &sha(&body), &|| true, &|_| {})
            .expect_err("cancel");
        assert_eq!(e.code, "job.cancelled");
        // Nothing listening: a network error with what to do.
        let e =
            fetch(&W, "http://127.0.0.1:9/x", &dir.path().join("n.bin"), 5, "x", &|| false, &|_| {}).expect_err("net");
        assert_eq!(e.code, "test.download");
    }

    #[test]
    fn small_documents_are_size_limited_and_missing_ones_are_none() {
        let (url, _) = serve(b"{\"v\":1}".to_vec(), None, 1);
        assert_eq!(get_small(&W, &url, 100).expect("get"), Some(b"{\"v\":1}".to_vec()));
        let (url, _) = serve(vec![b'x'; 5000], None, 0);
        assert_eq!(get_small(&W, &url, 100).expect_err("big").code, "test.download");
        let l = TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!("http://{}/latest.json", l.local_addr().expect("addr"));
        std::thread::spawn(move || {
            for mut s in l.incoming().flatten() {
                let mut line = String::new();
                let mut r = std::io::BufReader::new(s.try_clone().expect("clone"));
                while r.read_line(&mut line).unwrap_or(0) > 2 {
                    line.clear();
                }
                let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            }
        });
        assert_eq!(get_small(&W, &url, 100).expect("404"), None);
    }
}
