#!/usr/bin/env bash
set -euo pipefail

usage="usage: pack.sh [--repo <ax-repo>] <list|find|search|info|install|add> [args...] [--registry <source>]"
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
pack_args=()

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
      pack_args+=("$1")
      shift
      ;;
  esac
done

if [ "${#pack_args[@]}" -eq 0 ]; then
  echo "$usage" >&2
  exit 2
fi

ax="${AX_BIN:-$("$script_dir/ensure-ax.sh" "$repo")}"
"$ax" pack "${pack_args[@]}"
