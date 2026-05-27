#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/dist/bin"
SH_BIN_DIR="$ROOT/.ax-out/release-install-sh"
PS_BIN_DIR="$ROOT/.ax-out/release-install-ps"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) CAN_RUN_WINDOWS=1 ;;
  *) CAN_RUN_WINDOWS=0 ;;
esac

"$ROOT/dist/build-release.sh"

rm -rf "$SH_BIN_DIR"
AX_RELEASE_BASE="file://$OUT" AX_BIN_DIR="$SH_BIN_DIR" "$ROOT/dist/install.sh"
"$SH_BIN_DIR/ax" version
std_backup="$ROOT/.ax-out/release-std-hidden"
runtime_backup="$ROOT/.ax-out/release-runtime-hidden"
rm -rf "$std_backup"
rm -rf "$runtime_backup"
mv "$ROOT/std" "$std_backup"
mv "$ROOT/crates/ax_runtime/src" "$runtime_backup"
restore_std() {
  if [ -d "$std_backup" ] && [ ! -d "$ROOT/std" ]; then
    mv "$std_backup" "$ROOT/std"
  fi
  if [ -d "$runtime_backup" ] && [ ! -d "$ROOT/crates/ax_runtime/src" ]; then
    mv "$runtime_backup" "$ROOT/crates/ax_runtime/src"
  fi
}
trap restore_std EXIT
"$SH_BIN_DIR/ax" packs
"$SH_BIN_DIR/ax" run "$ROOT/examples/agents/fs_journal.ax"
"$SH_BIN_DIR/ax" run "$ROOT/examples/release_smoke.ax" -- --mode release
restore_std
trap - EXIT

if command -v pwsh >/dev/null 2>&1 && [ "$CAN_RUN_WINDOWS" -eq 1 ] && [ -f "$OUT/ax-windows-x64.exe" ]; then
  rm -rf "$PS_BIN_DIR"
  AX_RELEASE_BASE="file://$OUT" AX_BIN_DIR="$PS_BIN_DIR" pwsh -NoProfile -File "$ROOT/dist/install.ps1"
  if [ -x "$PS_BIN_DIR/ax.exe" ]; then
    "$PS_BIN_DIR/ax.exe" version
    "$PS_BIN_DIR/ax.exe" packs
    "$PS_BIN_DIR/ax.exe" run "$ROOT/examples/release_smoke.ax" -- --mode release
  else
    "$PS_BIN_DIR/ax" version
    "$PS_BIN_DIR/ax" packs
    "$PS_BIN_DIR/ax" run "$ROOT/examples/release_smoke.ax" -- --mode release
  fi
else
  echo "pwsh unavailable, non-Windows host, or ax-windows-x64.exe not staged; skipped PowerShell installer smoke"
fi

echo "Ax release verification passed"
