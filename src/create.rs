//! `wt init` and `wt add`.

use std::path::{Path, PathBuf};

use crate::git;
use crate::out::{blue, bold, fatal, green, red, reset, yellow};
use crate::paths;
use crate::repo;
use crate::say;

/// Directory name `wt init` derives from a clone URL.
pub fn repo_dir_name(url: &str) -> String {
    let name = url.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    name.strip_suffix(".git").unwrap_or(name).to_string()
}

fn dir_is_nonempty(path: &Path) -> bool {
    match std::fs::read_dir(path) {
        Ok(mut entries) => entries.next().is_some(),
        Err(err) => fatal(&format!("cannot read '{}': {err}", path.display())),
    }
}

fn set_upstream(branch: &str, cwd: &Path) -> bool {
    let target = format!("origin/{branch}");
    git::run(
        &["branch", "--set-upstream-to", target.as_str(), branch],
        Some(cwd),
    )
    .ok()
}

pub fn cmd_init(repo_url: &str, path: Option<&str>, worktrees: Option<&[String]>) -> i32 {
    let path = match path {
        Some(p) => p.to_string(),
        None => repo_dir_name(repo_url),
    };
    let target = paths::resolve(Path::new(&path));
    let bare = target.join(".bare");
    let bare_str = bare.to_string_lossy().into_owned();

    if target.exists() && dir_is_nonempty(&target) {
        say!(
            "{}Error:{} Directory '{}' already exists and is not empty.",
            red(),
            reset(),
            target.display()
        );
        return 1;
    }

    say!("{}Initializing worktree repository{}", bold(), reset());
    say!("  URL:    {repo_url}");
    say!("  Path:   {}", target.display());
    say!();

    say!("{}Step 1:{} Clone bare repository", bold(), reset());
    if let Err(err) = std::fs::create_dir_all(&target) {
        fatal(&format!("cannot create '{}': {err}", target.display()));
    }
    let result = git::run(&["clone", "--bare", repo_url, bare_str.as_str()], None);
    if !result.ok() {
        say!("  {}FAIL{} git clone --bare failed", red(), reset());
        if !result.stderr.is_empty() {
            say!("  {}", result.stderr.trim());
        }
        return 1;
    }
    say!("  {}OK{} cloned to {}", green(), reset(), bare.display());

    say!();
    say!("{}Step 2:{} Configure fetch refspec", bold(), reset());
    let refspec = git::run(
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ],
        Some(&bare),
    );
    if refspec.ok() {
        say!("  {}OK{} configured fetch refspec", green(), reset());
    } else {
        say!(
            "  {}WARN{} failed to configure fetch refspec",
            yellow(),
            reset()
        );
    }
    if git::run(&["fetch", "--all"], Some(&bare)).ok() {
        say!("  {}OK{} fetched all branches", green(), reset());
    }

    say!();
    say!("{}Step 3:{} Create worktrees", bold(), reset());
    let default_branch = repo::get_default_branch(&bare);
    say!("  Default branch: {default_branch}");

    // A branch like release/2026 gets a flat directory, as with -w.
    let default_dir = default_branch.replace('/', "-");
    let default_wt = target.join(&default_dir);
    let default_wt_str = default_wt.to_string_lossy().into_owned();
    let result = git::run(
        &[
            "worktree",
            "add",
            default_wt_str.as_str(),
            default_branch.as_str(),
        ],
        Some(&bare),
    );
    if !result.ok() {
        say!(
            "  {}FAIL{} failed to create worktree for {default_branch}",
            red(),
            reset()
        );
        if !result.stderr.is_empty() {
            say!("  {}", result.stderr.trim());
        }
        return 1;
    }
    say!(
        "  {}OK{} created worktree: {default_dir}/",
        green(),
        reset()
    );
    if set_upstream(&default_branch, &default_wt) {
        say!(
            "  {}OK{} set upstream to origin/{default_branch}",
            green(),
            reset()
        );
    } else {
        say!(
            "  {}WARN{} failed to set upstream for {default_branch}",
            yellow(),
            reset()
        );
    }

    let mut created: Vec<String> = vec![default_branch.clone()];

    say!();
    say!(
        "{}Step 4:{} Create common branch worktrees",
        bold(),
        reset()
    );
    for branch in ["main", "master", "staging"] {
        if created.iter().any(|b| b == branch) || !repo::has_remote_branch(&bare, branch) {
            continue;
        }
        let wt_path = target.join(branch);
        let wt_str = wt_path.to_string_lossy().into_owned();
        let result = git::run(&["worktree", "add", wt_str.as_str(), branch], Some(&bare));
        if !result.ok() {
            say!(
                "  {}WARN{} {branch} - {}",
                yellow(),
                reset(),
                result.stderr.trim()
            );
        } else {
            say!("  {}OK{} created worktree: {branch}/", green(), reset());
            if !set_upstream(branch, &wt_path) {
                say!(
                    "  {}WARN{} failed to set upstream for {branch}",
                    yellow(),
                    reset()
                );
            }
            created.push(branch.to_string());
        }
    }
    if created.len() == 1 {
        say!(
            "  {}INFO{} no additional common branches found on remote",
            blue(),
            reset()
        );
    }

    if let Some(extra) = worktrees.filter(|w| !w.is_empty()) {
        say!();
        say!("{}Step 5:{} Create additional worktrees", bold(), reset());
        for raw in extra {
            let branch = raw.trim();
            if branch.is_empty() || created.iter().any(|b| b == branch) {
                continue;
            }
            let wt_path = target.join(branch.replace('/', "-"));
            if !repo::has_remote_branch(&bare, branch) {
                say!("  {}SKIP{} {branch} (no remote branch)", yellow(), reset());
                continue;
            }
            let wt_str = wt_path.to_string_lossy().into_owned();
            let result = git::run(&["worktree", "add", wt_str.as_str(), branch], Some(&bare));
            if !result.ok() {
                say!(
                    "  {}WARN{} {branch} - {}",
                    yellow(),
                    reset(),
                    result.stderr.trim()
                );
            } else {
                say!(
                    "  {}OK{} created worktree: {}/",
                    green(),
                    reset(),
                    paths::name(&wt_path)
                );
                if set_upstream(branch, &wt_path) {
                    say!("  {}OK{} set upstream to origin/{branch}", green(), reset());
                } else {
                    say!(
                        "  {}WARN{} failed to set upstream for {branch}",
                        yellow(),
                        reset()
                    );
                }
                created.push(branch.to_string());
            }
        }
    }

    // starship's git_status skips any repo with core.bare=true, even from a
    // linked worktree. Set it only after the first worktrees exist; earlier,
    // git would treat .bare as using the default branch worktree.
    if git::run(&["config", "core.bare", "false"], Some(&bare)).ok() {
        say!("  {}OK{} set core.bare=false", green(), reset());
    } else {
        say!(
            "  {}WARN{} failed to set core.bare=false",
            yellow(),
            reset()
        );
    }

    say!();
    say!("{}✓ Initialization complete!{}", green(), reset());
    say!();
    say!("Next steps:");
    say!("  cd {}/{default_dir}", target.display());
    say!("  wt status");
    say!();
    say!("To add more worktrees:");
    say!("  cd {}", bare.display());
    say!("  git worktree add ../<branch-name> <branch-name>");
    0
}

