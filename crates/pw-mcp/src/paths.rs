//! Folder policy: the agent may only read and write inside folders the user approved when starting
//! the server. Paths are resolved (symlinks included) before they are compared.

use pw_core::{ErrorCategory, PwError, PwResult};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Roots {
    roots: Vec<PathBuf>,
}

fn denied(msg: impl Into<String>) -> PwError {
    PwError::new(ErrorCategory::SecurityPolicy, "mcp.path_denied", msg)
}

fn canonical(p: &Path) -> std::io::Result<PathBuf> {
    // Plain paths on Windows where equivalent (no \\?\ prefix), the same form the app shows.
    dunce::canonicalize(p)
}

impl Roots {
    /// Approved folders; each must exist and be a directory.
    pub fn new(dirs: &[PathBuf]) -> PwResult<Roots> {
        if dirs.is_empty() {
            return Err(PwError::invalid("mcp.no_roots", "Give at least one folder with --root."));
        }
        let mut roots = Vec::new();
        for d in dirs {
            let c = canonical(d).map_err(|e| PwError::from_io(&e, "folder"))?;
            if !c.is_dir() {
                return Err(PwError::invalid("mcp.root_not_dir", format!("{} is not a folder.", d.display())));
            }
            roots.push(c);
        }
        Ok(Roots { roots })
    }

    pub fn list(&self) -> &[PathBuf] {
        &self.roots
    }

    fn inside(&self, p: &Path) -> bool {
        self.roots.iter().any(|r| p.starts_with(r))
    }

    /// Relative paths are taken from the first approved folder.
    fn absolute(&self, raw: &str) -> PwResult<PathBuf> {
        if raw.trim().is_empty() {
            return Err(PwError::invalid("mcp.empty_path", "A path is required."));
        }
        if raw.contains('\0') {
            return Err(denied("The path contains a NUL character."));
        }
        let p = PathBuf::from(raw);
        Ok(if p.is_absolute() { p } else { self.roots[0].join(p) })
    }

    /// An existing file inside an approved folder.
    pub fn input(&self, raw: &str) -> PwResult<PathBuf> {
        let p = self.absolute(raw)?;
        let c = canonical(&p).map_err(|e| PwError::from_io(&e, "file"))?;
        if !self.inside(&c) {
            return Err(denied("That file is outside the folders PageWeave was allowed to use."));
        }
        if !c.is_file() {
            return Err(PwError::invalid("mcp.not_a_file", "That path is not a file."));
        }
        Ok(c)
    }

