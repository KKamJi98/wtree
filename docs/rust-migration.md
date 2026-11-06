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
| `out.rs` | stdout writing (EPIPE -> quiet exit: 0 for `list`/`status`, 1 otherwise), colors |

### Parity harness

`scripts/golden.py` builds a fresh deterministic scenario (fixed author, dates
and config) for every case and runs Python 1.2.2 and the Rust binary in it:

- 68 command cases (stdout, stderr, exit code) and 7 completion cases
  (candidate sets).
- `tests/golden/python-1.2.2.json` is the recorded Python output (Mac,
  git 2.56.0). CI compares against a live Python 1.2.2 run on the same runner,
  so git-version-dependent text cannot cause false failures.
- `normalise` replaces the scenario root, curl's `after N ms` timing and the
  curl transport error after `unable to access '<url>':`, whose wording differs
  between the macOS and Linux runners.
- Exceptions (`EXCEPTIONS` in the script) compare the exit code only: help
  text and usage-error wording (exit 2 kept). At the end of Phase 1 the list
  also held `--version` and the 1.x traceback for `wt init <url> <file>`;
  both became intentional changes later (see the table below).
- At the end of Phase 1 every case matched 1.2.2 outside the exceptions, on
  macOS locally and on the ubuntu and macos CI runners.

Not covered by the harness because the 1.x output is a crash or a terminal
effect, verified separately:

| Situation | 1.x | 2.0 |
|---|---|---|
| stdout closed early (`wt list \| true`) | `BrokenPipeError` message, exit 120 | silent; exit 0 for `list`, `status` and help, exit 1 for every command that changes something, because it stopped before finishing (`tests/cli.rs`) |
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

## Phase 2: CI and release

- `ci.yml`: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and
  `cargo test --locked` on ubuntu, macos and windows; the golden harness live on
  ubuntu and macos; `cargo-deny` (`deny.toml`); an MSRV job (Rust 1.85, the
  `rust-version` of clap 4.6) that also runs `cargo publish --dry-run`.
- `release.yml` on `v*` tags: Cargo.toml/tag guard (replaces the
  `uv version --short` guard), five targets, `SHA256SUMS` plus build provenance
  attestation, GitHub Release, crates.io through Trusted Publishing in the
  `release` environment, and `Formula/wtree.rb` pushed to
  `KKamJi98/homebrew-tap` with the repository's `HOMEBREW_TAP_DEPLOY_KEY`
  deploy key (`scripts/release/render-formula.sh`, `conflicts_with` worktrunk
  and wiredtiger).
- All actions are pinned to commit SHAs; every workflow starts from
  `permissions: {}`.
- The crates.io job skips while the crate does not exist yet: Trusted
  Publishing can only be configured for an existing crate, so 2.0.0 is
  published by hand from the tagged commit.
- Removed: the Python package, its tests, `pyproject.toml`, `uv.lock` and the
  PyPI publish job. The `pypi` environment and PyPI releases stay.

## Phase 3: bug fixes

One commit per bug, each with a test that failed before the fix.

| # | 1.x behaviour | 2.0 |
|---|---|---|
| 1 | `status --json`, `list --json` started with the `Bare repo:` header, so the output was not JSON | stdout carries only the JSON document |
| 2 | `wt rm in` matched `main` and `feat-login` (string suffix) | suffixes match whole trailing path components |
| 3 | default branch `release/2026` became `2026` and `wt init` failed | full branch name kept; worktree dir `release-2026` |
| 4 | a failed `wt init` left `<path>/.bare`, blocking a retry | only what the run created is removed |
| 5 | `wt init <url> <file>` crashed with `NotADirectoryError` | `Error: '<path>' exists and is not a directory.` |
| 6 | in a regular clone, `wt add` nested the worktree inside the checkout | an explicit path is required there (a bare clone named `.git` keeps the default path) |
| 7 | `wt add 'ma*'` thought the branch existed (`git branch --list` glob) | exact ref lookups (`show-ref --verify`; revisions such as `main~1` name no branch) |
| 8 | `wt init git@host:repo.git` created `git@host:repo` | directory `repo` |
| 9 | `wt init https://user:token@...` printed the token | userinfo shown as `***` |
| 10 | two `wt rm` failure lines were Korean | English like the rest |

## Phase 4: P1 additions

- `wt --version` (`wt 2.0.0`).
- `Error:` messages on stderr; progress and results stay on stdout.
- `wt rm`: an identifier that matches several worktrees needs `--all-matches`,
  even with `-y`; `--dry-run` previews with a warning.
- `wt status`: `UPSTREAM` column and `upstream` JSON field.
- P2/P3 ideas are recorded in `docs/ROADMAP.md`, not implemented.

## Golden differences from 1.2.2

Cases that intentionally differ from Python 1.2.2 (`CHANGED` in
`scripts/golden.py`); the 2.0 output is recorded in `tests/golden/rust-2.0.json`.

