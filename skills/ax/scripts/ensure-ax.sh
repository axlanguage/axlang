#!/usr/bin/env bash
set -euo pipefail

repo="${1:-${AX_HOME:-.}}"
if [ -d "$repo" ]; then
  repo="$(cd "$repo" && pwd)"
fi
bin_dir="${AX_BIN_DIR:-$HOME/.ax/bin}"

if [ -f "$repo/Cargo.toml" ] && [ -d "$repo/crates/ax_cli" ]; then
  (cd "$repo" && cargo build --release >/dev/null)
  printf '%s\n' "$repo/target/release/ax"
  exit 0
fi

if command -v ax >/dev/null 2>&1; then
  command -v ax
  exit 0
fi

if [ -x "$bin_dir/ax" ]; then
  printf '%s\n' "$bin_dir/ax"
  exit 0
fi

if [ -f "$repo/Cargo.toml" ]; then
  (cd "$repo" && cargo build --release >/dev/null)
  printf '%s\n' "$repo/target/release/ax"
  exit 0
fi

if [ -x "$repo/dist/install.sh" ]; then
  AX_BIN_DIR="$bin_dir" "$repo/dist/install.sh" >/dev/null
  printf '%s\n' "$bin_dir/ax"
  exit 0
fi

if command -v curl >/dev/null 2>&1; then
  curl -fsSL https://raw.githubusercontent.com/axlanguage/axlang/main/dist/install.sh | AX_BIN_DIR="$bin_dir" sh >/dev/null
  printf '%s\n' "$bin_dir/ax"
  exit 0
fi

echo "Ax binary not found. Set AX_HOME, install Ax, or install curl for the release installer." >&2
exit 1
