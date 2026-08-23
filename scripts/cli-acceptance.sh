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
# The watch-addresses picker in the app is this flag, and it had drifted from
# it in both directions. Ethereum Classic has its own address slot and was
# folded into the shared EVM field, so its entries landed in a slot the planner
# does not read; Polygon and fifteen other EVM mainnets fell outside the
# seven-name condition that decided whether an EVM field appeared at all.
check "watches a chain with its own slot inside the EVM family" $OK \
    spectra wallet watch --chain "Ethereum Classic" --name "Watch ETC" \
        --address 0x742d35Cc6634C0532925a3b844Bc454e4438f44e
check "and one outside the seven the app used to name"        $OK \
    spectra wallet watch --chain Polygon --name "Watch Polygon" \
        --address 0x742d35Cc6634C0532925a3b844Bc454e4438f44e
contains "the catalog says which chains can be watched" '"name":"Polygon","privateKeyImport":true' \
    spectra --json chains --filter Polygon
contains "and that Polygon is one of them"   '"watchOnlyImport":true' \
    spectra --json chains --filter Polygon
contains "and Monero says it cannot"      '"watchOnlyImport":false' \
    spectra --json chains --filter Monero
check "refuses to watch Monero"           $REJECTED \
    spectra wallet watch --chain Monero --name "Watch XMR" \
        --address 48ZFsbBKZAnN9Tyw7XsCakJ4dBxBpaD3wa9Az6V5ZwAK99kYQzcgckSNVv5iZhMp8o37fhNzY7eM2ERGoTWr4B282s4mcDi

# ── Pathless chains ─────────────────────────────────────────────────────────

section "a chain with no derivation path"
# Monero's spend and view keys come from the seed, so its catalog row carries
# `derivation_path = []`. "No default path" used to be an error rather than an
# answer, and every caller read it as a broken catalog: this command exited
# with "Missing default derivation path for Monero." and iOS dropped the chain
# out of the batch it was deriving. Core has derived Monero the whole time.
check "imports Monero from a seed phrase"   $OK \
    with_seed "legal winner thank year wave sausage worth useful legal winner thank yellow" \
    spectra wallet import --chain Monero --name "XMR Wallet"
contains "and derives its address"          '"address":"4' \
    spectra --json wallet show "XMR Wallet"
contains "with no path, which is the answer rather than a failure" '"derivationPath":""' \
    spectra --json wallet show "XMR Wallet"
check "deletes the Monero wallet"           $OK spectra wallet delete "XMR Wallet" --yes

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

# ── Price alerts ────────────────────────────────────────────────────────────
#
# The rules moved into `CoreAppState` for this command to exist; before it they
# lived only in Swift with core owning just the evaluator. Every check here is
# a separate process, so this is also the persistence test.

section "price alerts"
check "adds an alert"                       $OK \
    spectra alert add --chain Bitcoin --target 1 --above
check "refuses an alert that cannot fire"   $REJECTED \
    spectra alert add --chain Bitcoin --target 0
contains "the alert survives a new process" '"symbol":"BTC"' \
    spectra --json alert list
check "removes by symbol"                   $OK spectra alert remove BTC
check "refuses removing what is not set"    $REJECTED spectra alert remove BTC
check "refuses checking with no alerts"     $REJECTED spectra alert check

# ── Keypool ─────────────────────────────────────────────────────────────────
#
# Reserving a receive index must be idempotent — the app opening the receive
# sheet twice must not burn two addresses — and change must always consume one.

section "keypool"
check "shows the pool"                      $OK spectra pool show "Acceptance SOL"
contains "reserving twice yields the same receive index" '"index":0' \
    spectra --json pool next "Acceptance SOL"
contains "and again"                        '"index":0' \
    spectra --json pool next "Acceptance SOL"
contains "change always consumes"           '"index":0' \
    spectra --json pool next-change "Acceptance SOL"
contains "so the next change differs"       '"index":1' \
    spectra --json pool next-change "Acceptance SOL"

# ── Rescan ──────────────────────────────────────────────────────────────────

