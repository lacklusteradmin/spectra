#!/usr/bin/env bash
# Ownership checks, exclusively temporary stores and loopback fixtures.
set -euo pipefail
source "$(dirname "$0")/cli-assertions.sh"
PASSED=0
FAILED=0
cd "$(dirname "$0")/.."
BIN="${1:-$PWD/target/debug/spectra}"
TASK_DIR=$(mktemp -d)
trap 'rm -rf "$TASK_DIR"' EXIT
"$BIN" --data-dir "$TASK_DIR" --json wallet derived > "$TASK_DIR/derived.json"
"$BIN" --data-dir "$TASK_DIR" --json diagnostics state --command '{"Degraded":{"chain_name":"Solana","detail":"timeout"}}' > /dev/null
"$BIN" --data-dir "$TASK_DIR" --json diagnostics state --command '{"Healthy":{"chain_name":"Solana"}}' > /dev/null
"$BIN" --data-dir "$TASK_DIR" --json diagnostics state > "$TASK_DIR/diagnostics.json"
python3 - "$TASK_DIR" <<'PY'
import json,sys,pathlib
p=pathlib.Path(sys.argv[1]); d=json.loads((p/'diagnostics.json').read_text())['state']
assert not d['degraded'] and 'Solana' in d['last_good_unix']
assert len(d['logs'])==2 and d['logs'][0]['input']['message']=='Chain recovered'
assert json.loads((p/'derived.json').read_text())['signing_material_wallet_ids']==[]
PY
contains_exit 1 "missing transaction cannot be rebroadcast" 'transaction not found' \
    "$BIN" --data-dir "$TASK_DIR" --json send rebroadcast missing --yes

python3 scripts/cli-balance-refresh.py "$BIN"

[[ "$FAILED" == 0 ]]
