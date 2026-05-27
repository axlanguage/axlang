#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/dist/bin"
mkdir -p "$OUT"

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) target="darwin-arm64" ;;
  Darwin-x86_64) target="darwin-x64" ;;
  Linux-x86_64) target="linux-x64" ;;
  Linux-aarch64|Linux-arm64) target="linux-arm64" ;;
  *) target="$(uname -s | tr '[:upper:]' '[:lower:]')-$(uname -m)" ;;
esac

cd "$ROOT"
cargo build --release
cp target/release/ax "$OUT/ax-$target"
chmod +x "$OUT/ax-$target"

cat > "$OUT/SHA256SUMS" <<EOF
$(shasum -a 256 "$OUT/ax-$target" | awk '{print $1}')  ax-$target
EOF

echo "wrote $OUT/ax-$target"
echo "wrote $OUT/SHA256SUMS"
