//! Downloading translation models on demand (decision D-023): one folder per direction in the
//! user's PageWeave data folder, so app updates never download models again. Each file is fetched
//! from its pinned URL over HTTPS (the operating system's certificate store), into `<file>.part`
//! (an interrupted download resumes where it stopped), checked against the pinned size and SHA-256,
//! and only then renamed into place. Nothing else is ever downloaded or run.

use crate::{lock, LockPair};
use pw_core::{ErrorCategory, PwError, PwResult};
use std::path::{Path, PathBuf};

/// Explicit folder (tests); always honoured, since downloaded models are data verified by hash.
pub const USER_DIR_ENV: &str = "PAGEWEAVE_MODELS_DIR";

/// Where downloaded directions live: `%LOCALAPPDATA%\io.github.kuyamcliff.pageweave\translate`
/// on Windows, `$XDG_DATA_HOME/io.github.kuyamcliff.pageweave/translate` elsewhere.
pub fn user_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os(USER_DIR_ENV) {
        return Some(PathBuf::from(d));
    }
    Some(pw_net::data_dir()?.join("translate"))
}

/// Download wording for model files.
const WHAT: pw_net::What = pw_net::What {
    code: "translate.download",
    noun: "The translation model",
    folder: "model folder",
    user_agent: "PageWeave model download",
};

fn sha256_file(p: &Path) -> PwResult<String> {
    pw_net::sha256_file(p, WHAT.folder)
}

fn fetch(
    url: &str,
    target: &Path,
    size: u64,
    sha: &str,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64),
) -> PwResult<()> {
    pw_net::fetch(&WHAT, url, target, size, sha, cancelled, progress)
}

fn pinned(source: &str, target: &str) -> PwResult<LockPair> {
    lock()?.pairs.into_iter().find(|p| p.source == source && p.target == target).ok_or_else(|| {
        PwError::new(
            ErrorCategory::UnsupportedOperation,
            "translate.direction",
            format!(
                "Translating from {} to {} is not available.",
                crate::language_name(source),
                crate::language_name(target)
            ),
        )
    })
}

/// Bytes to download for a direction (0 when it is already installed).
pub fn download_size(source: &str, target: &str) -> PwResult<u64> {
    let p = pinned(source, target)?;
    if crate::pair_dir(source, target).is_some() {
        return Ok(0);
    }
    Ok(p.files.values().map(|f| f.bytes).sum())
}

/// Download and verify the model for `source` → `target` into the user's data folder.
/// `progress(done, total)` counts bytes. Returns the folder.
pub fn install(
    source: &str,
    target: &str,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<PathBuf> {
    let p = pinned(source, target)?;
    // Shipped or development models are used as they are (verified when loaded).
    if let Some(d) = crate::pair_dir(source, target) {
        if Some(d.parent().map(Path::to_path_buf)) != Some(user_dir()) {
            return Ok(d);
        }
    }
    // Downloads are checked file by file below: good files are kept, damaged or missing ones
    // fetched again.
    let root = user_dir().ok_or_else(|| {
        PwError::new(
            ErrorCategory::Permission,
            "translate.no_folder",
            "There is no folder to store translation models in.",
        )
    })?;
    let dir = root.join(p.folder());
    std::fs::create_dir_all(&dir).map_err(|e| PwError::from_io(&e, "model folder"))?;
    // One download per direction at a time (app and command line may both try). An operating
    // system lock: released by itself when a process ends, even if it is killed.
    let lock_path = dir.join(".downloading");
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|e| PwError::from_io(&e, "model folder"))?;
    if lock_file.try_lock().is_err() {
        return Err(PwError::new(
            ErrorCategory::OutputConflict,
            "translate.busy",
            "This language is already being downloaded.",
        )
        .with_action("Wait for that download to finish."));
    }
    let total: u64 = p.files.values().map(|f| f.bytes).sum();
    let mut names: Vec<&String> = p.files.keys().collect();
    names.sort();
    let mut done_before = 0u64;
    let result = (|| -> PwResult<()> {
        for name in names {
            let f = &p.files[name];
            let target_file = dir.join(name);
            let ok = std::fs::metadata(&target_file).map(|m| m.len() == f.bytes).unwrap_or(false)
                && sha256_file(&target_file)? == f.sha256;
            if !ok {
                let _ = std::fs::remove_file(&target_file);
                let url = p.file_url(f);
                let base = done_before;
                fetch(&url, &target_file, f.bytes, &f.sha256, cancelled, &|b| progress(base + b, total))?;
            }
            done_before += f.bytes;
            progress(done_before, total);
        }
        Ok(())
    })();
    drop(lock_file);
    let _ = std::fs::remove_file(&lock_path);
    result?;
    Ok(dir)
}

/// Delete a downloaded direction (bundled or development models are never touched).
pub fn remove(source: &str, target: &str) -> PwResult<bool> {
    let p = pinned(source, target)?;
    let Some(root) = user_dir() else { return Ok(false) };
    let dir = root.join(p.folder());
    if !dir.is_dir() {
        return Ok(false);
    }
    std::fs::remove_dir_all(&dir).map_err(|e| PwError::from_io(&e, "model folder"))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_directions_are_refused() {
        assert_eq!(install("en", "xx", &|| false, &|_, _| {}).expect_err("dir").code, "translate.direction");
        assert_eq!(remove("en", "xx").expect_err("dir").code, "translate.direction");
    }
}
