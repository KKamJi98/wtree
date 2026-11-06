//! Running the `git` CLI.
//!
//! git is always a subprocess, never libgit2/gix: those can disagree with the
//! CLI on `insteadOf`, `includeIf`, worktree layout and `safe.directory`.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::out::fatal;

/// Outcome of one git invocation.
#[derive(Debug, Clone, Default)]
pub struct GitOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl GitOutput {
    pub fn ok(&self) -> bool {
        self.code == 0
    }
}

/// Run `git <args>` in `cwd` (or the current directory) and capture output.
///
/// `LC_ALL=C` keeps git's messages in English regardless of the user's locale,
/// because several callers parse them. stdin is inherited, so a credential or
/// passphrase prompt still reaches the user's terminal.
pub fn run<S: AsRef<str>>(args: &[S], cwd: Option<&Path>) -> GitOutput {
    let mut cmd = Command::new("git");
    cmd.args(args.iter().map(AsRef::as_ref))
        .env("LC_ALL", "C")
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = cwd {
        if !dir.is_dir() {
            fatal(&format!("directory not found: {}", dir.display()));
        }
        cmd.current_dir(dir);
    }
    let output = match cmd.output() {
        Ok(output) => output,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            fatal("git executable not found; install git and make sure it is on PATH")
        }
        Err(err) => fatal(&format!("failed to run git: {err}")),
    };
    let decode = |bytes: Vec<u8>| match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => fatal("git produced output that is not valid UTF-8"),
    };
    GitOutput {
        // A signal-terminated git has no code; treat it like any failure.
        code: output.status.code().unwrap_or(-1),
        stdout: decode(output.stdout),
        stderr: decode(output.stderr),
    }
}
