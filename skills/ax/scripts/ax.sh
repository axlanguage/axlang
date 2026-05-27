#!/usr/bin/env bash
set -euo pipefail

usage="usage: ax.sh [--repo <ax-repo>] <check|graph|explain|build|compile|run|test|version|packs|init> [args...]"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

default_repo() {
  if [ -n "${AX_HOME:-}" ]; then
    cd "$AX_HOME" && pwd
    return
  fi

  local dir="$PWD"
  while [ "$dir" != "/" ]; do
    if [ -f "$dir/Cargo.toml" ] && [ -d "$dir/crates/ax_cli" ]; then
      printf '%s\n' "$dir"
      return
    fi
    dir="$(dirname "$dir")"
  done

  cd "$script_dir/../../.." && pwd
}

repo="$(default_repo)"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --repo)
      repo="$(cd "${2:?$usage}" && pwd)"
      shift 2
      ;;
    --help|-h)
      echo "$usage"
      exit 0
      ;;
    *)
      break
      ;;
  esac
done

cmd="${1:-}"
if [ -z "$cmd" ]; then
  echo "$usage" >&2
  exit 2
fi
shift

ax="${AX_BIN:-$("$script_dir/ensure-ax.sh" "$repo")}"

has_output_arg() {
  for arg in "$@"; do
    if [ "$arg" = "-o" ]; then
      return 0
    fi
  done
  return 1
}

case "$cmd" in
  check|graph|explain|run|test|version|packs|init)
    "$ax" "$cmd" "$@"
    ;;
  build|compile)
    file="${1:-}"
    if [ -z "$file" ]; then
      echo "$usage" >&2
      exit 2
    fi
    if has_output_arg "$@"; then
      "$ax" build "$@"
    else
      out_dir="${AX_OUT:-.ax-out/skill-builds}"
      mkdir -p "$out_dir"
      base="$(basename "$file" .ax)"
      output="$out_dir/$base"
      "$ax" build "$file" -o "$output" "${@:2}"
      echo "$output"
    fi
    ;;
  *)
    echo "unknown ax skill command: $cmd" >&2
    echo "$usage" >&2
    exit 2
    ;;
esac
