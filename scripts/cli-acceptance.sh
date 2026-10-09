#!/usr/bin/env bash
#
# Drives the real `spectra` binary end to end against a throwaway data
# directory.
#
# Rules are tested in core with `cargo test`. This suite covers what only the
# built binary shows: state that crosses a process boundary, what lands on
# disk (the SQLite store and the file secret store), the exit-code contract,
# the CLI's own confirmation gates, the staged sends and staking that the
# Python suites drive through loopback nodes, and the network guard.
#
# No external network. Core enforces this: `SPECTRA_LOOPBACK_ONLY` refuses and
# journals every request to a non-loopback host, and any journaled request
# fails the run.
#
# Usage:  scripts/cli-acceptance.sh [path/to/spectra]

set -uo pipefail

BIN="${1:-}"
if [[ -z "$BIN" ]]; then
    # The Zcash suite's lightwalletd and the MWEB suite's Litecoin node are
    # fixtures of their own, built beside it.
    cargo build -p spectra_cli -p spectra_zcash_fixture -p spectra_litecoin_mweb_fixture --quiet || exit 1
    BIN="$(cd "$(dirname "$0")/.." && pwd)/target/debug/spectra"
fi

SCRIPTS="$(cd "$(dirname "$0")" && pwd)"
DATA_DIR="$(mktemp -d)"
NETWORK_JOURNAL="$(mktemp)"
CANARY_JOURNAL="$(mktemp)"
trap 'rm -rf "$DATA_DIR" "$NETWORK_JOURNAL" "$CANARY_JOURNAL"' EXIT
# Inherited by every `spectra` process, the Python suites' included.
export SPECTRA_LOOPBACK_ONLY="$NETWORK_JOURNAL"

# Wallets created here are throwaway, so the password is too.
export SPECTRA_PASSWORD="acceptance-password"

PASSED=0
FAILED=0

spectra() { "$BIN" --data-dir "$DATA_DIR" "$@"; }

# `spectra` is a shell function, so `env VAR=x spectra ...` cannot find it.
# These wrappers set the variable for one call instead.
with_seed() { local seed="$1"; shift; SPECTRA_SEED="$seed" "$@"; }
with_password() { local password="$1"; shift; SPECTRA_PASSWORD="$password" "$@"; }
with_journal() { local journal="$1"; shift; SPECTRA_LOOPBACK_ONLY="$journal" "$@"; }

# Shared assertions check both the command's exit status and its output.
source "$SCRIPTS/cli-assertions.sh"

section() { printf '\n\033[1m%s\033[0m\n' "$1"; }
pass_if() {
    if eval "$2"; then
        PASSED=$((PASSED + 1))
        printf '  \033[32m✓\033[0m %s\n' "$1"
    else
        FAILED=$((FAILED + 1))
        printf '  \033[31m✗\033[0m %s\n' "$1"
    fi
}
secret_files() { find "$DATA_DIR/secrets" -type f 2>/dev/null | wc -l | tr -d ' '; }

# Exit codes are part of the interface: 0 done, 1 failed, 2 the caller asked
# wrongly, 3 core considered it and said no.
readonly OK=0 USAGE=2 REJECTED=3

readonly SEED="legal winner thank year wave sausage worth useful legal winner thank yellow"

check "assertions reject failed commands and wrong output" $OK bash "$SCRIPTS/test-cli-assertions.sh"

section "exit codes"
check "a valid request succeeds"            $OK \
    spectra address validate --chain Bitcoin bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4
check "an unknown wallet fails"             1 spectra wallet show "no such wallet"
check "an unknown option value is a usage error" $USAGE spectra chains --tag sidechain
check "a refusal by core is rejected"       $REJECTED \
    spectra address validate --chain Ethereum 0xnothex

# ── State across processes ──────────────────────────────────────────────────

section "wallets across processes"
check "imports a known mnemonic"            $OK \
    with_seed "$SEED" spectra wallet import --chain Solana --name "Acceptance SOL"
check "an import is on one network"         $USAGE \
    with_seed "$SEED" spectra wallet import --chain Solana --chain Ethereum --name "Two networks"
contains "a new process reads back its derived address" \
    "BLeUXTx9thHGT7VJUtF9vHEmfMDgW1nnKZ9UVer2CoLX" \
    spectra --json wallet show "Acceptance SOL"
