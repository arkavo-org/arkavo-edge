//! Workspace confinement shared by the tools that take a path from the model.
//!
//! Each tool's zero-config root is the process working directory, because an
//! agent process runs with its workspace as its working directory; embedders
//! that know better build the tool with `with_root`.

use std::path::{Path, PathBuf};

use crate::{Result, ToolError};

/// Resolve a model-supplied path inside `root`, or refuse the call. Every
/// file, TDF and git tool goes through this one mapping so an escape is always
/// a `PolicyDenied` and never a tool-specific error a caller might retry.
pub(crate) fn within_root(root: &Path, requested: &str) -> Result<PathBuf> {
    arkavo_validation::resolve_within_root(root, requested)
        .map_err(|e| ToolError::PolicyDenied(e.to_string()))
}

/// Require an already-existing `location` to live inside `root`, comparing
/// canonical forms so symlinked temp roots (macOS `/var`) still match. Used
/// where a library resolves a location itself (git repository discovery walks
/// up to an enclosing repository), so the path the caller passed proves nothing
/// about what was actually opened.
pub(crate) fn require_inside_root(root: &Path, location: &Path) -> Result<()> {
    let canonical_root = std::fs::canonicalize(root)
        .map_err(|e| ToolError::PolicyDenied(format!("workspace root: {e}")))?;
    let canonical = std::fs::canonicalize(location)
        .map_err(|e| ToolError::PolicyDenied(format!("{}: {e}", location.display())))?;
    if canonical.starts_with(&canonical_root) {
        Ok(())
    } else {
        Err(ToolError::PolicyDenied(format!(
            "{} is outside allowed root {}",
            canonical.display(),
            canonical_root.display()
        )))
    }
}

/// Repository internals a workspace-controlled symlink could redirect into
/// another repository's objects, refs or index.
const GIT_INTERNALS: [&str; 7] = [
    "objects",
    "objects/info",
    "refs",
    "HEAD",
    "index",
    "packed-refs",
    "logs",
];

/// Require every location an opened repository reads or writes to lie inside
/// `root`: its working directory (or git directory when bare), its git
/// directory, and its common directory. A workspace-controlled `.git` can name
/// an outer repository through `commondir` (refs and objects are then written
/// there), so the working directory alone proves nothing. A legitimate linked
/// worktree passes only when its main repository is inside the root too.
///
/// Object alternates and symlinked core internals are refused outright rather
/// than resolved: libgit2's alternates resolution (nested chains, non-dot
/// relative entries against the process cwd) is not something a path check
/// should mirror. Other crafted-`.git` vectors are a residual for OS
/// confinement.
pub(crate) fn require_repo_inside_root(root: &Path, repo: &git2::Repository) -> Result<()> {
    require_inside_root(root, repo.workdir().unwrap_or_else(|| repo.path()))?;
    let common = repo.commondir();
    require_inside_root(root, repo.path())?;
    require_inside_root(root, common)?;
    require_no_alternates(common)?;
    for dir in [repo.path(), common] {
        require_no_symlinked_internals(dir)?;
    }
    Ok(())
}

fn require_no_alternates(common: &Path) -> Result<()> {
    let listing = match std::fs::read_to_string(common.join("objects/info/alternates")) {
        Ok(listing) => listing,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(ToolError::PolicyDenied(format!(
                "objects/info/alternates: {e}"
            )));
        }
    };
    if listing
        .lines()
        .map(str::trim)
        .any(|line| !line.is_empty() && !line.starts_with('#'))
    {
        return Err(ToolError::PolicyDenied(
            "repositories with object alternates are not supported inside the workspace"
                .to_string(),
        ));
    }
    Ok(())
}

fn require_no_symlinked_internals(git_dir: &Path) -> Result<()> {
    for name in GIT_INTERNALS {
        let entry = git_dir.join(name);
        match std::fs::symlink_metadata(&entry) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(ToolError::PolicyDenied(format!(
                    "{} is a symlink; git internals must not be symlinks inside the workspace",
                    entry.display()
                )));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(ToolError::PolicyDenied(format!("{}: {e}", entry.display())));
            }
        }
    }
    Ok(())
}

