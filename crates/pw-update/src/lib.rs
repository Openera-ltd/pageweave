//! PageWeave app updates (decision D-024).
//!
//! Each GitHub release publishes `latest.json` (version, notes, and per platform the installer's
//! URL, size, SHA-256 and minisign signature, made by `tauri signer sign --app-version`). The app:
//!
//! 1. reads `latest.json` from this repository's latest release (nothing else is sent);
//! 2. offers the version only when it is newer than the running one;
//! 3. downloads the installer into the user's data folder (`updates/`), resuming an interrupted
//!    download, checking size and SHA-256;
//! 4. checks the signature against the public key built into the app, and that the signed version
//!    is the announced one (so an old signed installer cannot be passed off as new);
//! 5. checks the signature again just before running the installer in passive update mode.
//!
//! Downloaded models and settings live outside the install folder, so an update replaces only the
//! program. Builds without a public key (development builds) can check but never install.

use pw_core::{ErrorCategory, PwError, PwResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Where the newest release's manifest is.
pub const MANIFEST_URL: &str = "https://github.com/kuyamcliff/pageweave/releases/latest/download/latest.json";
/// Installers are only ever downloaded from this repository's releases.
pub const RELEASE_PREFIX: &str = "https://github.com/kuyamcliff/pageweave/releases/download/";
/// The platform key in `latest.json` for this build.
pub const PLATFORM: &str = "windows-x86_64";
/// Development builds only: read the manifest from here instead (tests).
pub const MANIFEST_ENV: &str = "PAGEWEAVE_UPDATE_MANIFEST";

const MAX_MANIFEST: u64 = 64 * 1024;
const MAX_INSTALLER: u64 = 1 << 30;

const WHAT: pw_net::What = pw_net::What {
    code: "update.download",
    noun: "The update",
    folder: "update folder",
    user_agent: "PageWeave update check",
};

/// The public key that signs releases (minisign, base64 as `tauri signer generate` prints it),
/// built in by the release workflow.
pub fn public_key() -> Option<&'static str> {
    option_env!("PAGEWEAVE_UPDATE_PUBKEY").map(str::trim).filter(|k| !k.is_empty())
}

/// Whether this build can install updates (it has a release key).
pub fn can_install() -> bool {
    public_key().is_some()
}

#[derive(Debug, Clone, Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    pub_date: Option<String>,
    platforms: HashMap<String, Asset>,
}

#[derive(Debug, Clone, Deserialize)]
struct Asset {
    url: String,
    signature: String,
    size: u64,
    sha256: String,
}

/// A newer version that can be installed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Update {
    pub version: String,
    pub current: String,
    pub notes: String,
    pub pub_date: Option<String>,
    /// Installer size in bytes.
    pub bytes: u64,
    /// Already downloaded and verified (installing needs no download).
    pub downloaded: bool,
}

fn bad_manifest(why: impl std::fmt::Display) -> PwError {
    PwError::new(
        ErrorCategory::CorruptResource,
        "update.manifest",
        format!("The update information could not be read ({why})."),
    )
    .with_action("Try again later.")
}

fn no_key() -> PwError {
    PwError::new(
        ErrorCategory::SecurityPolicy,
        "update.unsigned_build",
        "This copy of PageWeave cannot install updates because it was not built with the release key.",
    )
    .with_action("Install the new version from the PageWeave releases page.")
}

fn bad_signature(why: impl std::fmt::Display) -> PwError {
    PwError::new(
        ErrorCategory::SecurityPolicy,
        "update.signature",
        format!("The update was not installed because its signature is not valid ({why})."),
    )
    .with_action("Try again later. If this keeps happening, install the new version from the releases page.")
}

fn version(v: &str) -> PwResult<semver::Version> {
    semver::Version::parse(v.trim().trim_start_matches('v')).map_err(|_| bad_manifest(format!("version \"{v}\"")))
}

