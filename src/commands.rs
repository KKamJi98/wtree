//! status, fetch, prune, pull, list and upstream.

use std::path::Path;

use crate::git;
use crate::json::{self, Value};
use crate::out::{blue, bold, colorize, green, red, reset, yellow};
use crate::parallel;
use crate::repo::{self, Worktree};
use crate::say;

fn ljust(text: &str, width: usize) -> String {
    format!("{text:<width$}")
}

fn char_len(text: &str) -> usize {
    text.chars().count()
}

/// SYNC column value for a worktree whose status was probed.
fn sync_value(wt: &Worktree, status: &repo::Status) -> String {
    if wt.is_detached {
        "detached".to_string()
    } else if status.upstream.is_some() {
        status.sync.clone().unwrap_or_default()
    } else {
        "no upstream".to_string()
    }
}

pub fn cmd_status(bare_repo: &Path, json_output: bool) -> i32 {
    let worktrees = repo::get_worktrees(bare_repo);
    if worktrees.is_empty() {
        if json_output {
            let empty = Value::Object(vec![
                ("worktrees".into(), Value::Array(vec![])),
                (
                    "summary".into(),
                    Value::Object(vec![
                        ("total".into(), Value::Int(0)),
                        ("clean".into(), Value::Int(0)),
                        ("dirty".into(), Value::Int(0)),
                    ]),
                ),
            ]);
            say!("{}", json::dumps_compact(&empty));
        } else {
            say!("No worktrees found.");
        }
        return 1;
    }

    // One git call per worktree, run concurrently.
    let statuses = parallel::map(&worktrees, repo::probe_status);

    if json_output {
        let mut dirty_count = 0;
        let items: Vec<Value> = worktrees
            .iter()
            .zip(&statuses)
            .map(|(wt, st)| {
                if st.dirty {
                    dirty_count += 1;
                }
                Value::Object(vec![
                    ("branch".into(), Value::Str(wt.branch.clone())),
                    ("path".into(), Value::Str(wt.path_str())),
                    ("commit".into(), Value::Str(wt.commit.clone())),
                    (
                        "status".into(),
                        Value::Str(if st.dirty { "dirty" } else { "clean" }.into()),
                    ),
                    ("sync".into(), Value::Str(sync_value(wt, st))),
                    ("detached".into(), Value::Bool(wt.is_detached)),
                ])
            })
            .collect();
        let total = items.len() as i64;
        let output = Value::Object(vec![
            ("worktrees".into(), Value::Array(items)),
            (
                "summary".into(),
                Value::Object(vec![
                    ("total".into(), Value::Int(total)),
                    ("clean".into(), Value::Int(total - dirty_count)),
                    ("dirty".into(), Value::Int(dirty_count)),
                ]),
            ),
        ]);
        say!("{}", json::dumps_indent(&output));
        return if dirty_count > 0 { 2 } else { 0 };
    }

    let max_branch = worktrees
        .iter()
        .map(|wt| char_len(&wt.branch))
        .max()
        .unwrap_or(0)
        .max(10);

    say!();
    say!(
        "{}{} {} {} PATH{}",
        bold(),
        ljust("STATUS", 8),
        ljust("BRANCH", max_branch),
        ljust("SYNC", 14),
        reset()
    );
    say!(
        "{} {} {} {}",
        "-".repeat(8),
        "-".repeat(max_branch),
        "-".repeat(14),
        "-".repeat(30)
    );

    let mut dirty_count = 0;
    for (wt, st) in worktrees.iter().zip(&statuses) {
        let status = if st.dirty {
            dirty_count += 1;
            colorize(&ljust("DIRTY", 8), red())
        } else {
            colorize(&ljust("CLEAN", 8), green())
        };
        let sync_raw = sync_value(wt, st);
        let sync = if sync_raw.starts_with('↓') || sync_raw == "detached" {
            colorize(&ljust(&sync_raw, 14), yellow())
        } else {
            ljust(&sync_raw, 14)
        };
        say!(
            "{} {} {} {}",
            status,
            ljust(&wt.branch, max_branch),
            sync,
            wt.path_str()
        );
    }

    say!();
    let clean_count = worktrees.len() - dirty_count;
    say!(
        "Summary: total={} {}clean={}{} {}dirty={}{}",
        worktrees.len(),
        green(),
        clean_count,
        reset(),
        red(),
        dirty_count,
        reset()
    );
    if dirty_count > 0 {
        2
    } else {
        0
    }
}

