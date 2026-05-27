#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="$ROOT/.ax-out/benchmarks"
kind="${1:?usage: run-one.sh ax|c|rust|python|node}"
requests="${AX_WEB_REQUESTS:-1000}"

case "$kind" in
  ax)
    port=3200
    command=("$OUT/web_ax")
    ;;
  c)
    port=3201
    command=("$OUT/web_c")
    ;;
  rust)
    port=3202
    command=("$OUT/web_rust")
    ;;
  python)
    port=3203
    command=(python3 "$ROOT/benchmarks/python/web.py")
    ;;
  node)
    port=3204
    command=(node "$ROOT/benchmarks/js/web.js")
    ;;
  *)
    echo "unknown web benchmark runtime: $kind" >&2
    exit 2
    ;;
esac

"${command[@]}" > "$OUT/web-$kind.log" 2>&1 &
server_pid=$!

cleanup() {
  kill "$server_pid" >/dev/null 2>&1 || true
  wait "$server_pid" >/dev/null 2>&1 || true
}
trap cleanup EXIT

ready=0
for _ in $(seq 1 80); do
  if python3 "$ROOT/benchmarks/web_client.py" --port "$port" --requests 1 >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 0.05
done

if [ "$ready" -ne 1 ]; then
  cat "$OUT/web-$kind.log" >&2 || true
  exit 1
fi

python3 "$ROOT/benchmarks/web_client.py" --port "$port" --requests "$requests" >/dev/null
