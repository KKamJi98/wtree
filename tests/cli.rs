//! Integration tests against real git repositories.
//!
//! The 1.x pytest suite mocked `run_git`; these build actual repositories in a
//! temp dir and run the `wt` binary, so git's own behaviour is part of what is
//! verified. Byte-level parity with 1.x lives in `scripts/golden.py`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use assert_cmd::cargo::CommandCargoExt;
use tempfile::TempDir;

struct Env {
    _tmp: TempDir,
    root: PathBuf,
}

impl Env {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let real = tmp.path().canonicalize().unwrap();
        // Windows canonicalize yields a `\\?\` verbatim path that git rejects.
        let text = real.to_string_lossy().into_owned();
        let root = PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text));
        fs::create_dir_all(root.join("home")).unwrap();
        fs::write(
            root.join("home/.gitconfig"),
            "[init]\n\tdefaultBranch = main\n[advice]\n\tdetachedHead = false\n",
        )
        .unwrap();
        Env { _tmp: tmp, root }
    }

    fn apply(&self, cmd: &mut Command) {
        cmd.env("HOME", self.root.join("home"))
            .env("GIT_CONFIG_GLOBAL", self.root.join("home/.gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "T")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "T")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .env_remove("NO_COLOR")
            .env_remove("COMPLETE");
    }

    fn git(&self, cwd: &Path, args: &[&str]) -> String {
        let mut cmd = Command::new("git");
        self.apply(&mut cmd);
        let out = cmd.args(args).current_dir(cwd).output().unwrap();
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn commit(&self, repo: &Path, file: &str, msg: &str) {
        fs::write(repo.join(file), msg).unwrap();
        self.git(repo, &["add", file]);
        self.git(repo, &["commit", "-q", "-m", msg]);
    }

    /// origin.git with the given branches, and proj/ laid out like `wt init`.
    fn layout(&self, branches: &[&str], worktrees: &[&str]) -> PathBuf {
        let src = self.root.join("src");
        self.git(
            &self.root,
            &["init", "-q", "-b", "main", src.to_str().unwrap()],
        );
        self.commit(&src, "a.txt", "init");
        for b in branches.iter().filter(|b| **b != "main") {
            self.git(&src, &["branch", b]);
        }
        let origin = self.root.join("origin.git");
        self.git(
            &self.root,
            &[
                "clone",
                "-q",
                "--bare",
                src.to_str().unwrap(),
                origin.to_str().unwrap(),
            ],
        );
        let proj = self.root.join("proj");
        let bare = proj.join(".bare");
        fs::create_dir_all(&proj).unwrap();
        self.git(
            &self.root,
            &[
                "clone",
                "-q",
                "--bare",
                origin.to_str().unwrap(),
                bare.to_str().unwrap(),
            ],
        );
        self.git(
            &bare,
            &[
                "config",
                "remote.origin.fetch",
                "+refs/heads/*:refs/remotes/origin/*",
            ],
        );
        self.git(&bare, &["fetch", "-q", "--all"]);
        self.git(&bare, &["remote", "set-head", "origin", "main"]);
        for b in worktrees {
            let path = proj.join(b.replace('/', "-"));
            self.git(&bare, &["worktree", "add", "-q", path.to_str().unwrap(), b]);
            let up = format!("origin/{b}");
            self.git(&path, &["branch", "--set-upstream-to", &up, b]);
        }
        self.git(&bare, &["config", "core.bare", "false"]);
        proj
    }

    fn advance_origin(&self, branch: &str, file: &str) {
        let src = self.root.join("src");
        self.git(&src, &["switch", "-q", branch]);
        self.commit(&src, file, &format!("advance {branch}"));
        let origin = self.root.join("origin.git");
        self.git(&src, &["push", "-q", origin.to_str().unwrap(), branch]);
    }

    fn wt(&self, cwd: &Path, args: &[&str]) -> (i32, String, String) {
        self.wt_stdin(cwd, args, "")
    }

    fn wt_stdin(&self, cwd: &Path, args: &[&str], stdin: &str) -> (i32, String, String) {
        use std::io::Write;
        use std::process::Stdio;
        let mut cmd = Command::cargo_bin("wt").unwrap();
        self.apply(&mut cmd);
        let mut child = cmd
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8(out.stdout).unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    }
}

#[test]
fn status_reports_clean_dirty_and_sync() {
    let env = Env::new();
    let proj = env.layout(&["main", "feat/a", "b"], &["main", "feat/a", "b"]);
    env.advance_origin("main", "m.txt");
    env.git(&proj.join(".bare"), &["fetch", "-q", "--all"]);
    fs::write(proj.join("b/a.txt"), "changed").unwrap();

    let (code, out, _) = env.wt(&proj.join("main"), &["status"]);
    assert_eq!(code, 2, "a dirty worktree makes status exit 2\n{out}");
    assert!(out.contains("DIRTY    b "), "{out}");
    assert!(out.contains("CLEAN    main"), "{out}");
    assert!(out.contains("↓1"), "{out}");
    assert!(out.contains("Summary: total=3 clean=2 dirty=1"), "{out}");
}

#[test]
fn status_json_reports_detached_and_no_upstream() {
    let env = Env::new();
    let proj = env.layout(&["main"], &["main"]);
    let bare = proj.join(".bare");
    let det = proj.join("det");
    env.git(
        &bare,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            det.to_str().unwrap(),
            "main",
        ],
    );
    let lone = proj.join("lone");
    env.git(
        &bare,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "lone",
            lone.to_str().unwrap(),
            "main",
        ],
    );

    let (code, out, _) = env.wt(&proj.join("main"), &["status", "--json"]);
    assert_eq!(code, 0);
    assert!(out.contains("\"sync\": \"detached\""), "{out}");
    assert!(out.contains("\"sync\": \"no upstream\""), "{out}");
    assert!(out.contains("\"total\": 3"), "{out}");
}

