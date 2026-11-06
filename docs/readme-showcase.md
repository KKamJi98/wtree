# README showcase verification

## Scope

This is a documentation and demo refresh, not a CLI behavior change. The demo
uses the unmodified, checksummed wt 2.0.1 Linux release. Its setup and source
tape live in `docs/demo/`; the layout diagram lives in `docs/assets/`.

## Differences from the initial plan

- The zsh first-Tab fix is already in 2.0.1 (PR #3), not pending work. The
  migration notes record Mac/WSL checks. The README describes the fixed behavior
  and the remaining alias/option/matcher differences instead of listing the
  resolved bug as a current limit.
- Since `wt` prints absolute paths, a host-local temporary HOME alone would
  still expose host paths. Recording therefore takes place in a disposable
  container with no host mounts, no network, neutral hostname `demo`, and a
  synthetic `/demo/home`. Its only remote is `/demo/remote.git`.
- `pull` skips dirty work and returns 2. The demo deliberately shows this, not
  an invented all-green run. The final checkout state keeps the unfinished
  login work unchanged.
- Pushed-but-unmerged branch deletion is a real limitation, not a merged-only
  safety guarantee. The misleading old blanket safety claim was replaced with
  a warning, and both it and the init cleanup gap are in `docs/ROADMAP.md`.
- The independent review found that dry-run can predict branch preservation
  even when actual removal deletes both unmerged copies. This was reproduced
  with fresh refs in the isolated fixture. The README now states that fetching
  does not correct the mismatch; no CLI behavior is changed.
- The review also found that init can ignore fetch failure, or warn on a failed
  additional checkout, and still return 0 with the setup retained. Both cases
  were reproduced with fake Git failure injection inside the container. The
  README, roadmap, and migration notes no longer claim cleanup for those cases.

## Checks before publication

- `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`,
  and `cargo test --locked` pass: 21 unit tests and 31 CLI integration tests.
- Shell syntax checks and `vhs validate docs/demo/wtree.tape` pass.
- The sandbox executes status, pull, add, remove, and final status with the
  real binary. The observed exit-2 status/pull results match the dirty fixture.
- The final GIF and MP4 are both 26.44 seconds, at 1200 x 640 pixels. The GIF
  is 191,451 bytes after gifsicle optimization; the MP4 is 229,712 bytes.
- All 26 one-second GIF samples were visually inspected, including transitions
  and the final result. They show only generic `/demo/project/...` paths and
  the fake branch names, with no real users, hosts, accounts, or credentials.
- The SVG has an explicit white background, 1120 x 600 viewBox, labeled arrows,
  and system sans-serif labels of at least 15px. A full-width PNG rendering was
  inspected: all four checkout cards and all command labels are visible.
- README image URLs are absolute raw.githubusercontent.com URLs, so the same
  README can display its media on GitHub and crates.io. The companion section
  is copied unchanged from the shared family text.

Release, remote CI, repository metadata, and installed-version results must be
read back after publication; the local checks above do not establish them.