check "creates a wallet"                    $OK \
    spectra wallet new --chain Bitcoin --name "Acceptance BTC"
check "renames it"                          $OK \
    spectra wallet rename "Acceptance BTC" "Renamed BTC"
contains "the rename survives reopening"    '"name":"Renamed BTC"' spectra --json wallet list
contains "a wallet's page offers what its network adds" '"action":"stake"' \
    spectra --json wallet actions "Acceptance SOL"
check "a network that keeps only a balance has no account to show" $REJECTED \
    spectra wallet account "Acceptance SOL"
check "a transaction this device did not sign as Monero has no payment proof" $USAGE \
    spectra wallet prove-payment "Acceptance SOL" --txid 00
contains "a test network names its faucet" '"faucet":"https://sepolia-faucet.pk910.de/"' \
    spectra --json chains --testnets --filter Sepolia

# Incompatible stored records must refuse loading without changing the bytes.
section "stored data integrity"
if command -v sqlite3 >/dev/null 2>&1; then
    rewrite_row() {
        sqlite3 "$DATA_DIR/spectra.sqlite" "UPDATE wallets SET payload = REPLACE(payload, '$1', '$2')
            WHERE name = 'Acceptance SOL';"
    }
    readonly FRESH='"derivationOverrides":{"passphrase":null,"hmacKey":null}'
    readonly STALE='"derivationOverrides":{"passphrase":null,"mnemonicWordlist":null,"hmacKey":null}'
    rewrite_row "$FRESH" "$STALE"
    contains_exit 1 "an incompatible wallet refuses loading" "wallet_load_all decode" spectra wallet list
    rewrite_row "$STALE" "$FRESH"
    contains "failed loading leaves the stored wallet intact" "Acceptance SOL" spectra wallet list
else
    printf '  \033[33m-\033[0m %s\n' "skipped (no sqlite3): undecodable wallet row"
fi
# Each malformed metadata case uses a copy; refusal must preserve it exactly.
check "stored metadata strictly requires the current format" $OK python3 - "$BIN" "$DATA_DIR" <<'PYSTORED'
import json, pathlib, shutil, sqlite3, subprocess, sys, tempfile
binary, source = sys.argv[1:]
for case in ["version", "missing_version", "unknown_key", "missing_setting", "unknown_setting", "preferences", "alerts", "rates", "quotes"]:
    with tempfile.TemporaryDirectory() as root:
        path = pathlib.Path(root) / "spectra.sqlite"
        with sqlite3.connect(pathlib.Path(source) / "spectra.sqlite") as original, sqlite3.connect(path) as db:
            original.backup(db)
            if case == "missing_version":
                db.execute("DELETE FROM app_state_meta WHERE key = 'schema_version'")
            else:
                key, value = {"version": ("schema_version", "999"), "unknown_key": ("unknown", "null"),
                    "preferences": ("token_preferences", "{}"), "alerts": ("price_alerts", "{}"),
                    "rates": ("fiat_rates_from_usd", "null"), "quotes": ("quotes", "null")}.get(case, ("settings", None))
                if key == "settings":
                    settings = json.loads(db.execute("SELECT value FROM app_state_meta WHERE key='settings'").fetchone()[0])
                    if case == "missing_setting": del settings["fiatCurrency"]
                    else: settings["obsolete"] = True
                    value = json.dumps(settings)
                db.execute("INSERT OR REPLACE INTO app_state_meta VALUES (?, ?)", (key, value))
            db.commit()
            before = list(db.iterdump())
        result = subprocess.run([binary, "--data-dir", root, "wallet", "list"], capture_output=True, text=True)
        assert result.returncode == 1, (case, result.stdout, result.stderr)
        with sqlite3.connect(path) as db:
            assert list(db.iterdump()) == before, case
PYSTORED

# ── Secrets on disk ─────────────────────────────────────────────────────────

section "sealed secrets"
contains "export returns the phrase it sealed" "\"seedPhrase\":\"$SEED\"" \
    spectra --json wallet export "Acceptance SOL" --yes
check "a wrong password cannot unseal it"   $REJECTED \
    with_password wrong spectra wallet export "Acceptance SOL" --yes
contains "a Solana key exports as the keypair its wallets import" '"format":"solanaKeypair"' \
    spectra --json wallet export "Acceptance SOL" --key private-key --yes
