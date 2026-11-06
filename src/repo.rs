//! Locating the bare repository, listing worktrees and probing their state.

use std::path::{Path, PathBuf};

use crate::git;
use crate::out::{red, reset};
use crate::paths;
use crate::say;

#[derive(Debug, Clone)]
pub struct Worktree {
    /// Path exactly as git reported it.
    pub path: PathBuf,
    pub branch: String,
    pub commit: String,
    pub is_detached: bool,
}

impl Worktree {
    /// `str(Path(path))` as the 1.x CLI printed it.
    pub fn path_str(&self) -> String {
        paths::display(&self.path.to_string_lossy())
    }

    /// `Path(path).name`.
    pub fn dir_name(&self) -> String {
        paths::name(&self.path)
    }
}

/// dirty/upstream/sync state read from one `git status --porcelain=v2 --branch`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub dirty: bool,
    pub upstream: Option<String>,
    /// `None` when there is no upstream; "" when the upstream ref is missing.
    pub sync: Option<String>,
}

/// Find the bare repository from `start` (default: the current directory).
///
/// 1. Inside a worktree, `git rev-parse --git-common-dir` answers directly.
/// 2. Otherwise walk up looking for a `.bare` directory that git accepts.
pub fn find_bare_repo(start: Option<&Path>) -> Option<PathBuf> {
    let start = match start {
        Some(p) => p.to_path_buf(),
        None => std::env::current_dir().ok()?,
    };
    let result = git::run(&["rev-parse", "--git-common-dir"], Some(&start));
    if result.ok() {
        let common = PathBuf::from(result.stdout.trim());
        if common.is_absolute() {
            return Some(common);
        }
        return Some(paths::canonical(&start.join(common)));
    }

    let mut current = paths::canonical(&start);
    loop {
        let bare = current.join(".bare");
        if bare.is_dir() {
            // --resolve-git-dir rather than --is-bare-repository: the latter is
            // false once core.bare=false (set by `wt init` for starship), but
            // the layout is still a valid bare repository.
            let bare_str = bare.to_string_lossy().into_owned();
            if git::run(&["rev-parse", "--resolve-git-dir", &bare_str], None).ok() {
                return Some(bare);
            }
        }
        if !current.pop() {
            return None;
        }
    }
}

/// Parse `git worktree list --porcelain -z` output into worktrees, skipping
/// bare entries and paths that no longer exist.
pub fn parse_porcelain_z(raw: &str) -> Vec<Worktree> {
    let mut out = Vec::new();
    for record in raw.split("\0\0") {
        let mut path = None;
        let mut head = "";
        let mut branch = "";
        let mut bare = false;
        for field in record.split('\0') {
            if let Some(v) = field.strip_prefix("worktree ") {
                path = Some(v);
            } else if let Some(v) = field.strip_prefix("HEAD ") {
                head = v;
            } else if let Some(v) = field.strip_prefix("branch ") {
                branch = v;
            } else if field == "bare" {
                bare = true;
            }
        }
        let Some(path) = path else { continue };
        if bare {
            continue;
        }
        let commit: String = head.chars().take(8).collect();
        let mut branch = branch.replace("refs/heads/", "");
        let mut detached = false;
        if branch.is_empty() && !commit.is_empty() {
            branch = format!("(detached {commit})");
            detached = true;
        }
        let path = PathBuf::from(path);
        if !path.exists() {
            continue;
        }
        out.push(Worktree {
            path,
            branch,
            commit,
            is_detached: detached,
        });
    }
    out
}

/// All worktrees of `bare_repo`, excluding the bare directory itself.
///
/// With core.bare=false git no longer tags the bare entry as `bare`, so it is
/// filtered by path; otherwise `wt remove` could match and destroy it.
pub fn get_worktrees(bare_repo: &Path) -> Vec<Worktree> {
    list_worktrees(bare_repo, false)
}

