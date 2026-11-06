//! `wt remove`: worktree removal and the safe local/remote branch cleanup.

use std::collections::{BTreeMap, BTreeSet};
use std::io::BufRead;
use std::path::Path;

use crate::fnmatch::fnmatch;
use crate::git;
use crate::out::{emit_flush, green, red, reset, yellow};
use crate::paths;
use crate::repo::{self, Worktree};
use crate::say;

/// Matching worktrees for an identifier, by priority:
/// exact branch or exact path, then directory name, then path suffix.
pub fn find_by_identifier<'a>(worktrees: &'a [Worktree], identifier: &str) -> Vec<&'a Worktree> {
    let identifier_path = if identifier.contains('/') || identifier.starts_with('.') {
        Some(paths::resolve(Path::new(identifier)))
    } else {
        None
    };
    let mut exact = Vec::new();
    let mut dirname = Vec::new();
    let mut suffix = Vec::new();
    for wt in worktrees {
        if wt.branch == identifier {
            exact.push(wt);
            continue;
        }
        if identifier_path
            .as_ref()
            .is_some_and(|p| Path::new(&wt.path_str()) == p.as_path())
        {
            exact.push(wt);
            continue;
        }
        if wt.dir_name() == identifier {
            dirname.push(wt);
            continue;
        }
        if wt.path_str().ends_with(identifier) {
            suffix.push(wt);
        }
    }
    if !exact.is_empty() {
        exact
    } else if !dirname.is_empty() {
        dirname
    } else {
        suffix
    }
}

