# Ax Agent Patterns

## Context Handoff

For agent-to-agent work, hand off only AI-min `.ax` plus bounded diagnostics. Start from:

```bash
ax check <file.ax> --json
ax graph <file.ax>
```

Do not include alternate source forms in handoff artifacts. Repository `.ax` examples are already AI-min source; quote only the checked-in source that is necessary for the task.

For handoff artifacts, prefer compact JSON from std helpers and bounded previews over raw large text:

- `fs.read_text_limit`, `fs.read_text_range`, `fs.read_text_tail`, `fs.read_lines_json`
- `process.run_json`, `process.run_log_json`, `process.run_lines_json`
- `json.compact`, `json.query*`, `json.keys_json`
- `crypto.sha256_*_json` and range hash/sign helpers for provenance

## Native Tool Shape

Use the checked-in AI-min agent examples as capability references:

```bash
ax check examples/agents/file_digest.ax
ax check examples/agents/fs_journal.ax
ax check examples/agents/json_tool.ax
ax check examples/agents/process_probe.ax
ax check examples/agents/tool_server.ax
ax check examples/agents/http_fetch.ax
```

Typical local-tool capabilities:

- Files: bounded previews, JSONL windows, base64 pages, recursive walks/finds/globs, atomic writes, copies, renames, structured stat JSON.
- Integrity: SHA-256 and HMAC for strings, full files, and file ranges; constant-time equality checks.
- Environment and CLI: defaulted reads, dotenv loading, prefix snapshots, argv JSON, option parsing.
- JSON and strings: compacting, scalar reads, nested path queries, key listing, array item access, normalization, slicing, case conversion.
- Processes: bounded stdout, status JSON, merged logs, line arrays.
- Paths, time, URLs: normalized output paths, UTC timestamps, query extraction, route pieces.
- Networking: local HTTP tool endpoints and bounded HTTP client calls.

## Diagnostics Loop

For generated or edited code, run:

```bash
ax check app.ax --json
ax graph app.ax
ax build app.ax -o .ax-out/app
```

Use `ax run app.ax` when the command-line behavior is part of the task.
