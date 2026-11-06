//! Command-line definition, tuned to accept what the 1.x argparse CLI accepted.
//!
//! argparse differences that matter to existing aliases and scripts:
//! - a repeated option keeps its last value (`args_override_self`);
//! - unambiguous long-option prefixes work (`--dry-r`, `--js`) (`infer_long_args`);
//! - subcommand prefixes do not (`wt stat` is an error), so `infer_subcommands`
//!   stays off, and there is no `help` subcommand;
//! - positional arguments must form one contiguous run ([`check_positional_runs`]).

use std::ffi::OsString;

use clap::error::ErrorKind;
use clap::{ArgAction, CommandFactory, Parser, Subcommand};
use clap_complete::engine::ArgValueCompleter;

use crate::complete;

#[derive(Debug, Parser)]
#[command(
    name = "wt",
    version,
    about = "Git worktree management CLI - manage multiple worktrees from anywhere",
    disable_help_subcommand = true,
    args_override_self = true,
    infer_long_args = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Initialize a new bare repository with worktree structure
    #[command(args_override_self = true, infer_long_args = true)]
    Init {
        /// Git repository URL to clone
        repo_url: String,
        /// Target directory (default: repo name)
        path: Option<String>,
        /// Additional branches to create worktrees for (comma-separated)
        #[arg(short = 'w', long)]
        worktrees: Option<String>,
    },

    /// Add a new worktree
    #[command(visible_alias = "a", args_override_self = true, infer_long_args = true)]
    Add {
        /// Branch name
        #[arg(add = ArgValueCompleter::new(complete::remote_branches))]
        branch: String,
        /// Worktree path (default: ../<branch>)
        path: Option<String>,
        /// Create new branch if it doesn't exist
        #[arg(short = 'c', long)]
        create: bool,
        /// Base branch to create new branch from
        #[arg(short = 'b', long, add = ArgValueCompleter::new(complete::remote_branches))]
        base: Option<String>,
    },

    /// Remove a worktree (keeps local branch unless -b)
    #[command(
        visible_alias = "rm",
        args_override_self = true,
        infer_long_args = true
    )]
    Remove {
        /// Branch/path identifiers to remove (supports multiple)
        #[arg(add = ArgValueCompleter::new(complete::worktree_identifiers))]
        identifier: Vec<String>,
        /// Glob pattern to match branch/path for batch removal (repeatable)
        #[arg(short = 'm', long = "match", action = ArgAction::Append)]
        patterns: Vec<String>,
        /// Show matched worktrees without removing them
        #[arg(long)]
        dry_run: bool,
        /// Force removal even if dirty
        #[arg(short = 'f', long)]
        force: bool,
        /// Also delete the local branch after removing the worktree
        #[arg(short = 'b', long)]
        branch: bool,
        /// Also delete remote branch (requires -b/--branch)
        #[arg(long)]
        remote: bool,
        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
        /// Remove every worktree an identifier matches when it matches several
        #[arg(long)]
        all_matches: bool,
    },

    /// Show status of all worktrees
    #[command(
        visible_alias = "st",
        args_override_self = true,
        infer_long_args = true
    )]
    Status {
        /// Output in JSON format
        #[arg(long = "json")]
        json_output: bool,
    },

    /// Fetch all remotes (git fetch --all --prune)
    #[command(visible_alias = "f", args_override_self = true, infer_long_args = true)]
    Fetch,

    /// Fetch and sync all worktrees
    #[command(visible_alias = "p", args_override_self = true, infer_long_args = true)]
    Pull {
        /// Use rebase instead of fast-forward merge
        #[arg(short = 'r', long)]
        rebase: bool,
    },

    /// List all worktrees
    #[command(
        visible_alias = "ls",
        args_override_self = true,
        infer_long_args = true
    )]
    List {
        /// Output in JSON format
        #[arg(long = "json")]
        json_output: bool,
    },

    /// Set upstream to origin/<branch> for all worktrees
    #[command(
        visible_alias = "up",
        args_override_self = true,
        infer_long_args = true
    )]
    Upstream,

    /// Remove stale worktree references
    #[command(args_override_self = true, infer_long_args = true)]
    Prune {
        /// Show what would be pruned without removing
        #[arg(long)]
        dry_run: bool,
    },
}

pub fn command() -> clap::Command {
    Cli::command()
}