pub fn cmd_fetch(bare_repo: &Path) -> i32 {
    say!(
        "{}Fetching from bare repo: {}{}",
        blue(),
        bare_repo.display(),
        reset()
    );
    let result = git::run(&["fetch", "--all", "--prune"], Some(bare_repo));
    if !result.ok() {
        say!("{}FAIL{} fetch failed", red(), reset());
        if !result.stderr.is_empty() {
            // Printed untrimmed, as 1.x did.
            say!("{}", result.stderr);
        }
        return 1;
    }
    say!("{}OK{} fetch --all --prune completed", green(), reset());
    if !result.stderr.is_empty() {
        say!("{}", result.stderr.trim());
    }
    0
}

pub fn cmd_prune(bare_repo: &Path, dry_run: bool) -> i32 {
    let mut args = vec!["worktree", "prune"];
    if dry_run {
        args.push("--dry-run");
    }
    say!("{}Pruning worktree references...{}", blue(), reset());
    let result = git::run(&args, Some(bare_repo));
    if !result.ok() {
        say!("{}FAIL{} prune failed", red(), reset());
        if !result.stderr.is_empty() {
            say!("{}", result.stderr.trim());
        }
        return 1;
    }
    if dry_run {
        if !result.stdout.trim().is_empty() {
            say!("{}", result.stdout.trim());
        } else {
            say!("{}OK{} no stale references found", green(), reset());
        }
    } else {
        say!("{}OK{} pruned stale worktree references", green(), reset());
    }
    0
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Ok,
    Skip,
    Fail,
}

/// Sync one worktree; output is buffered so parallel runs never interleave.
fn pull_one(wt: &Worktree, rebase: bool) -> (Outcome, Vec<String>) {
    let mut lines = vec![format!("\n==> {} ({})", wt.branch, wt.dir_name())];
    let skip = |lines: &mut Vec<String>, why: &str| {
        lines.push(format!("  {}SKIP{} {why}", yellow(), reset()));
    };

    if wt.is_detached {
        skip(&mut lines, "detached HEAD");
        return (Outcome::Skip, lines);
    }
    if repo::is_dirty(wt) {
        skip(&mut lines, "dirty working tree");
        return (Outcome::Skip, lines);
    }
    let Some(upstream) = repo::get_upstream_ref(wt) else {
        skip(&mut lines, "no upstream configured");
        return (Outcome::Skip, lines);
    };
    if !repo::has_git_ref(&wt.path, &upstream) {
        skip(&mut lines, &format!("upstream ref not found {upstream}"));
        return (Outcome::Skip, lines);
    }

    let head = || {
        git::run(&["rev-parse", "HEAD"], Some(&wt.path))
            .stdout
            .trim()
            .to_string()
    };
    let before = head();

    if rebase {
        let result = git::run(&["rebase", upstream.as_str()], Some(&wt.path));
        if result.ok() {
            let msg = if before == head() {
                "already up to date"
            } else {
                "rebased"
            };
            lines.push(format!("  {}OK{} {msg}", green(), reset()));
            return (Outcome::Ok, lines);
        }
        git::run(&["rebase", "--abort"], Some(&wt.path));
        lines.push(format!(
            "  {}FAIL{} rebase failed (conflict?), aborted",
            red(),
            reset()
        ));
        return (Outcome::Fail, lines);
    }

    let result = git::run(&["merge", "--ff-only", upstream.as_str()], Some(&wt.path));
    if result.ok() {
        let msg = if before == head() {
            "already up to date"
        } else {
            "fast-forwarded"
        };
        lines.push(format!("  {}OK{} {msg}", green(), reset()));
        return (Outcome::Ok, lines);
    }
    lines.push(format!(
        "  {}FAIL{} cannot fast-forward (diverged?)",
        red(),
        reset()
    ));
    (Outcome::Fail, lines)
}

