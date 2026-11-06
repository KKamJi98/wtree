#!/usr/bin/env bash
# Run ONLY inside the disposable demo container. No host directories are mounted.
set -euo pipefail
if [[ ! -f /wtree-demo-container ]]; then
  printf 'Refusing to create fixtures outside the demo container.\n' >&2
  exit 1
fi
export HOME=/demo/home TMPDIR=/demo/tmp
export GIT_CONFIG_NOSYSTEM=1 GIT_TERMINAL_PROMPT=0
export GIT_AUTHOR_NAME='Demo Developer' GIT_COMMITTER_NAME='Demo Developer'
export GIT_AUTHOR_EMAIL='demo@example.invalid' GIT_COMMITTER_EMAIL='demo@example.invalid'
export GIT_AUTHOR_DATE='2026-01-01T00:00:00Z' GIT_COMMITTER_DATE='2026-01-01T00:00:00Z'
mkdir -p "$HOME/.aws" "$HOME/.ssh" "$TMPDIR"
printf '[profile demo]\nregion = us-east-1\n' > "$HOME/.aws/config"
printf 'Host demo\n  HostName demo.invalid\n' > "$HOME/.ssh/config"
git config --global init.defaultBranch main
git config --global protocol.allow never
git config --global protocol.file.allow always
git config --global commit.gpgsign false
git config --global core.hooksPath /dev/null
git init -q --bare /demo/remote.git
git clone -q /demo/remote.git /demo/seed 2>/dev/null
cd /demo/seed
printf '# Demo project\n' > README.md
git add README.md
git commit -qm 'Initial demo project'
for branch in feat/login fix/typo release/1.2; do git branch "$branch"; done
git push -q --all origin
wt init /demo/remote.git /demo/project -w feat/login,fix/typo,release/1.2 >/dev/null
cd /demo/project/feat-login
printf 'Login form\n' > login.txt
git add login.txt
git commit -qm 'Draft login form'
printf 'Unfinished styling\n' >> login.txt
cd /demo/seed
printf 'Updated guide\n' > guide.txt
git add guide.txt
git commit -qm 'Update the guide'
git push -q origin main
git switch -q fix/typo
printf 'Corrected label\n' > label.txt
git add label.txt
git commit -qm 'Correct a label'
git push -q origin fix/typo
# Refresh remote-tracking refs so the opening status shows real behind counts.
cd /demo/project/.bare
git fetch -q --all
cd /demo/project
