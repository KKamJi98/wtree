# Rust migration (wtree 1.2.2 -> 2.0.0)

This records the decisions and evidence behind rewriting `wt` from Python to
Rust. The plan it follows lives outside the repository
(`~/.local/state/rust-migration/wtree-rust-migration-plan.md`); where reality
differed from the plan, the difference and its evidence are written here.

- Behaviour reference: Python `wtree/cli.py` at v1.2.2 (`83329a4`).
- Distribution: crates.io crate `kkamji-wtree` (binary `wt`), Homebrew tap
  `KKamJi98/homebrew-tap` formula `wtree`, GitHub Release binaries. PyPI 1.2.x
  stays published (not yanked) as the rollback path.

## Phase 0: spike results (2026-11-06)

### Shell completion: clap_complete dynamic engine

`clap_complete` 4.6.11 with the `unstable-dynamic` feature (`CompleteEnv`),
pinned with `=4.6.11` because the API is unstable.

- `COMPLETE=zsh wt` prints a registration script starting with `#compdef wt`, so
  it can live in `~/.zfunc/_wt` and be autoloaded. `.bin("wt").completer("wt")`
  keep the script free of absolute paths (it calls `wt` from `PATH`).
- Candidates come from `ArgValueCompleter`, which receives the current word and
  returns candidates unfiltered by the engine. The completers filter by
  case-insensitive substring like the 1.x argcomplete validator:
  `wt rm og` and `wt rm LOG` both give `feat-login`.
- zsh: the generated script adds candidates with `_describe` without `-U`, the
  same as argcomplete's script, so the user's `matcher-list` applies in both.
  The Mac zshrc has `'m:{a-zA-Z}={A-Za-z}' 'r:|[._-]=* r:|=*' 'l:|=* r:|=*'`,
  which already matches substrings, so substring candidates are displayed.
- bash: the script assigns `COMPREPLY` from the binary's output without
  `compgen` filtering, again like argcomplete.
- Known differences, accepted (not value completion):
  - Subcommand aliases (`a`, `rm`, `st`, ...) are not offered; the engine
    de-duplicates them against the full name. argcomplete listed both.
  - With an empty word, options are offered as long forms only; argcomplete
    listed short forms too. Typing `-` still completes them.
  - Options and subcommands are filtered by prefix, not substring.
- Homebrew supports this protocol directly:
  `generate_completions_from_executable(bin/"wt", shell_parameter_format: :clap)`
  (`Library/Homebrew/formula.rb`, documented `:clap` format that sets
  `COMPLETE=<shell>`). The formula uses it.

### `--match` globs: own fnmatch, not the `glob` crate

