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
check "a ticker is not a network name"      $USAGE spectra token catalog --chain ETH
check "a refusal by core is rejected"       $REJECTED \
    spectra address validate --chain Ethereum 0xnothex

# ── State across processes ──────────────────────────────────────────────────

section "wallets across processes"
check "imports a known mnemonic"            $OK \
    with_seed "$SEED" spectra wallet import --chain Solana --name "Acceptance SOL"
contains "a new process reads back its derived address" \
    "BLeUXTx9thHGT7VJUtF9vHEmfMDgW1nnKZ9UVer2CoLX" \
    spectra --json wallet show "Acceptance SOL"
check "an asset the wallet does not hold cannot be hidden" $REJECTED \
    spectra wallet hide "Acceptance SOL" DAI
check "creates a wallet"                    $OK \
    spectra wallet new --chain Bitcoin --name "Acceptance BTC"

# A store a build cannot read is refused, never rewritten, until the user
# discards it. Discarding runs on a copy: the store and secrets below are
# still needed.
section "an unreadable store"
if command -v sqlite3 >/dev/null 2>&1; then
    DISCARD_DIR="$(mktemp -d)"
    cp -R "$DATA_DIR/." "$DISCARD_DIR/"
    # A wallet row in a shape this build does not read.
    readonly FRESH='"derivationOverrides":{"passphrase":null,"hmacKey":null}'
    readonly STALE='"derivationOverrides":{"passphrase":null,"mnemonicWordlist":null,"hmacKey":null}'
    sqlite3 "$DISCARD_DIR/spectra.sqlite" "UPDATE wallets SET payload = REPLACE(payload, '$FRESH', '$STALE');"
    contains_exit 1 "an unreadable store names the way out" "settings discard --yes" \
        "$BIN" --data-dir "$DISCARD_DIR" wallet list
    check "discarding it asks for --yes"        $USAGE "$BIN" --data-dir "$DISCARD_DIR" settings discard
    check "discards it"                         $OK "$BIN" --data-dir "$DISCARD_DIR" settings discard --yes
    contains "and starts an empty store"        "no wallets yet" "$BIN" --data-dir "$DISCARD_DIR" wallet list
    rm -rf "$DISCARD_DIR"
else
    printf '  \033[33m-\033[0m %s\n' "skipped (no sqlite3): undecodable wallet row"
fi

# ── Secrets on disk ─────────────────────────────────────────────────────────

section "sealed secrets"
contains "export returns the phrase it sealed" "\"seedPhrase\":\"$SEED\"" \
    spectra --json wallet export "Acceptance SOL" --yes
check "a wrong password cannot unseal it"   $REJECTED \
    with_password wrong spectra wallet export "Acceptance SOL" --yes
check "a key export asks for --yes"         $USAGE \
    spectra wallet export "Acceptance SOL" --key private-key
check "imports without a password"          $OK \
    with_seed "$SEED" spectra wallet import --chain Solana --account 1 --name "Open SOL" --no-password
# No password is no plaintext: core seals the phrase under its device key, so
# neither the words nor their base64 are in the seed bucket.
pass_if "every phrase is stored sealed" \
    '[[ -n "$(find "$DATA_DIR/secrets/device_key" -type f 2>/dev/null)" ]] && ! grep -rqE "legal winner|bGVnYWwgd2lubmVy" "$DATA_DIR/secrets/seed"'
check "a whitespace-only password is refused" $REJECTED \
    with_password "   " with_seed "$SEED" spectra wallet import --chain Solana --name "Blank SOL"
check "and so is an empty password file"    $REJECTED \
    spectra wallet new --chain Solana --name "Blank SOL" --password-file /dev/null

section "private-key wallets"
printf '4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318\n' > "$DATA_DIR/pk.hex"
check "imports a key from a file"           $OK \
    with_password "correct horse" spectra wallet import --chain Ethereum \
        --name "PK Wallet" --private-key-file "$DATA_DIR/pk.hex"

# ── Confirmation gates ──────────────────────────────────────────────────────
#
# Commands that disclose a secret, move funds or destroy state need --yes.

section "confirmation gates"
check "export will not print a seed without --yes" $USAGE spectra wallet export "Acceptance BTC"
check "a broadcast without --yes is refused" $USAGE \
    spectra send broadcast --from "Acceptance BTC" --to bc1qgkju4yvvtuz0s8vqn837q396jezu2h8ex7gk98 --amount 0.001
check "delete will not run without --yes"   $USAGE spectra wallet delete "Acceptance BTC"
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
SEED_FILE="$DATA_DIR/secrets/seed/$(wallet_id "Acceptance BTC")%2Eseed"
KEY_FILE="$DATA_DIR/secrets/private_key/$(wallet_id "PK Wallet")%2Eprivatekey"
check "the sealed seed is on disk"          $OK test -f "$SEED_FILE"
check "the sealed key is on disk"           $OK test -f "$KEY_FILE"
check "deletes the seed wallet"             $OK spectra wallet delete "Acceptance BTC" --yes
check "deletes the key wallet"              $OK spectra wallet delete "PK Wallet" --yes
check "the deleted wallet is gone"          1 spectra wallet show "Acceptance BTC"
check "its sealed seed went with it"        1 test -e "$SEED_FILE"
check "its sealed key went with it"         1 test -e "$KEY_FILE"

# ── Loopback suites ─────────────────────────────────────────────────────────
#
# Each suite owns its data directory and its local nodes.

section "loopback suites"
suite() { local name="$1"; shift; check "$name" $OK python3 -B "$SCRIPTS/$1" "$BIN" "${@:2}"; }
suite "staged EVM sends across processes"                        cli-send-stages.py
suite "Polkadot Asset Hub staged sends and finalized outcomes"   cli-send-polkadot.py
suite "Cardano staged send broadcast once as signed"             cli-send-cardano.py
suite "a 2-of-3 Bitcoin PSBT across three data directories"       cli-multisig-psbt.py
suite "Safe owners sign, join and execute a Safe transaction"      cli-multisig-safe.py
suite "Tron active-permission spends signed in two data directories" cli-multisig-tron.py
suite "XRP signer-list payments signed across data directories"  cli-multisig-xrp.py
suite "Stellar payments signed across data directories"           cli-multisig-stellar.py
suite "Aptos MultiKey transfers signed across data directories"   cli-multisig-aptos.py
suite "Cardano native scripts witnessed by CIP-1854 keys"         cli-multisig-cardano.py
suite "Asset Hub pallet-multisig approvals execute a transfer"     cli-multisig-substrate.py
suite "TON multisig v2 orders proposed and approved by signers"     cli-multisig-ton.py
suite "wallet operations: closing accounts, deleting keys"         cli-wallet-operations.py
suite "XRP Ledger and Stellar trust lines opened, paid and removed" cli-issued-assets.py
suite "ERC-721 and ERC-1155 transfers broadcast as signed"        cli-nfts.py
suite "Zcash shielded scanning, transfers and recovery"          cli-zcash-shielded.py
suite "Litecoin MWEB scanning, peg-ins, peg-outs and recovery"   cli-litecoin-mweb.py
suite "TRC-10 staged send, broadcast and confirmation"           cli-trc10.py
suite "staking signed, retried and recovered across processes"   cli-staking.py
suite "ICP neuron staking reopened and refused on a forged certificate" cli-icp-staking.py
suite "NEAR durable retries and final status recovery"          cli-send-near.py
for domain in send send-tokens send-icp-zcash endpoints; do
    suite "$domain integration checks" "cli-$domain.py"
done

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
