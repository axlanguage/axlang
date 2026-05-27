---
name: ax
description: "Use this skill when working with Ax as an AI-only programming language: creating AI-min .ax programs, checking effects, compiling native binaries, running tests, finding or installing packs, writing external pack manifests/native C shims, building AI-agent tools, and diagnosing Ax compiler/runtime issues."
---

# Ax

## Operating Rule

Ax source handled by this skill is AI-min source. Generate, edit, compare, and hand off `.ax` directly in the smallest parser-checked form. Do not write expanded Ax and do not introduce a normalization step. Do not emit alternate source forms in explanations, examples, or artifacts.

## Quick Workflow

1. Locate the Ax repo or binary:
   - Prefer `AX_HOME` when set.
   - Otherwise use the nearest parent with `Cargo.toml` and `crates/ax_cli`, then an `ax` binary on `PATH`.
2. If no binary exists, run `cargo build --release` in the Ax repo and use `target/release/ax`.
3. For every `.ax` edit:
   - Write the file directly in AI-min form.
   - `scripts/ax.sh check <file.ax>`
   - `scripts/ax.sh graph <file.ax>` when effects, packs, or call structure matter
   - `scripts/ax.sh build <file.ax>` or `scripts/ax.sh run <file.ax>` for executable behavior
4. For pack work, inspect `ax.toml`, `std/*/pack.axpack`, and any registry passed with `--registry`.
5. Always verify native behavior with the produced binary when changing runtime, codegen, or packs.

Install this skill for Codex from the Ax repository with:

```bash
npx skills add https://github.com/axlanguage/axlang --skill ax -a codex
```

Direct skill-path installation is also supported:

```bash
npx skills add https://github.com/axlanguage/axlang/tree/main/skills/ax -a codex
```

Install this skill for Claude Code as a personal skill:

```bash
tmp="$(mktemp -d)"
git clone --depth 1 https://github.com/axlanguage/axlang.git "$tmp/axlang"
mkdir -p ~/.claude/skills
rm -rf ~/.claude/skills/ax
cp -R "$tmp/axlang/skills/ax" ~/.claude/skills/ax
rm -rf "$tmp"
```

For a project-local Claude Code skill in another repository:

```bash
mkdir -p .claude/skills
cp -R /path/to/axlang/skills/ax .claude/skills/ax
```

Claude Code uses `~/.claude/skills/<skill-name>/SKILL.md` for personal skills and `.claude/skills/<skill-name>/SKILL.md` for project skills. Invoke directly with `/ax` or let Claude Code load it automatically when the request matches the description.

## Common Commands

```bash
ax init app
ax pack find http
ax pack info std.net.http
ax pack install std.net.http
ax check examples/hello.ax
ax graph examples/hello.ax
ax build examples/hello.ax -o .ax-out/hello
ax run examples/hello.ax
ax test
./scripts/verify-all.sh
./scripts/verify-skill.sh
./dist/verify-release.sh
```

Use `--backend custom` only for scalar and `std.io` programs on macOS arm64. Use the default LLVM backend for full language support.

Core primitive types include `i8`, `i16`, `i32`, `i64`, `u8`, `u16`, `u32`, `u64`, `f32`, `f64`, `bool`, `str`, and `void`. Use explicit annotations when width matters; native codegen coerces numeric literals for typed locals, returns, record fields, and user function parameters.

## Syntax Essentials

Read `references/syntax.md` before writing non-trivial `.ax`, networking code, async code, compact std aliases, or code that hits a parser/semantic error. The skill must be usable without repository docs.

Core item forms:

```txt
{;"hello"}                         // bare main block
@add(a:#,b:#):#{^a+b}              // function, # = i32
@@work(a:#):#{^a+1}                // async function
%User{id:i64,name:$}               // record, $ = str in type positions
%%Status{Ok,Fail}                  // enum
%!AppErr{Bad,Missing}              // error enum
+acme.telemetry                    // external pack import
?"math"{:add(20,22):42}            // pure test block
```

Core statement forms inside `{...}`:

```txt
$x=1                 // local let
$x:i64=1             // typed local let
$=expr               // implicit min-name let
x=x+1                // assignment
^x                   // return
?x>0{^x}|{^-x}       // if / else
~x<10{x=x+1}         // while
~{...}               // infinite loop
:expr                // assert truthy
:actual:expected     // equality assertion in tests
;x                   // io.println(x)
;path,value          // fs.write_text(path,value)
;;path,json          // fs.write_json_atomic(path,json)
;+path               // fs.mkdir(path)
```

Expression notes:

- Boolean literals are `!1` and `!0`; boolean ops are `&`, `|`, and unary `!`.
- Equality is `:` and inequality is `!:`. Numeric comparisons are `<`, `<=`, `>`, `>=`.
- Await a future with prefix `@`: `^@(work(1))`.
- Adjacent postfixes are meaningful: `s!` is `str.len(s)`, `s~needle` is `str.contains(s,needle)`, `p@` is `fs.is_file(p)`.
- JSON postfixes are `j\`"a"` for `json.query`, `j'"ok"` for `json.query_bool`, `j#"n"` for `json.query_int`, and `j\"items"` for `json.query_len`.
- JSON object shorthand is `{"k":expr}` for `json.pair` and `{"k"=text}` for `json.string_pair`.

Server forms:

```txt
&3000{G/ping>"pong" P/echo>~ G/state>#{ok:!1}}      // HTTP
&!3443{G/ping>"pong"}                               // HTTPS/TLS flag
&&3000{"PING\n">"PONG\n"*>"ERR\n"}                  // TCP exact/wildcard
&&!3444{"PING\n">"PONG\n"}                          // TCP TLS flag
```

Compact std aliases are valid source and should be preferred when emitting AI-min. Critical aliases: `C` = `io.println`, `Ib` = `io.print`, `Ic` = `io.eprintln`, `Id` = `io.read_line`, `Aa..Ag` = `cli.argc..cli.parse_json`, `Ta..Td` = `time.now..time.sleep_ms`, `Qa` = `tcp.listen`, `Qb` = `tcp.connect`, `Qc` = `tcp.serve_text`, `La` = `map.new`, `Na` = `async.cancel`, `Nb` = `async.detach`, `Ha..Hd` = `http.get..http.post_json`, `Su` = `str.token_upper`, `Ma` = `heap.alloc`, `Mb` = `heap.free`.

Do not emit old expanded syntax such as `fn`, `let`, `return`, `if`, `server`, or `tcp :3000`. For std calls in generated AI-min, use compact aliases or verified method calls; do not assume every expanded std root parses in source.

## AI-Min Source

AI-min `.ax` is the source of truth. Write AI-min directly and validate that exact file.

AI-min form may omit inferred std imports/effects, rename user symbols, pool repeated strings, use bare main blocks, implicit lets, short std aliases, semicolon std statements, and postfix forms for hot pure checks such as JSON queries, `str.contains`, `str.len`, and `fs.is_file`.

Guardrails:

- Do not manually invent new punctuation forms in generated `.ax` files; change the parser and formatter together.
- After editing `.ax`, immediately run `ax check` on the same file.
- For executables, run `ax run` or `ax build` on the AI-min file.
- After editing compact syntax or formatting behavior, run `cargo test -p ax_parser -p ax_fmt` and `./scripts/verify-all.sh`.
- Every new compact form needs roundtrip tests plus a separator/adjoining-token regression test.
- Prefer structural compression such as string pools, key/path reuse, and repeated-pattern lowering over adding more single-character aliases.
- Treat benchmark results as regression signals. If reporting performance, separate compile-time constant folding from dynamic runtime measurements.

## Effects And Packs

Ax functions declare effects with `!`. Missing effects produce `AX_EFFECT_MISSING`. Missing pack imports produce `AX_PACK_REQUIRED`.

Built-in packs:

- `std.io`: `io.println`, `io.print`, `io.eprintln`, `io.read_line`
- `std.fs`: `fs.read_text`, `fs.read_text_or`, `fs.read_text_limit`, `fs.read_text_range`, `fs.read_text_tail`, `fs.read_lines`, `fs.read_lines_json`, `fs.read_jsonl`, `fs.read_json`, `fs.read_json_or`, `fs.read_base64`, `fs.read_base64_range`, `fs.read_base64_tail`, `fs.write_text`, `fs.write_text_atomic`, `fs.write_json_atomic`, `fs.write_base64`, `fs.append_text`, `fs.append_jsonl`, `fs.exists`, `fs.remove`, `fs.remove_dir`, `fs.mkdir`, `fs.mkdir_all`, `fs.ensure_parent`, `fs.list`, `fs.list_json`, `fs.list_stat_json`, `fs.walk`, `fs.walk_json`, `fs.walk_stat_json`, `fs.find`, `fs.glob`, `fs.copy`, `fs.rename`, `fs.size`, `fs.stat_json`, `fs.cwd`, `fs.temp_dir`, `fs.is_file`, `fs.is_dir`, `fs.modified`
- `std.crypto`: `crypto.sha256_hex`, `crypto.sha256_json`, `crypto.sha256_verify_hex`, `crypto.hmac_sha256_hex`, `crypto.hmac_sha256_json`, `crypto.hmac_sha256_verify_hex`, `crypto.hmac_sha256_file_hex`, `crypto.hmac_sha256_file_json`, `crypto.hmac_sha256_file_verify_hex`, `crypto.hmac_sha256_file_range_hex`, `crypto.hmac_sha256_file_range_json`, `crypto.hmac_sha256_file_range_verify_hex`, `crypto.sha256_file_hex`, `crypto.sha256_file_json`, `crypto.sha256_file_verify_hex`, `crypto.sha256_file_range_hex`, `crypto.sha256_file_range_json`, `crypto.sha256_file_range_verify_hex`, `crypto.base64_encode`, `crypto.base64_decode`, `crypto.constant_time_eq`, `crypto.random_hex`, `crypto.random_base64url`, `crypto.uuid_v4`
- `std.env`: `env.get`, `env.has`, `env.set`, `env.get_or`, `env.snapshot_json`, `env.load_dotenv`, `env.load_dotenv_json`
- `std.process`: `process.exec`, `process.exec_limit`, `process.status`, `process.run_json`, `process.run_log_json`, `process.run_lines_json`, `process.run_log_lines_json`
- `std.cli`: `cli.argc`, `cli.arg`, `cli.has`, `cli.value`, `cli.value_or`, `cli.args_json`, `cli.parse_json`
- `std.json`: `json.escape`, `json.quote`, `json.pair`, `json.string_pair`, `json.object`, `json.array`, `json.array_push`, `json.string_array_push`, `json.set`, `json.string_set`, `json.remove`, `json.compact`, `json.valid`, `json.get`, `json.get_or`, `json.query`, `json.query_or`, `json.query_int`, `json.query_bool`, `json.query_kind`, `json.has`, `json.query_has`, `json.int`, `json.bool`, `json.contains`, `json.query_contains`, `json.kind`, `json.len`, `json.query_len`, `json.at`, `json.query_at`, `json.keys`, `json.keys_json`, `json.query_keys_json`
- `std.str`: `str.len`, `str.contains`, `str.index_of`, `str.count`, `str.starts_with`, `str.ends_with`, `str.trim`, `str.upper`, `str.lower`, `str.concat`, `str.repeat`, `str.replace`, `str.slice`, `str.split_json`, `str.lines_json`, `str.from_i64`, `str.parse_i64`, `str.parse_i32`, `str.token`, `str.token_upper`, `str.line`
- `std.path`: `path.normalize`, `path.join`, `path.basename`, `path.dirname`, `path.extname`, `path.stem`, `path.is_absolute`
- `std.time`: `time.now`, `time.now_ms`, `time.iso_utc`, `time.sleep_ms`
- `std.url`: `url.encode`, `url.decode`, `url.query_get`, `url.query_or`, `url.query_has`, `url.query_json`, `url.path`, `url.host`, `url.scheme`
- `std.net.tcp`: TCP runtime calls including `tcp.listen`, `tcp.connect`, `tcp.serve_text`, `accept`, `read_text`, `write_text`, `request_text`, and `close`
- `std.net.http`: local HTTP server calls, wildcard routes, streaming request bodies, optional TLS
- `std.net.http.client`: `http.get`, `http.post`, `http.get_json`, `http.post_json`

