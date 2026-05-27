<p align="center">
  <img src="docs/assets/ax-logo.png" alt="Ax logo" width="132">
</p>

<h1 align="center">Ax v1.0</h1>

<p align="center">
  <a href="https://axlanguage.github.io/axlang/">Website</a> ·
  <a href="https://axlanguage.github.io/axlang/install.html">Install</a> ·
  <a href="https://axlanguage.github.io/axlang/language.html">Language Docs</a> ·
  <a href="https://github.com/axlanguage/axlang/releases/latest">Latest Release</a>
</p>

Ax is an AI-native systems language preview: compact source, native binaries, explicit semantics, and a pack model built for tools that humans and agents can both inspect quickly.

Ax source is already written in its compact form. There is no expanded authoring syntax to normalize away during normal development. The parser, formatter, examples, tests, and docs all treat AI-min `.ax` as the source of truth.

## Why Ax

Modern software often spends more tokens on framework ceremony than on behavior. Ax is designed around the opposite constraint:

- Keep source small enough to fit inside model context windows.
- Compile to native executables through Rust, semantic analysis, Ax IR, LLVM IR, `clang`, and a small C runtime ABI.
- Keep the language core narrow and move real-world power into packs.
- Make effects and native pack calls analyzable before codegen.
- Let agents edit the same checked-in source that the compiler consumes.

Ax language, pack registry, and agent-native tooling will continue opening up from here. The goal is extreme code compression without giving up native performance or stability.

## Website And Docs

The integrated website and docs are published with GitHub Pages:

- Website: https://axlanguage.github.io/axlang/
- Install: https://axlanguage.github.io/axlang/install.html
- Language: https://axlanguage.github.io/axlang/language.html
- Packs: https://axlanguage.github.io/axlang/packs.html
- Agent samples: https://axlanguage.github.io/axlang/agents.html
- Benchmarks: https://axlanguage.github.io/axlang/benchmarks.html

## Install

Prebuilt native `ax` binaries are published on GitHub Releases for macOS
arm64/x64, Linux x64/arm64, and Windows x64. Install the latest release:

```bash
curl -fsSL https://raw.githubusercontent.com/axlanguage/axlang/main/dist/install.sh | sh
export PATH="$HOME/.ax/bin:$PATH"
ax version
```

Windows PowerShell:

```powershell
iwr https://raw.githubusercontent.com/axlanguage/axlang/main/dist/install.ps1 -useb | iex
$env:Path = "$HOME\.ax\bin;$env:Path"
ax version
```

Install a specific release tag:

```bash
curl -fsSL https://raw.githubusercontent.com/axlanguage/axlang/main/dist/install.sh | AX_VERSION=v1.0.0 sh
```

The installers verify `SHA256SUMS` when available and install to `~/.ax/bin`
by default. Set `AX_BIN_DIR` to choose another directory. The release binary
embeds the standard pack manifests and runtime C sources, so installation does
not require a source checkout. Native `ax build` and `ax run` require a C
toolchain with `clang` available on `PATH`.

Build from source when you want to hack on the compiler:

```bash
cargo build --release
target/release/ax version
```

## Quick Start

```bash
ax run examples/hello.ax
ax check examples/agents/tool_server.ax
ax build examples/http_ping.ax -o .ax-out/http_ping
.ax-out/http_ping
```

In another terminal:

```bash
curl http://127.0.0.1:3000/ping
```

## Compact Source

Hello world:

```ax
{;"hello world"}
```

Function and return:

```ax
@add(a:#,b:#):#{^a+b} {^add(20,22)}
```

Records and enums:

```ax
%User{id:i64,name:str}
%%Status{Ok,Fail}
{$u=User{id:1,name:"ax"}^u.id}
```

Pure test block:

```ax
@fib(n:#):#{$i=0$a=0$b=1 ~i<n{$next=a+b a=b b=next i=i+1}^a}
?"fib"{:fib(7):13}
```

