#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
BIN="${1:-$PWD/target/debug/spectra}"
TASK_DIR=$(mktemp -d)
trap 'rm -rf "$TASK_DIR"' EXIT
"$BIN" --data-dir "$TASK_DIR" --json wallet watch --chain ethereum --address 0x1111111111111111111111111111111111111111 --name Followup > "$TASK_DIR/wallet.json"
"$BIN" --data-dir "$TASK_DIR" --json wallet rename Followup Renamed > /dev/null
"$BIN" --data-dir "$TASK_DIR" --json wallet inclusion Renamed false > /dev/null
"$BIN" --data-dir "$TASK_DIR" --json wallet list > "$TASK_DIR/list.json"
"$BIN" --data-dir "$TASK_DIR" --json price --stored > "$TASK_DIR/prices.json"
"$BIN" --data-dir "$TASK_DIR" --json txs --maintenance > "$TASK_DIR/maintenance.json"
python3 - "$TASK_DIR" <<'PY'
import json,pathlib,sys
p=pathlib.Path(sys.argv[1])
assert json.loads((p/'prices.json').read_text())['quotes']['prices']=={}
assert json.loads((p/'maintenance.json').read_text())['chains']==[]
wallets=json.loads((p/'list.json').read_text())['wallets']
assert len(wallets)==1 and wallets[0]['name']=='Renamed',wallets
PY

"$BIN" --data-dir "$TASK_DIR" --json diagnostics maintenance --conditions '{"appIsActive":true,"isNetworkReachable":false,"isConstrainedNetwork":false,"isExpensiveNetwork":false,"isLowPowerMode":false,"batteryLevel":1,"wantsPriceRefresh":true}' > "$TASK_DIR/policy.json"
python3 - "$TASK_DIR/policy.json" <<'PYTHON'
import json,sys
plan=json.load(open(sys.argv[1]))['plan']
assert plan['runBackgroundTick'] is False,plan
assert plan['allowHeavyBackgroundWork'] is False,plan
assert plan['pollSeconds'] > 0,plan
PYTHON
python3 scripts/cli-transaction-recheck.py "$BIN"

python3 scripts/cli-owned-send.py "$BIN"