contains "a Bitcoin account exports its zpub" '"accountKey":"zpub' \
    spectra --json wallet export "Renamed BTC" --key account-key --yes
check "but no one key for its many addresses" $REJECTED \
    spectra wallet export "Renamed BTC" --key private-key --yes
check "a key export asks for --yes"         $USAGE \
    spectra wallet export "Acceptance SOL" --key private-key
check "imports without a password"          $OK \
    with_seed "$SEED" spectra wallet import --chain Solana --account 1 --name "Open SOL" --no-password
contains "and exports with no password asked" "\"seedPhrase\":\"$SEED\"" \
    spectra --json wallet export "Open SOL" --yes
# No password is no plaintext: core seals the phrase under its device key, so
# neither the words nor their base64 are in the seed bucket.
pass_if "every phrase is stored sealed" \
    '[[ -n "$(find "$DATA_DIR/secrets/device_key" -type f 2>/dev/null)" ]] && ! grep -rqE "legal winner|bGVnYWwgd2lubmVy" "$DATA_DIR/secrets/seed"'
# A blank password asks for a seal it cannot make, so it is refused before
# anything is stored rather than stored in the clear.
SECRETS_BEFORE="$(secret_files)"
check "a whitespace-only password is refused" $REJECTED \
    with_password "   " with_seed "$SEED" spectra wallet import --chain Solana --name "Blank SOL"
check "and so is an empty password file"    $REJECTED \
    spectra wallet new --chain Solana --name "Blank SOL" --password-file /dev/null
lacks "neither stored a wallet"             '"Blank SOL"' spectra --json wallet list
pass_if "nor a secret" '[[ "$(secret_files)" == "$SECRETS_BEFORE" ]]'

section "private-key wallets"
printf '4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318\n' > "$DATA_DIR/pk.hex"
contains "imports a key from a file and derives its address" \
    '0x2c7536e3605d9c16a7a3d7b1898e529396a65c23' \
    with_password "correct horse" spectra --json wallet import --chain Ethereum \
        --name "PK Wallet" --private-key-file "$DATA_DIR/pk.hex"
contains "export returns the key it sealed" '"privateKey":"4c0883a6' \
    with_password "correct horse" spectra --json wallet export "PK Wallet" --yes
# The key @dfinity/identity's vectors use: 32 bytes of 0x01.
printf '01%.0s' $(seq 32) > "$DATA_DIR/icp.hex"
check "imports an ICP key"                  $OK \
    spectra wallet import --chain internet-computer --name "ICP Key" \
        --private-key-file "$DATA_DIR/icp.hex" --no-password
contains "and a new process reads back its principal" '"icpPrincipal":"wf3fv-4c4nr-7ks2b' \
    spectra --json wallet show "ICP Key"
# Core refuses before sealing: a refusal after sealing would leave a key stored
# under an id no wallet references.
SECRETS_BEFORE="$(secret_files)"
check "refuses a chain that cannot derive from a key" $REJECTED \
    with_password "correct horse" spectra wallet import --chain Cardano \
        --name "No PK" --private-key-file "$DATA_DIR/pk.hex"
pass_if "and seals no key on the way to refusing" '[[ "$(secret_files)" == "$SECRETS_BEFORE" ]]'

section "adding a wallet to another network"
contains "a phrase wallet lists the networks its phrase restores on" '"sui"' \
    spectra --json wallet copy-targets "Acceptance SOL"
check "a copy needs the source's password"  $REJECTED \
    with_password wrong spectra wallet copy "Acceptance SOL" --chain sui --name "Copied SUI"
check "a network of another phrase format is refused" $REJECTED \
    spectra wallet copy "Acceptance SOL" --chain monero --name "Copied XMR"
lacks "neither stored a wallet"             '"Copied' spectra --json wallet list
check "adds the phrase to another network"  $OK \
    spectra wallet copy "Acceptance SOL" --chain sui --name "Copied SUI"
pass_if "the copy is sealed in its own file" \
    '[[ -f "$DATA_DIR/secrets/seed/$(spectra --json wallet show "Copied SUI" | python3 -c "import json,sys;print(json.load(sys.stdin)[\"wallet\"][\"id\"])")%2Eseed" ]]'
check "and signs under the source's password" $OK spectra send identity --from "Copied SUI"

