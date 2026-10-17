//! Safe output writing (CLAUDE.md §4.8, PRD 05 "Editing and saving").
//!
//! Every output is written to a temporary file in the destination directory, flushed to disk,
//! validated by the caller, and only then committed with an atomic rename. Source inputs are
//! never overwritten, and same-name collisions follow an explicit [`CollisionPolicy`].

use crate::error::{ErrorCategory, PwError, PwResult};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollisionPolicy {
    /// Refuse if the destination exists (default — never silently overwrite).
    #[default]
    Fail,
    /// Pick the next free name: `report (1).pdf`, `report (2).pdf`, …
    Rename,
    /// Atomically replace an existing destination that is not one of the inputs.
    Replace,
}

const MAX_RENAME_ATTEMPTS: u32 = 9999;

/// Resolve a path for comparison: canonical if it exists, else canonical parent + file name.
fn comparable(path: &Path) -> Option<PathBuf> {
    if let Ok(p) = path.canonicalize() {
        return Some(p);
    }
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    Some(parent.canonicalize().ok()?.join(path.file_name()?))
}

/// True if `candidate` refers to the same file as any of `protected`.
pub fn is_protected(candidate: &Path, protected: &[PathBuf]) -> bool {
    let Some(c) = comparable(candidate) else { return false };
    protected.iter().filter_map(|p| comparable(p)).any(|p| {
        if cfg!(windows) {
            p.as_os_str().to_string_lossy().to_lowercase() == c.as_os_str().to_string_lossy().to_lowercase()
        } else {
            p == c
        }
    })
}

/// Choose the final output path according to `policy`. Does not create anything.
pub fn plan_output(desired: &Path, policy: CollisionPolicy, protected: &[PathBuf]) -> PwResult<PathBuf> {
    let file_name = desired
        .file_name()
        .ok_or_else(|| PwError::invalid("output.no_file_name", "Choose a file name for the output."))?;
    if file_name.to_string_lossy().trim().is_empty() {
        return Err(PwError::invalid("output.no_file_name", "Choose a file name for the output."));
    }
    let parent = desired.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    match fs::metadata(parent) {
        Ok(m) if m.is_dir() => {}
        Ok(_) => return Err(PwError::invalid("output.parent_not_dir", "The output folder is not a folder.")),
        Err(e) => return Err(PwError::from_io(&e, "output folder")),
    }
    if desired.is_dir() {
        return Err(PwError::invalid("output.is_dir", "The output name refers to a folder."));
    }
    if is_protected(desired, protected) {
        return Err(PwError::new(
            ErrorCategory::SecurityPolicy,
            "output.would_overwrite_source",
            "The output would overwrite an input file. Originals are preserved; choose another name.",
        ));
    }
    if !desired.exists() {
        return Ok(desired.to_path_buf());
    }
    match policy {
        CollisionPolicy::Fail => {
            Err(PwError::new(ErrorCategory::OutputConflict, "output.exists", "A file with that name already exists."))
        }
        CollisionPolicy::Replace => Ok(desired.to_path_buf()),
        CollisionPolicy::Rename => next_free_name(desired, protected),
    }
}

fn next_free_name(desired: &Path, protected: &[PathBuf]) -> PwResult<PathBuf> {
    let parent = desired.parent().unwrap_or(Path::new("."));
    let stem = desired.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = desired.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    for i in 1..=MAX_RENAME_ATTEMPTS {
        let candidate = parent.join(format!("{stem} ({i}){ext}"));
        if !candidate.exists() && !is_protected(&candidate, protected) {
            return Ok(candidate);
        }
    }
    Err(PwError::new(ErrorCategory::OutputConflict, "output.no_free_name", "Could not find a free output name."))
}

/// Name prefix of staged (not yet committed) output files written by process `pid`.
pub fn staged_prefix(pid: u32) -> String {
    format!(".pw-staged-{pid}-")
}

/// Outcome of a committed write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommittedOutput {
    pub path: PathBuf,
    pub bytes: u64,
}