#[test]
fn pull_classifies_each_worktree_and_keeps_list_order() {
    let env = Env::new();
    let proj = env.layout(&["main", "a", "b", "c"], &["main", "a", "b", "c"]);
    env.advance_origin("a", "a2.txt");
    env.advance_origin("b", "b2.txt");
    env.commit(&proj.join("b"), "local.txt", "diverge");
    fs::write(proj.join("c/new.txt"), "untracked").unwrap();

    let (code, out, _) = env.wt(&proj.join("main"), &["pull"]);
    assert_eq!(code, 2, "{out}");
    let a = out.find("==> a (a)").unwrap();
    let b = out.find("==> b (b)").unwrap();
    let c = out.find("==> c (c)").unwrap();
    assert!(a < b && b < c, "output follows worktree order\n{out}");
    assert!(out.contains("OK fast-forwarded"), "{out}");
    assert!(
        out.contains("FAIL cannot fast-forward (diverged?)"),
        "{out}"
    );
    assert!(out.contains("SKIP dirty working tree"), "{out}");
    assert!(out.contains("Summary: ok=2 skip=1 fail=1"), "{out}");
}

#[test]
fn pull_rebase_aborts_on_conflict() {
    let env = Env::new();
    let proj = env.layout(&["main", "a"], &["main", "a"]);
    env.advance_origin("a", "same.txt");
    env.commit(&proj.join("a"), "same.txt", "conflicting content");

    let (code, out, _) = env.wt(&proj.join("main"), &["pull", "--rebase"]);
    assert_eq!(code, 2);
    assert!(
        out.contains("FAIL rebase failed (conflict?), aborted"),
        "{out}"
    );
    let status = env.git(&proj.join("a"), &["status", "--porcelain"]);
    assert!(status.is_empty(), "rebase was aborted: {status}");
}

#[test]
fn upstream_sets_missing_and_skips_existing() {
    let env = Env::new();
    let proj = env.layout(&["main", "x"], &["main"]);
    let bare = proj.join(".bare");
    let x = proj.join("x");
    env.git(&bare, &["worktree", "add", "-q", x.to_str().unwrap(), "x"]);

    let (code, out, _) = env.wt(&proj.join("main"), &["upstream"]);
    assert_eq!(code, 2, "a skip makes the run partial\n{out}");
    assert!(out.contains("SKIP upstream already set"), "{out}");
    assert!(out.contains("OK set upstream to origin/x"), "{out}");
    let up = env.git(&x, &["rev-parse", "--abbrev-ref", "@{u}"]);
    assert_eq!(up.trim(), "origin/x");
}

