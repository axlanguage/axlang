#!/usr/bin/env bash
set -euo pipefail

usage="usage: expect-exit.sh <expected-code> -- <command> [args...]"
expected="${1:?$usage}"
shift
if [ "${1:-}" = "--" ]; then
  shift
fi
if [ "$#" -eq 0 ]; then
  echo "$usage" >&2
  exit 2
fi

set +e
"$@"
actual=$?
set -e

if [ "$actual" -ne "$expected" ]; then
  echo "benchmark result mismatch: expected exit $expected, got $actual: $*" >&2
  exit 1
fi