| Case | Reason |
|---|---|
| `abbrev-long-json`, `status-json-mixed` | fix 1, plus the P1 `upstream` field |
| `status-json-empty`, `list-json-mixed`, `list-json-empty`, `list-json-korean` | fix 1 |
| `rm-suffix-in` | fix 2 (error now on stderr) |
| `init-release-default` | fix 3 |
| `init-clone-fails` | fix 4 |
| `init-credential-url` | fixes 4 and 9 |
| `init-into-file` | fix 5 (error on stderr) |
| `add-normal-clone` | fix 6 (error on stderr) |
| `add-glob-branch` | fix 7 |
| `rm-locked-fails` | fix 10 |
| `usage-version` | P1 `--version` |
| `not-a-repo`, `add-path-exists`, `rm-no-match`, `rm-no-args`, `rm-remote-without-b`, `init-nonempty-dir` | P1 errors on stderr |
| `status-std`, `status-mixed`, `status-korean`, `status-normal-clone` | P1 `UPSTREAM` column |

Cases still compared on exit code only: `help-noargs`, `help-flag`,
`usage-bogus`, `usage-subcommand-prefix`, `usage-interleaved`,
`usage-add-missing`.

## Differences from the plan

- Worktree path: the plan named `/Users/ethan/code/code-personal/wtree-rust-rewrite`.
  The local Git workflow rules forbid sibling worktrees outside the repository
  directory, so the work happened in `wtree/feat-rust-rewrite` instead (the
  same layout prjump used).
- Local pitfall: `cargo publish --dry-run` verifies the package in
  `target/package/`, and on cargo 1.99 it rewrote the dev fingerprint of the
  `wt` binary to point at the packaged sources. Later `cargo build` runs then
  reported the binary as fresh although `src/` had changed. `cargo clean -p
  kkamji-wtree` fixed it; the CI MSRV job runs the dry run in a throwaway
  checkout, so CI is not affected.

## Final review

An independent review of the whole diff (2026-11-07) found four defects, fixed
with tests in `tests/cli.rs` before the merge:

- `wt add` rejected a bare clone named `.git` as a regular clone (fix 6 was too
  broad). A regular clone is now recognised by its index file.
- Branch lookups used `rev-parse --verify`, which accepts revisions:
  `wt add 'main~1'` created a detached worktree. They now use
  `show-ref --verify`.
- `render-formula.sh` computed checksums inside the heredoc, where a failure
  does not stop the script, so a missing `.sha256` produced `sha256 ""` and
  exit 0. Checksums are now read before rendering.
- A closed stdout ended every command with exit 0, so `wt rm -y ... | head`
  reported success although it stopped before removing anything. Only
  read-only commands exit 0 now.

Known limitation, not fixed: if git itself fails unexpectedly (non-UTF-8
output, the working directory disappears) during `wt init` after the clone,
`wt` ends with `Error:` and exit 1 without the cleanup of fix 4, and the new
`.bare` stays behind. Normal git failures (`clone`, `fetch`, `worktree add`
returning non-zero) still clean up.

## Release (2026-11-07)

- PR #1 squash-merged as `3274e8b`; main CI green.
- `v2.0.0` (annotated) pushed; the release run built all five targets,
  `SHA256SUMS` and per-archive `.sha256`, build provenance attestations
  (`gh attestation verify` passes, signer `release.yml@refs/tags/v2.0.0`), and
  pushed `Formula/wtree.rb` 2.0.0 to the tap.
- crates.io: `kkamji-wtree` 2.0.0 was published by hand from a clean clone of
  the tag (the main checkout holds a git-ignored `.venv/` whose `LICENSE`
  files match the `include` pattern and make `cargo publish` refuse a dirty
  tree). The Trusted Publisher (`KKamJi98/wtree`, `release.yml`, environment
  `release`) was registered right after, so the release run's crates.io job
  found 2.0.0 already published and skipped.

### Differences from the plan found during the release

- Mac install order: after `brew install`, `command -v wt` still pointed at
  the uv tool shim in `~/.local/bin`, which comes first in `PATH`. The brew
  binary (`/opt/homebrew/bin/wt`, `wt 2.0.0`) was checked by full path, then
  `uv tool uninstall wtree` ran, and `command -v wt` became the brew path.
- zsh completion: the `COMPLETE=zsh wt` script from clap_complete 4.6.11 only
  registers its completer when zsh autoloads it from `fpath`. The first
  `wt <Tab>` in each shell therefore rings the bell without candidates; from
  the second Tab on, worktree directory names and options are offered
  (checked on macOS with `~/.zfunc/_wt` and on WSL with the Homebrew
  site-functions file). The 1.x argcomplete script called its completer on
  the autoload pass. A wrapper that does the same would need a custom zsh
  completer in `wt` and a new release; it was not done in 2.0.0.
