#!/usr/bin/env bash
set -euo pipefail

usage="usage: new-pack.sh <registry-dir> <pack-name> <effect> [operation]"
registry="${1:?$usage}"
pack="${2:?$usage}"
effect="${3:?$usage}"
operation="${4:-run}"
dir="$registry/$pack"

if [[ ! "$operation" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]]; then
  echo "invalid operation name: $operation" >&2
  echo "$usage" >&2
  exit 2
fi

sanitize_symbol_part() {
  printf '%s' "$1" | LC_ALL=C sed 's/[^A-Za-z0-9_]/_/g'
}

symbol="ax_pack_$(sanitize_symbol_part "$pack")_$(sanitize_symbol_part "$operation")"

mkdir -p "$dir"
cat > "$dir/pack.axpack" <<EOF
name = "$pack"
version = "1.0.0"
syntax = []
operations = ["$(printf '%s' "$pack" | awk -F. '{print $NF}').$operation"]
effects = ["$effect"]
native = ["native.c"]
EOF

cat > "$dir/native.c" <<EOF
#include <stdio.h>

void $symbol(void) {
  puts("$pack $operation");
}
EOF

echo "created $dir"
echo "operation call: $(printf '%s' "$pack" | awk -F. '{print $NF}').$operation()"
echo "operation symbol: $symbol"
