#!/usr/bin/env bash
set -euo pipefail

VERSION="${AX_VERSION:-latest}"
if [ -n "${AX_RELEASE_BASE:-}" ]; then
  BASE="$AX_RELEASE_BASE"
elif [ "$VERSION" = "latest" ]; then
  BASE="https://github.com/axlanguage/axlang/releases/latest/download"
else
  BASE="https://github.com/axlanguage/axlang/releases/download/$VERSION"
fi
BIN_DIR="${AX_BIN_DIR:-$HOME/.ax/bin}"

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) target="darwin-arm64" ;;
  Darwin-x86_64) target="darwin-x64" ;;
  Linux-x86_64) target="linux-x64" ;;
  Linux-aarch64|Linux-arm64) target="linux-arm64" ;;
  *) echo "unsupported platform: $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac

mkdir -p "$BIN_DIR"
tmp="$(mktemp)"
sums="$(mktemp)"
trap 'rm -f "$tmp" "$sums"' EXIT

curl -fsSL "$BASE/ax-$target" -o "$tmp"

if curl -fsSL "$BASE/SHA256SUMS" -o "$sums"; then
  expected="$(awk -v name="ax-$target" '$2 == name { print $1 }' "$sums")"
  if [ -n "$expected" ]; then
    if command -v sha256sum >/dev/null 2>&1; then
      actual="$(sha256sum "$tmp" | awk '{ print $1 }')"
    elif command -v shasum >/dev/null 2>&1; then
      actual="$(shasum -a 256 "$tmp" | awk '{ print $1 }')"
    else
      echo "warning: SHA256SUMS found but no sha256sum or shasum command is available" >&2
      actual=""
    fi
    if [ -n "$actual" ] && [ "$actual" != "$expected" ]; then
      echo "checksum mismatch for ax-$target" >&2
      echo "expected $expected" >&2
      echo "actual   $actual" >&2
      exit 1
    fi
  else
    echo "warning: no SHA256SUMS entry for ax-$target" >&2
  fi
else
  echo "warning: SHA256SUMS unavailable; installing without checksum verification" >&2
fi

chmod 755 "$tmp"
mv "$tmp" "$BIN_DIR/ax"

echo "installed ax to $BIN_DIR/ax"
echo "add this to PATH if needed:"
echo "  export PATH=\"$BIN_DIR:\$PATH\""
