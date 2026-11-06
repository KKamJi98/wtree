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

/// clap_complete's zsh registration, plus one change: when zsh autoloads the
/// script from `fpath` (Homebrew's site-functions, `~/.zfunc/_wt`), it also
/// completes on that pass. Stock clap_complete only registers its completer
/// then, so the first Tab in every shell rang the bell without candidates.
/// argcomplete, which 1.x used, completed on the autoload pass as well.
pub struct ZshAutoload;

impl clap_complete::env::EnvCompleter for ZshAutoload {
    fn name(&self) -> &'static str {
        clap_complete::env::Zsh.name()
    }

    fn is(&self, name: &str) -> bool {
        clap_complete::env::Zsh.is(name)
    }

    fn write_registration(
        &self,
        var: &str,
        name: &str,
        bin: &str,
        completer: &str,
        buf: &mut dyn std::io::Write,
    ) -> Result<(), std::io::Error> {
        let mut stock = Vec::new();
        clap_complete::env::Zsh.write_registration(var, name, bin, completer, &mut stock)?;
        let stock = String::from_utf8_lossy(&stock);
        let function = format!("_clap_dynamic_completer_{}", name.replace('-', "_"));
        match stock.trim_end().rsplit_once('\n') {
            Some((body, last)) if last.starts_with(&format!("compdef {function} ")) => {
                writeln!(buf, "{body}")?;
                writeln!(
                    buf,
                    "if [[ ${{zsh_eval_context[-1]}} == loadautofunc ]]; then\n    {function} \"$@\"\nelse\n    {last}\nfi"
                )
            }
            // An unexpected layout from a newer clap_complete: keep it as is.
            _ => buf.write_all(stock.as_bytes()),
        }
    }

    fn write_complete(
        &self,
        cmd: &mut clap::Command,
        args: Vec<std::ffi::OsString>,
        current_dir: Option<&std::path::Path>,
        buf: &mut dyn std::io::Write,
    ) -> Result<(), std::io::Error> {
        clap_complete::env::Zsh.write_complete(cmd, args, current_dir, buf)
    }
}