pub fn cmd_add(
    bare_repo: &Path,
    branch: &str,
    path: Option<&str>,
    create: bool,
    base: Option<&str>,
) -> i32 {
    let wt_path: PathBuf = match path {
        None => bare_repo
            .parent()
            .unwrap_or(Path::new("/"))
            .join(branch.replace('/', "-")),
        Some(p) => paths::resolve(Path::new(p)),
    };
    let wt_str = wt_path.to_string_lossy().into_owned();

    if wt_path.exists() {
        say!(
            "{}Error:{} Path '{}' already exists.",
            red(),
            reset(),
            wt_path.display()
        );
        return 1;
    }

    let remote_exists = repo::has_remote_branch(bare_repo, branch);
    let local_exists = repo::has_local_branch(bare_repo, branch);

    let mut create = create;
    if !create && !remote_exists && !local_exists {
        create = true;
        say!(
            "{}Note:{} Branch '{branch}' does not exist. Creating new branch.",
            yellow(),
            reset()
        );
    }

    say!("Adding worktree for branch: {branch}");
    say!("  Path: {}", wt_path.display());
    if let Some(b) = base.filter(|b| !b.is_empty()) {
        say!("  Base: {b}");
    }
    say!();

    let remote_ref = format!("origin/{branch}");
    let result = if create {
        if let Some(b) = base.filter(|b| !b.is_empty()) {
            let base_ref = if repo::has_remote_branch(bare_repo, b) {
                format!("origin/{b}")
            } else {
                b.to_string()
            };
            git::run(
                &[
                    "worktree",
                    "add",
                    "-b",
                    branch,
                    wt_str.as_str(),
                    base_ref.as_str(),
                ],
                Some(bare_repo),
            )
        } else if remote_exists {
            git::run(
                &[
                    "worktree",
                    "add",
                    "--track",
                    "-b",
                    branch,
                    wt_str.as_str(),
                    remote_ref.as_str(),
                ],
                Some(bare_repo),
            )
        } else {
            git::run(
                &["worktree", "add", "-b", branch, wt_str.as_str()],
                Some(bare_repo),
            )
        }
    } else if remote_exists && !local_exists {
        git::run(
            &[
                "worktree",
                "add",
                "--track",
                "-b",
                branch,
                wt_str.as_str(),
                remote_ref.as_str(),
            ],
            Some(bare_repo),
        )
    } else {
        git::run(
            &["worktree", "add", wt_str.as_str(), branch],
            Some(bare_repo),
        )
    };

    if !result.ok() {
        say!("{}FAIL{} {}", red(), reset(), result.stderr.trim());
        return 1;
    }
    say!(
        "{}OK{} created worktree at {}",
        green(),
        reset(),
        wt_path.display()
    );

    if remote_exists {
        if set_upstream(branch, &wt_path) {
            say!("{}OK{} set upstream to origin/{branch}", green(), reset());
        } else {
            say!(
                "{}WARN{} failed to set upstream for {branch}",
                yellow(),
                reset()
            );
        }
    } else if create {
        say!();
        say!(
            "{}Note:{} New branch created. To push and set upstream:",
            yellow(),
            reset()
        );
        say!("  cd {}", wt_path.display());
        say!("  git push -u origin {branch}");
    }
    0
}

#[cfg(test)]
mod tests {
    use super::repo_dir_name;

    #[test]
    fn dir_name_from_url() {
        assert_eq!(repo_dir_name("git@github.com:org/repo.git"), "repo");
        assert_eq!(repo_dir_name("https://github.com/org/repo/"), "repo");
        // 1.x behaviour, fixed in 2.0 (see docs/rust-migration.md, bug 8).
        assert_eq!(repo_dir_name("git@host:repo.git"), "git@host:repo");
    }
}
