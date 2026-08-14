#!/usr/bin/env bash
#
# Drives `spectra` end to end against a throwaway data directory.
#
# This is the acceptance gate `PLAN.md` rule 1 asks for: "if `spectra` cannot
# drive it, it is in the wrong place." Every check here exercises a rule that
# lives in core — address validation, import planning, the address-book
# reducer, the shared display currency — through the same entry points the iOS
# app uses. A slice of Swift is not deleted until the rule it held is provable
# from this script.
#
# No network. Everything here is state, crypto and validation, so it runs in CI
# and in an offline checkout. Balance, history, price and send are deliberately
# absent: they need a live chain and would make this flaky.
#
# Usage:  scripts/cli-acceptance.sh [path/to/spectra]

set -uo pipefail

BIN="${1:-}"
if [[ -z "$BIN" ]]; then
    cargo build -p spectra-cli --quiet || exit 1
    BIN="$(cd "$(dirname "$0")/.." && pwd)/target/debug/spectra"
fi

DATA_DIR="$(mktemp -d)"
trap 'rm -rf "$DATA_DIR"' EXIT

# Wallets created here are throwaway, so the password is too.
export SPECTRA_PASSWORD="acceptance-password"

PASSED=0
FAILED=0

spectra() { "$BIN" --data-dir "$DATA_DIR" "$@"; }

# `spectra` is a shell function, so `env VAR=x spectra ...` cannot find it.
# These wrappers set the variable for one call instead.
with_seed() { local seed="$1"; shift; SPECTRA_SEED="$seed" "$@"; }
with_password() { local password="$1"; shift; SPECTRA_PASSWORD="$password" "$@"; }

# check <description> <expected-exit> <command...>
check() {
    local description="$1" expected="$2"
    shift 2
    local output status
    output="$("$@" 2>&1)"
    status=$?
    if [[ "$status" == "$expected" ]]; then
        PASSED=$((PASSED + 1))
        printf '  \033[32m✓\033[0m %s\n' "$description"
    else
        FAILED=$((FAILED + 1))
        printf '  \033[31m✗\033[0m %s \033[2m(exit %s, wanted %s)\033[0m\n' \
            "$description" "$status" "$expected"
        printf '    %s\n' "$output"
    fi
}

# contains <description> <needle> <command...>
contains() {
    local description="$1" needle="$2"
    shift 2
    local output
    output="$("$@" 2>&1)"
    if [[ "$output" == *"$needle"* ]]; then
        PASSED=$((PASSED + 1))
        printf '  \033[32m✓\033[0m %s\n' "$description"
    else
        FAILED=$((FAILED + 1))
        printf '  \033[31m✗\033[0m %s \033[2m(no %s)\033[0m\n' "$description" "$needle"
        printf '    %s\n' "$output"
    fi
}

section() { printf '\n\033[1m%s\033[0m\n' "$1"; }

# Exit codes are part of the interface: 0 done, 2 the caller asked wrongly,
# 3 core considered it and said no.
readonly OK=0 USAGE=2 REJECTED=3

# ── Registry ────────────────────────────────────────────────────────────────

section "chain registry"
check "lists chains"                        $OK spectra chains
contains "resolves a chain by symbol"  '"symbol":"BTC"' \
    spectra --json chains --filter btc
contains "hides testnets by default"   '"chains":[]' \
    spectra --json chains --filter "bitcoin testnet"

# ── Address validation ──────────────────────────────────────────────────────
#
# The rule that every chain's import address is validated. Both halves matter:
# a chain that used to be lenient must now refuse, and a valid address must
# come back normalised by core rather than as typed.

section "address validation"
check "accepts a valid Bitcoin address"     $OK \
    spectra address validate --chain Bitcoin bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4
check "refuses a malformed Solana address"  $REJECTED \
    spectra address validate --chain Solana definitely-not-an-address
check "refuses a malformed Tron address"    $REJECTED \
    spectra address validate --chain Tron nonsense
check "refuses a malformed EVM address"     $REJECTED \
    spectra address validate --chain Ethereum 0xnothex
contains "normalises EVM case" '"normalized":"0x742d35cc6634c0532925a3b844bc454e4438f44e"' \
    spectra --json address validate --chain Ethereum 0x742D35CC6634C0532925A3B844BC454E4438F44E

# ── Wallet lifecycle ────────────────────────────────────────────────────────

section "wallet lifecycle"
check "creates a wallet"                    $OK \
    spectra wallet new --chain Bitcoin --name "Acceptance BTC"
contains "stores the catalog derivation path" "m/84'/0'/0'/0/0" \
    spectra --json wallet show "Acceptance BTC"
