#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

mkdir -p .ax-out

cargo fmt --all -- --check
cargo test --workspace
cargo run --quiet -- test
cargo run --quiet -- packs
./scripts/verify-skill.sh
./scripts/verify-pack-examples.sh

while IFS= read -r file; do
  case "$file" in
    examples/packs/*) continue ;;
  esac
  cargo run --quiet -- check "$file"
done < <(find examples benchmarks/ax -type f -name '*.ax' | sort)

cargo run --quiet -- run examples/agents/file_digest.ax
cargo run --quiet -- run examples/agents/fs_journal.ax
cargo run --quiet -- run examples/release_smoke.ax -- --mode release
cargo run --quiet -- run examples/agents/signed_manifest.ax
cargo run --quiet -- run examples/agents/secure_token.ax
AX_AGENT_NAME=ax-verifier cargo run --quiet -- run examples/agents/env_report.ax
cargo run --quiet -- run examples/agents/process_probe.ax
cargo run --quiet -- run examples/agents/json_tool.ax
cargo run --quiet -- run examples/agents/string_agent.ax
cargo run --quiet -- run examples/agents/path_manifest.ax
cargo run --quiet -- run examples/agents/cli_probe.ax -- --input examples/agents/path_manifest.ax --mode digest --upper
cargo run --quiet -- run examples/agents/io_diagnostics.ax
cargo run --quiet -- run examples/agents/time_probe.ax
cargo run --quiet -- run examples/agents/url_router.ax
cargo run --quiet -- build examples/agents/tool_server.ax -o .ax-out/http_fetch_server >/dev/null
.ax-out/http_fetch_server > .ax-out/http_fetch_server.log 2>&1 &
http_fetch_pid=$!
cleanup_http_fetch() {
  kill "$http_fetch_pid" >/dev/null 2>&1 || true
}
trap cleanup_http_fetch EXIT
http_fetch_ready=0
for _ in 1 2 3 4 5 6 7 8 9 10; do
  if curl -fsS http://127.0.0.1:3010/health >/dev/null; then
    http_fetch_ready=1
    break
  fi
  sleep 0.2
done
if [ "$http_fetch_ready" -ne 1 ]; then
  kill "$http_fetch_pid" >/dev/null 2>&1 || true
  wait "$http_fetch_pid" >/dev/null 2>&1 || true
  cat .ax-out/http_fetch_server.log >&2
  exit 1
fi
cargo run --quiet -- run examples/agents/http_fetch.ax
cleanup_http_fetch
wait "$http_fetch_pid" >/dev/null 2>&1 || true
trap - EXIT
cargo run --quiet -- build examples/agents/async_score.ax -o .ax-out/async_score_agent
set +e
.ax-out/async_score_agent
async_score_status=$?
set -e
if [ "$async_score_status" -ne 100 ]; then
  echo "expected examples/agents/async_score.ax to exit 100, got $async_score_status" >&2
  exit 1
fi

DOCS_PORT="${AX_DOCS_PORT:-4173}"
python3 -m http.server "$DOCS_PORT" -d docs > .ax-out/docs-server.log 2>&1 &
docs_pid=$!
cleanup() {
  kill "$docs_pid" >/dev/null 2>&1 || true
}
trap cleanup EXIT
sleep 1

for page in index.html install.html language.html packs.html pack-authoring.html agents.html benchmarks.html styles.css; do
  curl -fsS "http://127.0.0.1:$DOCS_PORT/$page" >/dev/null
done

if [ "${AX_VERIFY_BENCHMARKS:-0}" = "1" ]; then
  ./benchmarks/run.sh
fi

if [ "${AX_VERIFY_RELEASE:-0}" = "1" ]; then
  ./dist/verify-release.sh
fi

echo "Ax verification passed"