/// Options that take a value, per subcommand: (short, long).
fn value_options(subcommand: &str) -> &'static [(char, &'static str)] {
    match subcommand {
        "init" => &[('w', "worktrees")],
        "add" | "a" => &[('b', "base")],
        "remove" | "rm" => &[('m', "match")],
        _ => &[],
    }
}

/// Every long option of a subcommand, for prefix resolution.
fn long_options(subcommand: &str) -> &'static [&'static str] {
    match subcommand {
        "init" => &["worktrees", "help"],
        "add" | "a" => &["create", "base", "help"],
        "remove" | "rm" => &[
            "match",
            "dry-run",
            "force",
            "branch",
            "remote",
            "yes",
            "all-matches",
            "help",
        ],
        "status" | "st" | "list" | "ls" => &["json", "help"],
        "pull" | "p" => &["rebase", "help"],
        "prune" => &["dry-run", "help"],
        _ => &["help"],
    }
}

fn resolve_long(subcommand: &str, name: &str) -> Option<&'static str> {
    let longs = long_options(subcommand);
    if let Some(exact) = longs.iter().find(|l| **l == name) {
        return Some(exact);
    }
    let mut hits = longs.iter().filter(|l| l.starts_with(name));
    match (hits.next(), hits.next()) {
        (Some(one), None) => Some(one),
        _ => None,
    }
}

/// Positional tokens after the subcommand, grouped into runs that are not
/// separated by options.
fn positional_runs(args: &[String]) -> Vec<Vec<String>> {
    let Some(sub_idx) = args.iter().position(|a| !a.starts_with('-')) else {
        return Vec::new();
    };
    let sub = args[sub_idx].as_str();
    let values = value_options(sub);
    let mut runs: Vec<Vec<String>> = Vec::new();
    let mut in_run = false;
    let mut i = sub_idx + 1;
    while i < args.len() {
        let tok = &args[i];
        if tok == "--" {
            for rest in &args[i + 1..] {
                if !in_run {
                    runs.push(Vec::new());
                    in_run = true;
                }
                runs.last_mut().expect("run pushed").push(rest.clone());
            }
            break;
        }
        if let Some(long) = tok.strip_prefix("--") {
            in_run = false;
            if !long.contains('=') {
                if let Some(name) = resolve_long(sub, long) {
                    if values.iter().any(|(_, l)| *l == name) {
                        i += 1;
                    }
                }
            }
        } else if tok.len() > 1 && tok.starts_with('-') {
            in_run = false;
            let chars: Vec<char> = tok.chars().skip(1).collect();
            for (k, c) in chars.iter().enumerate() {
                if values.iter().any(|(s, _)| s == c) {
                    if k + 1 == chars.len() {
                        i += 1;
                    }
                    break;
                }
            }
        } else {
            if !in_run {
                runs.push(Vec::new());
                in_run = true;
            }
            runs.last_mut().expect("run pushed").push(tok.clone());
        }
        i += 1;
    }
    runs
}

/// argparse consumes every positional slot (including optional ones) with the
/// first run of positional tokens, so a second run is rejected as
/// "unrecognized arguments". Reproduce that instead of clap's permissive merge.
pub fn check_positional_runs(raw: &[OsString]) {
    let args: Vec<String> = raw
        .iter()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let runs = positional_runs(&args);
    if runs.len() > 1 {
        let extra: Vec<String> = runs[1..].iter().flatten().cloned().collect();
        command()
            .error(
                ErrorKind::UnknownArgument,
                format!("unrecognized arguments: {}", extra.join(" ")),
            )
            .exit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(line: &str) -> Vec<Vec<String>> {
        let args: Vec<String> = line.split_whitespace().map(String::from).collect();
        positional_runs(&args)
    }

    #[test]
    fn contiguous_positionals_are_one_run() {
        assert_eq!(runs("rm a b --dry-run").len(), 1);
        assert_eq!(runs("rm -f a b").len(), 1);
        assert_eq!(runs("add -b main feat/x path").len(), 1);
        assert_eq!(runs("rm -m pat a").len(), 1);
        assert_eq!(runs("rm --mat pat a").len(), 1);
        assert_eq!(runs("init -w a,b url path").len(), 1);
        assert_eq!(runs("status").len(), 0);
    }

    #[test]
    fn interleaved_positionals_split() {
        assert_eq!(runs("rm --dry-run feat-x -f hotfix").len(), 2);
        assert_eq!(runs("add br -c path").len(), 2);
        assert_eq!(runs("init url -w x path").len(), 2);
    }

    #[test]
    fn clap_definition_is_valid() {
        command().debug_assert();
    }
}
