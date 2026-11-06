# Roadmap

Ideas that were considered during the Rust rewrite (2.0) and deliberately left
out. Nothing here is scheduled; each item needs its own design and tests.

## Known limitations

- **Pushed is not merged.** Local branch cleanup first uses `git branch -d`,
  whose upstream-based check can succeed before a branch is merged into main.
  `wt rm <branch> -b --remote` can then delete both copies. Consider a separate,
  explicitly designed merged-only cleanup mode; the README warns about the
  current behavior. No behavior is changed by the README showcase work.
- **Dry-run branch prediction differs from actual deletion.** The preview uses
  `assess` directly; actual cleanup first tries Git's upstream-based
  `branch -d`. With fresh refs, a pushed-but-unmerged branch can be previewed
  as kept, then actually deleted locally and remotely. Fetching does not fix
  this mismatch. Make the preview and execution use one assessment in a
  separately tested behavior change; the README does not promise safety here.
- **Init can succeed with an incomplete setup.** A failed fetch can be ignored;
  additional-worktree creation failures warn and retain the setup. Init can
  return 0 in both cases. Non-zero clone/default-worktree creation failures
  do trigger cleanup. Non-UTF-8 output or a disappearing directory after
  cloning can bypass cleanup and leave `.bare`. See `docs/rust-migration.md`.

## P2

- **Golden Git diagnostic portability.** `add-glob-branch` currently records
  Git-owned ref-syntax hint text verbatim. Apple Git 2.50.1 says
  "See `man git check-ref-format`" where the fixture says
  "See 'git help check-ref-format'". Normalize just this version-dependent
  advice in a separately tested change, preserving branch-validation behavior
  and the error/exit-code checks. The 2.0.2 release only updates the version
  output expectation; it does not change the comparison rules.

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
