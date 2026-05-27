#!/usr/bin/env bash
set -euo pipefail

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

if [ "${1:-}" ]; then
  repo="$(cd "$1" && pwd)"
else
  repo="$(default_repo)"
fi
ax="${AX_BIN:-$("$script_dir/ensure-ax.sh" "$repo")}"
work="${AX_SKILL_SMOKE_DIR:-$repo/.ax-out/skill-pack-smoke}"
registry="$work/registry"
pack="acme.telemetry"
effect="telemetry.write"
operation="track"

rm -rf "$work"
mkdir -p "$work"

"$script_dir/new-pack.sh" "$registry" "$pack" "$effect" "$operation" >/dev/null

cat > "$work/app.ax" <<'EOF'
+acme.telemetry {telemetry.track()}
EOF

(
  cd "$work"
  AX_BIN="$ax" "$script_dir/pack.sh" --repo "$repo" list --registry "$registry" | grep -F "$pack 1.0.0" >/dev/null
  AX_BIN="$ax" "$script_dir/pack.sh" --repo "$repo" find telemetry --registry "$registry" | grep -F "$pack 1.0.0" >/dev/null
  AX_BIN="$ax" "$script_dir/pack.sh" --repo "$repo" find telemetry.track --registry "$registry" | grep -F "$pack 1.0.0" >/dev/null
  AX_BIN="$ax" "$script_dir/pack.sh" --repo "$repo" info "$pack" --registry "$registry" | grep -F "effects $effect" >/dev/null
  AX_BIN="$ax" "$script_dir/pack.sh" --repo "$repo" info "$pack" --registry "$registry" | grep -F "operations telemetry.$operation" >/dev/null
  AX_BIN="$ax" "$script_dir/pack.sh" --repo "$repo" install "$pack" --registry "$registry" >/dev/null
  grep -F 'acme.telemetry = "1.0"' ax.toml >/dev/null
  "$ax" check app.ax --registry "$registry" >/dev/null
  "$ax" graph app.ax --registry "$registry" | grep -F "$effect" >/dev/null
  "$ax" build app.ax -o app --registry "$registry" >/dev/null
  ./app | grep -F "$pack $operation" >/dev/null
)

echo "Ax skill pack smoke passed"
