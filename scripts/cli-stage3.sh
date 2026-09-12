#!/usr/bin/env bash
# Ownership checks, exclusively temporary stores and loopback/Rust fixtures.
set -euo pipefail
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
if "$BIN" --data-dir "$TASK_DIR" --json send rebroadcast missing --yes > /dev/null 2>&1; then
    echo 'missing transaction was rebroadcast' >&2; exit 1
fi
# Domain commits and submission use deterministic local fixtures.
cargo test -p spectra_core --lib service::balance_refresh::
cargo test -p spectra_core --lib service::funds_scan::
cargo test -p spectra_core --lib service::diagnostic_state::
cargo test -p spectra_core --lib service::send_records::
cargo test -p spectra_core --lib audit_stored_wallets_reach_solana_sui_aptos_and_tron_submission
cargo test -p spectra_core --lib audit_fix5

python3 scripts/cli-balance-refresh.py "$BIN"
