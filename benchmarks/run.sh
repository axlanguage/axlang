#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/.ax-out/benchmarks"
mkdir -p "$OUT"
REPORT="$OUT/report.md"
BENCH_RUNS="${AX_BENCH_RUNS:-20}"
BENCH_WARMUP="${AX_BENCH_WARMUP:-5}"
AX_WEB_REQUESTS="${AX_WEB_REQUESTS:-1000}"
export AX_WEB_REQUESTS
AX_TEXT_VALUE="${AX_TEXT_VALUE:-agent-native-compiler-runtime-pack}"

cd "$ROOT"
printf '# Ax Benchmark Report\n\nruns: %s\nwarmup: %s\nweb requests: %s\n\n' "$BENCH_RUNS" "$BENCH_WARMUP" "$AX_WEB_REQUESTS" > "$REPORT"

expect_cmd() {
  local expected="$1"
  shift
  printf '%q ' "$ROOT/benchmarks/expect-exit.sh" "$expected" "--" "$@"
}

cargo run --quiet -- build benchmarks/ax/fib.ax -o "$OUT/fib_ax" >/dev/null
cargo run --quiet -- build benchmarks/ax/mix.ax -o "$OUT/mix_ax" >/dev/null
cargo run --quiet -- build benchmarks/ax/text.ax -o "$OUT/text_ax" >/dev/null
cargo run --quiet -- build benchmarks/ax/path.ax -o "$OUT/path_ax" >/dev/null
cargo run --quiet -- build benchmarks/ax/url.ax -o "$OUT/url_ax" >/dev/null
cargo run --quiet -- build benchmarks/ax/cli.ax -o "$OUT/cli_ax" >/dev/null
cargo run --quiet -- build benchmarks/ax/json.ax -o "$OUT/json_ax" >/dev/null
cargo run --quiet -- build benchmarks/ax/web.ax -o "$OUT/web_ax" >/dev/null
clang -O2 benchmarks/c/fib.c -o "$OUT/fib_c"
clang -O2 benchmarks/c/mix.c -o "$OUT/mix_c"
clang -O2 benchmarks/c/text.c -o "$OUT/text_c"
clang -O2 benchmarks/c/path.c -o "$OUT/path_c"
clang -O2 benchmarks/c/url.c -o "$OUT/url_c"
clang -O2 benchmarks/c/cli.c -o "$OUT/cli_c"
clang -O2 benchmarks/c/json.c -o "$OUT/json_c"
clang -O2 benchmarks/c/web.c -o "$OUT/web_c"

fib_commands=(
  "$(expect_cmd 115 "$OUT/fib_ax")"
  "$(expect_cmd 115 "$OUT/fib_c")"
)
mix_commands=(
  "$(expect_cmd 79 "$OUT/mix_ax")"
  "$(expect_cmd 79 "$OUT/mix_c")"
)
text_commands=(
  "$(expect_cmd 199 env AX_TEXT="$AX_TEXT_VALUE" "$OUT/text_ax")"
  "$(expect_cmd 199 env AX_TEXT="$AX_TEXT_VALUE" "$OUT/text_c")"
)
path_commands=(
  "$(expect_cmd 21 "$OUT/path_ax")"
  "$(expect_cmd 21 "$OUT/path_c")"
)
url_commands=(
  "$(expect_cmd 56 "$OUT/url_ax")"
  "$(expect_cmd 56 "$OUT/url_c")"
)
cli_commands=(
  "$(expect_cmd 111 "$OUT/cli_ax" --input examples/agents/path_manifest.ax --mode digest)"
  "$(expect_cmd 111 "$OUT/cli_c" --input examples/agents/path_manifest.ax --mode digest)"
)
json_commands=(
  "$(expect_cmd 88 "$OUT/json_ax")"
  "$(expect_cmd 88 "$OUT/json_c")"
)
web_commands=(
  "$ROOT/benchmarks/web/run-one.sh ax"
  "$ROOT/benchmarks/web/run-one.sh c"
)