/// [`get_worktrees`] without the failure line, for shell completion where
/// stdout carries the candidates.
pub fn get_worktrees_quiet(bare_repo: &Path) -> Vec<Worktree> {
    list_worktrees(bare_repo, true)
}

fn list_worktrees(bare_repo: &Path, quiet: bool) -> Vec<Worktree> {
    let result = git::run(&["worktree", "list", "--porcelain", "-z"], Some(bare_repo));
    if !result.ok() {
        if !quiet && !result.stderr.is_empty() {
            say!(
                "{}FAIL{} worktree list failed: {}",
                red(),
                reset(),
                result.stderr.trim()
            );
        }
        return Vec::new();
    }
    let bare_resolved = paths::canonical(bare_repo);
    parse_porcelain_z(&result.stdout)
        .into_iter()
        .filter(|wt| paths::canonical(&wt.path) != bare_resolved)
        .collect()
}

/// Uncommitted changes (untracked files included). A failed probe counts as
/// dirty so destructive commands skip safely.
pub fn is_dirty(wt: &Worktree) -> bool {
    let result = git::run(&["status", "--porcelain"], Some(&wt.path));
    if !result.ok() {
        return true;
    }
    !result.stdout.trim().is_empty()
}

/// The configured upstream (`@{u}`) of the worktree's branch.
pub fn get_upstream_ref(wt: &Worktree) -> Option<String> {
    let result = git::run(
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
        Some(&wt.path),
    );
    if !result.ok() {
        return None;
    }
    let upstream = result.stdout.trim();
    if upstream.is_empty() {
        None
    } else {
        Some(upstream.to_string())
    }
}

pub fn has_git_ref(repo: &Path, reference: &str) -> bool {
    git::run(&["rev-parse", "--verify", "--quiet", reference], Some(repo)).ok()
}

/// Render an ahead/behind pair.
pub fn sync_display(ahead: i64, behind: i64) -> String {
    match (ahead > 0, behind > 0) {
        (true, true) => format!("↑{ahead} ↓{behind}"),
        (true, false) => format!("↑{ahead}"),
        (false, true) => format!("↓{behind}"),
        (false, false) => "=".to_string(),
    }
}

/// Render a porcelain v2 `branch.ab` value (`+<ahead> -<behind>`). Git omits
/// it when the upstream is configured but its ref is missing locally.
pub fn sync_from_ab(ab: Option<&str>) -> String {
    let Some(ab) = ab.filter(|s| !s.is_empty()) else {
        return String::new();
    };
    let parts: Vec<&str> = ab.split_whitespace().collect();
    if parts.len() != 2 {
        return String::new();
    }
    match (parts[0].parse::<i64>(), parts[1].parse::<i64>()) {
        (Ok(ahead), Ok(behind)) => sync_display(ahead, -behind),
        _ => String::new(),
    }
}

