//! `wt`: manage the worktrees of a bare git repository.

mod cli;
mod commands;
mod complete;
mod create;
mod fnmatch;
mod git;
mod json;
mod out;
mod parallel;
mod paths;
mod remove;
mod repo;

use clap::Parser;

use cli::{Cli, Command};
use out::{bold, reset};

fn run(cli: Cli) -> i32 {
    let Some(command) = cli.command else {
        let mut cmd = cli::command();
        let help = cmd.render_help().to_string();
        out::emit(help.trim_end_matches('\n'));
        return 0;
    };

    if let Command::Init {
        repo_url,
        path,
        worktrees,
    } = &command
    {
        let list: Option<Vec<String>> = worktrees
            .as_deref()
            .filter(|w| !w.is_empty())
            .map(|w| w.split(',').map(|b| b.trim().to_string()).collect());
        return create::cmd_init(repo_url, path.as_deref(), list.as_deref());
    }

    let Some(bare_repo) = repo::find_bare_repo(None) else {
        say_err!("Error: Not inside a git worktree repository.");
        say_err!();
        say_err!("To initialize a new repository:");
        say_err!("  wt init <repo-url> [path]");
        say_err!();
        say_err!("Or run this command from within an existing worktree directory.");
        return 1;
    };

    // With --json, stdout carries the JSON document and nothing else.
    let json_output = matches!(
        command,
        Command::Status { json_output: true } | Command::List { json_output: true }
    );
    if !json_output {
        say!(
            "{}Bare repo:{} {}",
            bold(),
            reset(),
            paths::display(&bare_repo.to_string_lossy())
        );
        say!();
    }

    match command {
        Command::Status { json_output } => commands::cmd_status(&bare_repo, json_output),
        Command::Fetch => commands::cmd_fetch(&bare_repo),
        Command::Pull { rebase } => commands::cmd_pull(&bare_repo, rebase),
        Command::List { json_output } => commands::cmd_list(&bare_repo, json_output),
        Command::Upstream => commands::cmd_upstream(&bare_repo),
        Command::Prune { dry_run } => commands::cmd_prune(&bare_repo, dry_run),
        Command::Add {
            branch,
            path,
            create,
            base,
        } => create::cmd_add(
            &bare_repo,
            &branch,
            path.as_deref(),
            create,
            base.as_deref(),
        ),
        Command::Remove {
            identifier,
            patterns,
            dry_run,
            force,
            branch,
            remote,
            yes,
            all_matches,
        } => {
            if remote && !branch {
                say_err!("Error: --remote requires -b/--branch flag");
                return 1;
            }
            remove::cmd_remove(
                &bare_repo,
                &remove::RemoveOptions {
                    identifiers: &identifier,
                    patterns: &patterns,
                    force,
                    dry_run,
                    delete_branch: branch,
                    delete_remote: remote,
                    yes,
                    all_matches,
                },
            )
        }
        Command::Init { .. } => unreachable!("handled above"),
    }
}

fn main() {
    // Shell completion requests (COMPLETE=<shell>) are answered and exit here.
    clap_complete::CompleteEnv::with_factory(cli::command)
        .bin("wt")
        .completer("wt")
        .complete();

    out::init_colors();
    let raw: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let cli = Cli::parse_from(&raw);
    cli::check_positional_runs(&raw);
    if matches!(
        cli.command,
        None | Some(Command::List { .. } | Command::Status { .. })
    ) {
        out::read_only();
    }
    std::process::exit(run(cli));
}