if command -v rustc >/dev/null 2>&1; then
  rustc -O benchmarks/rust/fib.rs -o "$OUT/fib_rust"
  rustc -O benchmarks/rust/mix.rs -o "$OUT/mix_rust"
  rustc -O benchmarks/rust/text.rs -o "$OUT/text_rust"
  rustc -O benchmarks/rust/path.rs -o "$OUT/path_rust"
  rustc -O benchmarks/rust/url.rs -o "$OUT/url_rust"
  rustc -O benchmarks/rust/cli.rs -o "$OUT/cli_rust"
  rustc -O benchmarks/rust/json.rs -o "$OUT/json_rust"
  rustc -O benchmarks/rust/web.rs -o "$OUT/web_rust"
  fib_commands+=("$(expect_cmd 115 "$OUT/fib_rust")")
  mix_commands+=("$(expect_cmd 79 "$OUT/mix_rust")")
  text_commands+=("$(expect_cmd 199 env AX_TEXT="$AX_TEXT_VALUE" "$OUT/text_rust")")
  path_commands+=("$(expect_cmd 21 "$OUT/path_rust")")
  url_commands+=("$(expect_cmd 56 "$OUT/url_rust")")
  cli_commands+=("$(expect_cmd 111 "$OUT/cli_rust" --input examples/agents/path_manifest.ax --mode digest)")
  json_commands+=("$(expect_cmd 88 "$OUT/json_rust")")
  web_commands+=("$ROOT/benchmarks/web/run-one.sh rust")
fi

if command -v python3 >/dev/null 2>&1; then
  fib_commands+=("$(expect_cmd 115 python3 benchmarks/python/fib.py)")
  mix_commands+=("$(expect_cmd 79 python3 benchmarks/python/mix.py)")
  text_commands+=("$(expect_cmd 199 env AX_TEXT="$AX_TEXT_VALUE" python3 benchmarks/python/text.py)")
  path_commands+=("$(expect_cmd 21 python3 benchmarks/python/path.py)")
  url_commands+=("$(expect_cmd 56 python3 benchmarks/python/url.py)")
  cli_commands+=("$(expect_cmd 111 python3 benchmarks/python/cli.py --input examples/agents/path_manifest.ax --mode digest)")
  json_commands+=("$(expect_cmd 88 python3 benchmarks/python/json_bench.py)")
  web_commands+=("$ROOT/benchmarks/web/run-one.sh python")
fi

if command -v node >/dev/null 2>&1; then
  fib_commands+=("$(expect_cmd 115 node benchmarks/js/fib.js)")
  mix_commands+=("$(expect_cmd 79 node benchmarks/js/mix.js)")
  text_commands+=("$(expect_cmd 199 env AX_TEXT="$AX_TEXT_VALUE" node benchmarks/js/text.js)")
  path_commands+=("$(expect_cmd 21 node benchmarks/js/path.js)")
  url_commands+=("$(expect_cmd 56 node benchmarks/js/url.js)")
  cli_commands+=("$(expect_cmd 111 node benchmarks/js/cli.js --input examples/agents/path_manifest.ax --mode digest)")
  json_commands+=("$(expect_cmd 88 node benchmarks/js/json.js)")
  web_commands+=("$ROOT/benchmarks/web/run-one.sh node")
fi

run_group() {
  local slug="$1"
  local name="$2"
  shift 2
  echo "== $name =="
  if command -v hyperfine >/dev/null 2>&1; then
    hyperfine --warmup "$BENCH_WARMUP" --runs "$BENCH_RUNS" --export-json "$OUT/$slug.json" --export-markdown "$OUT/$slug.md" "$@"
    cat "$OUT/$slug.md"
  else
    python3 "$ROOT/benchmarks/measure.py" --group "$name" --slug "$slug" --out "$OUT" --runs "$BENCH_RUNS" --warmup "$BENCH_WARMUP" "$@"
  fi
  cat "$OUT/$slug.md" >> "$REPORT"
  printf '\n' >> "$REPORT"
}

run_group "fib" "fib: recursive integer calls" "${fib_commands[@]}"
run_group "mix" "mix: mutable while-loop arithmetic" "${mix_commands[@]}"
run_group "text" "text: string search and length loop" "${text_commands[@]}"
run_group "path" "path: lexical path normalization loop" "${path_commands[@]}"
run_group "url" "url: URL parse, query lookup, and percent codec loop" "${url_commands[@]}"
run_group "cli" "cli: command-line argument lookup loop" "${cli_commands[@]}"
run_group "json" "json: JSON validation, nested query, and array lookup loop" "${json_commands[@]}"
run_group "web" "web: simple HTTP backend route handling" "${web_commands[@]}"

echo "wrote $REPORT"