/// Parse `git status --porcelain=v2 --branch` output.
pub fn parse_status_v2(stdout: &str) -> Status {
    let mut dirty = false;
    let mut upstream = None;
    let mut ab = None;
    for line in stdout.lines() {
        if let Some(v) = line.strip_prefix("# branch.upstream ") {
            upstream = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("# branch.ab ") {
            ab = Some(v.trim().to_string());
        } else if !line.is_empty() && !line.starts_with('#') {
            dirty = true;
        }
    }
    let sync = upstream.as_ref().map(|_| sync_from_ab(ab.as_deref()));
    Status {
        dirty,
        upstream,
        sync,
    }
}

/// dirty/upstream/sync for one worktree in a single git call.
pub fn probe_status(wt: &Worktree) -> Status {
    let result = git::run(&["status", "--porcelain=v2", "--branch"], Some(&wt.path));
    if !result.ok() {
        return Status {
            dirty: true,
            upstream: None,
            sync: Some(String::new()),
        };
    }
    parse_status_v2(&result.stdout)
}

pub fn has_remote_branch(bare_repo: &Path, branch: &str) -> bool {
    let pattern = format!("origin/{branch}");
    !git::run(&["branch", "-r", "--list", &pattern], Some(bare_repo))
        .stdout
        .trim()
        .is_empty()
}

pub fn has_local_branch(bare_repo: &Path, branch: &str) -> bool {
    !git::run(&["branch", "--list", branch], Some(bare_repo))
        .stdout
        .trim()
        .is_empty()
}

/// Default branch name of `origin`.
pub fn get_default_branch(bare_repo: &Path) -> String {
    let result = git::run(
        &["symbolic-ref", "refs/remotes/origin/HEAD"],
        Some(bare_repo),
    );
    if result.ok() {
        // refs/remotes/origin/release/2026 -> release/2026 (1.x kept only the
        // last component and turned it into "2026").
        let full = result.stdout.trim();
        return full
            .strip_prefix("refs/remotes/origin/")
            .unwrap_or(full)
            .to_string();
    }
    for branch in ["main", "master"] {
        let pattern = format!("origin/{branch}");
        let r = git::run(&["branch", "-r", "--list", &pattern], Some(bare_repo));
        if !r.stdout.trim().is_empty() {
            return branch.to_string();
        }
    }
    "main".to_string()
}

/// The remote default ref (e.g. `origin/main`), or `None` when neither
/// `origin/HEAD` nor a verified fallback branch exists.
pub fn get_default_remote_ref(bare_repo: &Path) -> Option<String> {
    let sym = git::run(
        &["symbolic-ref", "refs/remotes/origin/HEAD"],
        Some(bare_repo),
    );
    if sym.ok() {
        if let Some(rest) = sym.stdout.trim().strip_prefix("refs/remotes/") {
            return Some(rest.to_string());
        }
    }
    let candidate = format!("origin/{}", get_default_branch(bare_repo));
    if git::run(&["rev-parse", "--verify", &candidate], Some(bare_repo)).ok() {
        Some(candidate)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_from_ab_cases() {
        assert_eq!(sync_from_ab(Some("+0 -0")), "=");
        assert_eq!(sync_from_ab(Some("+2 -0")), "↑2");
        assert_eq!(sync_from_ab(Some("+0 -3")), "↓3");
        assert_eq!(sync_from_ab(Some("+1 -4")), "↑1 ↓4");
        assert_eq!(sync_from_ab(None), "");
        assert_eq!(sync_from_ab(Some("garbage")), "");
    }

    #[test]
    fn status_v2_parsing() {
        let clean = "# branch.oid abc\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +1 -0\n";
        assert_eq!(
            parse_status_v2(clean),
            Status {
                dirty: false,
                upstream: Some("origin/main".into()),
                sync: Some("↑1".into())
            }
        );
        let dirty_no_upstream = "# branch.oid abc\n# branch.head x\n? new.txt\n";
        assert_eq!(
            parse_status_v2(dirty_no_upstream),
            Status {
                dirty: true,
                upstream: None,
                sync: None
            }
        );
        let missing_ref = "# branch.upstream origin/gone\n";
        assert_eq!(parse_status_v2(missing_ref).sync, Some(String::new()));
    }

    #[test]
    fn porcelain_z_skips_bare_and_missing() {
        let tmp = std::env::temp_dir();
        let existing = tmp.to_string_lossy();
        let raw = format!(
            "worktree /nonexistent/bare\0bare\0\0worktree {existing}\0HEAD 0123456789abcdef\0branch refs/heads/feat/x\0\0worktree {existing}\0HEAD fedcba9876543210\0detached\0\0worktree /nonexistent/gone\0HEAD 00\0branch refs/heads/y\0\0"
        );
        let wts = parse_porcelain_z(&raw);
        assert_eq!(wts.len(), 2);
        assert_eq!(wts[0].branch, "feat/x");
        assert_eq!(wts[0].commit, "01234567");
        assert!(!wts[0].is_detached);
        assert_eq!(wts[1].branch, "(detached fedcba98)");
        assert!(wts[1].is_detached);
    }
}