/// Write an output atomically.
///
/// `produce` writes the complete content to the staged temporary path it is given. `validate`
/// re-opens the staged file and must return `Ok` only if it is a correct output. Nothing appears at
/// the destination unless both succeed; on any error the staged file is removed.
pub fn write_atomic(
    desired: &Path,
    policy: CollisionPolicy,
    protected: &[PathBuf],
    produce: impl FnOnce(&Path) -> PwResult<()>,
    validate: impl FnOnce(&Path) -> PwResult<()>,
) -> PwResult<CommittedOutput> {
    let target = plan_output(desired, policy, protected)?;
    let dir = target.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let mut builder = tempfile::Builder::new();
    // The owning process id is part of the name so restart recovery can tell which leftovers belong
    // to a process that is gone (see `journal`).
    let prefix = staged_prefix(std::process::id());
    builder.prefix(&prefix).suffix(".tmp");
    // Temp files default to 0600; a saved document should get normal permissions (0666 & !umask).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o666));
    }
    let staged = builder.tempfile_in(dir).map_err(|e| PwError::from_io(&e, "output folder"))?;
    produce(staged.path())?;
    // Flush the produced bytes to stable storage before validating and committing.
    fs::OpenOptions::new()
        .write(true)
        .open(staged.path())
        .and_then(|f| f.sync_all())
        .map_err(|e| PwError::from_io(&e, "output file"))?;
    validate(staged.path())?;
    let bytes = fs::metadata(staged.path()).map_err(|e| PwError::from_io(&e, "output file"))?.len();

    let committed = match policy {
        CollisionPolicy::Replace => staged.persist(&target).map(|_| target.clone()),
        CollisionPolicy::Fail => staged.persist_noclobber(&target).map(|_| target.clone()),
        CollisionPolicy::Rename => {
            // Another process may have created the name since planning; pick again on conflict.
            let mut staged = staged;
            let mut candidate = target.clone();
            let mut attempts = 0;
            loop {
                match staged.persist_noclobber(&candidate) {
                    Ok(_) => break Ok(candidate),
                    Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists && attempts < 16 => {
                        attempts += 1;
                        staged = e.file;
                        candidate = next_free_name(desired, protected)?;
                    }
                    Err(e) => break Err(e),
                }
            }
        }
    };
    let path = committed.map_err(|e| {
        if e.error.kind() == io::ErrorKind::AlreadyExists {
            PwError::new(ErrorCategory::OutputConflict, "output.exists", "A file with that name already exists.")
        } else {
            PwError::from_io(&e.error, "output file")
        }
    })?;
    sync_dir(dir);
    Ok(CommittedOutput { path, bytes })
}

/// Convenience wrapper for outputs already held in memory.
pub fn write_atomic_bytes(
    desired: &Path,
    policy: CollisionPolicy,
    protected: &[PathBuf],
    data: &[u8],
    validate: impl FnOnce(&Path) -> PwResult<()>,
) -> PwResult<CommittedOutput> {
    write_atomic(
        desired,
        policy,
        protected,
        |p| {
            let mut f = fs::File::create(p).map_err(|e| PwError::from_io(&e, "output file"))?;
            f.write_all(data).map_err(|e| PwError::from_io(&e, "output file"))
        },
        validate,
    )
}

