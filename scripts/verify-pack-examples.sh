#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
AX="${AX_BIN:-$ROOT/target/release/ax}"

if [ -n "${AX_BIN:-}" ] && [ -x "$AX_BIN" ]; then
  AX="$AX_BIN"
else
  (cd "$ROOT" && cargo build --release --bin ax >/dev/null)
  AX="$ROOT/target/release/ax"
fi

OUT="$ROOT/.ax-out/pack-examples"
PROJECT="$ROOT/examples/packs/telemetry"
mkdir -p "$OUT"

(
  cd "$PROJECT"
  "$AX" pack list --registry registry | grep -F "acme.telemetry 1.0.0" >/dev/null
  "$AX" pack find telemetry --registry registry | grep -F "acme.telemetry 1.0.0" >/dev/null
  "$AX" pack find telemetry.track --registry registry | grep -F "acme.telemetry 1.0.0" >/dev/null
  "$AX" pack info acme.telemetry --registry registry | grep -F "effects telemetry.write" >/dev/null
  "$AX" pack info acme.telemetry --registry registry | grep -F "operations telemetry.track" >/dev/null
  "$AX" check app.ax --registry registry >/dev/null
  "$AX" graph app.ax --registry registry | grep -F "telemetry.write" >/dev/null
  "$AX" build app.ax -o "$OUT/telemetry_app" --registry registry >/dev/null
)

"$OUT/telemetry_app" | grep -F "acme.telemetry track" >/dev/null

echo "Ax pack examples verification passed"