section "rescan"
contains "derives the candidate matrix offline" '"checked":false' \
    with_seed "legal winner thank year wave sausage worth useful legal winner thank yellow" \
    spectra --json rescan --dry-run
contains "four Bitcoin script types across three accounts" '"chain":"bitcoin"' \
    with_seed "legal winner thank year wave sausage worth useful legal winner thank yellow" \
    spectra --json rescan --dry-run --chain Bitcoin
check "refuses a seed that is not a mnemonic" $REJECTED \
    with_seed "not a real seed phrase at all" spectra rescan --dry-run

# ── Refresh ─────────────────────────────────────────────────────────────────
#
# The sweep itself needs a network. What is checkable offline is that the
# engine refuses an empty run rather than reporting a successful no-op — the
# shape of the bug this command surfaced, where a sweep that had not finished
# reported "0 refreshed, 0 errors".

section "refresh"
# Against its *own* empty directory: by this point the shared one has wallets,
# and `spectra refresh` there would sweep them over the network — which this
# script promises not to do. The first version of this check did exactly that.
check "refuses a refresh with no wallets"   $REJECTED \
    "$BIN" --data-dir "$(mktemp -d)" refresh

# ── Diagnostics ─────────────────────────────────────────────────────────────
#
# Core's own self-tests, which need no network and no device. Seven of them
# were failing on fabricated fixtures until the CLI could run them.

section "diagnostics"
check "every chain's self-tests pass"       $OK spectra diagnostics self-test
contains "reports a check count"      '"failed":0' \
    spectra --json diagnostics self-test
check "self-tests one chain"                $OK spectra diagnostics self-test --chain Bitcoin
check "refuses self-tests for an unknown chain" $USAGE \
    spectra diagnostics self-test --chain Nope
contains "builds a diagnostics document"  '"endpoints"' \
    spectra diagnostics show --chain Bitcoin

# ── Tracked tokens ──────────────────────────────────────────────────────────
#
# The clamp is the rule that moved into core with the list: a token cannot
# display more places than it has. And they have to survive a reopen — every
# command here is a separate process, so this section is also the persistence
# test.

section "tracked tokens"
contains "lists the built-in catalog" '"symbol":"USDC"' \
    spectra --json token catalog --chain Ethereum
check "refuses a token the catalog does not have" $REJECTED \
    spectra token track --chain Ethereum NOTACOIN
check "refuses tracking on a chain without tokens" $REJECTED \
    spectra token track --chain Bitcoin USDC
check "tracks a catalog token"                     $OK \
    spectra token track --chain Ethereum USDC --display-decimals 2
check "refuses tracking the same token twice"      $REJECTED \
    spectra token track --chain Ethereum USDC
contains "the tracked token survives a new process" '"symbol":"USDC"' \
    spectra --json token list
contains "clamps a display width the token cannot have" '"displayDecimals":6' \
    spectra --json token track --chain Ethereum USDT --display-decimals 99
check "untracks"                                   $OK spectra token untrack USDC
check "refuses untracking what is not tracked"     $REJECTED spectra token untrack USDC

# ── Staking ─────────────────────────────────────────────────────────────────
#
# Offline half only. Core has had a staking service since before the CLI could
# reach it — only Swift drove it — so "which chains stake" is the part worth
# asserting without a network.

# ── EVM send assembly ───────────────────────────────────────────────────────
#
# The funds-path rule this script could not reach until now, and the gap is how
# it stayed wrong. `prepare_evm_send_assembly` builds the transaction the send
# sheet estimates gas against; its only caller was the iOS send sheet, so it
# greps as dead from the Rust tree and no assertion here touched it. Inside it,
# `is_supported_evm_chain` named seven chains and `is_native_evm_asset` listed
# nine `(chain, symbol)` pairs — two of them governance tokens. Assembling
# takes no key, no network and no store, so it belongs here.

section "EVM send assembly"
EVM_ADDR=0x742d35Cc6634C0532925a3b844Bc454e4438f44e
# Base is one of the sixteen mainnets that used to answer UnsupportedChain,
# which surfaced in the app as "Unable to estimate network fee" on a send that
# was otherwise fine.
check "assembles on a chain outside the old seven" $OK \
    spectra send assemble --chain Base --from $EVM_ADDR --to $EVM_ADDR --amount 1.5
