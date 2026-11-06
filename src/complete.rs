//! Dynamic shell completion candidates (clap_complete `CompleteEnv`).
//!
//! The 1.x CLI used argcomplete with a substring, case-insensitive validator,
//! so these completers filter the same way instead of by prefix. zsh's
//! `_describe` still applies the user's matcher-list on top.

use std::ffi::OsStr;

use clap_complete::engine::CompletionCandidate;

use crate::git;
use crate::repo;

fn needle(current: &OsStr) -> String {
    current.to_string_lossy().to_lowercase()
}

/// `wt rm` identifiers: worktree directory names. Branch names would break
/// shell words at `/` (`feat/topic`), and `rm` resolves directory names too.
pub fn worktree_identifiers(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(bare) = repo::find_bare_repo(None) else {
        return Vec::new();
    };
    let prefix = needle(current);
    let mut seen = Vec::new();
    for wt in repo::get_worktrees_quiet(&bare) {
        let dirname = wt.dir_name();
        if !seen.contains(&dirname)
            && (dirname.to_lowercase().contains(&prefix)
                || wt.branch.to_lowercase().contains(&prefix))
        {
            seen.push(dirname);
        }
    }
    seen.into_iter().map(CompletionCandidate::new).collect()
}

/// `wt add <branch>` and `--base`: remote branch names without `origin/`.
pub fn remote_branches(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(bare) = repo::find_bare_repo(None) else {
        return Vec::new();
    };
    let result = git::run(&["branch", "-r", "--format=%(refname:short)"], Some(&bare));
    if !result.ok() {
        return Vec::new();
    }
    let prefix = needle(current);
    result
        .stdout
        .trim()
        .split('\n')
        .filter_map(|line| line.strip_prefix("origin/"))
        .filter(|b| *b != "HEAD" && b.to_lowercase().contains(&prefix))
        .map(CompletionCandidate::new)
        .collect()
}