Python `fnmatch` and `glob::Pattern` 0.3.4 were run on the same corpus.
They agree on `*` crossing `/`, `?`, `[a-f]`, `[!m]`, `[]]`, `[!]]` and case
sensitivity, but `glob` rejects an unclosed `[` (`[`, `a[`: "invalid range
pattern") where fnmatch treats it as a literal. `src/fnmatch.rs` reproduces
`fnmatch.translate` semantics instead (unit-tested against the CPython corpus).
On Windows it lowercases and maps `/` to `\` like `os.path.normcase`.

### Release tooling: hand-written GitHub Actions

dist (axodotdev/cargo-dist) 0.34.0 was the candidate. Its Homebrew publishing
needs a personal access token in `HOMEBREW_TAP_TOKEN`
(`book/src/installers/homebrew.md` at v0.34.0: "Create a GitHub personal access
token ... with the repo scope"), while the shared tap is written with a
per-repository SSH deploy key (`HOMEBREW_TAP_DEPLOY_KEY`) and no PAT. dist does
not publish to crates.io itself either. A hand-written `release.yml` covers the
five targets, attestation, the deploy-key tap push and crates.io Trusted
Publishing with fewer moving parts. The prjump migration had not recorded its
Phase 0 decision when this was decided.

### Windows: `wt.exe`

Windows Terminal registers an App Execution Alias named `wt.exe` under
`%LOCALAPPDATA%\Microsoft\WindowsApps`, which is on the user `PATH` (Microsoft
docs, "Using command line arguments for Windows Terminal"). Which `wt` wins
depends on `PATH` order. Not verified on a Windows machine. The README tells
Windows users to turn the alias off (Settings > Apps > Advanced app settings >
App execution aliases) or call the binary by full path.

## Phase 1: behaviour-preserving port

### Structure

| Module | Role |
|---|---|
| `cli.rs` | clap definition with argparse-compatible settings |
| `git.rs` | git subprocess (`LC_ALL=C`, captured output, inherited stdin) |
| `repo.rs` | bare-repo discovery, `worktree list --porcelain -z`, probes |
| `commands.rs` | status, fetch, prune, pull, list, upstream |
| `create.rs` / `remove.rs` | init and add / remove with branch cleanup |
| `parallel.rs` | bounded (16) scoped-thread map, results in input order |
| `json.rs` | `json.dumps`-compatible writer (`ensure_ascii`, separators) |
| `fnmatch.rs`, `paths.rs` | Python `fnmatch` and `pathlib` behaviour |
| `complete.rs` | dynamic completion candidates |
| `out.rs` | stdout writing (EPIPE -> quiet exit 0), colors |

### Parity harness

`scripts/golden.py` builds 73 deterministic scenarios (fixed author, dates and
config) and runs Python 1.2.2 and the Rust binary in each:

- 66 command cases (stdout, stderr, exit code) and 7 completion cases
  (candidate sets).
- `tests/golden/python-1.2.2.json` is the recorded Python output (Mac,
  git 2.56.0). CI compares against a live Python 1.2.2 run on the same runner,
  so git-version-dependent text cannot cause false failures.
- Exceptions (`EXCEPTIONS` in the script) compare the exit code only: help
  text, usage-error wording (exit 2 kept) and the 1.x traceback for
  `wt init <url> <existing file>`.

Not covered by the harness because the 1.x output is a crash or a terminal
effect, verified separately:

| Situation | 1.x | 2.0 |
|---|---|---|
| stdout closed early (`wt list \| true`) | `BrokenPipeError` message, exit 120 | silent, exit 0 (`tests/cli.rs`) |
| Ctrl-C | `KeyboardInterrupt` traceback, exit 130 | default SIGINT, exit 130, no traceback |
| git missing / non-UTF-8 git output | traceback, exit 1 | `Error: ...` on stderr, exit 1 |
| colors | only on a TTY, off when `NO_COLOR` is set (even empty) | same (`out.rs`) |

### Test migration

The 121 pytest tests mostly mocked `run_git` (70 monkeypatch sites); only one
used a real repository. Their intent was moved, not their mocks:

| pytest area | Tests | Where it went |
|---|---|---|
| `test_parallel.py` run_parallel (4) | order, every item, empty, single | `parallel.rs` unit tests (+ cap test) |
| `test_parallel.py` populate_status (8) | porcelain v2 parsing, failure = dirty | `repo.rs` `status_v2_parsing`, `sync_from_ab_cases`; golden `status-*` |
| `test_utilities.py` dirty/upstream/sync (14) | probe results | golden `status-mixed`, `pull-mixed`; `tests/cli.rs` status tests |
| `test_utilities.py` branch existence, default branch/ref (9) | ref lookups | golden `add-*`, `rm-branch-*`, `init-*` |
| `test_utilities.py` worktree records, find_bare_repo (9) | parsing, bare filter, `.bare` walk-up | `repo.rs` `porcelain_z_skips_bare_and_missing`; `tests/cli.rs` `remove_never_matches_the_bare_directory`, `init_builds_layout...` (walk-up) |
| `test_utilities.py` completers and validator (11) | substring, case, dedupe, no repo | golden `complete-*`; `tests/cli.rs` completion test |
| `test_commands.py` status (6) | clean, dirty=2, no upstream, detached, empty, json | golden `status-*`; `tests/cli.rs` |
| `test_commands.py` pull (12) | ff, up to date, diverged, skips, rebase, conflict abort, order | golden `pull-*`; `tests/cli.rs` pull tests |
| `test_commands.py` upstream (2), add (7), list (3), prune (4) | command flows | golden `upstream-*`, `add-*`, `list-*`, `prune-*`; `tests/cli.rs` |
| `test_commands.py` remove (14) + delete branch (5) | prompt, matching, dry run, assessments, remote skip | golden `rm-*` (merged/squash/preserved/unmerged scenario); `tests/cli.rs` |
| `test_remove_matching.py` (3) | identifier priority, suffix, pattern | `remove.rs` unit tests |
| `test_init_behavior.py` (3) | no root `.git`, core.bare=false, walk-up | `tests/cli.rs` `init_builds_layout_and_sets_core_bare_false` |
| `test_git_integration.py` (1) | unmerged branch kept in a real repo | `tests/cli.rs` `remove_keeps_unmerged_branch...` |
| `TestCaching` (2) | a probe is not repeated | dropped: the Rust code passes probe results by value instead of caching on the struct, so there is no cache to test |

The pytest fakes at `tests/test_commands.py:497` and `:556` created
`feat/from-main` and `feat/from-staging` in the current directory (they treated
the branch argument as the path). They are not ported.

## Differences from the plan

- Worktree path: the plan named `/Users/ethan/code/code-personal/wtree-rust-rewrite`.
  The local Git workflow rules forbid sibling worktrees outside the repository
  directory, so the work happened in `wtree/feat-rust-rewrite` instead (the
  same layout prjump used).