## Pack Authoring

For external packs:

1. Create `pack.axpack` with `name`, `version`, `syntax`, `operations`, `effects`, and optional `native`.
2. Put native C sources beside the manifest for local/file registries.
3. Export native symbols as `ax_pack_<pack_parts>_<operation>`.
4. Find and inspect packs with `ax pack find <query> --registry <path-or-url>` and `ax pack info <pack> --registry <path-or-url>`; queries match declared operations as well as names, effects, syntax, and native files.
5. Add the pack with `ax pack install <pack> --registry <path-or-url>`.
6. Verify with `ax check`, `ax graph`, and `ax build`.

Use the helper scripts when scaffolding or validating a pack:

```bash
scripts/ax.sh check examples/hello.ax
scripts/ax.sh build examples/hello.ax
scripts/ax.sh run examples/hello.ax
scripts/pack.sh find telemetry --registry ./registry
scripts/pack.sh info acme.telemetry --registry ./registry
scripts/pack.sh install acme.telemetry --registry ./registry
scripts/new-pack.sh ./registry acme.telemetry telemetry.write track
scripts/pack-smoke.sh .
```

For a checked-in external pack reference, inspect and run `examples/packs/telemetry`.

Read `references/pack-authoring.md` before writing non-trivial packs.

## AI-Agent Programs

Use Ax for compact native tools that agents can read and modify quickly:

- Bounded prefix/range/tail/line file previews, JSON line windows, bounded base64 file transport, full/range file hashing, file processing, and digesting: `examples/agents/file_digest.ax`
- Filesystem journals, JSONL event log append/readback, parent-directory preparation, nested artifact directories, recursive walks/finds/globs, copies, renames, cleanup, structured stat JSON, and metadata checks: `examples/agents/fs_journal.ax`
- Signed manifests plus string, full-file, and range HMAC request artifacts: `examples/agents/signed_manifest.ax`
- Random hex/base64url token generation, UUID trace IDs, base64 payloads, and constant-time signature checks: `examples/agents/secure_token.ax`
- Environment metadata defaults, dotenv loading, and prefix-scoped JSON snapshots: `examples/agents/env_report.ax`
- IO diagnostics and protocol output: `examples/agents/io_diagnostics.ax`
- CLI flag/default parsing, structured argv JSON, and parsed option manifests: `examples/agents/cli_probe.ax`
- Pure compiler-side integration tests with local bindings, assignment, conditionals, loops, and assertions: `tests/integration/control.ax`
- JSON payload validation, defaulted reads, nested path queries, typed scalar reads, valid JSON artifact construction, appended array manifests, key checks, array membership, array item access, value kind inspection, and key listing: `examples/agents/json_tool.ax`
- Structured command output/status capture, line-array output, and merged diagnostic logs: `examples/agents/process_probe.ax`
- String normalization, byte indexes, match counts, and labels: `examples/agents/string_agent.ax`
- Path manifests and generated file targets: `examples/agents/path_manifest.ax`
- Time, UTC timestamp, and scheduling checks: `examples/agents/time_probe.ax`
- URL routing, query defaults/presence checks, and decoded query JSON: `examples/agents/url_router.ax`
- HTTP tool endpoints: `examples/agents/tool_server.ax`
- HTTP client fetches with structured bounded result JSON: `examples/agents/http_fetch.ax`
- Async scoring/work dispatch: `examples/agents/async_score.ax`

Read `references/agent-patterns.md` when building agent-facing services or tools.

## Helpful Scripts

- `scripts/ensure-ax.sh <repo>` builds or finds an Ax binary.
- `scripts/ax.sh [--repo <repo>] <check|graph|explain|build|compile|run|test|version|packs|init> ...` ensures Ax is available, then runs common compiler workflows. `build <file.ax>` defaults to `.ax-out/skill-builds/<file>`.
- `scripts/pack.sh <list|find|search|info|install|add> ...` ensures Ax is available and runs the pack workflow.
- `scripts/new-pack.sh <registry-dir> <pack-name> <effect> [operation]` creates a minimal external pack skeleton.
- `scripts/pack-smoke.sh <repo>` generates, installs, builds, and runs a native external-pack smoke test.