HTTP server:

```ax
&3000{G/ping>"pong" G/health>#{ok:!1,service:"ax"} P/echo>~}
```

TCP server:

```ax
&&3000{"PING\n">"PONG\n"*>"ERR\n"}
```

External pack import:

```ax
+acme.telemetry {telemetry.track()}
```

Useful aliases include `@` for function items, `@@` for async function items, `{...}` for a bare main block, `$` for locals, `^` for return, `?` for control/test forms, `:` for equality/assertion forms, `#` for `i32`, `!1` and `!0` for booleans, `&` and `|` for boolean conjunction/disjunction, `&` for HTTP servers, and `&&` for TCP servers.

## Packs

Built-in packs are resolved from `std/*/pack.axpack` and implemented by the compiler/runtime:

- `std.io`: stdout, stderr, stdin
- `std.fs`: text, JSON, JSONL, base64 windows, atomic writes, walks, glob/find, stat, cwd/temp
- `std.crypto`: SHA-256, HMAC-SHA256, random hex/base64url, UUID v4, base64, constant-time compare
- `std.env`: reads, defaults, dotenv, scoped snapshots, writes
- `std.process`: shell output, status, bounded JSON and line captures
- `std.cli`: argv, flags, options, parsed JSON
- `std.json`: compaction, construction, lookup, nested query, typed reads, arrays, keys
- `std.str`: length, search, token/line access, trim/case/replace/slice/split
- `std.map`: native string-keyed in-memory maps
- `std.path`: lexical path helpers
- `std.time`: epoch seconds/ms, UTC ISO, sleep
- `std.url`: encode/decode, scheme/host/path, query helpers
- `std.net.tcp`: TCP servers and low-level connection calls
- `std.net.http`: HTTP servers, wildcard routes, streaming request bodies, TLS
- `std.net.http.client`: HTTP GET/POST and bounded structured results

Install or inspect packs:

```bash
target/release/ax pack list
target/release/ax pack find crypto
target/release/ax pack info std.crypto
target/release/ax pack install std.net.http
```

## Interop

Ax servers are normal TCP/HTTP programs, so any language can call them.

HTTP from JavaScript:

```js
const health = await fetch("http://127.0.0.1:3000/health");
console.log(await health.json());

const echo = await fetch("http://127.0.0.1:3000/echo", {
  method: "POST",
  body: "hello",
});
console.log(await echo.text());
```

HTTP from Python:

```python
from urllib.request import Request, urlopen

print(urlopen("http://127.0.0.1:3000/health").read().decode())

req = Request("http://127.0.0.1:3000/echo", data=b"hello", method="POST")
print(urlopen(req).read().decode())
```

TCP from Node.js:

```js
import net from "node:net";

const socket = net.createConnection(3000, "127.0.0.1", () => {
  socket.write("PING\n");
});
socket.on("data", data => {
  console.log(data.toString());
  socket.end();
});
```

TCP from Python:

```python
import socket

with socket.create_connection(("127.0.0.1", 3000)) as sock:
    sock.sendall(b"PING\n")
    print(sock.recv(1024).decode())
```

## Benchmarks

Run the local harness:

```bash
./benchmarks/run.sh
```

The harness builds Ax, C, Rust, Python, and Node baselines when available, validates expected exit codes, uses 5 warmups and 20 measured runs by default, and writes `.ax-out/benchmarks/report.md`.

Current local sample results from this workspace:

| Workload | Ax median | Closest native baseline | Node median | Python median |
|---|---:|---:|---:|---:|
| `fib` | 0.009309s | C 0.010527s | 0.036418s | 0.067490s |
| `mix` | 0.009894s | Rust 0.049540s | 0.074051s | 0.539994s |
| `text` | 0.047921s | C 0.049565s | 0.207213s | 0.629452s |
| `path` | 0.009937s | C 0.105617s | 0.405022s | 0.469781s |
| `url` | 0.009617s | C 0.031807s | 0.130238s | 0.807595s |
| `cli` | 0.011860s | C 0.015727s | 0.050839s | 0.165068s |
| `json` | 0.010295s | C 0.018925s | 0.044746s | 0.061773s |
| `web` | 0.200535s | C 0.204578s | 0.297787s | 0.430939s |