/// Worktrees whose branch, directory name or full path matches a glob.
pub fn find_by_pattern<'a>(worktrees: &'a [Worktree], pattern: &str) -> Vec<&'a Worktree> {
    worktrees
        .iter()
        .filter(|wt| {
            fnmatch(&wt.branch, pattern)
                || fnmatch(&wt.dir_name(), pattern)
                || fnmatch(&wt.path_str(), pattern)
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Delete,
    Keep,
    Error,
}

#[derive(Debug, Clone)]
pub struct Assessment {
    pub verdict: Verdict,
    pub reason: String,
    pub detail: String,
    pub hint: String,
}

impl Assessment {
    fn new(verdict: Verdict, reason: impl Into<String>) -> Self {
        Self {
            verdict,
            reason: reason.into(),
            detail: String::new(),
            hint: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BranchResult {
    Deleted,
    Kept,
    Error,
}

fn strip_star(line: &str) -> String {
    let t = line.trim();
    t.strip_prefix("* ").unwrap_or(t).trim().to_string()
}

/// Remote-tracking branches (other than origin/<branch> and the HEAD pointer)
/// that already contain the branch tip.
fn remote_branches_containing(branch: &str, bare_repo: &Path) -> BTreeSet<String> {
    let result = git::run(&["branch", "-r", "--contains", branch], Some(bare_repo));
    if !result.ok() {
        return BTreeSet::new();
    }
    let self_ref = format!("origin/{branch}");
    result
        .stdout
        .lines()
        .map(strip_star)
        .filter(|r| !r.is_empty() && *r != self_ref && !r.contains("->"))
        .collect()
}

/// Remote-tracking refs already merged into `reference` (computed once).
fn merged_remote_refs(bare_repo: &Path, reference: &str) -> BTreeSet<String> {
    let result = git::run(&["branch", "-r", "--merged", reference], Some(bare_repo));
    if !result.ok() {
        return BTreeSet::new();
    }
    result
        .stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(strip_star)
        .collect()
}

/// Squash/rebase-merge detection: merging `branch` into `target` would not
/// change `target`'s tree (needs `git merge-tree --write-tree`, Git 2.38+).
fn is_squash_merged(branch: &str, target: &str, bare_repo: &Path) -> bool {
    let result = git::run(
        &["merge-tree", "--write-tree", target, branch],
        Some(bare_repo),
    );
    if !result.ok() {
        return false;
    }
    let merge_tree = result
        .stdout
        .trim()
        .split('\n')
        .next()
        .unwrap_or("")
        .to_string();
    let tree_ref = format!("{target}^{{tree}}");
    let target_tree = git::run(&["rev-parse", tree_ref.as_str()], Some(bare_repo));
    target_tree.ok() && merge_tree == target_tree.stdout.trim()
}

/// Decide whether a branch that `git branch -d` refused can be force-deleted.
fn assess(
    bare_repo: &Path,
    branch: &str,
    default_ref: Option<&str>,
    merged: Option<&BTreeSet<String>>,
    refs_fresh: bool,
) -> Assessment {
    let Some(reference) = default_ref else {
        let mut a = Assessment::new(
            Verdict::Error,
            "unable to resolve remote default ref (origin/HEAD)",
        );
        a.hint = format!(
            "run `git -C {} remote set-head origin --auto` to initialise origin/HEAD",
            bare_repo.display()
        );
        return a;
    };
    let computed;
    let merged = match merged {
        Some(m) => m,
        None => {
            computed = merged_remote_refs(bare_repo, reference);
            &computed
        }
    };
    let remote_ref = format!("origin/{branch}");
    let remote_merged = merged.contains(&remote_ref);

    let ancestor = git::run(
        &["merge-base", "--is-ancestor", branch, reference],
        Some(bare_repo),
    );
    if ancestor.code != 0 && ancestor.code != 1 {
        let mut a = Assessment::new(
            Verdict::Error,
            format!("unable to verify ancestry against {reference}"),
        );
        a.detail = ancestor.stderr.trim().to_string();
        return a;
    }
    let local_merged = ancestor.code == 0;

    if remote_merged && local_merged {
        return Assessment::new(Verdict::Delete, "merged on remote");
    }
    if remote_merged {
        return Assessment::new(
            Verdict::Keep,
            format!("local branch has commits not in {reference}"),
        );
    }

    let remote_gone = if refs_fresh {
        !repo::has_git_ref(bare_repo, &remote_ref)
    } else {
        let ls = git::run(&["ls-remote", "--heads", "origin", branch], Some(bare_repo));
        if !ls.ok() {
            let mut a = Assessment::new(Verdict::Error, "unable to verify remote branch state");
            a.detail = ls.stderr.trim().to_string();
            return a;
        }
        ls.stdout.trim().is_empty()
    };

    if remote_gone && local_merged {
        return Assessment::new(Verdict::Delete, "remote branch gone, local merged");
    }
    if remote_gone {
        let preserved = remote_branches_containing(branch, bare_repo);
        if !preserved.is_empty() {
            let list: Vec<&str> = preserved.iter().map(String::as_str).collect();
            return Assessment::new(
                Verdict::Delete,
                format!(
                    "remote branch gone, commits preserved on {}",
                    list.join(", ")
                ),
            );
        }
        if is_squash_merged(branch, reference, bare_repo) {
            return Assessment::new(Verdict::Delete, "squash-merged");
        }
        return Assessment::new(
            Verdict::Keep,
            format!("remote branch gone, local commits not in {reference}"),
        );
    }
    Assessment::new(
        Verdict::Keep,
        format!("branch exists on origin and not merged into {reference}"),
    )
}

fn delete_local_branch(
    bare_repo: &Path,
    branch: &str,
    force: bool,
    default_ref: Option<&str>,
    merged: Option<&BTreeSet<String>>,
    refs_fresh: bool,
) -> BranchResult {
    let flag = if force { "-D" } else { "-d" };
    let result = git::run(&["branch", flag, branch], Some(bare_repo));
    if result.ok() {
        say!("  {}OK{} deleted local branch {branch}", green(), reset());
        return BranchResult::Deleted;
    }
    if force {
        say!(
            "  {}FAIL{} branch delete: {}",
            red(),
            reset(),
            result.stderr.trim()
        );
        return BranchResult::Error;
    }
    let a = assess(bare_repo, branch, default_ref, merged, refs_fresh);
    match a.verdict {
        Verdict::Delete => {
            let forced = git::run(&["branch", "-D", branch], Some(bare_repo));
            if forced.ok() {
                say!(
                    "  {}OK{} deleted local branch {branch} ({})",
                    green(),
                    reset(),
                    a.reason
                );
                BranchResult::Deleted
            } else {
                say!(
                    "  {}FAIL{} branch delete: {}",
                    red(),
                    reset(),
                    forced.stderr.trim()
                );
                BranchResult::Error
            }
        }
        Verdict::Keep => {
            say!("  {}WARN{} branch kept: {}", yellow(), reset(), a.reason);
            // The 1.x message used an em dash; it is kept byte for byte.
            say!(
                "    hint: not deleted to avoid losing commits \u{2014} force with `git -C {} branch -D {branch}` or `wt rm -f -b {branch}`",
                bare_repo.display()
            );
            BranchResult::Kept
        }
        Verdict::Error => {
            say!("  {}FAIL{} branch delete: {}", red(), reset(), a.reason);
            if !a.detail.is_empty() {
                say!("    {}", a.detail);
            }
            if !a.hint.is_empty() {
                say!("    hint: {}", a.hint);
            }
            BranchResult::Error
        }
    }
}

fn describe_dry_run(
    bare_repo: &Path,
    wt: &Worktree,
    force: bool,
    delete_remote: bool,
    default_ref: Option<&str>,
    merged: Option<&BTreeSet<String>>,
) -> String {
    let mut message = format!("  {}: worktree will be removed", wt.branch);
    if wt.branch.is_empty() || wt.branch.starts_with("(detached") {
        return message;
    }
    if force {
        message.push_str(" + local branch will be force-deleted");
        if delete_remote {
            message.push_str(" + remote branch will be deleted");
        }
        return message;
    }
    let a = assess(bare_repo, &wt.branch, default_ref, merged, false);
    match a.verdict {
        Verdict::Delete => {
            message.push_str(&format!(" + local branch will be deleted ({})", a.reason));
            if delete_remote {
                message.push_str(" + remote branch will be deleted");
            }
        }
        Verdict::Keep => {
            message.push_str(&format!(" + local branch will be kept ({})", a.reason));
            if delete_remote {
                message.push_str(" + remote branch will be skipped");
            }
        }
        Verdict::Error => {
            message.push_str(&format!(
                " + local branch delete cannot be verified ({})",
                a.reason
            ));
            if delete_remote {
                message.push_str(" + remote branch will be skipped");
            }
        }
    }
    message
}

/// Read one answer line; `None` on EOF or a read error.
fn read_answer() -> Option<String> {
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line),
    }
}

pub struct RemoveOptions<'a> {
    pub identifiers: &'a [String],
    pub patterns: &'a [String],
    pub force: bool,
    pub dry_run: bool,
    pub delete_branch: bool,
    pub delete_remote: bool,
    pub yes: bool,
}

pub fn cmd_remove(bare_repo: &Path, opts: &RemoveOptions) -> i32 {
    let worktrees = repo::get_worktrees(bare_repo);
    if worktrees.is_empty() {
        say!("No worktrees found.");
        return 1;
    }
    if opts.identifiers.is_empty() && opts.patterns.is_empty() {
        say!(
            "{}Error:{} Please provide identifier(s) or --match pattern(s).",
            red(),
            reset()
        );
        return 1;
    }

    // Keyed by path like the 1.x dict; order does not matter because targets
    // are sorted by (branch, path) below.
    let mut targets: BTreeMap<String, &Worktree> = BTreeMap::new();
    let mut missing_ids: Vec<&str> = Vec::new();
    let mut missing_patterns: Vec<&str> = Vec::new();

    for identifier in opts.identifiers {
        let matches = find_by_identifier(&worktrees, identifier);
        if matches.is_empty() {
            missing_ids.push(identifier);
        }
        for wt in matches {
            targets.insert(wt.path_str(), wt);
        }
    }
    for pattern in opts.patterns {
        let matches = find_by_pattern(&worktrees, pattern);
        if matches.is_empty() {
            missing_patterns.push(pattern);
        }
        for wt in matches {
            targets.insert(wt.path_str(), wt);
        }
    }

    if targets.is_empty() {
        say!("{}Error:{} No matching worktrees found.", red(), reset());
        if !missing_ids.is_empty() {
            say!("  identifiers: {}", missing_ids.join(", "));
        }
        if !missing_patterns.is_empty() {
            say!("  patterns: {}", missing_patterns.join(", "));
        }
        say!("\nAvailable worktrees:");
        for wt in &worktrees {
            say!("  {:<30}  {}", wt.branch, wt.path_str());
        }
        return 1;
    }

    let mut target_list: Vec<&Worktree> = targets.into_values().collect();
    target_list.sort_by(|a, b| (&a.branch, a.path_str()).cmp(&(&b.branch, b.path_str())));

    say!("Matched worktrees: {}", target_list.len());
    for wt in &target_list {
        say!("  {:<30}  {}", wt.branch, wt.path_str());
    }
    if !missing_ids.is_empty() {
        say!(
            "{}WARN{} no match for identifier(s): {}",
            yellow(),
            reset(),
            missing_ids.join(", ")
        );
    }
    if !missing_patterns.is_empty() {
        say!(
            "{}WARN{} no match for pattern(s): {}",
            yellow(),
            reset(),
            missing_patterns.join(", ")
        );
    }
    let partial_match = !missing_ids.is_empty() || !missing_patterns.is_empty();

    if opts.dry_run {
        let mut default_ref = None;
        let mut merged = None;
        if opts.delete_branch && !opts.force {
            default_ref = repo::get_default_remote_ref(bare_repo);
            if let Some(r) = &default_ref {
                merged = Some(merged_remote_refs(bare_repo, r));
            }
        }
        if opts.delete_branch {
            say!();
            for wt in &target_list {
                say!(
                    "{}",
                    describe_dry_run(
                        bare_repo,
                        wt,
                        opts.force,
                        opts.delete_remote,
                        default_ref.as_deref(),
                        merged.as_ref(),
                    )
                );
            }
        }
        say!();
        say!(
            "{}OK{} dry run complete (no changes made)",
            green(),
            reset()
        );
        return if partial_match { 2 } else { 0 };
    }

    if !opts.yes {
        emit_flush(&format!(
            "\nProceed with removal of {} worktree(s)? [y/N] ",
            target_list.len()
        ));
        let Some(answer) = read_answer() else {
            say!("\nAborted.");
            return 1;
        };
        let answer = answer.trim().to_lowercase();
        if answer != "y" && answer != "yes" {
            say!("Aborted.");
            return 1;
        }
    }

    // Fetch first so branch deletion sees remote merges. After a successful
    // prune-fetch the local origin/* refs are authoritative (refs_fresh).
    let mut default_ref = None;
    let mut merged = None;
    let mut refs_fresh = false;
    if opts.delete_branch {
        refs_fresh = git::run(&["fetch", "--all", "--prune"], Some(bare_repo)).ok();
        default_ref = repo::get_default_remote_ref(bare_repo);
        if let Some(r) = &default_ref {
            merged = Some(merged_remote_refs(bare_repo, r));
        }
    }

    say!();
    let (mut ok, mut skip, mut kept, mut fail) = (0, 0, 0, 0);
    for wt in &target_list {
        say!("Removing worktree: {}", wt.branch);
        say!("  Path: {}", wt.path_str());

        if !opts.force && repo::is_dirty(wt) {
            say!(
                "  {}SKIP{} uncommitted changes (use --force)",
                yellow(),
                reset()
            );
            skip += 1;
            say!();
            continue;
        }

        let path_str = wt.path_str();
        let mut args = vec!["worktree", "remove"];
        if opts.force {
            args.push("--force");
        }
        args.push(&path_str);
        let result = git::run(&args, Some(bare_repo));
        let branch_deletable = !wt.branch.is_empty() && !wt.branch.starts_with("(detached");
        if !result.ok() {
            say!(
                "  {}FAIL{} worktree 제거 실패: {}",
                red(),
                reset(),
                result.stderr.trim()
            );
            if opts.delete_branch && branch_deletable {
                say!(
                    "  {}SKIP{} worktree 제거 실패로 branch 삭제를 건너뜁니다 ({})",
                    yellow(),
                    reset(),
                    wt.branch
                );
            }
            fail += 1;
            say!();
            continue;
        }

        say!("  {}OK{} removed worktree", green(), reset());
        ok += 1;

        if opts.delete_branch && branch_deletable {
            let status = delete_local_branch(
                bare_repo,
                &wt.branch,
                opts.force,
                default_ref.as_deref(),
                merged.as_ref(),
                refs_fresh,
            );
            match status {
                BranchResult::Error => fail += 1,
                BranchResult::Kept => kept += 1,
                BranchResult::Deleted => {}
            }
            // Never drop the remote copy when the local branch survived:
            // it may be the only other copy of unmerged commits.
            if opts.delete_remote {
                if status == BranchResult::Deleted {
                    let rr = git::run(
                        &["push", "origin", "--delete", wt.branch.as_str()],
                        Some(bare_repo),
                    );
                    if rr.ok() {
                        say!(
                            "  {}OK{} deleted remote branch origin/{}",
                            green(),
                            reset(),
                            wt.branch
                        );
                    } else {
                        say!(
                            "  {}FAIL{} remote delete: {}",
                            red(),
                            reset(),
                            rr.stderr.trim()
                        );
                        fail += 1;
                    }
                } else {
                    say!(
                        "  {}WARN{} skipping remote delete: local branch {} was not deleted",
                        yellow(),
                        reset(),
                        wt.branch
                    );
                }
            }
        }
        say!();
    }

    say!("Summary: ok={ok} skip={skip} kept={kept} fail={fail}");
    if fail > 0 || skip > 0 || kept > 0 || partial_match {
        2
    } else {
        0
    }
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn wt(branch: &str, path: &str) -> Worktree {
        Worktree {
            path: PathBuf::from(path),
            branch: branch.into(),
            commit: "abcdef12".into(),
            is_detached: false,
        }
    }

    #[test]
    fn identifier_prefers_exact_branch() {
        let wts = [wt("main", "/r/main"), wt("feat/main", "/r/feat-main")];
        let hits = find_by_identifier(&wts, "main");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].branch, "main");
    }

    #[test]
    fn identifier_dirname_then_suffix() {
        let wts = [wt("feat/login", "/r/feat-login"), wt("x", "/r/x-login")];
        assert_eq!(
            find_by_identifier(&wts, "feat-login")[0].branch,
            "feat/login"
        );
        // Character suffix: both paths end with "login" (1.x behaviour).
        assert_eq!(find_by_identifier(&wts, "login").len(), 2);
    }

    #[test]
    fn pattern_matches_branch_dirname_or_path() {
        let wts = [wt("feat/a", "/r/feat-a"), wt("hotfix", "/r/hotfix")];
        assert_eq!(find_by_pattern(&wts, "feat/*").len(), 1);
        assert_eq!(find_by_pattern(&wts, "hot*").len(), 1);
        assert_eq!(find_by_pattern(&wts, "/r/*").len(), 2);
    }
}