/// The installer file name, taken from a release URL of this repository.
fn installer_name(url: &str) -> PwResult<String> {
    let allowed = url.starts_with(RELEASE_PREFIX) || (cfg!(debug_assertions) && url.starts_with("http://127.0.0.1:"));
    if !allowed {
        return Err(bad_manifest("the installer is not on this project's releases page"));
    }
    let name = url.rsplit('/').next().unwrap_or("");
    let ok = !name.is_empty()
        && name.len() <= 100
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && !name.starts_with('.')
        && name.to_ascii_lowercase().ends_with("-setup.exe");
    if !ok {
        return Err(bad_manifest("unexpected installer name"));
    }
    Ok(name.to_string())
}

/// Read a manifest: `Some` when it offers a newer version for this platform.
fn parse(json: &[u8], current: &str) -> PwResult<Option<(Manifest, Asset)>> {
    let m: Manifest = serde_json::from_slice(json).map_err(|e| bad_manifest(e.to_string()))?;
    if version(&m.version)? <= version(current)? {
        return Ok(None);
    }
    let a = m.platforms.get(PLATFORM).cloned().ok_or_else(|| bad_manifest("no Windows installer"))?;
    installer_name(&a.url)?;
    if a.size == 0 || a.size > MAX_INSTALLER {
        return Err(bad_manifest("installer size"));
    }
    if a.sha256.len() != 64 || !a.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(bad_manifest("installer checksum"));
    }
    Ok(Some((m, a)))
}

fn fetch_manifest(current: &str) -> PwResult<Option<(Manifest, Asset)>> {
    let url = match std::env::var(MANIFEST_ENV) {
        Ok(u) if cfg!(debug_assertions) && !u.is_empty() => u,
        _ => MANIFEST_URL.to_string(),
    };
    // 404: no release has been published yet, so nothing is newer.
    match pw_net::get_small(&WHAT, &url, MAX_MANIFEST)? {
        None => Ok(None),
        Some(body) => parse(&body, current),
    }
}

/// Where downloaded installers are kept.
pub fn updates_dir() -> Option<PathBuf> {
    Some(pw_net::data_dir()?.join("updates"))
}

fn update_from(m: &Manifest, a: &Asset, current: &str, dir: Option<&Path>) -> PwResult<Update> {
    let downloaded = match (dir, public_key()) {
        (Some(d), Some(k)) => verify_file(&d.join(installer_name(&a.url)?), &a.signature, k, &m.version).is_ok(),
        _ => false,
    };
    Ok(Update {
        version: m.version.trim().trim_start_matches('v').to_string(),
        current: current.to_string(),
        notes: m.notes.chars().take(20_000).collect(),
        pub_date: m.pub_date.clone(),
        bytes: a.size,
        downloaded,
    })
}

/// Look for a newer version than `current`. `None` means this is the newest.
pub fn check(current: &str) -> PwResult<Option<Update>> {
    match fetch_manifest(current)? {
        None => Ok(None),
        Some((m, a)) => Ok(Some(update_from(&m, &a, current, updates_dir().as_deref())?)),
    }
}

/// Check `file` against a base64 minisign signature (as `tauri signer sign` writes it) and the
/// version it was signed for.
pub fn verify_file(file: &Path, signature_b64: &str, public_key_b64: &str, version: &str) -> PwResult<()> {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;
    let text = |s: &str| -> PwResult<String> {
        let raw = b64.decode(s.trim()).map_err(|_| bad_signature("not base64"))?;
        String::from_utf8(raw).map_err(|_| bad_signature("not text"))
    };
    let key = minisign_verify::PublicKey::decode(&text(public_key_b64)?).map_err(bad_signature)?;
    let sig = minisign_verify::Signature::decode(&text(signature_b64)?).map_err(bad_signature)?;
    let mut verifier = key.verify_stream(&sig).map_err(bad_signature)?;
    let mut f = std::fs::File::open(file).map_err(|e| PwError::from_io(&e, "downloaded update"))?;
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf).map_err(|e| PwError::from_io(&e, "downloaded update"))?;
        if n == 0 {
            break;
        }
        verifier.update(&buf[..n]);
    }
    verifier.finalize().map_err(bad_signature)?;
    // The trusted comment is covered by the signature checked above.
    let signed = sig.trusted_comment().split('\t').find_map(|f| f.strip_prefix("version:"));
    let want = version.trim().trim_start_matches('v');
    match signed {
        Some(v) if v.trim_start_matches('v') == want => Ok(()),
        Some(v) => Err(bad_signature(format!("signed for version {v}, offered as {want}"))),
        None => Err(bad_signature("no signed version")),
    }
}