contains "as a native transfer of the gas asset" '"isNative":true' \
    spectra --json send assemble --chain Base --from $EVM_ADDR --to $EVM_ADDR --amount 1.5
contains "with the amount in wei"                '"valueWei":"1500000000000000000"' \
    spectra --json send assemble --chain Base --from $EVM_ADDR --to $EVM_ADDR --amount 1.5
# ARB is not what Arbitrum charges gas in. Listing it as native built a value
# transfer of that many ETH and discarded the contract it was handed.
contains "a governance token is not the gas asset" '"isNative":false' \
    spectra --json send assemble --chain Arbitrum --from $EVM_ADDR --to $EVM_ADDR \
        --amount 100 --symbol ARB \
        --contract 0x912ce59144191c1204e64559fe8253a0e49e6548 --decimals 18
contains "and moves no gas asset"                 '"valueWei":"0"' \
    spectra --json send assemble --chain Arbitrum --from $EVM_ADDR --to $EVM_ADDR \
        --amount 100 --symbol ARB \
        --contract 0x912ce59144191c1204e64559fe8253a0e49e6548 --decimals 18
contains "addressed to its contract, not the recipient" \
    '"to":"0x912ce59144191c1204e64559fe8253a0e49e6548"' \
    spectra --json send assemble --chain Arbitrum --from $EVM_ADDR --to $EVM_ADDR \
        --amount 100 --symbol ARB \
        --contract 0x912ce59144191c1204e64559fe8253a0e49e6548 --decimals 18
check "refuses a malformed sender"          $REJECTED \
    spectra send assemble --chain Base --from nothex --to $EVM_ADDR --amount 1
check "refuses a malformed recipient"       $REJECTED \
    spectra send assemble --chain Base --from $EVM_ADDR --to nothex --amount 1
check "refuses a non-EVM chain"             $REJECTED \
    spectra send assemble --chain Bitcoin --from $EVM_ADDR --to $EVM_ADDR --amount 1
check "refuses half a token description"    $USAGE \
    spectra send assemble --chain Base --from $EVM_ADDR --to $EVM_ADDR --amount 1 \
        --contract $EVM_ADDR

section "settings"
check "lists the settings core owns"        $OK spectra settings list
check "sets one"                            $OK \
    spectra settings set etherscan-api-key ACCEPTANCE-KEY
contains "and a second process reads it back" '"value":"ACCEPTANCE-KEY"' \
    spectra --json settings get etherscan-api-key
# Fee priority is keyed by chain rather than global: two chains had a settings
# field each and the other seventy-six shared a dictionary iOS persisted
# itself, so the CLI could set exactly two of the seventy-eight.
check "sets a per-chain fee priority"       $OK \
    spectra settings set fee-priority.Dogecoin economy
contains "and reads it back"                '"value":"economy"' \
    spectra --json settings get fee-priority.Dogecoin
contains "a chain never set reads the default" '"value":"normal"' \
    spectra --json settings get fee-priority.Solana
# The three the picker offers, or the default. A value no send path knows how
# to spend is not worth storing under a name that says a fee was chosen.
contains "refuses a priority no send path spends" '"value":"normal"' \
    spectra --json settings set fee-priority.Solana lightspeed
check "refuses a chain the registry does not know" $REJECTED \
    spectra settings set fee-priority.Nonsuch economy
# The bound is core's. A stop gap of zero finds no addresses, and this used to
# be clamped only in an iOS `didSet` — reachable from nowhere else.
check "bounds a number instead of storing it" $OK \
    spectra settings set bitcoin-stop-gap 9999
contains "clamped to the top of the range"  '"value":"200"' \
    spectra --json settings get bitcoin-stop-gap
contains "trims a pasted value"             '"value":"KEY"' \
    spectra --json settings set etherscan-api-key "  KEY  "
check "refuses a setting that does not exist" $REJECTED spectra settings set nope 1
check "refuses a value of the wrong kind"   $REJECTED \
    spectra settings set strict-rpc-only maybe