section "giving a watched wallet its keys"
check "watches an address"                  $OK \
    spectra wallet watch --chain ethereum --name "Watched ETH" \
        --address 0x58a57ed9d8d624cbd12e2c467d34787555bb1b25
contains "its page offers to add its keys"  '"action":"addKeys"' spectra --json wallet actions "Watched ETH"
check "a phrase that does not hold it is refused" $REJECTED \
    with_seed "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about" \
        spectra wallet import --chain ethereum --upgrade "Watched ETH"
contains "the phrase that holds it gives it its keys" '"upgraded":true' \
    with_seed "$SEED" spectra --json wallet import --chain ethereum --upgrade "Watched ETH"
contains "and it keeps its name"            '"isWatchOnly":false' spectra --json wallet show "Watched ETH"

section "proving an address"
readonly SOL_ADDRESS="BLeUXTx9thHGT7VJUtF9vHEmfMDgW1nnKZ9UVer2CoLX"
SOL_SIGNATURE="$(spectra --json wallet sign-message "Acceptance SOL" --message "Spectra acceptance" \
    | python3 -c 'import json, sys; print(json.load(sys.stdin)["signature"])')"
check "a Solana wallet's signature verifies for its address" $OK \
    spectra address verify-message --chain solana --address "$SOL_ADDRESS" \
        --message "Spectra acceptance" --signature "$SOL_SIGNATURE"
check "and for no other message"            $REJECTED \
    spectra address verify-message --chain solana --address "$SOL_ADDRESS" \
        --message "Spectra acceptance!" --signature "$SOL_SIGNATURE"
check "EIP-712 typed data is not signed as a message" $REJECTED \
    spectra wallet sign-message "Watched ETH" \
        --message '{"domain":{},"types":{},"primaryType":"Permit","message":{}}'

# ── Confirmation gates ──────────────────────────────────────────────────────
#
# Commands that disclose a secret, move funds or destroy state need --yes.

section "confirmation gates"
check "export will not print a seed without --yes" $USAGE spectra wallet export "Renamed BTC"
check "a broadcast without --yes is refused" $USAGE \
    spectra send broadcast --from "Renamed BTC" --to bc1qgkju4yvvtuz0s8vqn837q396jezu2h8ex7gk98 --amount 0.001
check "delete will not run without --yes"   $USAGE spectra wallet delete "Renamed BTC"
check "reset will not run without --yes"    $USAGE spectra settings reset

# ── Deletion ────────────────────────────────────────────────────────────────
#
# Checked on disk rather than through `export`, which stops at "no such wallet"
# before it reaches the secret store. A wallet row can go while its sealed
# material stays behind, and that is the leak worth asserting against. The
# file store percent-encodes `.`, so the key `<id>.seed` is `seed/<id>%2Eseed`.
# Each file must exist before deletion, or its absence afterwards proves
# nothing about where the check looked.

section "deletion"
wallet_id() { spectra --json wallet show "$1" | python3 -c 'import json, sys; print(json.load(sys.stdin)["wallet"]["id"])'; }
SEED_FILE="$DATA_DIR/secrets/seed/$(wallet_id "Renamed BTC")%2Eseed"
KEY_FILE="$DATA_DIR/secrets/private_key/$(wallet_id "PK Wallet")%2Eprivatekey"
check "the sealed seed is on disk"          $OK test -f "$SEED_FILE"
check "the sealed key is on disk"           $OK test -f "$KEY_FILE"
check "deletes the seed wallet"             $OK spectra wallet delete "Renamed BTC" --yes
check "deletes the key wallet"              $OK spectra wallet delete "PK Wallet" --yes
check "the deleted wallet is gone"          1 spectra wallet show "Renamed BTC"
check "its sealed seed went with it"        1 test -e "$SEED_FILE"
check "its sealed key went with it"         1 test -e "$KEY_FILE"

# ── Loopback suites ─────────────────────────────────────────────────────────
#
# Each suite owns its data directory and its local nodes.

