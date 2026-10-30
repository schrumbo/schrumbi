#!/bin/bash
# Starts the debug build, which walks through every page and window, and checks that it finished.
set -u
export SCHRUMBI_SMOKE=1 SCHRUMBI_HOME="${RUNNER_TEMP:-/tmp}/schrumbi-smoke-home"
BIN=target/debug/schrumbi
case "$(uname -s)" in
  Linux) RUN=(timeout 120 xvfb-run -a "$BIN") ;;
  Darwin) RUN=(perl -e 'alarm 120; exec @ARGV' "$BIN") ;;
  *) RUN=(timeout 120 "$BIN.exe") ;;
esac
OUT=$("${RUN[@]}" 2>&1); CODE=$?
echo "$OUT" | tail -40
echo "exit code $CODE"
echo "$OUT" | grep -q "smoke ok"