#[test]
fn add_tracks_remote_branch_and_auto_creates_new_one() {
    let env = Env::new();
    let proj = env.layout(&["main", "feat/remote"], &["main"]);
    let main = proj.join("main");

    let (code, out, _) = env.wt(&main, &["add", "feat/remote"]);
    assert_eq!(code, 0, "{out}");
    assert!(proj.join("feat-remote").is_dir());
    let up = env.git(
        &proj.join("feat-remote"),
        &["rev-parse", "--abbrev-ref", "@{u}"],
    );
    assert_eq!(up.trim(), "origin/feat/remote");

    let (code, out, _) = env.wt(&main, &["add", "brand/new"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("does not exist. Creating new branch."),
        "{out}"
    );
    assert!(proj.join("brand-new").is_dir());

    let (code, _, err) = env.wt(&main, &["add", "other", "../main"]);
    assert_eq!(code, 1);
    assert!(err.contains("already exists"), "{err}");
}

#[test]
fn remove_prompts_and_aborts_on_eof() {
    let env = Env::new();
    let proj = env.layout(&["main", "x"], &["main", "x"]);
    let (code, out, _) = env.wt_stdin(&proj.join("main"), &["rm", "x"], "");
    assert_eq!(code, 1);
    assert!(out.contains("[y/N] \nAborted."), "{out}");
    assert!(proj.join("x").is_dir());

    let (code, _, _) = env.wt_stdin(&proj.join("main"), &["rm", "x"], "y\n");
    assert_eq!(code, 0);
    assert!(!proj.join("x").exists());
}

#[test]
fn remove_keeps_unmerged_branch_and_skips_remote_delete() {
    let env = Env::new();
    let proj = env.layout(&["main", "wip"], &["main", "wip"]);
    env.commit(&proj.join("wip"), "local.txt", "unpushed");

    let (code, out, _) = env.wt(&proj.join("main"), &["rm", "-y", "-b", "--remote", "wip"]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("WARN branch kept"), "{out}");
    assert!(out.contains("skipping remote delete"), "{out}");
    let branches = env.git(&proj.join(".bare"), &["branch", "--list", "wip"]);
    assert!(!branches.trim().is_empty(), "branch must survive");
}

#[test]
fn remove_never_matches_the_bare_directory() {
    let env = Env::new();
    let proj = env.layout(&["main"], &["main"]);
    let (code, out, _) = env.wt(&proj.join("main"), &["rm", "--dry-run", "-m", "*"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Matched worktrees: 1"), "{out}");
}

#[test]
fn init_builds_layout_and_sets_core_bare_false() {
    let env = Env::new();
    env.layout(&["main", "staging", "feat/a"], &[]);
    let origin = env.root.join("origin.git");
    let (code, out, _) = env.wt(
        &env.root,
        &["init", origin.to_str().unwrap(), "fresh", "-w", "feat/a"],
    );
    assert_eq!(code, 0, "{out}");
    let fresh = env.root.join("fresh");
    assert!(fresh.join("main").is_dir() && fresh.join("staging").is_dir());
    assert!(fresh.join("feat-a").is_dir());
    assert!(!fresh.join(".git").exists(), "no root .git file");
    let bare_cfg = env.git(&fresh.join(".bare"), &["config", "core.bare"]);
    assert_eq!(bare_cfg.trim(), "false");

    // From the container root the .bare directory is found by walking up.
    let (code, out, _) = env.wt(&fresh, &["list"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("feat/a"), "{out}");
}

#[test]
fn outside_a_repository_is_an_error() {
    let env = Env::new();
    let (code, _, err) = env.wt(&env.root, &["status"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("Not inside a git worktree repository."),
        "{err}"
    );
}

#[test]
fn no_subcommand_prints_help_and_exits_zero() {
    let env = Env::new();
    let (code, out, _) = env.wt(&env.root, &[]);
    assert_eq!(code, 0);
    assert!(out.contains("Usage: wt"), "{out}");
}

#[test]
fn argparse_compatible_parsing() {
    let env = Env::new();
    let proj = env.layout(&["main", "x"], &["main", "x"]);
    let main = proj.join("main");
    // long-option prefix
    let (code, out, _) = env.wt(&main, &["rm", "--dry-r", "x"]);
    assert_eq!(code, 0, "{out}");
    // repeated option: last value wins
    let (code, out, _) = env.wt(&main, &["add", "-b", "nope", "-b", "main", "y"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Base: main"), "{out}");
    // positionals split by an option are rejected like argparse
    let (code, _, err) = env.wt(&main, &["rm", "--dry-run", "x", "-f", "main"]);
    assert_eq!(code, 2);
    assert!(err.contains("unrecognized arguments: main"), "{err}");
    // no subcommand prefix inference
    let (code, _, _) = env.wt(&main, &["stat"]);
    assert_eq!(code, 2);
}

#[test]
fn broken_pipe_exits_quietly() {
    let env = Env::new();
    let proj = env.layout(&["main"], &["main"]);
    let mut cmd = Command::cargo_bin("wt").unwrap();
    env.apply(&mut cmd);
    let mut child = cmd
        .args(["list"])
        .current_dir(proj.join("main"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[cfg(unix)]
#[test]
fn broken_pipe_fails_a_command_that_changes_things() {
    let env = Env::new();
    let proj = env.layout(&["main"], &["main"]);
    let wt = |args: &[&str]| {
        // A socket whose peer is gone before wt starts: the first write fails
        // with EPIPE. (std::io::pipe would need Rust 1.87; the MSRV is 1.85.)
        let (writer, reader) = std::os::unix::net::UnixStream::pair().unwrap();
        drop(reader);
        let writer = std::os::fd::OwnedFd::from(writer);
        let mut cmd = Command::cargo_bin("wt").unwrap();
        env.apply(&mut cmd);
        cmd.args(args)
            .current_dir(proj.join("main"))
            .stdout(writer)
            .stderr(std::process::Stdio::piped())
            .output()
            .unwrap()
    };
    let out = wt(&["add", "feat/y"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(wt(&["status"]).status.code(), Some(0));
}

#[test]
fn completion_offers_worktree_dirs_and_remote_branches() {
    let env = Env::new();
    let proj = env.layout(&["main", "feat/login", "staging"], &["main", "feat/login"]);
    let complete = |words: &[&str]| -> Vec<String> {
        let mut cmd = Command::cargo_bin("wt").unwrap();
        env.apply(&mut cmd);
        let out = cmd
            .env("COMPLETE", "zsh")
            .env("_CLAP_IFS", "\n")
            .env("_CLAP_COMPLETE_INDEX", (words.len() - 1).to_string())
            .arg("--")
            .args(words)
            .current_dir(proj.join("main"))
            .output()
            .unwrap();
        String::from_utf8(out.stdout)
            .unwrap()
            .lines()
            .map(|l| l.split(':').next().unwrap().to_string())
            .filter(|c| !c.starts_with('-'))
            .collect()
    };
    assert_eq!(complete(&["wt", "rm", "LOG"]), ["feat-login"]);
    assert_eq!(complete(&["wt", "add", "-b", "st"]), ["staging"]);
}

// --- 2.0 fixes (docs/rust-migration.md, Phase 3) ---------------------------

#[test]
fn json_output_is_only_json() {
    let env = Env::new();
    let proj = env.layout(&["main"], &["main"]);
    let (_, out, _) = env.wt(&proj.join("main"), &["status", "--json"]);
    assert!(out.starts_with('{') && out.ends_with("}\n"), "{out}");
    let (_, out, _) = env.wt(&proj.join("main"), &["list", "--json"]);
    assert!(out.starts_with('[') && out.ends_with("]\n"), "{out}");
}

#[test]
fn identifier_suffix_matches_whole_path_components_only() {
    let env = Env::new();
    let proj = env.layout(&["main", "feat/login"], &["main", "feat/login"]);
    let main = proj.join("main");
    // "in" is a character suffix of both "main" and "feat-login".
    let (code, _, err) = env.wt(&main, &["rm", "--dry-run", "in"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("No matching worktrees found."), "{err}");
    // A trailing run of whole components still matches.
    let (code, out, _) = env.wt(&main, &["rm", "--dry-run", "proj/main"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Matched worktrees: 1"), "{out}");
}

#[test]
fn init_keeps_a_default_branch_with_a_slash() {
    let env = Env::new();
    let src = env.root.join("src");
    env.git(
        &env.root,
        &["init", "-q", "-b", "release/2026", src.to_str().unwrap()],
    );
    env.commit(&src, "a.txt", "init");
    let origin = env.root.join("origin.git");
    env.git(
        &env.root,
        &[
            "clone",
            "-q",
            "--bare",
            src.to_str().unwrap(),
            origin.to_str().unwrap(),
        ],
    );
    let (code, out, _) = env.wt(&env.root, &["init", origin.to_str().unwrap(), "rel"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Default branch: release/2026"), "{out}");
    let wt = env.root.join("rel/release-2026");
    assert!(wt.is_dir(), "{out}");
    let up = env.git(&wt, &["rev-parse", "--abbrev-ref", "@{u}"]);
    assert_eq!(up.trim(), "origin/release/2026");
}

#[test]
fn failed_init_removes_only_what_it_created() {
    let env = Env::new();
    let missing = env.root.join("missing.git");
    let (code, out, _) = env.wt(&env.root, &["init", missing.to_str().unwrap(), "bad"]);
    assert_eq!(code, 1, "{out}");
    assert!(
        !env.root.join("bad").exists(),
        "new directory is removed\n{out}"
    );

    // A directory that existed (empty) before is kept, and left empty.
    fs::create_dir(env.root.join("keep")).unwrap();
    let (code, out, _) = env.wt(&env.root, &["init", missing.to_str().unwrap(), "keep"]);
    assert_eq!(code, 1, "{out}");
    assert!(env.root.join("keep").is_dir());
    assert_eq!(
        fs::read_dir(env.root.join("keep")).unwrap().count(),
        0,
        "{out}"
    );

    // So the same command can simply be run again.
    let (code, out, _) = env.wt(&env.root, &["init", missing.to_str().unwrap(), "bad"]);
    assert_eq!(code, 1);
    assert!(!out.contains("already exists"), "{out}");
}

#[test]
fn init_onto_a_file_is_a_clear_error() {
    let env = Env::new();
    fs::write(env.root.join("afile"), "x").unwrap();
    let (code, out, err) = env.wt(&env.root, &["init", "unused-url", "afile"]);
    assert_eq!(code, 1);
    assert!(err.contains("exists and is not a directory."), "{out}{err}");
    assert!(!err.contains("cannot read"), "{err}");
}

#[test]
fn add_in_a_regular_clone_needs_an_explicit_path() {
    let env = Env::new();
    env.layout(&["main", "feat/x"], &[]);
    let clone = env.root.join("clone");
    let origin = env.root.join("origin.git");
    env.git(
        &env.root,
        &[
            "clone",
            "-q",
            origin.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    let (code, _, err) = env.wt(&clone, &["add", "feat/x"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("wt add feat/x <path>"), "{err}");
    assert!(
        !clone.join("feat-x").exists(),
        "nothing created inside the clone"
    );

    let (code, out, _) = env.wt(&clone, &["add", "feat/x", "../clone-feat-x"]);
    assert_eq!(code, 0, "{out}");
    assert!(env.root.join("clone-feat-x").is_dir());
}

#[test]
fn add_in_a_bare_clone_named_dot_git_keeps_the_container_layout() {
    let env = Env::new();
    env.layout(&["main", "feat/x"], &[]);
    let cont = env.root.join("cont");
    let bare = cont.join(".git");
    let origin = env.root.join("origin.git");
    env.git(
        &env.root,
        &[
            "clone",
            "-q",
            "--bare",
            origin.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    env.git(&bare, &["worktree", "add", "-q", "../main", "main"]);
    let (code, out, err) = env.wt(&cont.join("main"), &["add", "feat/x"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(cont.join("feat-x").is_dir());
}

#[test]
fn branch_lookups_are_not_globs() {
    let env = Env::new();
    let proj = env.layout(&["main"], &["main"]);
    // 1.x passed the name to `git branch --list`, where "ma*" matched main.
    let (code, out, _) = env.wt(&proj.join("main"), &["add", "ma*"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("Branch 'ma*' does not exist"), "{out}");
    assert!(!out.contains("invalid reference: ma*"), "{out}");

    // Nor revisions: "main~1" names no branch, so no detached worktree.
    env.commit(&proj.join("main"), "b.txt", "second");
    let (code, out, _) = env.wt(&proj.join("main"), &["add", "main~1"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("Branch 'main~1' does not exist"), "{out}");
    assert!(!proj.join("main~1").exists(), "{out}");
}

#[test]
fn init_does_not_print_url_credentials() {
    let env = Env::new();
    // Port 9 on localhost refuses immediately; nothing leaves the machine.
    let url = "https://user:SECRET-TOKEN@127.0.0.1:9/x.git";
    let (code, out, err) = env.wt(&env.root, &["init", url, "cred"]);
    assert_eq!(code, 1);
    assert!(
        !out.contains("SECRET-TOKEN") && !err.contains("SECRET-TOKEN"),
        "{out}{err}"
    );
    assert!(
        out.contains("URL:    https://***@127.0.0.1:9/x.git"),
        "{out}"
    );
}

#[test]
fn remove_failure_messages_are_english() {
    let env = Env::new();
    let proj = env.layout(&["main", "x"], &["main", "x"]);
    env.git(
        &proj.join(".bare"),
        &["worktree", "lock", proj.join("x").to_str().unwrap()],
    );
    let (code, out, _) = env.wt(&proj.join("main"), &["rm", "-y", "-b", "x"]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("FAIL worktree remove failed: "), "{out}");
    assert!(
        out.contains("SKIP branch delete skipped because worktree removal failed (x)"),
        "{out}"
    );
    assert!(out.is_ascii(), "{out}");
}

// --- 2.0 additions (Phase 4, P1) --------------------------------------------

#[test]
fn version_flag_prints_the_crate_version() {
    let env = Env::new();
    let (code, out, _) = env.wt(&env.root, &["--version"]);
    assert_eq!(code, 0);
    assert_eq!(out, format!("wt {}\n", env!("CARGO_PKG_VERSION")));
    // Wrong input: the subcommands do not take it.
    let (code, _, _) = env.wt(&env.root, &["status", "--version"]);
    assert_eq!(code, 2);
}

#[test]
fn errors_go_to_stderr() {
    let env = Env::new();
    let (code, out, err) = env.wt(&env.root, &["status"]);
    assert_eq!(code, 1);
    assert!(out.is_empty(), "{out}");
    assert!(
        err.starts_with("Error: Not inside a git worktree repository."),
        "{err}"
    );

    let proj = env.layout(&["main"], &["main"]);
    let main = proj.join("main");
    let (code, out, err) = env.wt(&main, &["rm", "ghost"]);
    assert_eq!(code, 1);
    assert!(err.contains("Error: No matching worktrees found."), "{err}");
    assert!(err.contains("Available worktrees:"), "{err}");
    assert!(!out.contains("Error:"), "{out}");

    let (code, _, err) = env.wt(&main, &["add", "x", "../main"]);
    assert_eq!(code, 1);
    assert!(err.contains("Error: Path '"), "{err}");

    // Progress and results stay on stdout.
    let (code, out, err) = env.wt(&main, &["list"]);
    assert_eq!(code, 0);
    assert!(out.contains("main") && err.is_empty(), "{out}{err}");
}

#[test]
fn ambiguous_identifier_needs_all_matches() {
    let env = Env::new();
    let proj = env.layout(&["main", "a", "b"], &["main"]);
    let bare = proj.join(".bare");
    for (branch, dir) in [("a", "one/api"), ("b", "two/api")] {
        let path = proj.join(dir);
        env.git(
            &bare,
            &["worktree", "add", "-q", path.to_str().unwrap(), branch],
        );
    }
    let main = proj.join("main");

    // Wrong input for a destructive run: one name, two worktrees.
    let (code, out, err) = env.wt(&main, &["rm", "-y", "api"]);
    assert_eq!(code, 1, "{out}{err}");
    assert!(
        err.contains("identifier 'api' matches 2 worktrees"),
        "{err}"
    );
    assert!(proj.join("one/api").is_dir() && proj.join("two/api").is_dir());

    // A dry run still previews, with a warning.
    let (code, out, _) = env.wt(&main, &["rm", "--dry-run", "api"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("needs --all-matches"), "{out}");

    // Explicit consent removes both.
    let (code, out, _) = env.wt(&main, &["rm", "-y", "--all-matches", "api"]);
    assert_eq!(code, 0, "{out}");
    assert!(!proj.join("one/api").exists() && !proj.join("two/api").exists());
}

#[test]
fn status_shows_the_upstream_ref() {
    let env = Env::new();
    let proj = env.layout(&["main"], &["main"]);
    let bare = proj.join(".bare");
    let lone = proj.join("lone");
    env.git(
        &bare,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "lone",
            lone.to_str().unwrap(),
            "main",
        ],
    );
    let det = proj.join("det");
    env.git(
        &bare,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            det.to_str().unwrap(),
            "main",
        ],
    );

    let (_, out, _) = env.wt(&proj.join("main"), &["status", "--json"]);
    assert!(out.contains("\"upstream\": \"origin/main\""), "{out}");
    // No upstream and detached HEAD both report null.
    assert_eq!(out.matches("\"upstream\": null").count(), 2, "{out}");

    let (_, out, _) = env.wt(&proj.join("main"), &["status"]);
    assert!(out.contains("UPSTREAM"), "{out}");
    let main_row = out.lines().find(|l| l.contains("CLEAN    main")).unwrap();
    assert!(main_row.contains("origin/main"), "{out}");
}
