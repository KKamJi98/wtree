#!/usr/bin/env python3
"""Golden parity harness: wtree 1.2.2 (Python) vs the Rust `wt`.

Every case builds a fresh, deterministic git scenario (fixed author, dates and
config), runs one `wt` command in it and records stdout, stderr and the exit
code. Temp paths are normalised to <ROOT>.

    python3 scripts/golden.py record                  # Python 1.2.2 -> tests/golden/python-1.2.2.json
    python3 scripts/golden.py record-rust --wt BIN    # Rust -> tests/golden/rust-2.0.json (CHANGED cases)
    python3 scripts/golden.py check --wt BIN          # live Python vs Rust, exceptions applied
    python3 scripts/golden.py check --wt BIN --offline  # recorded Python fixture vs Rust

The Python reference runs as `uvx --from wtree==1.2.2 wt` unless PY_WT says
otherwise. Standard library only.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
GOLDEN = REPO / "tests" / "golden"
PY_FIXTURE = GOLDEN / "python-1.2.2.json"
RS_FIXTURE = GOLDEN / "rust-2.0.json"

# Outputs that may legitimately differ. Values: (kind, reason).
#   code-only    compare the exit code only (text owned by argparse/clap or a traceback)
#   stdout-code  compare stdout and exit code, ignore stderr
#   changed      intentional 2.0 change; compare with the recorded Rust fixture
EXCEPTIONS: dict[str, tuple[str, str]] = {
    "help-noargs": ("code-only", "help text is rendered by argparse vs clap"),
    "help-flag": ("code-only", "help text is rendered by argparse vs clap"),
    "usage-bogus": ("code-only", "usage error wording differs (argparse vs clap); exit 2 kept"),
    "usage-subcommand-prefix": ("code-only", "usage error wording differs; exit 2 kept"),
    "usage-interleaved": ("code-only", "usage error wording differs; exit 2 kept"),
    "usage-add-missing": ("code-only", "usage error wording differs; exit 2 kept"),
}

# Filled in as Phase 3/4 changes land: case id -> reason.
CHANGED: dict[str, str] = {
    "abbrev-long-json": "fix 1 (+ P1 upstream field): --json prints only JSON (no Bare repo header)",
    "status-json-mixed": "fix 1 (+ P1 upstream field): --json prints only JSON (no Bare repo header)",
    "status-json-empty": "fix 1: --json prints only JSON (no Bare repo header)",
    "list-json-mixed": "fix 1: --json prints only JSON (no Bare repo header)",
    "list-json-empty": "fix 1: --json prints only JSON (no Bare repo header)",
    "list-json-korean": "fix 1: --json prints only JSON (no Bare repo header)",
    "rm-suffix-in": "fix 2 (+ P1 stderr): identifiers match whole trailing path components, so in no longer matches main and feat-login",
    "init-release-default": "fix 3: a default branch with a slash (release/2026) is kept whole; its worktree dir is release-2026",
    "init-clone-fails": "fix 4: a failed init removes the directory it created",
    "init-credential-url": "fix 4 and 9: partial setup removed; URL credentials shown as ***",
    "init-into-file": "fix 5 (+ P1 stderr): a file at the target path is a clear error instead of a NotADirectoryError traceback",
    "add-normal-clone": "fix 6 (+ P1 stderr): in a regular clone wt add needs an explicit path instead of nesting the worktree inside the checkout",
    "add-glob-branch": "fix 7: branch existence is an exact ref lookup, so ma* no longer matches main",
    "rm-locked-fails": "fix 10: the worktree-removal failure lines are in English like the rest of the output",
    "usage-version": "P1: wt --version prints wt <version> (1.x had no such option)",
    "not-a-repo": "P1: Error messages go to stderr; stdout keeps progress and results",
    "add-path-exists": "P1: Error messages go to stderr; stdout keeps progress and results",
    "rm-no-match": "P1: Error messages go to stderr; stdout keeps progress and results",
    "rm-no-args": "P1: Error messages go to stderr; stdout keeps progress and results",
    "rm-remote-without-b": "P1: Error messages go to stderr; stdout keeps progress and results",
    "init-nonempty-dir": "P1: Error messages go to stderr; stdout keeps progress and results",
    "status-std": "P1: status shows the upstream ref (UPSTREAM column, upstream JSON field)",
    "status-mixed": "P1: status shows the upstream ref (UPSTREAM column, upstream JSON field)",
    "status-korean": "P1: status shows the upstream ref (UPSTREAM column, upstream JSON field)",
    "status-normal-clone": "P1: status shows the upstream ref (UPSTREAM column, upstream JSON field)",
}

ENV_BASE = {
    "GIT_AUTHOR_NAME": "Golden",
    "GIT_AUTHOR_EMAIL": "golden@example.com",
    "GIT_COMMITTER_NAME": "Golden",
    "GIT_COMMITTER_EMAIL": "golden@example.com",
    "GIT_AUTHOR_DATE": "2026-01-01T00:00:00+0000",
    "GIT_COMMITTER_DATE": "2026-01-01T00:00:00+0000",
    "GIT_CONFIG_NOSYSTEM": "1",
    "LC_ALL": "C",
    "TZ": "UTC",
}


def make_env(root: Path) -> dict[str, str]:
    home = root / "home"
    home.mkdir(parents=True, exist_ok=True)
    gitconfig = home / ".gitconfig"
    if not gitconfig.exists():
        gitconfig.write_text("[init]\n\tdefaultBranch = main\n[advice]\n\tdetachedHead = false\n")
    env = {
        k: v
        for k, v in os.environ.items()
        if not k.startswith(("GIT_", "NO_COLOR", "COMPLETE", "_ARGCOMPLETE", "_CLAP"))
    }
    env.update(ENV_BASE)
    env["HOME"] = str(home)
    env["GIT_CONFIG_GLOBAL"] = str(gitconfig)
    return env


class Scene:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.env = make_env(root)

    def git(self, *args: str, cwd: Path | None = None) -> str:
        r = subprocess.run(
            ["git", *args],
            cwd=cwd or self.root,
            env=self.env,
            capture_output=True,
            text=True,
            check=False,
        )
        if r.returncode != 0:
            raise RuntimeError(f"git {' '.join(args)} failed in {cwd}: {r.stderr}")
        return r.stdout

    def commit(self, repo: Path, name: str, content: str, msg: str) -> None:
        (repo / name).write_text(content)
        self.git("add", name, cwd=repo)
        self.git("commit", "-q", "-m", msg, cwd=repo)

    def origin(self, branches: list[str], default: str = "main") -> Path:
        """Source repo with one commit per branch, cloned bare to origin.git."""
        src = self.root / "src"
        self.git("init", "-q", "-b", default, str(src))
        self.commit(src, "a.txt", "base\n", "init")
        for b in branches:
            if b != default:
                self.git("branch", b, cwd=src)
        origin = self.root / "origin.git"
        self.git("clone", "-q", "--bare", str(src), str(origin))
        return origin

    def advance_origin(self, branch: str, name: str) -> None:
        src = self.root / "src"
        self.git("switch", "-q", branch, cwd=src)
        self.commit(src, name, f"{branch} {name}\n", f"advance {branch}")
        self.git("push", "-q", str(self.root / "origin.git"), branch, cwd=src)

    def layout(self, worktrees: list[str], default: str = "main") -> Path:
        """The layout `wt init` builds: proj/.bare plus one worktree per branch."""
        proj = self.root / "proj"
        bare = proj / ".bare"
        proj.mkdir()
        self.git("clone", "-q", "--bare", str(self.root / "origin.git"), str(bare))
        self.git("config", "remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*", cwd=bare)
        self.git("fetch", "-q", "--all", cwd=bare)
        self.git("remote", "set-head", "origin", default, cwd=bare)
        for b in worktrees:
            path = proj / b.replace("/", "-")
            self.git("worktree", "add", "-q", str(path), b, cwd=bare)
            self.git("branch", "--set-upstream-to", f"origin/{b}", b, cwd=path)
        self.git("config", "core.bare", "false", cwd=bare)
        return proj


# --- scenarios -------------------------------------------------------------

STD_BRANCHES = ["main", "staging", "feat/login", "feat/x", "hotfix", "remote-only"]
STD_WORKTREES = ["main", "staging", "feat/login", "feat/x", "hotfix"]


def sc_std(s: Scene) -> Path:
    s.origin(STD_BRANCHES)
    return s.layout(STD_WORKTREES)


def sc_mixed(s: Scene) -> Path:
    s.origin(STD_BRANCHES + ["upstream-me"])
    proj = s.layout(STD_WORKTREES)
    bare = proj / ".bare"
    # behind: origin main moves on
    s.advance_origin("main", "m.txt")
    # diverged: both sides of feat/login move
    s.advance_origin("feat/login", "l.txt")
    s.commit(proj / "feat-login", "local.txt", "local\n", "local login")
    # ahead: local commit on staging
    s.commit(proj / "staging", "s.txt", "staged\n", "local staging")
    s.git("fetch", "-q", "--all", "--prune", cwd=bare)
    # dirty (tracked change) and untracked-only
    (proj / "feat-x" / "a.txt").write_text("changed\n")
    (proj / "hotfix" / "new.txt").write_text("untracked\n")
    # detached HEAD worktree
    s.git("worktree", "add", "-q", "--detach", str(proj / "det"), "main", cwd=bare)
    # worktree whose branch has no upstream and no remote branch
    s.git("worktree", "add", "-q", "-b", "local-only", str(proj / "local-only"), "main", cwd=bare)
    # worktree whose branch exists on origin but has no upstream configured
    # (a bare clone creates local branches without tracking configuration)
    s.git("worktree", "add", "-q", str(proj / "upstream-me"), "upstream-me", cwd=bare)
    return proj


def sc_korean(s: Scene) -> Path:
    s.origin(["main", "기능/한글"])
    return s.layout(["main", "기능/한글"])


def sc_empty(s: Scene) -> Path:
    proj = s.root / "proj"
    proj.mkdir()
    s.git("init", "-q", "--bare", str(proj / ".bare"))
    return proj


def sc_normal(s: Scene) -> Path:
    s.origin(STD_BRANCHES)
    s.git("clone", "-q", str(s.root / "origin.git"), str(s.root / "normal"))
    return s.root / "normal"


def sc_origin_only(s: Scene) -> Path:
    s.origin(STD_BRANCHES)
    return s.root


def sc_release(s: Scene) -> Path:
    s.origin(["release/2026", "main"], default="release/2026")
    return s.root


def sc_merged(s: Scene) -> Path:
    """Branches in every state `wt rm -b` distinguishes."""
    s.origin(["main", "staging", "done", "squashed", "wip", "promoted"])
    src = s.root / "src"
    origin = str(s.root / "origin.git")
    # done: merged into main, remote branch still there
    s.git("switch", "-q", "done", cwd=src)
    s.commit(src, "done.txt", "done\n", "done work")
    s.git("push", "-q", origin, "done", cwd=src)
    s.git("switch", "-q", "main", cwd=src)
    s.git("merge", "-q", "--no-ff", "-m", "merge done", "done", cwd=src)
    # squashed: squash-merged into main, remote branch deleted
    s.git("switch", "-q", "squashed", cwd=src)
    s.commit(src, "sq.txt", "squash\n", "squash work")
    s.git("push", "-q", origin, "squashed", cwd=src)
    s.git("switch", "-q", "main", cwd=src)
    s.git("merge", "-q", "--squash", "squashed", cwd=src)
    s.git("commit", "-q", "-m", "squash squashed", cwd=src)
    s.git("push", "-q", origin, "main", cwd=src)
    # wip: unmerged, remote branch exists
    s.git("switch", "-q", "wip", cwd=src)
    s.commit(src, "wip.txt", "wip\n", "wip work")
    s.git("push", "-q", origin, "wip", cwd=src)
    # promoted: merged into staging only, remote branch deleted
    s.git("switch", "-q", "promoted", cwd=src)
    s.commit(src, "pr.txt", "promoted\n", "promoted work")
    s.git("push", "-q", origin, "promoted", cwd=src)
    s.git("switch", "-q", "staging", cwd=src)
    s.git("merge", "-q", "--no-ff", "-m", "merge promoted", "promoted", cwd=src)
    s.git("push", "-q", origin, "staging", cwd=src)
    s.git("switch", "-q", "main", cwd=src)
    proj = s.layout(["main", "staging", "done", "squashed", "wip", "promoted"])
    # The local branches only know their pre-merge tips; delete two remotes.
    s.git("push", "-q", origin, "--delete", "squashed", "promoted", cwd=src)
    # wip also carries an unpushed local commit, so it must be kept.
    s.commit(proj / "wip", "local.txt", "local\n", "unpushed wip")
    return proj


def sc_stale(s: Scene) -> Path:
    proj = sc_std(s)
    shutil.rmtree(proj / "hotfix")
    return proj


def sc_locked(s: Scene) -> Path:
    """A locked worktree: `git worktree remove` refuses it without -f -f."""
    proj = sc_std(s)
    s.git("worktree", "lock", str(proj / "hotfix"), cwd=proj / ".bare")
    return proj


SCENES = {
    "std": sc_std,
    "mixed": sc_mixed,
    "korean": sc_korean,
    "empty": sc_empty,
    "normal": sc_normal,
    "origin": sc_origin_only,
    "release": sc_release,
    "merged": sc_merged,
    "stale": sc_stale,
    "locked": sc_locked,
}

# --- cases -----------------------------------------------------------------
# (id, scene, cwd relative to the scene's returned dir, argv, stdin)

CASES: list[tuple[str, str, str, list[str], str | None]] = [
    # CLI surface
    ("help-noargs", "std", "main", [], None),
    ("help-flag", "std", "main", ["-h"], None),
    ("usage-version", "std", "main", ["--version"], None),
    ("usage-bogus", "std", "main", ["bogus"], None),
    ("usage-subcommand-prefix", "std", "main", ["stat"], None),
    ("usage-interleaved", "std", "main", ["rm", "--dry-run", "feat-x", "-f", "hotfix"], None),
    ("usage-add-missing", "std", "main", ["add"], None),
    ("abbrev-long-dry-run", "std", "main", ["rm", "--dry-r", "hotfix"], None),
    ("abbrev-long-json", "std", "main", ["st", "--js"], None),
    (
        "repeat-option-last-wins",
        "std",
        "main",
        ["add", "-b", "main", "-b", "staging", "newbr"],
        None,
    ),
    ("not-a-repo", "origin", ".", ["status"], None),
    ("from-container-root", "std", ".", ["list"], None),
    # status / list
    ("status-std", "std", "main", ["status"], None),
    ("status-mixed", "mixed", "main", ["st"], None),
    ("status-json-mixed", "mixed", "main", ["status", "--json"], None),
    ("status-empty", "empty", ".", ["status"], None),
    ("status-json-empty", "empty", ".", ["status", "--json"], None),
    ("list-mixed", "mixed", "main", ["ls"], None),
    ("list-json-mixed", "mixed", "main", ["list", "--json"], None),
    ("list-empty", "empty", ".", ["list"], None),
    ("list-json-empty", "empty", ".", ["list", "--json"], None),
    ("list-json-korean", "korean", "main", ["list", "--json"], None),
    ("status-korean", "korean", "main", ["status"], None),
    ("status-normal-clone", "normal", ".", ["status"], None),
    # fetch / prune / pull / upstream
    ("fetch", "mixed", "main", ["fetch"], None),
    ("prune", "stale", "main", ["prune"], None),
    ("prune-dry-run-stale", "stale", "main", ["prune", "--dry-run"], None),
    ("prune-dry-run-clean", "std", "main", ["prune", "--dry-run"], None),
    ("pull-mixed", "mixed", "main", ["pull"], None),
    ("pull-rebase-mixed", "mixed", "main", ["p", "-r"], None),
    ("pull-std", "std", "main", ["pull"], None),
    ("upstream-mixed", "mixed", "main", ["upstream"], None),
    ("upstream-std", "std", "main", ["up"], None),
    # add
    ("add-remote-branch", "std", "main", ["add", "remote-only"], None),
    ("add-new-branch", "std", "main", ["a", "brand-new"], None),
    (
        "add-create-from-base",
        "std",
        "main",
        ["add", "-c", "feat/from-staging", "-b", "staging"],
        None,
    ),
    ("add-existing-local", "mixed", "main", ["add", "local-only", "../elsewhere"], None),
    ("add-path-exists", "std", "main", ["add", "hotfix", "../main"], None),
    ("add-glob-branch", "std", "main", ["add", "ma*"], None),
    ("add-normal-clone", "normal", ".", ["add", "feat/x"], None),
    # remove
    ("rm-suffix-in", "std", "main", ["rm", "--dry-run", "in"], None),
    ("rm-yes", "std", "main", ["rm", "-y", "hotfix"], None),
    ("rm-prompt-no", "std", "main", ["rm", "hotfix"], "n\n"),
    ("rm-prompt-yes", "std", "main", ["rm", "hotfix"], "yes\n"),
    ("rm-prompt-eof", "std", "main", ["rm", "hotfix"], ""),
    ("rm-no-match", "std", "main", ["rm", "nothing-here"], None),
    ("rm-no-args", "std", "main", ["rm"], None),
    ("rm-remote-without-b", "std", "main", ["rm", "--remote", "hotfix"], None),
    ("rm-match-dry-run", "std", "main", ["rm", "-m", "feat*", "-m", "nomatch*", "--dry-run"], None),
    ("rm-partial-missing", "std", "main", ["rm", "--dry-run", "hotfix", "ghost"], None),
    ("rm-dirty-skip", "mixed", "main", ["rm", "-y", "feat-x"], None),
    ("rm-force-dirty", "mixed", "main", ["rm", "-y", "-f", "feat-x"], None),
    ("rm-detached", "mixed", "main", ["rm", "-y", "-b", "det"], None),
    (
        "rm-branch-dry-run",
        "merged",
        "main",
        ["rm", "--dry-run", "-b", "--remote", "done", "squashed", "wip", "promoted"],
        None,
    ),
    ("rm-branch-force-dry-run", "merged", "main", ["rm", "--dry-run", "-f", "-b", "done"], None),
    (
        "rm-branch-yes",
        "merged",
        "main",
        ["rm", "-y", "-b", "done", "squashed", "wip", "promoted"],
        None,
    ),
    ("rm-branch-remote-yes", "merged", "main", ["rm", "-y", "-b", "--remote", "done", "wip"], None),
    ("rm-branch-force", "merged", "main", ["rm", "-y", "-f", "-b", "wip"], None),
    ("rm-locked-fails", "locked", "main", ["rm", "-y", "-b", "hotfix"], None),
    # init
    ("init-basic", "origin", ".", ["init", "file://{root}/origin.git", "target"], None),
    (
        "init-worktrees",
        "origin",
        ".",
        ["init", "{root}/origin.git", "t2", "-w", "feat/login, hotfix,missing,main,"],
        None,
    ),
    ("init-default-name", "origin", ".", ["init", "{root}/origin.git"], None),
    ("init-empty-w", "origin", ".", ["init", "{root}/origin.git", "t3", "-w", ""], None),
    ("init-release-default", "release", ".", ["init", "{root}/origin.git", "rel"], None),
    ("init-nonempty-dir", "origin", ".", ["init", "{root}/origin.git", "src"], None),
    ("init-into-file", "origin", ".", ["init", "{root}/origin.git", "src/a.txt"], None),
    ("init-clone-fails", "origin", ".", ["init", "{root}/missing.git", "bad"], None),
    # Port 9 (discard) on localhost refuses at once; no network is used.
    (
        "init-credential-url",
        "origin",
        ".",
        ["init", "https://user:SECRET-TOKEN@127.0.0.1:9/x.git", "cred"],
        None,
    ),
]

# Completion: (id, scene, cwd, command line). Compared as candidate sets of
# values (options and subcommand aliases are excluded; see docs).
COMPLETIONS: list[tuple[str, str, str, str]] = [
    ("complete-rm-empty", "std", "main", "wt rm "),
    ("complete-rm-substring", "std", "main", "wt rm og"),
    ("complete-rm-case", "std", "main", "wt rm LOG"),
    ("complete-add-branch", "std", "main", "wt add feat"),
    ("complete-add-base", "std", "main", "wt add -b st"),
    ("complete-add-empty", "std", "main", "wt add "),
    ("complete-outside", "origin", ".", "wt rm "),
]

ALIASES = {"a", "rm", "st", "f", "p", "ls", "up", "help"}


def normalise(text: str, root: Path) -> str:
    for variant in {str(root), str(root.resolve()), "/private" + str(root)}:
        text = text.replace(variant, "<ROOT>")
    # The transport error after "unable to access" comes from curl and its
    # wording differs between platforms and curl versions.
    return re.sub(r"(fatal: unable to access '[^']*'): .*", r"\1: <transport error>", text)


def build(scene: str, base: Path) -> tuple[Scene, Path]:
    root = Path(tempfile.mkdtemp(dir=base)).resolve()
    s = Scene(root)
    target = SCENES[scene](s)
    return s, target


def run_case(wt: list[str], case: tuple, base: Path) -> dict:
    cid, scene, cwd_rel, argv, stdin = case
    s, target = build(scene, base)
    argv = [a.replace("{root}", str(s.root)) for a in argv]
    r = subprocess.run(
        [*wt, *argv],
        cwd=target / cwd_rel,
        env=s.env,
        input=stdin if stdin is not None else "",
        capture_output=True,
        text=True,
        check=False,
    )
    return {
        "argv": argv and [normalise(a, s.root) for a in argv],
        "code": r.returncode,
        "stdout": normalise(r.stdout, s.root),
        "stderr": normalise(r.stderr, s.root),
    }


def _fd_open(fd: int) -> bool:
    try:
        os.fstat(fd)
    except OSError:
        return False
    return True


def complete_python(wt: list[str], s: Scene, cwd: Path, line: str) -> list[str]:
    env = dict(s.env)
    env.update(
        {
            "_ARGCOMPLETE": "1",
            "_ARGCOMPLETE_SHELL": "bash",
            "_ARGCOMPLETE_IFS": "\013",
            "COMP_LINE": line,
            "COMP_POINT": str(len(line)),
            "COMP_TYPE": "9",
        }
    )
    # argcomplete writes candidates to fd 8 and debug output to fd 9.
    r_fd, w_fd = os.pipe()
    null_fd = os.open(os.devnull, os.O_WRONLY)
    saved = {fd: os.dup(fd) if _fd_open(fd) else None for fd in (8, 9)}
    os.dup2(w_fd, 8)
    os.dup2(null_fd, 9)
    try:
        proc = subprocess.Popen(
            [*wt],
            cwd=cwd,
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            pass_fds=(8, 9),
        )
    finally:
        for fd, old in saved.items():
            if old is None:
                os.close(fd)
            else:
                os.dup2(old, fd)
                os.close(old)
        os.close(w_fd)
        os.close(null_fd)
    with os.fdopen(r_fd) as f:
        out = f.read()
    proc.wait()
    return [c.strip() for c in out.split("\013") if c.strip()]


def complete_rust(wt: list[str], s: Scene, cwd: Path, line: str) -> list[str]:
    words = line.split(" ")
    env = dict(s.env)
    env.update(
        {
            "COMPLETE": "zsh",
            "_CLAP_IFS": "\n",
            "_CLAP_COMPLETE_INDEX": str(len(words) - 1),
        }
    )
    r = subprocess.run(
        [*wt, "--", *words], cwd=cwd, env=env, capture_output=True, text=True, check=False
    )
    return [c.split(":", 1)[0] for c in r.stdout.split("\n") if c.strip()]


def values_only(cands: list[str]) -> list[str]:
    return sorted(c for c in cands if not c.startswith("-") and c not in ALIASES)


def run_completion(wt: list[str], comp: tuple, base: Path, rust: bool) -> list[str]:
    cid, scene, cwd_rel, line = comp
    s, target = build(scene, base)
    fn = complete_rust if rust else complete_python
    return values_only(fn(wt, s, target / cwd_rel, line))


def py_wt() -> list[str]:
    if "PY_WT" in os.environ:
        return shlex.split(os.environ["PY_WT"])
    # Resolve the uvx-managed interpreter once instead of paying uvx start-up
    # on every case; `-m wtree.cli` runs the same main() as the `wt` script.
    exe = subprocess.run(
        ["uvx", "--from", "wtree==1.2.2", "python", "-c", "import sys; print(sys.executable)"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()
    return [exe, "-m", "wtree.cli"]


def collect(wt: list[str], rust: bool) -> dict:
    out: dict = {"cases": {}, "completions": {}}
    with tempfile.TemporaryDirectory(prefix="wt-golden-") as tmp:
        base = Path(tmp)
        for case in CASES:
            out["cases"][case[0]] = run_case(wt, case, base)
        for comp in COMPLETIONS:
            out["completions"][comp[0]] = run_completion(wt, comp, base, rust)
    return out


def write(path: Path, data: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2, ensure_ascii=False, sort_keys=True) + "\n")


def compare(py: dict, rs: dict, rs_expected: dict | None) -> list[str]:
    problems = []
    for cid, _scene, _cwd, _argv, _stdin in CASES:
        got = rs["cases"][cid]
        if cid in CHANGED:
            if rs_expected is None or cid not in rs_expected.get("cases", {}):
                problems.append(f"{cid}: CHANGED but no recorded Rust expectation")
                continue
            want = rs_expected["cases"][cid]
            fields = ("code", "stdout", "stderr")
        else:
            want = py["cases"][cid]
            kind = EXCEPTIONS.get(cid, ("full", ""))[0]
            fields = {
                "full": ("code", "stdout", "stderr"),
                "stdout-code": ("code", "stdout"),
                "code-only": ("code",),
            }[kind]
        for field in fields:
            if got[field] != want[field]:
                problems.append(
                    f"{cid}: {field} differs\n--- expected\n{want[field]}\n--- got\n{got[field]}"
                )
    for comp in COMPLETIONS:
        cid = comp[0]
        if rs["completions"][cid] != py["completions"][cid]:
            problems.append(
                f"{cid}: candidates differ: expected {py['completions'][cid]} got {rs['completions'][cid]}"
            )
    return problems


def main() -> int:
    parser = argparse.ArgumentParser(description="wt golden parity harness")
    parser.add_argument("mode", choices=["record", "record-rust", "check"])
    parser.add_argument("--wt", help="path to the Rust wt binary")
    parser.add_argument("--offline", action="store_true", help="use the recorded Python fixture")
    args = parser.parse_args()

    if args.mode == "record":
        write(PY_FIXTURE, collect(py_wt(), rust=False))
        print(f"recorded {PY_FIXTURE.relative_to(REPO)}")
        return 0

    if not args.wt:
        parser.error("--wt is required")
    rust_cmd = [str(Path(args.wt).resolve())]
    rs = collect(rust_cmd, rust=True)

    if args.mode == "record-rust":
        subset = {
            "cases": {k: v for k, v in rs["cases"].items() if k in CHANGED},
            "reasons": CHANGED,
        }
        write(RS_FIXTURE, subset)
        print(f"recorded {RS_FIXTURE.relative_to(REPO)} ({len(CHANGED)} changed cases)")
        return 0

    py = json.loads(PY_FIXTURE.read_text()) if args.offline else collect(py_wt(), rust=False)
    rs_expected = json.loads(RS_FIXTURE.read_text()) if RS_FIXTURE.exists() else None
    problems = compare(py, rs, rs_expected)
    total = len(CASES) + len(COMPLETIONS)
    if problems:
        print("\n\n".join(problems))
        print(f"\nFAIL {len(problems)} difference(s) across {total} cases")
        return 1
    print(f"OK {total} cases match ({len(EXCEPTIONS)} exceptions, {len(CHANGED)} changed)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