check "refuses a seed length that is not 12 or 24" $USAGE \
    spectra wallet new --chain Bitcoin --name Bad --words 18
check "imports a known mnemonic"            $OK \
    with_seed "legal winner thank year wave sausage worth useful legal winner thank yellow" \
    spectra wallet import --chain Solana --name "Acceptance SOL"
contains "derives the documented address for that mnemonic" \
    "BLeUXTx9thHGT7VJUtF9vHEmfMDgW1nnKZ9UVer2CoLX" \
    spectra --json wallet show "Acceptance SOL"
check "refuses a mnemonic that fails its checksum" $REJECTED \
    with_seed "not a real seed phrase at all here" \
    spectra wallet import --chain Solana --name Bad
check "renames through the reducer"         $OK \
    spectra wallet rename "Acceptance BTC" "Renamed BTC"
check "refuses an empty name"               $REJECTED \
    spectra wallet rename "Renamed BTC" "   "
check "reports an unknown wallet"           1 spectra wallet show "no such wallet"

# ── Watch-only import ───────────────────────────────────────────────────────
#
# The path where the address is typed rather than derived, so the one that
# actually needs validating.

section "watch-only import"
check "accepts a valid watch address"       $OK \
    spectra wallet watch --chain Ethereum --name "Acceptance Watch" \
        --address 0x742d35Cc6634C0532925a3b844Bc454e4438f44e
check "refuses a malformed watch address"   $REJECTED \
    spectra wallet watch --chain Solana --address definitely-not-an-address
contains "names the address it refused" "definitely-not-an-address" \
    spectra wallet watch --chain Solana --address definitely-not-an-address
check "refuses to export a watch-only wallet" $REJECTED \
    spectra wallet export "Acceptance Watch" --yes

# ── Secrets ─────────────────────────────────────────────────────────────────

section "sealed secrets"
check "exports with the right password"     $OK \
    spectra wallet export "Renamed BTC" --yes
check "refuses the wrong password"          $REJECTED \
    with_password wrong spectra wallet export "Renamed BTC" --yes
check "will not print a seed without --yes" $USAGE \
    spectra wallet export "Renamed BTC"

# ── Address book ────────────────────────────────────────────────────────────

section "address book"
check "saves a contact"                     $OK \
    spectra address book add --chain Ethereum --name Alice \
        --address 0x742d35Cc6634C0532925a3b844Bc454e4438f44e
check "refuses a duplicate address"         $REJECTED \
    spectra address book add --chain Ethereum --name Bob \
        --address 0x742d35Cc6634C0532925a3b844Bc454e4438f44e
check "refuses an invalid address"          $REJECTED \
    spectra address book add --chain Solana --name Carol --address garbage
check "refuses an empty name"               $REJECTED \
    spectra address book add --chain Ethereum --name "" \
        --address 0x0000000000000000000000000000000000000001
contains "lists what it saved" '"name":"Alice"' spectra --json address book list
check "removes a contact"                   $OK spectra address book remove Alice
contains "removal empties the book" '"contacts":[]' spectra --json address book list

# ── Shared settings ─────────────────────────────────────────────────────────
#
# The setting the app reads from the same store. Stage 0 moved it; this is the
# check that it stayed moved.

section "display currency"
contains "defaults to USD"  '"currency":"USD"' spectra --json currency
check "sets a currency"                     $OK spectra currency CHF
contains "reads it back from the store" '"currency":"CHF"' spectra --json currency

# ── Deletion ────────────────────────────────────────────────────────────────

section "deletion"
check "will not delete without --yes"       $USAGE spectra wallet delete "Renamed BTC"
check "deletes a wallet"                    $OK spectra wallet delete "Renamed BTC" --yes
check "the deleted wallet is gone"          1 spectra wallet show "Renamed BTC"
# Checked on disk rather than through `export`, which stops at "no such wallet"
# before it ever reaches the secret store. A wallet row can go while its sealed
# seed stays behind, and that is exactly the leak worth asserting against.
if [[ -z "$(find "$DATA_DIR/secrets" -name "*.seed" -print -quit 2>/dev/null)" ]]; then
    PASSED=$((PASSED + 1))
    printf '  \033[32m✓\033[0m its sealed seed went with it\n'
else
    FAILED=$((FAILED + 1))
    printf '  \033[31m✗\033[0m its sealed seed went with it \033[2m(a .seed blob survived)\033[0m\n'
fi

# ── Result ──────────────────────────────────────────────────────────────────

printf '\n'
if [[ "$FAILED" -eq 0 ]]; then
    printf '\033[32m%s passed\033[0m\n' "$PASSED"
    exit 0
fi
printf '\033[31m%s failed\033[0m, %s passed\n' "$FAILED" "$PASSED"
exit 1