fn download_into(
    dir: &Path,
    key: &str,
    m: &Manifest,
    a: &Asset,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<PathBuf> {
    let name = installer_name(&a.url)?;
    std::fs::create_dir_all(dir).map_err(|e| PwError::from_io(&e, WHAT.folder))?;
    // Only one installer is kept: older downloads are removed.
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if !n.starts_with(&name) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let target = dir.join(&name);
    let sig_path = dir.join(format!("{name}.sig"));
    if verify_file(&target, &a.signature, key, &m.version).is_ok() {
        progress(a.size, a.size);
    } else {
        let _ = std::fs::remove_file(&target);
        pw_net::fetch(&WHAT, &a.url, &target, a.size, &a.sha256.to_ascii_lowercase(), cancelled, &|b| {
            progress(b, a.size)
        })?;
        if let Err(e) = verify_file(&target, &a.signature, key, &m.version) {
            let _ = std::fs::remove_file(&target);
            return Err(e);
        }
    }
    std::fs::write(&sig_path, format!("{}\n{}\n", m.version.trim().trim_start_matches('v'), a.signature.trim()))
        .map_err(|e| PwError::from_io(&e, WHAT.folder))?;
    Ok(target)
}

/// Download and verify the installer for `version` (which must still be the newest).
pub fn download(
    current: &str,
    version: &str,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<PathBuf> {
    let key = public_key().ok_or_else(no_key)?;
    let (m, a) = fetch_manifest(current)?
        .ok_or_else(|| PwError::new(ErrorCategory::NotFound, "update.none", "There is no newer version to install."))?;
    if m.version.trim().trim_start_matches('v') != version.trim().trim_start_matches('v') {
        return Err(PwError::new(
            ErrorCategory::OutputConflict,
            "update.changed",
            format!("A different version ({}) is now available.", m.version),
        )
        .with_action("Check for updates again."));
    }
    let dir = updates_dir().ok_or_else(|| {
        PwError::new(ErrorCategory::Permission, "update.no_folder", "There is no folder to store the update in.")
    })?;
    download_into(&dir, key, &m, &a, cancelled, progress)
}

/// The downloaded installer for `version`, verified again now. Its arguments run it as an update
/// (`/P` passive: progress only, no questions; `/UPDATE`; `/R` start PageWeave again afterwards).
pub fn prepared_installer(version: &str) -> PwResult<(PathBuf, Vec<&'static str>)> {
    let key = public_key().ok_or_else(no_key)?;
    let dir = updates_dir()
        .ok_or_else(|| PwError::new(ErrorCategory::NotFound, "update.missing", "The update is not downloaded."))?;
    prepared_in(&dir, key, version)
}

fn prepared_in(dir: &Path, key: &str, version: &str) -> PwResult<(PathBuf, Vec<&'static str>)> {
    let missing = || {
        PwError::new(ErrorCategory::NotFound, "update.missing", "The update is not downloaded.")
            .with_action("Download the update again.")
    };
    let want = version.trim().trim_start_matches('v');
    let rd = std::fs::read_dir(dir).map_err(|_| missing())?;
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        let Some(exe) = n.strip_suffix(".sig") else { continue };
        let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
        let mut lines = text.lines();
        if lines.next() != Some(want) {
            continue;
        }
        let sig = lines.next().unwrap_or("");
        let path = dir.join(exe);
        verify_file(&path, sig, key, want)?;
        return Ok((path, vec!["/P", "/UPDATE", "/R"]));
    }
    Err(missing())
}

/// Remove downloaded installers for versions at or below `current` (after an update finished).
pub fn clean(current: &str) {
    let Some(dir) = updates_dir() else { return };
    clean_in(&dir, current);
}

fn clean_in(dir: &Path, current: &str) {
    let Ok(cur) = version(current) else { return };
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut stale = false;
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if n.ends_with(".sig") {
            let v = std::fs::read_to_string(e.path()).ok().and_then(|t| t.lines().next().map(str::to_string));
            if v.and_then(|v| version(&v).ok()).is_none_or(|v| v <= cur) {
                stale = true;
            }
        }
    }
    if stale {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A throwaway key made for these tests (`tauri signer generate`); never used for releases.
    const KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEY4MkQwMEMwQTNCNjBBQzQKUldURUNyYWp3QUF0K05NenZDUVNrVUVyOHhoTW1UWjl6VXNMNTNDNFNXTTFLYnA3YWR6L0VaNDEK";
    const BODY: &[u8] = b"PageWeave update test installer\n";
    const SIG_020: &str = include_str!("../tests/fixtures/v0.2.0.sig");
    const SIG_015: &str = include_str!("../tests/fixtures/v0.1.5.sig");

    fn sha(b: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
    }

    fn manifest(version: &str, url: &str, sig: &str) -> String {
        serde_json::json!({
            "version": version, "notes": "Faster search.", "pub_date": "2026-11-01T00:00:00Z",
            "platforms": { PLATFORM: { "url": url, "signature": sig, "size": BODY.len(), "sha256": sha(BODY) } }
        })
        .to_string()
    }

    const URL: &str = "https://github.com/kuyamcliff/pageweave/releases/download/v0.2.0/PageWeave_0.2.0_x64-setup.exe";

    #[test]
    fn only_newer_versions_from_this_project_are_offered() {
        let m = manifest("0.2.0", URL, SIG_020);
        assert!(parse(m.as_bytes(), "0.2.0").expect("same").is_none());
        assert!(parse(m.as_bytes(), "0.3.0").expect("older").is_none());
        let (m2, a) = parse(m.as_bytes(), "0.1.0").expect("parse").expect("newer");
        let u = update_from(&m2, &a, "0.1.0", None).expect("update");
        assert_eq!((u.version.as_str(), u.bytes, u.downloaded), ("0.2.0", BODY.len() as u64, false));
        for url in [
            "https://example.com/PageWeave_0.2.0_x64-setup.exe",
            "https://github.com/someone/else/releases/download/v1/PageWeave_0.2.0_x64-setup.exe",
            "https://github.com/kuyamcliff/pageweave/releases/download/v0.2.0/evil.bat",
            "https://github.com/kuyamcliff/pageweave/releases/download/v0.2.0/..%2Fx-setup.exe",
        ] {
            let e = parse(manifest("0.2.0", url, SIG_020).as_bytes(), "0.1.0").expect_err(url);
            assert_eq!(e.code, "update.manifest", "{url}");
        }
        assert_eq!(parse(b"not json", "0.1.0").expect_err("json").code, "update.manifest");
        assert_eq!(parse(manifest("soon", URL, SIG_020).as_bytes(), "0.1.0").expect_err("v").code, "update.manifest");
    }

    #[test]
    fn signatures_must_match_the_file_and_the_announced_version() {
        let dir = tempfile::tempdir().expect("tmp");
        let f = dir.path().join("PageWeave_0.2.0_x64-setup.exe");
        std::fs::write(&f, BODY).expect("write");
        verify_file(&f, SIG_020, KEY, "0.2.0").expect("valid");
        // An old signed installer offered as a new version.
        assert_eq!(verify_file(&f, SIG_015, KEY, "0.2.0").expect_err("replay").code, "update.signature");
        // A changed file.
        std::fs::write(&f, b"PageWeave update test installer!\n").expect("write");
        assert_eq!(verify_file(&f, SIG_020, KEY, "0.2.0").expect_err("tampered").code, "update.signature");
        // Another key.
        let other = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDk1Qzk5QTZGMjBCMjM5RDAKUldUUU9iSWdiNXJKbFZYWTFqZ3JIUzBYZDJEWDVrSzh4NUxRaHRKNkgrUEQ1aGllZXpwQUI4ekMK";
        std::fs::write(&f, BODY).expect("write");
        assert_eq!(verify_file(&f, SIG_020, other, "0.2.0").expect_err("key").code, "update.signature");
        assert_eq!(verify_file(&f, "garbage", KEY, "0.2.0").expect_err("junk").code, "update.signature");
    }

    /// Serve `body` for any request.
    fn serve(body: &'static [u8]) -> String {
        use std::io::{BufRead, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!("http://{}/PageWeave_0.2.0_x64-setup.exe", l.local_addr().expect("addr"));
        std::thread::spawn(move || {
            for mut s in l.incoming().flatten() {
                let mut r = std::io::BufReader::new(s.try_clone().expect("clone"));
                let mut line = String::new();
                while r.read_line(&mut line).unwrap_or(0) > 2 {
                    line.clear();
                }
                let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(body);
            }
        });
        url
    }

    #[test]
    fn downloads_are_verified_before_and_again_when_installing() {
        let dir = tempfile::tempdir().expect("tmp");
        let updates = dir.path().join("updates");
        std::fs::create_dir_all(&updates).expect("dir");
        std::fs::write(updates.join("PageWeave_0.1.9_x64-setup.exe"), b"old").expect("old");
        let url = serve(BODY);
        let (m, a) = parse(manifest("0.2.0", &url, SIG_020).as_bytes(), "0.1.0").expect("parse").expect("newer");
        let path = download_into(&updates, KEY, &m, &a, &|| false, &|_, _| {}).expect("download");
        assert_eq!(std::fs::read(&path).expect("read"), BODY);
        assert!(!updates.join("PageWeave_0.1.9_x64-setup.exe").exists(), "older installers are removed");
        assert!(update_from(&m, &a, "0.1.0", Some(&updates)).is_ok());
        let (exe, args) = prepared_in(&updates, KEY, "0.2.0").expect("prepared");
        assert_eq!((exe, args), (path.clone(), vec!["/P", "/UPDATE", "/R"]));
        assert_eq!(prepared_in(&updates, KEY, "0.3.0").expect_err("other").code, "update.missing");
        // Changed after download: never run.
        std::fs::write(&path, b"PageWeave update test installer?\n").expect("tamper");
        assert_eq!(prepared_in(&updates, KEY, "0.2.0").expect_err("tampered").code, "update.signature");

        // A server answering with other bytes of the right size: deleted, nothing kept.
        let bad = serve(b"PageWeave update test installeR\n");
        let (m, a) = parse(manifest("0.2.0", &bad, SIG_020).as_bytes(), "0.1.0").expect("parse").expect("newer");
        let e = download_into(&updates, KEY, &m, &a, &|| false, &|_, _| {}).expect_err("bad");
        assert_eq!(e.code, "update.download_corrupt");
        assert!(!updates.join("PageWeave_0.2.0_x64-setup.exe").exists());

        // After updating, installers for the running version are cleaned up.
        std::fs::write(updates.join("PageWeave_0.2.0_x64-setup.exe.sig"), "0.2.0\nx\n").expect("sig");
        clean_in(&updates, "0.1.0");
        assert!(updates.exists(), "a newer download is kept");
        clean_in(&updates, "0.2.0");
        assert!(!updates.exists());
    }

    #[test]
    fn builds_without_a_release_key_never_install() {
        if public_key().is_none() {
            assert_eq!(
                download("0.1.0", "0.2.0", &|| false, &|_, _| {}).expect_err("key").code,
                "update.unsigned_build"
            );
            assert_eq!(prepared_installer("0.2.0").expect_err("key").code, "update.unsigned_build");
            assert!(!can_install());
        }
    }
}