pub fn cmd_pull(bare_repo: &Path, rebase: bool) -> i32 {
    say!("{}Step 1: Fetch{}", bold(), reset());
    let fetched = cmd_fetch(bare_repo);
    if fetched != 0 {
        return fetched;
    }
    let mode = if rebase { "Rebase" } else { "Sync" };
    say!();
    say!("{}Step 2: {mode} worktrees{}", bold(), reset());

    let worktrees = repo::get_worktrees(bare_repo);
    if worktrees.is_empty() {
        say!("No worktrees found.");
        return 1;
    }

    // Each worktree updates only its own branch and working tree, so they can
    // run in parallel; output is printed afterwards in list order.
    let results = parallel::map(&worktrees, |wt| pull_one(wt, rebase));
    let (mut ok, mut skip, mut fail) = (0, 0, 0);
    for (outcome, lines) in results {
        say!("{}", lines.join("\n"));
        match outcome {
            Outcome::Ok => ok += 1,
            Outcome::Skip => skip += 1,
            Outcome::Fail => fail += 1,
        }
    }
    say!();
    say!("Summary: ok={ok} skip={skip} fail={fail}");
    if fail > 0 || skip > 0 {
        2
    } else {
        0
    }
}

pub fn cmd_list(bare_repo: &Path, json_output: bool) -> i32 {
    let worktrees = repo::get_worktrees(bare_repo);
    if worktrees.is_empty() {
        if json_output {
            say!("{}", json::dumps_indent(&Value::Array(vec![])));
        } else {
            say!("No worktrees found.");
        }
        return 1;
    }
    if json_output {
        let items = worktrees
            .iter()
            .map(|wt| {
                Value::Object(vec![
                    ("branch".into(), Value::Str(wt.branch.clone())),
                    ("path".into(), Value::Str(wt.path_str())),
                    ("commit".into(), Value::Str(wt.commit.clone())),
                    ("detached".into(), Value::Bool(wt.is_detached)),
                ])
            })
            .collect();
        say!("{}", json::dumps_indent(&Value::Array(items)));
        return 0;
    }
    let max_branch = worktrees
        .iter()
        .map(|wt| char_len(&wt.branch))
        .max()
        .unwrap_or(0);
    for wt in &worktrees {
        let branch = if wt.branch.starts_with("(detached") {
            colorize(&ljust(&wt.branch, max_branch), yellow())
        } else {
            ljust(&wt.branch, max_branch)
        };
        say!("  {}  {}  {}", branch, wt.commit, wt.path_str());
    }
    0
}

/// Read-only upstream assessment: (needs set-upstream, output lines).
fn upstream_assess(bare_repo: &Path, wt: &Worktree) -> (bool, Vec<String>) {
    let mut lines = vec![format!("==> {}", wt.branch)];
    if wt.is_detached {
        lines.push(format!("  {}SKIP{} detached HEAD", yellow(), reset()));
        return (false, lines);
    }
    if repo::get_upstream_ref(wt).is_some() {
        lines.push(format!("  {}SKIP{} upstream already set", blue(), reset()));
        return (false, lines);
    }
    if !repo::has_remote_branch(bare_repo, &wt.branch) {
        lines.push(format!(
            "  {}SKIP{} no remote branch origin/{}",
            yellow(),
            reset(),
            wt.branch
        ));
        return (false, lines);
    }
    (true, lines)
}

pub fn cmd_upstream(bare_repo: &Path) -> i32 {
    let worktrees = repo::get_worktrees(bare_repo);
    if worktrees.is_empty() {
        say!("No worktrees found.");
        return 1;
    }
    // Probes in parallel; the writes go one at a time because they all lock
    // the shared config file.
    let assessments = parallel::map(&worktrees, |wt| upstream_assess(bare_repo, wt));
    let (mut ok, mut skip, mut fail) = (0, 0, 0);
    for (wt, (needs_set, lines)) in worktrees.iter().zip(assessments) {
        for line in &lines {
            say!("{line}");
        }
        if !needs_set {
            skip += 1;
            continue;
        }
        let target = format!("origin/{}", wt.branch);
        let result = git::run(
            &[
                "branch",
                "--set-upstream-to",
                target.as_str(),
                wt.branch.as_str(),
            ],
            Some(&wt.path),
        );
        if result.ok() {
            say!("  {}OK{} set upstream to {target}", green(), reset());
            ok += 1;
        } else {
            say!("  {}FAIL{} {}", red(), reset(), result.stderr.trim());
            fail += 1;
        }
    }
    say!();
    say!("Summary: ok={ok} skip={skip} fail={fail}");
    if fail > 0 || skip > 0 {
        2
    } else {
        0
    }
}
