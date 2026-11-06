#!/usr/bin/env bash
set -euo pipefail
source /opt/demo/setup.sh
exec env -i HOME=/demo/home TMPDIR=/demo/tmp PATH=/usr/local/bin:/usr/bin:/bin \
  TERM=xterm-256color PS1='$ ' HISTFILE=/dev/null \
  GIT_CONFIG_NOSYSTEM=1 GIT_TERMINAL_PROMPT=0 \
  bash --noprofile --norc -i
