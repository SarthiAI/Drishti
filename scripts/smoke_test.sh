#!/usr/bin/env bash
# Smoke test: confirm the CLI and the HTTP server return identical results for
# the same input and config (invariant I5). Uses the regex PII path so it needs
# no model download. Latency fields, which vary per run, are stripped before the
# comparison.
#
# Usage: scripts/smoke_test.sh   (run from the repo root)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "building cli and server..."
cargo build --release -p drishti-cli -p drishti-server >/dev/null

CFG="$(mktemp)"
cat > "$CFG" <<'TOML'
[pii]
regex_enabled = true
  [pii.redaction]
  default = "mask"
    [pii.redaction.per_kind]
    CreditCard = "refuse"
TOML

INPUT="card 4111 1111 1111 1111 and email a@b.com"
strip_latency() { python3 -c "import sys,json; d=json.load(sys.stdin); d.pop('latency_ms',None); print(json.dumps(d,sort_keys=True))"; }

echo "running via CLI..."
CLI_OUT="$(./target/release/drishti --config "$CFG" pii --text "$INPUT" | strip_latency)"

echo "starting server..."
./target/release/drishti-server --config "$CFG" --bind 127.0.0.1:8791 --token smoke >/dev/null 2>&1 &
SRV=$!
trap 'kill $SRV 2>/dev/null || true; rm -f "$CFG"' EXIT
sleep 1

echo "running via server..."
SRV_OUT="$(curl -s -X POST http://127.0.0.1:8791/v1/check/pii \
  -H 'authorization: Bearer smoke' -H 'content-type: application/json' \
  -d "{\"input\": \"$INPUT\"}" | strip_latency)"

if [ "$CLI_OUT" = "$SRV_OUT" ]; then
  echo "PASS: CLI and server results match"
  echo "  $CLI_OUT"
else
  echo "FAIL: results differ"
  echo "  cli:    $CLI_OUT"
  echo "  server: $SRV_OUT"
  exit 1
fi