Current release compiler binary in this workspace:

```txt
target/release/ax: 1.7 MiB
```

Benchmarks are regression signals for this toolchain and machine, not universal language rankings.

## CLI

```bash
ax init <name>
ax check <file.ax> [--json] [--registry <source>]
ax fmt <file.ax> [--write]
ax build <file.ax> -o <output> [--registry <source>] [--backend llvm|custom]
ax run <file.ax> [--registry <source>] [--backend llvm|custom] [-- <args>...]
ax test
ax pack list [--registry <source>]
ax pack find [query] [--registry <source>]
ax pack info <pack> [--registry <source>]
ax pack install <pack> [--registry <source>]
ax add <pack> [--registry <source>]
ax packs [--registry <source>]
ax graph <file.ax> [--registry <source>]
ax explain <file.ax> [--registry <source>]
ax version
```

## Repository Verification

```bash
./scripts/verify-all.sh
```

Set `AX_VERIFY_BENCHMARKS=1` to include benchmarks and `AX_VERIFY_RELEASE=1` to include release/install smoke tests.

## Agent Skills

The Ax agent skill is checked in under `skills/ax`. It is a standard `SKILL.md` directory with supporting references and scripts.

Codex install:

```bash
npx skills add https://github.com/axlanguage/axlang --skill ax -a codex
npx skills add https://github.com/axlanguage/axlang/tree/main/skills/ax -a codex
```

Use `$ax` when asking Codex to write `.ax`, check effects, build native binaries, inspect packs, or create agent tools.

Claude Code personal install:

```bash
tmp="$(mktemp -d)"
git clone --depth 1 https://github.com/axlanguage/axlang.git "$tmp/axlang"
mkdir -p ~/.claude/skills
rm -rf ~/.claude/skills/ax
cp -R "$tmp/axlang/skills/ax" ~/.claude/skills/ax
rm -rf "$tmp"
```

Then run `claude` in any project and invoke the skill directly with:

```txt
/ax
```

Claude Code also supports project-local skills. In another repository, copy the skill into that project:

```bash
mkdir -p .claude/skills
cp -R /path/to/axlang/skills/ax .claude/skills/ax
```

Project-local skills are discovered from `.claude/skills/<skill-name>/SKILL.md`, while personal skills live under `~/.claude/skills/<skill-name>/SKILL.md`. See the official [Claude Code skills documentation](https://code.claude.com/docs/en/skills). For shared, versioned Claude Code distribution, package the same `skills/ax` directory as a plugin skill under `skills/ax/SKILL.md`; see [Claude Code plugins](https://code.claude.com/docs/en/plugins).

## Docs

Static docs live in `docs/`:

```bash
python3 -m http.server 4173 -d docs
```

Open `http://127.0.0.1:4173/index.html`.

## Release Artifacts

```bash
./dist/build-release.sh
./dist/verify-release.sh
```

One-line installers:

```bash
curl -fsSL https://raw.githubusercontent.com/axlanguage/axlang/main/dist/install.sh | sh
```

```powershell
iwr https://raw.githubusercontent.com/axlanguage/axlang/main/dist/install.ps1 -useb | iex
```

Windows release binaries support the CLI and basic native builds. POSIX hosts
currently provide the full TCP/HTTP runtime.

## Architecture

```txt
.ax source
  -> lexer
  -> parser
  -> AST
  -> semantic analysis
  -> Ax IR
  -> LLVM IR
  -> clang object
  -> native executable linked with Ax runtime
```

LLVM is the full backend. `--backend custom` currently emits macOS arm64 assembly for scalar and `std.io` programs.