/// A workspace `.git` (directory, or the file linked worktrees use) decides
/// which repository the git tools open and where they write. A file-tool write
/// there could re-point it at another repository, so mutations refuse any path
/// with a `.git` component; reads stay allowed.
pub(crate) fn refuse_git_component(resolved: &Path) -> Result<()> {
    let has_git = resolved
        .components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case(".git"));
    if has_git {
        Err(ToolError::PolicyDenied(format!(
            "{} is inside a .git entry; file tools cannot modify it",
            resolved.display()
        )))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn escape_maps_to_policy_denied() {
        let ws = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let err = within_root(ws.path(), outside.path().to_str().unwrap()).unwrap_err();
        assert!(matches!(err, ToolError::PolicyDenied(_)), "{err}");
    }

    #[test]
    fn require_inside_root_accepts_inside_and_refuses_outside_or_missing() {
        let ws = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        std::fs::create_dir(ws.path().join("sub")).unwrap();
        assert!(require_inside_root(ws.path(), &ws.path().join("sub")).is_ok());
        for bad in [outside.path().to_path_buf(), ws.path().join("missing")] {
            assert!(matches!(
                require_inside_root(ws.path(), &bad),
                Err(ToolError::PolicyDenied(_))
            ));
        }
        assert!(matches!(
            require_inside_root(&ws.path().join("missing"), ws.path()),
            Err(ToolError::PolicyDenied(_))
        ));
    }

    fn write_alternates(objects: &Path, contents: &str) {
        std::fs::create_dir_all(objects.join("info")).unwrap();
        std::fs::write(objects.join("info/alternates"), contents).unwrap();
    }

    #[test]
    fn any_alternates_entry_is_refused_but_blank_and_comment_only_files_pass() {
        let ws = TempDir::new().unwrap();
        let common = ws.path().join(".git");
        for entry in [
            "/abs/elsewhere/objects",
            "../../objects",
            "sibling/objects",
            "\"quoted\"",
            "objects2",
        ] {
            write_alternates(&common.join("objects"), &format!("{entry}\n"));
            assert!(
                matches!(
                    require_no_alternates(&common),
                    Err(ToolError::PolicyDenied(_))
                ),
                "{entry}"
            );
        }
        write_alternates(&common.join("objects"), "\n# nothing here\n  \n");
        assert!(require_no_alternates(&common).is_ok());
        assert!(require_no_alternates(&ws.path().join("no-such-git-dir")).is_ok());
    }

    #[test]
    fn git_component_is_refused_case_insensitively_and_other_paths_pass() {
        for bad in [
            "/w/.git",
            "/w/.git/objects/info/alternates",
            "/w/sub/.GIT/x",
        ] {
            assert!(
                matches!(
                    refuse_git_component(Path::new(bad)),
                    Err(ToolError::PolicyDenied(_))
                ),
                "{bad}"
            );
        }
        for ok in ["/w/.github/x", "/w/src/lib.rs", "/w/gitignore", "/w/a.git"] {
            assert!(refuse_git_component(Path::new(ok)).is_ok(), "{ok}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_git_internals_are_refused_and_missing_ones_pass() {
        let ws = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let git_dir = ws.path().join(".git");
        std::fs::create_dir(&git_dir).unwrap();
        assert!(require_no_symlinked_internals(&git_dir).is_ok());
        for name in ["objects", "refs", "HEAD", "index", "packed-refs", "logs"] {
            let link = git_dir.join(name);
            std::os::unix::fs::symlink(outside.path(), &link).unwrap();
            assert!(
                matches!(
                    require_no_symlinked_internals(&git_dir),
                    Err(ToolError::PolicyDenied(_))
                ),
                "{name}"
            );
            std::fs::remove_file(&link).unwrap();
        }
        // `objects/info` is checked separately from `objects`.
        std::fs::create_dir_all(git_dir.join("objects")).unwrap();
        std::os::unix::fs::symlink(outside.path(), git_dir.join("objects/info")).unwrap();
        assert!(require_no_symlinked_internals(&git_dir).is_err());
    }

    #[test]
    fn empty_path_is_denied_and_inside_path_resolves() {
        let ws = TempDir::new().unwrap();
        assert!(matches!(
            within_root(ws.path(), ""),
            Err(ToolError::PolicyDenied(_))
        ));
        let root = std::fs::canonicalize(ws.path()).unwrap();
        assert_eq!(within_root(&root, "a/b.txt").unwrap(), root.join("a/b.txt"));
    }
}
