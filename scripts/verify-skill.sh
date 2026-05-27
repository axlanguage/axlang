#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

test -f skills/ax/SKILL.md
test -f skills/ax/references/syntax.md
test -f skills/ax/references/pack-authoring.md
test -f skills/ax/references/agent-patterns.md
test -x skills/ax/scripts/ensure-ax.sh
test -x skills/ax/scripts/ax.sh
test -x skills/ax/scripts/pack.sh
test -x skills/ax/scripts/new-pack.sh
test -x skills/ax/scripts/pack-smoke.sh

bash -n skills/ax/scripts/ensure-ax.sh skills/ax/scripts/ax.sh skills/ax/scripts/pack.sh skills/ax/scripts/new-pack.sh skills/ax/scripts/pack-smoke.sh

if [ -n "${AX_BIN:-}" ] && [ -x "$AX_BIN" ]; then
  export AX_BIN
else
  unset AX_BIN
fi

skills/ax/scripts/pack-smoke.sh "$ROOT"

skill_out="$ROOT/.ax-out/skill-commands"
rm -rf "$skill_out"
mkdir -p "$skill_out"
skills/ax/scripts/ax.sh --repo "$ROOT" version >/dev/null
skills/ax/scripts/ax.sh --repo "$ROOT" check examples/hello.ax >/dev/null
skills/ax/scripts/ax.sh --repo "$ROOT" graph examples/agents/json_tool.ax | grep -F "json.query_int" >/dev/null
skills/ax/scripts/ax.sh --repo "$ROOT" explain examples/http_ping.ax | grep -F "native HTTP" >/dev/null
skills/ax/scripts/ax.sh --repo "$ROOT" test | grep -F "test result:" >/dev/null
AX_OUT="$skill_out" skills/ax/scripts/ax.sh --repo "$ROOT" build examples/hello.ax | grep -F "$skill_out/hello" >/dev/null
"$skill_out/hello" | grep -F "hello world" >/dev/null
skills/ax/scripts/ax.sh --repo "$ROOT" run examples/hello.ax | grep -F "hello world" >/dev/null

echo "Ax skill verification passed"
