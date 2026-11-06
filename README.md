# wtree

[![crates.io](https://img.shields.io/crates/v/kkamji-wtree)](https://crates.io/crates/kkamji-wtree)
[![CI](https://github.com/KKamJi98/wtree/actions/workflows/ci.yml/badge.svg)](https://github.com/KKamJi98/wtree/actions/workflows/ci.yml)

Manage a bare repository and its worktrees from any directory inside it.

`git worktree` lets you keep several branches checked out at once, but it leaves
the bookkeeping to you: which worktrees exist, which are behind, which have
uncommitted work, and which branch is safe to delete. `wt` answers those from
wherever you happen to be standing.

The command is `wt`. It is a single native binary: the Homebrew formula is
`wtree`, the crates.io crate is `kkamji-wtree` (the name `wtree` was taken).

## Features

- **wt init**: set up a bare repository and its first worktrees in one step
- **wt add / remove**: add and remove worktrees, with pattern matching and a dry run
- **wt status**: clean/dirty and ahead/behind for every worktree at once
- **wt fetch**: `git fetch --all --prune` against the bare repository
- **wt pull**: fetch, then sync every worktree (fast-forward only by default)
- **wt upstream**: point every worktree at `origin/<branch>`

## Installation

```bash
# Homebrew (macOS, Linux)
brew install KKamJi98/tap/wtree

# crates.io
cargo install --locked kkamji-wtree

# From a checkout
cargo install --locked --path .
```

Prebuilt archives for macOS (arm64, x86_64), Linux (x86_64, aarch64, static
musl) and Windows (x86_64) are attached to every
[GitHub Release](https://github.com/KKamJi98/wtree/releases), with SHA-256
checksums and build provenance attestations
(`gh attestation verify <archive> --repo KKamJi98/wtree`).

### Moving from the Python version (1.x)

Versions up to 1.2.2 were a Python package on PyPI. 2.0 is a Rust rewrite with
the same commands and output. Replace the old install:

```bash
uv tool uninstall wtree
brew install KKamJi98/tap/wtree     # or: cargo install --locked kkamji-wtree
```

Shell completion moved from argcomplete to the binary itself; regenerate it
(see [Shell completion](#shell-completion)). PyPI 1.2.2 stays available:
`uv tool install wtree==1.2.2` brings the old version back.

### `wt` name conflicts

Other tools also install a `wt` command:

- Homebrew `worktrunk` and `wiredtiger` (the formula declares `conflicts_with`
  for both, so `brew` refuses to install them side by side)
- the crates.io crates `wtree` and `worktrunk`; `cargo install` refuses to
  overwrite a `wt` that another crate installed
- on Windows, Windows Terminal's `wt.exe` App Execution Alias. Turn it off in
  Settings > Apps > Advanced app settings > App execution aliases, or call this
  `wt.exe` by its full path.

### Shell completion

Completion is dynamic: worktree directories and remote branches are looked up
when you press Tab, matched by case-insensitive substring.

```bash
# zsh: write it into a directory on $fpath
COMPLETE=zsh wt > ~/.zfunc/_wt

# bash
echo 'source <(COMPLETE=bash wt)' >> ~/.bashrc

# fish
echo 'COMPLETE=fish wt | source' >> ~/.config/fish/config.fish
```

The Homebrew formula installs completions for bash, zsh and fish.

## Quick Start

```bash
# Start a new project: bare repo and worktrees in one command
wt init git@github.com:org/repo.git my-project

# What you get:
# my-project/
# ├── .bare/    # bare repository
# └── main/     # worktree for the default branch

# Bring extra branches along
wt init git@github.com:org/repo.git my-project -w staging,develop
```

## Usage

### Initialise (wt init)

```bash
# Repository name is taken from the URL
wt init git@github.com:org/repo.git

# Or name the directory yourself
wt init git@github.com:org/repo.git my-project

# Create extra worktrees up front
wt init git@github.com:org/repo.git my-project -w staging,develop,feat/new-feature
```

### Add and remove worktrees

```bash
# Existing branch
wt add staging

# Create the branch as you add it
wt add feat/my-feature -c

# Branch off something other than the default
wt add chore/test123 -c --base staging

# Remove the worktree, keep the local branch
wt remove staging

# Remove the worktree and the local branch
wt remove feat/my-feature -b

# Remove the worktree, the local branch, and the remote branch
wt remove feat/my-feature -b --remote

# Several at once
wt remove provider exemplars

# By glob, matched against branch, directory name, and full path
wt remove --match "fix/*"

# See what would go without removing anything
wt remove --match "feat/*" --dry-run

# Remove even if the worktree is dirty
wt remove staging -f
```

An identifier is matched as an exact branch name or path first, then as a
worktree directory name, then as trailing path components (`proj/main`), never
as a partial word. When one identifier still matches several worktrees (two
directories called `api`, say), `wt remove` stops and lists them, even with
`-y`; name one by path, or pass `--all-matches` to remove them all.

`wt remove` takes the worktree entry and its directory, and nothing else. Pass
`-b/--branch` to delete the local branch too, and `--remote` on top of that to
delete the remote one. `--remote` without `-b` is an error rather than a
surprise.

When `-b` would delete a branch that is not merged anywhere, **the branch is kept**
and reported as `WARN branch kept`. That is not a failure, but it does set exit
code 2. Check whether the work is really gone, then force it with
`wt rm -f -b <id>` or `git -C <repo>/.bare branch -D <branch>`.

A kept local branch also stops `--remote`, even when you asked for it: the remote
may be the only copy left. `--dry-run -b` shows, against current refs, which
branches would be deleted, which would be kept, and which cannot be judged.

### Add a worktree for a new remote branch

```bash
wt fetch                      # see the branch first
wt add feature/new-remote
wt upstream                   # only if it has no upstream yet
```

### Status

```bash
wt status
wt st

# Machine-readable: stdout carries only the JSON document
wt status --json | jq '.worktrees[] | select(.status == "dirty") | .path'
```

Each worktree reports `status` (clean/dirty), `sync` (ahead/behind),
`upstream` (the tracked ref, or `null`) and `detached`.

### Sync

```bash
# Fetch into the bare repository
wt fetch
wt f

# Fetch, then fast-forward every worktree
wt pull
wt p

# Rebase instead
wt pull --rebase
wt p -r
```

`wt pull` follows each branch's configured upstream (`@{u}`), not a guess. That
is usually `origin/<branch>`, but a branch tracking a different remote is synced
against the remote it actually tracks.

### Everything else

```bash
# List worktrees
wt list
wt ls

# Set origin/<branch> as upstream everywhere it is missing
wt upstream
wt up
```

## Example Output

### `wt init`

```
Initializing worktree repository
  URL:    git@github.com:org/repo.git
  Path:   /Users/you/code/repo

Step 1: Clone bare repository
  OK cloned to /Users/you/code/repo/.bare

Step 2: Configure fetch refspec
  OK configured fetch refspec
  OK fetched all branches

Step 3: Create worktrees
  Default branch: main
  OK created worktree: main/

✓ Initialization complete!

Next steps:
  cd /Users/you/code/repo/main
  wt status

To add more worktrees:
  cd /Users/you/code/repo/.bare
  git worktree add ../<branch-name> <branch-name>
```

### `wt status`

```
Bare repo: /path/to/repo/.bare

STATUS   BRANCH           SYNC           UPSTREAM                PATH
-------- ---------------- -------------- ----------------------- ------------------------------
CLEAN    main             =              origin/main             /path/to/repo/main
CLEAN    staging          ↓3             origin/staging          /path/to/repo/staging
DIRTY    feat/new-feature ↑2             origin/feat/new-feature /path/to/repo/feat-new-feature

Summary: total=3 clean=2 dirty=1
```

### `wt pull`

```
Bare repo: /path/to/repo/.bare

Step 1: Fetch
Fetching from bare repo: /path/to/repo/.bare
OK fetch --all --prune completed

Step 2: Sync worktrees

==> main (main)
  OK already up to date

==> staging (staging)
  OK fast-forwarded

==> feat/new-feature (feat-new-feature)
  SKIP dirty working tree

Summary: ok=2 skip=1 fail=0
```

## The layout

```
my-project/
├── .bare/              # bare repository (git clone --bare)
├── main/               # worktree for main
├── staging/            # worktree for staging
└── feat-new-feature/   # worktree for a feature branch
```

The root directory is only a container. No `.git` file or directory is created
there, so nothing about it looks like a checkout.

### Doing it by hand

For reference, `wt init` is equivalent to:

```bash
# 1. Clone as a bare repository
git clone --bare git@github.com:org/repo.git my-project/.bare

# 2. Teach it to track remote branches
cd my-project/.bare
git config remote.origin.fetch "+refs/heads/*:refs/remotes/origin/*"
git fetch --all

# 3. Add worktrees
git worktree add ../main main
git worktree add ../staging staging
```

## Safety

- **Fast-forward only by default**: `wt pull` refuses to rewrite. A worktree with
  local commits fails rather than being merged behind your back.
- **Rebase is opt-in**: `--rebase` rebases, and aborts on conflict instead of
  leaving you mid-rebase in a directory you were not looking at.
- **Dirty worktrees are skipped**: uncommitted work is never touched, and a
  worktree whose status cannot be read counts as dirty rather than as safe.
- **Branches without an upstream are skipped**: nothing is invented for them.

## Command Reference

| Command | Alias | Description |
|---------|-------|-------------|
| `wt init <url> [path]` | - | Create the bare repo and its first worktrees |
| `wt add <branch>` | `a` | Add a worktree |
| `wt remove <identifier...> [--match <glob>] [--dry-run]` | `rm` | Remove worktrees; keeps the local branch unless `-b` |
| `wt status` | `st` | Clean/dirty and sync state for every worktree |
| `wt fetch` | `f` | `git fetch --all --prune` |
| `wt pull` | `p` | Fetch, then fast-forward |
| `wt pull --rebase` | `p -r` | Fetch, then rebase |
| `wt list` | `ls` | List worktrees |
| `wt upstream` | `up` | Set the missing upstreams |
| `wt prune [--dry-run]` | - | Remove stale worktree references |
| `wt --version` | - | Print the version |

## Exit Codes

`status`, `pull`, `upstream`, and `remove` share one convention.

| Code | Meaning |
|------|---------|
| `0` | Everything asked for succeeded |
| `1` | Usage error or a dead end: no worktrees, nothing matched, cancelled, `--remote` without `-b` |
| `2` | Partly done: some `SKIP`, `WARN(kept)`, or `FAIL`, or an identifier that matched nothing |

`status` returns `2` when any worktree is dirty, and `remove` returns `2` when it
kept a branch it was asked to delete. A command line that cannot be parsed
(unknown option, missing argument) also exits `2`.

`Error:` messages go to stderr; progress lines, results and summaries go to
stdout.

## Requirements

- `git` on `PATH` (the binary has no other runtime dependency)
- Git 2.15+ for worktrees
- Git 2.38+ for `wt remove -b` to recognise squash and rebase merges, which needs
  `merge-tree --write-tree`. On older Git those branches are kept and have to be
  deleted by hand.

## Development

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test

# Output parity with the last Python release (needs uv for wtree 1.2.2)
cargo build && python3 scripts/golden.py check --wt target/debug/wt
```

`docs/rust-migration.md` records how the Rust port was verified against 1.2.2
and which behaviour changed in 2.0.

## License

MIT