    /// A new file inside an approved folder: its folder must exist; the name must be plain.
    pub fn output(&self, raw: &str, extensions: &[&str]) -> PwResult<PathBuf> {
        let p = self.absolute(raw)?;
        let name = match p.components().next_back() {
            Some(Component::Normal(n)) => n.to_os_string(),
            _ => return Err(PwError::invalid("mcp.bad_output", "The output must name a file.")),
        };
        let ext_ok = Path::new(&name)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| extensions.iter().any(|x| e.eq_ignore_ascii_case(x)));
        if !ext_ok {
            return Err(PwError::invalid(
                "mcp.output_extension",
                format!("The output file name must end in .{}.", extensions.join(" or .")),
            ));
        }
        let parent = p.parent().ok_or_else(|| PwError::invalid("mcp.bad_output", "The output must name a file."))?;
        let dir = canonical(parent).map_err(|e| PwError::from_io(&e, "output folder"))?;
        if !self.inside(&dir) {
            return Err(denied("That output is outside the folders PageWeave was allowed to use."));
        }
        let out = dir.join(name);
        // An existing symlink could point anywhere; outputs never replace anything anyway.
        if std::fs::symlink_metadata(&out).is_ok() {
            return Err(PwError::new(
                ErrorCategory::OutputConflict,
                "output.exists",
                "A file with that name already exists; choose another name (existing files are never replaced).",
            ));
        }
        Ok(out)
    }

    /// An existing folder inside an approved folder (for operations that write several files).
    pub fn output_dir(&self, raw: &str) -> PwResult<PathBuf> {
        let p = self.absolute(raw)?;
        let c = canonical(&p).map_err(|e| PwError::from_io(&e, "output folder"))?;
        if !self.inside(&c) {
            return Err(denied("That folder is outside the folders PageWeave was allowed to use."));
        }
        if !c.is_dir() {
            return Err(PwError::invalid("mcp.not_a_dir", "That path is not a folder."));
        }
        Ok(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_and_outputs_stay_inside_the_roots() {
        let root = tempfile::tempdir().expect("tmp");
        let other = tempfile::tempdir().expect("tmp");
        std::fs::write(root.path().join("a.pdf"), b"%PDF").expect("w");
        std::fs::write(other.path().join("secret.pdf"), b"%PDF").expect("w");
        std::fs::create_dir(root.path().join("sub")).expect("mkdir");
        let r = Roots::new(&[root.path().to_path_buf()]).expect("roots");

        assert!(r.input("a.pdf").is_ok(), "relative to the first root");
        assert!(r.input(&root.path().join("a.pdf").to_string_lossy()).is_ok());
        let code = |e: PwError| e.code;
        assert_eq!(
            code(r.input(&other.path().join("secret.pdf").to_string_lossy()).expect_err("outside")),
            "mcp.path_denied"
        );
        assert_eq!(code(r.input("../secret.pdf").expect_err("traversal")), "io.not_found");
        let rel = format!("sub/../../{}/secret.pdf", other.path().file_name().expect("n").to_string_lossy());
        assert!(r.input(&rel).is_err(), "traversal out of the root");
        assert_eq!(code(r.input("sub").expect_err("dir")), "mcp.not_a_file");
        assert_eq!(code(r.input("").expect_err("empty")), "mcp.empty_path");

        assert!(r.output("sub/new.pdf", &["pdf"]).is_ok());
        assert_eq!(code(r.output("a.pdf", &["pdf"]).expect_err("exists")), "output.exists");
        assert_eq!(code(r.output("new.exe", &["pdf"]).expect_err("ext")), "mcp.output_extension");
        assert_eq!(code(r.output("sub/..", &["pdf"]).expect_err("dotdot")), "mcp.bad_output");
        assert_eq!(
            code(r.output(&other.path().join("x.pdf").to_string_lossy(), &["pdf"]).expect_err("outside")),
            "mcp.path_denied"
        );
        assert_eq!(code(r.output("missing/x.pdf", &["pdf"]).expect_err("no dir")), "io.not_found");
        assert!(r.output_dir("sub").is_ok());
        assert_eq!(code(r.output_dir(&other.path().to_string_lossy()).expect_err("outside")), "mcp.path_denied");
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_out_of_a_root_are_refused() {
        let root = tempfile::tempdir().expect("tmp");
        let other = tempfile::tempdir().expect("tmp");
        std::fs::write(other.path().join("secret.pdf"), b"%PDF").expect("w");
        std::os::unix::fs::symlink(other.path().join("secret.pdf"), root.path().join("link.pdf")).expect("ln");
        std::os::unix::fs::symlink(other.path(), root.path().join("door")).expect("ln");
        let r = Roots::new(&[root.path().to_path_buf()]).expect("roots");
        assert_eq!(r.input("link.pdf").expect_err("link").code, "mcp.path_denied");
        assert_eq!(r.output("door/x.pdf", &["pdf"]).expect_err("dir link").code, "mcp.path_denied");
        // A dangling or existing symlink is never written through.
        std::os::unix::fs::symlink(other.path().join("nope.pdf"), root.path().join("dangling.pdf")).expect("ln");
        assert_eq!(r.output("dangling.pdf", &["pdf"]).expect_err("dangling").code, "output.exists");
    }
}