#[cfg(unix)]
fn sync_dir(dir: &Path) {
    // Best effort: persist the rename itself. Failure here does not undo a completed commit.
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_validate(_: &Path) -> PwResult<()> {
        Ok(())
    }

    fn staged_files(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .expect("read dir")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".pw-staged-"))
            .collect()
    }

    #[test]
    fn writes_new_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("out.pdf");
        let c = write_atomic_bytes(&out, CollisionPolicy::Fail, &[], b"hello", no_validate).expect("write");
        assert_eq!(c.path, out);
        assert_eq!(c.bytes, 5);
        assert_eq!(fs::read(&out).expect("read"), b"hello");
        assert!(staged_files(dir.path()).is_empty());
    }

    #[test]
    fn fail_policy_refuses_existing_and_keeps_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("out.pdf");
        fs::write(&out, b"original").expect("seed");
        let e = write_atomic_bytes(&out, CollisionPolicy::Fail, &[], b"new", no_validate).expect_err("conflict");
        assert_eq!(e.category, ErrorCategory::OutputConflict);
        assert_eq!(fs::read(&out).expect("read"), b"original");
    }

    #[test]
    fn rename_policy_picks_next_free_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("out.pdf");
        fs::write(&out, b"original").expect("seed");
        fs::write(dir.path().join("out (1).pdf"), b"other").expect("seed");
        let c = write_atomic_bytes(&out, CollisionPolicy::Rename, &[], b"new", no_validate).expect("write");
        assert_eq!(c.path, dir.path().join("out (2).pdf"));
        assert_eq!(fs::read(&out).expect("read"), b"original");
    }

    #[test]
    fn replace_policy_replaces_non_source() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("out.pdf");
        fs::write(&out, b"original").expect("seed");
        write_atomic_bytes(&out, CollisionPolicy::Replace, &[], b"new", no_validate).expect("write");
        assert_eq!(fs::read(&out).expect("read"), b"new");
    }

    #[test]
    fn never_overwrites_a_source_even_with_replace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("in.pdf");
        fs::write(&src, b"source").expect("seed");
        // Same file reached through a different spelling of the path.
        let alias = dir.path().join(".").join("in.pdf");
        let e = write_atomic_bytes(&alias, CollisionPolicy::Replace, std::slice::from_ref(&src), b"x", no_validate)
            .expect_err("must refuse");
        assert_eq!(e.code, "output.would_overwrite_source");
        assert_eq!(fs::read(&src).expect("read"), b"source");
    }

    #[test]
    fn failed_validation_leaves_nothing_behind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("out.pdf");
        let e = write_atomic_bytes(&out, CollisionPolicy::Fail, &[], b"bad", |_| {
            Err(PwError::new(ErrorCategory::ValidationFailed, "test.invalid", "invalid"))
        })
        .expect_err("validation fails");
        assert_eq!(e.category, ErrorCategory::ValidationFailed);
        assert!(!out.exists());
        assert!(staged_files(dir.path()).is_empty(), "staged temp must be cleaned up");
    }

    #[test]
    fn failed_produce_leaves_existing_destination_untouched() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("out.pdf");
        fs::write(&out, b"original").expect("seed");
        let _ = write_atomic(&out, CollisionPolicy::Replace, &[], |_| Err(PwError::cancelled()), no_validate)
            .expect_err("cancelled");
        assert_eq!(fs::read(&out).expect("read"), b"original");
        assert!(staged_files(dir.path()).is_empty());
    }

    #[test]
    fn missing_parent_and_folder_targets_are_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let e = plan_output(&dir.path().join("nope").join("x.pdf"), CollisionPolicy::Fail, &[]).expect_err("missing");
        assert_eq!(e.category, ErrorCategory::NotFound);
        let e = plan_output(dir.path(), CollisionPolicy::Replace, &[]).expect_err("dir");
        assert!(e.code == "output.is_dir" || e.code == "output.no_file_name", "{}", e.code);
    }

    #[test]
    fn unicode_and_spaces_in_names() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("résumé – 文件 (final).pdf");
        let c = write_atomic_bytes(&out, CollisionPolicy::Fail, &[], b"ok", no_validate).expect("write");
        assert_eq!(c.path, out);
        let c2 = write_atomic_bytes(&out, CollisionPolicy::Rename, &[], b"ok", no_validate).expect("write");
        assert_eq!(c2.path, dir.path().join("résumé – 文件 (final) (1).pdf"));
    }

    #[cfg(unix)]
    #[test]
    fn committed_outputs_get_normal_permissions_not_private_temp_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("perm.pdf");
        write_atomic_bytes(&out, CollisionPolicy::Fail, &[], b"x", no_validate).expect("write");
        let mode = fs::metadata(&out).expect("meta").permissions().mode() & 0o777;
        assert_ne!(mode, 0o600, "output must not keep the temp file's private mode");
        assert_eq!(mode & 0o600, 0o600, "owner can read and write");
    }

    #[cfg(unix)]
    #[test]
    fn read_only_destination_maps_to_permission() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let ro = dir.path().join("ro");
        fs::create_dir(&ro).expect("mkdir");
        fs::set_permissions(&ro, fs::Permissions::from_mode(0o555)).expect("chmod");
        let probe = ro.join("probe");
        if fs::write(&probe, b"x").is_ok() {
            // Running as root: permission bits are not enforced, so the scenario cannot be produced here.
            eprintln!("NOT RUN: read_only_destination_maps_to_permission (permission bits not enforced for this user)");
            let _ = fs::remove_file(&probe);
            return;
        }
        let e = write_atomic_bytes(&ro.join("x.pdf"), CollisionPolicy::Fail, &[], b"x", no_validate)
            .expect_err("read only");
        assert_eq!(e.category, ErrorCategory::Permission);
    }
}
