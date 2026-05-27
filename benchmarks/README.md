# Ax Benchmarks

These benchmarks compare Ax native output with common language baselines using the same small workloads.

## Requirements

- `cargo`
- `clang`
- Optional: `hyperfine` for statistically useful timing
- Optional baselines: `rustc`, `python3`, `node`

## Run

```bash
./benchmarks/run.sh
```

The script builds Ax with the LLVM backend, builds C and Rust baselines when toolchains are present, and runs available interpreters.
Each benchmark command is wrapped by `benchmarks/expect-exit.sh`, which verifies the expected score exit code before timing. A mismatch fails the benchmark run instead of being hidden as a timing result.

By default the harness uses 5 warmup runs and 20 measured runs per command. Set `AX_BENCH_WARMUP` or `AX_BENCH_RUNS` to override them, and set `AX_WEB_REQUESTS` to change the HTTP request count. Results are written to `.ax-out/benchmarks/*.json`, `.ax-out/benchmarks/*.md`, and the combined `.ax-out/benchmarks/report.md`.

## Workloads

- `fib`: recursive integer function calls, a compact proxy for scalar call overhead and native code generation.
- `mix`: mutable `while` loop integer arithmetic, a proxy for local assignment, branches, and modulo-heavy scalar code.
- `text`: string length, prefix/suffix checks, and substring search in a loop.
- `path`: lexical path normalization and basename extraction in a loop.
- `url`: URL parse, query lookup, and percent encode/decode in a loop.
- `cli`: command-line flag and option lookup in a loop.
- `json`: JSON compaction, validation, nested query, kind checks, and array lookup in a loop.
- `web`: simple HTTP backend route handling for `/health` and `/ping`.

Benchmarks are not a language ranking. They are a regression harness for Ax code generation and runtime overhead.
