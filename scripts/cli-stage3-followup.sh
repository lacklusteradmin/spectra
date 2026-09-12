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
assert 'Renamed' in (p/'list.json').read_text()
PY
if "$BIN" --data-dir "$TASK_DIR" --json send preview --wallet Renamed --holding 'Ethereum|ETH' --amount NaN > /dev/null 2>&1; then
    echo 'invalid amount preview accepted' >&2; exit 1
fi
cargo test -p spectra_core --lib store::tests::wallet_import
cargo test -p spectra_core --lib field_intents_do_not_overwrite
cargo test -p spectra_core --lib service::network_prices::tests
cargo test -p spectra_core --lib owned_preview_uses_wallet_network

"$BIN" --data-dir "$TASK_DIR" --json diagnostics maintenance --conditions '{"appIsActive":true,"isNetworkReachable":false,"isConstrainedNetwork":false,"isExpensiveNetwork":false,"isLowPowerMode":false,"batteryLevel":1,"wantsPriceRefresh":true}' > "$TASK_DIR/policy.json"
cargo test -p spectra_core --lib service::maintenance::boundary_tests
cargo test -p spectra_core --lib service::history_cursor::tests
cargo test -p spectra_core --lib maintenance_scope_uses_registry

cargo test -p spectra_core --lib decred_and_kaspa_independent_mnemonic_vectors
cargo test -p spectra_core --lib http_probe_regressions

cargo test -p spectra_core --lib app_boundary_tests