section "loopback suites"
suite() { local name="$1"; shift; check "$name" $OK python3 -B "$SCRIPTS/$1" "$BIN" "${@:2}"; }
suite "transparent transaction stages"                          cli-send-stages.py
suite "Polkadot Asset Hub transfers and finalized outcomes"     cli-send-polkadot.py
suite "Bittensor Finney transfers and finalized outcomes"       cli-send-polkadot.py bittensor
suite "Cardano extended witnesses and safe ADA inputs"          cli-send-cardano.py
suite "account execution outcomes survive reopening"            cli-finality-account-chains.py
suite "locally derived transaction hashes survive uncertain submission" cli-send-local-digests.py
suite "network-correct raw keys and watch imports"              cli-chain-coverage.py
suite "UTXO recipients preserve output script types"            cli-send-utxo.py
suite "complete mined OP Stack fees and durable outcomes"       cli-receipt-fees.py
suite "Litecoin SegWit recovery and durable signing"            cli-litecoin.py
suite "Peercoin recovery, mature rewards and durable signing"   cli-peercoin.py
suite "account recovery and multi-address signing on every UTXO network" cli-account-utxo.py
suite "2-of-3 multisig PSBTs on Bitcoin and Litecoin"             cli-multisig-psbt.py
suite "2-of-3 P2SH multisig on Bitcoin Cash and Dogecoin"          cli-multisig-p2sh.py
suite "Safe owners sign, join and execute a Safe transaction"      cli-multisig-safe.py
suite "Tron permissions: refused sends and multi-signed spends"    cli-multisig-tron.py
suite "XRP signer lists: refused sends and multi-signed payments"  cli-multisig-xrp.py
suite "Stellar signers: refused sends and multi-signed payments"  cli-multisig-stellar.py
suite "Sui MultiSig and Aptos MultiKey accounts from their policies" cli-multisig-sui-aptos.py
suite "Cardano native scripts witnessed by CIP-1854 keys"         cli-multisig-cardano.py
suite "Asset Hub pallet-multisig approvals execute a transfer"     cli-multisig-substrate.py
suite "Bittensor pallet-multisig approvals execute a transfer"    cli-multisig-substrate.py bittensor
suite "TON multisig v2 orders proposed and approved by signers"     cli-multisig-ton.py
suite "XRP signing and protocol validation"                     cli-send-xrp.py
suite "XRP destination tags and Stellar memos, required and signed" cli-payment-memos.py
suite "wallet operations: closing accounts, deleting keys"         cli-wallet-operations.py
suite "XRP Ledger and Stellar issued assets, by issuer"           cli-issued-assets.py
suite "ERC-721 and ERC-1155 tokens, never balances"              cli-nfts.py
suite "Zcash shielded scanning, transfers and recovery"          cli-zcash-shielded.py
suite "Litecoin MWEB scanning, peg-ins, peg-outs and recovery"   cli-litecoin-mweb.py
suite "TRC-10 discovery, signing and execution receipts"        cli-trc10.py
suite "owned staking preparation, execution and durable recovery" cli-staking.py
suite "ICP neuron ownership, explicit review and certified refusal" cli-icp-staking.py
suite "NEAR protocol fees, storage reserve and reviewed retry budgets" cli-send-near.py
suite "editable price sources survive reopening"                cli-token-preferences.py
for domain in wallets portfolio history send send-tokens send-icp-zcash send-monero endpoints; do
    suite "$domain integration checks" "cli-$domain.py"
done
# Its local SOCKS proxy is the point: requests name remote hosts and never
# leave loopback, so the host-based guard would refuse the very traffic it
# proves is proxied.
check "transport integration checks" $OK \
    env -u SPECTRA_LOOPBACK_ONLY python3 -B "$SCRIPTS/cli-transport.py" "$BIN"

section "no external network"
# The guard must catch a real attempt, or an empty journal proves nothing.
# Health probes call every built-in provider directly, so this one is refused.
check "a probe of remote providers still exits" $OK \
    with_journal "$CANARY_JOURNAL" spectra --json endpoints --chain bitcoin
contains "and the refusal is journaled by host" 'https://blockstream.info/' cat "$CANARY_JOURNAL"
check "no command reached beyond loopback" $OK test ! -s "$NETWORK_JOURNAL"
if [[ -s "$NETWORK_JOURNAL" ]]; then
    printf '    refused, then journaled:\n'
    sed 's/^/      /' "$NETWORK_JOURNAL"
fi

# ── Result ──────────────────────────────────────────────────────────────────

printf '\n'

if [[ "$FAILED" -eq 0 ]]; then
    printf '\033[32m%s passed\033[0m\n' "$PASSED"
    exit 0
fi
printf '\033[31m%s failed\033[0m, %s passed\n' "$FAILED" "$PASSED"
exit 1