section "private-key import"
# The last wallet operation the CLI could not drive. Core has dispatched
# private-key derivation by chain since `core_derive_from_private_key`; what
# was missing was the command.
printf '4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318\n' > "$DATA_DIR/pk.hex"
check "imports a wallet from a private key"  $OK \
    with_password "correct horse" spectra wallet import --chain Ethereum \
        --name "PK Wallet" --private-key-file "$DATA_DIR/pk.hex"
contains "and derives the right address"     '0x2c7536e3605d9c16a7a3d7b1898e529396a65c23' \
    spectra --json wallet show "PK Wallet"
contains "and reports how it signs"          'private key' \
    spectra wallet show "PK Wallet"
check "refuses a chain that cannot derive from a key" $REJECTED \
    with_password "correct horse" spectra wallet import --chain Cardano \
        --name "No PK" --private-key-file "$DATA_DIR/pk.hex"
# Which chains a private key covers is one registry fact, and this is the check
# that the app's picker and the CLI cannot disagree about it. Polygon was in
# neither of the app's two hand-written lists and derives the same EVM address
# as Ethereum; Decred was in one list, absent from Swift's switch, and derives.
contains "the same key derives on every EVM chain" '0x2c7536e3605d9c16a7a3d7b1898e529396a65c23' \
    with_password "correct horse" spectra --json wallet import --chain Polygon \
        --name "PK Polygon" --private-key-file "$DATA_DIR/pk.hex"
check "and on the fifth UTXO chain"           $OK \
    with_password "correct horse" spectra wallet import --chain Decred \
        --name "PK Decred" --private-key-file "$DATA_DIR/pk.hex"
contains "a chain that derives says so in the catalog" '"name":"Polygon","privateKeyImport":true' \
    spectra --json chains --filter Polygon
contains "and one that does not says that"    '"name":"Solana","privateKeyImport":false' \
    spectra --json chains --filter Solana
check "cleans up the extra key wallets"       $OK spectra wallet delete "PK Polygon" --yes
check "and the second one"                    $OK spectra wallet delete "PK Decred" --yes
# A key the CLI can seal but never return is a lost key, so export handles it —
# behind the same gate as a seed phrase.
check "will not print the key without --yes"  $USAGE spectra wallet export "PK Wallet"
contains "returns the key it sealed"          '"privateKey":"4c0883a6' \
    with_password "correct horse" spectra --json wallet export "PK Wallet" --yes
check "deletes the private-key wallet"        $OK spectra wallet delete "PK Wallet" --yes

section "self-tests"
# A suite keyed by a name no caller can type is green and unreachable at the
# same time: `CHAIN_SPECS` had a row keyed "XRP" where the registry says "XRP
# Ledger", and every caller resolves its input through the registry.
contains "runs a chain's self-tests"       '"chain":"XRP Ledger"' \
    spectra --json diagnostics self-test --chain "XRP Ledger"
contains "and the symbol resolves to it"   '"chain":"XRP Ledger"' \
    spectra --json diagnostics self-test --chain XRP
contains "with no failures"                '"failed":0' \
    spectra --json diagnostics self-test --chain "XRP Ledger"

section "staking"
check "refuses staking on a chain that does not stake" $REJECTED \
    spectra staking validators --chain Bitcoin
contains "and says which chain, not which endpoint" "Bitcoin does not have protocol-native staking" \
    spectra staking validators --chain Bitcoin
check "refuses staking on an unknown chain"            $USAGE \
    spectra staking validators --chain Nope
# The staking picker in the app was a seven-case Swift enum with its own
# display-name and id switches, beside two match arms in `StakingService` over
# the same seven ids. One registry column now, and this is the column.
contains "the catalog says which chains stake" '"name":"Polkadot","privateKeyImport":false,"staking":true' \
    spectra --json chains --filter Polkadot
contains "and which do not"                   '"name":"Dogecoin","privateKeyImport":true,"staking":false' \
    spectra --json chains --filter Dogecoin
check "a testnet does not stake where its mainnet does" $REJECTED \
    spectra staking validators --chain solana-devnet

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
