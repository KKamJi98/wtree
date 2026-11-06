# Roadmap

Ideas that were considered during the Rust rewrite (2.0) and deliberately left
out. Nothing here is scheduled; each item needs its own design and tests.

## P2

- **Opt-in network timeout.** `--timeout <sec>` for the commands that talk to a
  remote (`fetch`, `pull`, `init`, `remove -b`). There is no default timeout on
  purpose: a large fetch can legitimately take minutes, and git's credential
  and passphrase prompts already reach the terminal.
- **`wt pull --allow-untracked`.** Untracked files alone make a worktree count
  as dirty, so `pull` skips it even though a fast-forward would succeed.
- **`--json` for `pull`, `upstream` and `remove`**, with the same per-worktree
  categories (`ok`, `skip`, `fail`, `kept`) the text output prints.
- **`wt path <identifier>`** to print a worktree's path, for shell `cd`
  integration (`cd "$(wt path feat-x)"`).
- **One JSON layout for an empty `status --json`.** With no worktrees it prints
  a compact one-line object (kept from 1.x); everything else is indented.

## P3

- **`wt prune --merged`**: remove every worktree whose branch is already merged,
  reusing the `remove -b` assessment.
- **Hooks after `wt add`** (for example `.wt/post-add`) to install dependencies
  or copy local config into a new worktree.
- **A man page** generated from the clap definition.
