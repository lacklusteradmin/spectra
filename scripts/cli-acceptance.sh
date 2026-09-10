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

# lacks <description> <needle> <command...>
lacks() {
    local description="$1" needle="$2"
    shift 2
    local output
    output="$("$@" 2>&1)"
    if [[ "$output" != *"$needle"* ]]; then
        PASSED=$((PASSED + 1))
        printf '  \033[32m✓\033[0m %s\n' "$description"
    else
        FAILED=$((FAILED + 1))
        printf '  \033[31m✗\033[0m %s \033[2m(found %s)\033[0m\n' "$description" "$needle"
        printf '    %s\n' "$output"
    fi
}

section() { printf '\n\033[1m%s\033[0m\n' "$1"; }

# Exit codes are part of the interface: 0 done, 2 the caller asked wrongly,
# 3 core considered it and said no.
readonly OK=0 USAGE=2 REJECTED=3

# ── Registry ────────────────────────────────────────────────────────────────

section "history read failures"
check "corrupt history is refused without deleting records" $OK \
    python3 "$(dirname "$0")/cli-history-corruption.py" "$BIN"

section "exact send amounts"
contains "Solana preserves units beyond f64 precision" '"rawAmount":"9007199254740993"' \
    spectra --json send amount --chain Solana --amount 9007199.254740993
contains "token precision uses exact units" '"rawAmount":"9007199254740993"' \
    spectra --json send amount --chain Tron --decimals 6 --amount 9007199254.740993
check "rejects excess amount precision" $REJECTED spectra send amount --chain Bitcoin --amount 0.000000001
check "rejects negative amount" $REJECTED spectra send amount --chain Ethereum --amount -1
check "rejects non-finite amount" $REJECTED spectra send amount --chain Ethereum --amount inf
check "rejects integer overflow" $REJECTED spectra send amount --chain Ethereum --decimals 0 --amount 340282366920938463463374607431768211456
check "rejects unreasonable token precision" $REJECTED spectra send amount --chain Solana --decimals 4294967295 --amount 1

section "checked fee units"
contains "Cardano fee is exact" '"rawFee":"170000"' spectra --json send fee-units --chain Cardano --amount 0.17
contains "Sui budget is exact" '"rawFee":"10000000"' spectra --json send fee-units --chain Sui --amount 0.01
for bad_fee in -1 0 NaN inf 0.0000001 18446744073710.0; do
    check "rejects invalid native fee $bad_fee" $REJECTED spectra send fee-units --chain Cardano --amount "$bad_fee"
done

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
# An EVM address whose letters are not all one case carries an EIP-55
# checksum, and that checksum exists to catch a mistyped or corrupted paste.
# The validator used to lowercase first and never look, so any forty hex digits
# passed.
check "accepts a correct EIP-55 checksum"   $OK \
    spectra address validate --chain Ethereum 0x742d35Cc6634C0532925a3b844Bc454e4438f44e
check "refuses a broken EIP-55 checksum"    $REJECTED \
    spectra address validate --chain Ethereum 0x742d35cC6634C0532925a3b844Bc454e4438f44e
check "accepts the unchecksummed lower-case form" $OK \
    spectra address validate --chain Ethereum 0x742d35cc6634c0532925a3b844bc454e4438f44e

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
# One seed, several chains, in one command. `--chain` could only be given once
# because the CLI derived the address itself before handing it to
# `import_wallets`; core derives them now, so the multi-chain rule — every EVM
# chain derives from Ethereum's path — lives with the registry rather than in
# whichever front end happened to import more than one chain.
check "imports one seed across three chains" $OK \
    with_seed "legal winner thank year wave sausage worth useful legal winner thank yellow" \
    spectra wallet import --chain Bitcoin --chain Ethereum --chain Solana --name "Multi"
contains "and derives each chain's own address" \
    "BLeUXTx9thHGT7VJUtF9vHEmfMDgW1nnKZ9UVer2CoLX" \
    spectra --json wallet show "Multi 3"
contains "the Bitcoin one too"                "bc1qgkju4yvvtuz0s8vqn837q396jezu2h8ex7gk98" \
    spectra --json wallet show "Multi 1"
check 'but wallet new still takes exactly one' $USAGE \
    spectra wallet new --chain Bitcoin --chain Ethereum --name Two
check "refuses a mnemonic that fails its checksum" $REJECTED \
    with_seed "not a real seed phrase at all here" \
    spectra wallet import --chain Solana --name Bad
# One verdict decides a seed phrase, and it says which of the two things is
# wrong: words that are in no wordlist are named, and only a phrase built
# entirely of real words is worth checksumming.
contains "names the words that are in no wordlist" "not in any BIP-39 word list" \
    with_seed "not a real seed phrase at all here" \
    spectra wallet import --chain Solana --name Bad
contains "and blames the checksum when the words are real" "checksum" \
    with_seed "legal winner thank year wave sausage worth useful legal winner thank legal" \
    spectra wallet import --chain Solana --name Bad
# Simplified and Traditional Chinese share most of their word list, so a
# Chinese mnemonic cannot be pinned to one of them. Language detection used
# to refuse exactly those phrases; a phrase valid in some language is valid.
check "imports a Chinese mnemonic" $OK \
    with_seed "的 的 的 的 的 的 的 的 的 的 的 在" \
    spectra wallet import --chain Bitcoin --name Chinese
contains "and derives a Bitcoin address for it" '"address":"bc1q' \
    spectra --json wallet show Chinese
# BIP-39 seeds the wallet from the *words*, not from the entropy they encode,
# so the Chinese phrase for the all-zero entropy must not land on the English
# phrase's address. Deriving under the wrong wordlist is how it would.
lacks "not the address the English phrase for the same entropy gives" \
    "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu" \
    spectra --json wallet show Chinese
check "renames through the reducer"         $OK \
    spectra wallet rename "Acceptance BTC" "Renamed BTC"
check "refuses an empty name"               $REJECTED \
    spectra wallet rename "Renamed BTC" "   "
check "reports an unknown wallet"           1 spectra wallet show "no such wallet"

section "stored signing identity"
contains "core resolves stored Bitcoin identity" 'bc1qgkju4yvvtuz0s8vqn837q396jezu2h8ex7gk98' \
    spectra --json send identity --from "Multi 1"
contains "core resolves stored Solana identity" 'BLeUXTx9thHGT7VJUtF9vHEmfMDgW1nnKZ9UVer2CoLX' \
    spectra --json send identity --from "Multi 3"
check "EVM sender identity works on shared-address chains" $OK \
    spectra send identity --from "Multi 2" --chain Arbitrum
check "refuses unrelated sender chain" $REJECTED \
    spectra send identity --from "Multi 1" --chain Solana
check "wrong password cannot unlock sender identity" $REJECTED \
    with_password wrong spectra send identity --from "Multi 2"

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
check "watch-only sender cannot resolve signing identity" $REJECTED \
    spectra send identity --from "Acceptance Watch"
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
contains "the catalog says which chains can be watched" '"name":"Polygon"' \
    spectra --json chains --filter Polygon
contains "and that Polygon is one of them"   '"watchOnlyImport":true' \
    spectra --json chains --filter Polygon
contains "and Monero says it cannot"      '"watchOnlyImport":false' \
    spectra --json chains --filter Monero
check "refuses to watch Monero"           $REJECTED \
    spectra wallet watch --chain Monero --name "Watch XMR" \
        --address 48ZFsbBKZAnN9Tyw7XsCakJ4dBxBpaD3wa9Az6V5ZwAK99kYQzcgckSNVv5iZhMp8o37fhNzY7eM2ERGoTWr4B282s4mcDi

# ── Pathless chains ─────────────────────────────────────────────────────────

# A watch-only import creates one wallet per address entry, and core mints the
# ids: a caller supplying them had to predict the count, which meant parsing
# the entries the same way the planner does.
contains "a multi-address watch import creates one wallet each" '"count":2' \
    spectra --json wallet watch --chain Bitcoin \
    --address bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu \
    --address bc1qgkju4yvvtuz0s8vqn837q396jezu2h8ex7gk98 --name "Watch Pair"

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

section "endpoint kinds"
# `roles` held two things at once: what an endpoint is, and what it is used
# for. Nothing kept them consistent, and both drifted — ten EVM chains' RPC
# nodes lost the `rpc` marker, and forty-six claimed a `history` capability no
# EVM node can serve, because `eth_getTransactionsByAddress` is not a method.
contains "an EVM node is an rpc-node"        '"kind":"rpc-node"' \
    spectra --json endpoints --chain Ethereum
contains "and does not claim address history" '"capabilities":["read","balance","fee","broadcast"]' \
    spectra --json endpoints --chain Ethereum

section "evm history source"
# Fourteen EVM mainnets read history from a keyless explorer; seven still need
# an Etherscan key because V2 has no keyless tier and V1 is shut down across
# the whole family; two are served by nobody. Offline assertions only — the
# table itself, not the fetch.
contains "a keyless chain names no api key"   '"needsApiKey":false' \
    spectra --json chains --filter Ethereum
contains "and a key-only chain says so"       '"needsApiKey":true' \
    spectra --json chains --filter "BNB Chain"
contains "a chain nobody serves says that"    '"historySource":"none"' \
    spectra --json chains --filter Cronos

section "utxo address discovery"
# The derive-and-probe walk the app runs on every UTXO refresh. It lived in
# Swift because the seed phrase was only readable there; core reads the seed,
# the derivation path, the keypool bound, the balance and the history.
contains "lists what a sealed UTXO wallet already holds" '"addressCount":1' \
    spectra --json pool discover "Renamed BTC"
# A chain with no walk answers empty rather than failing: the refresh loop asks
# for every chain a wallet is on.
contains "a non-UTXO chain discovers nothing" '"addressCount":0' \
    spectra --json pool discover "Acceptance SOL"
# Reserving, deriving and recording are one call now. The floor of 1 is core's
# rule: a deep-UTXO chain never hands out index 0 as a receive address.
contains "a UTXO wallet's receive address is never index 0" '"index":1' \
    spectra --json pool next "Renamed BTC"

section "wallets with no password"
# The state the iOS app has always had and core could not represent: a wallet
# whose material is stored without a password. Core held one key layout for
# sealed wallets and the app held another for unsealed ones, in the same
# keychain, neither able to read the other's.
check "imports without a password"          $OK \
    with_seed "legal winner thank year wave sausage worth useful legal winner thank yellow" \
    spectra wallet import --chain Solana --name "Open SOL" --no-password
contains "and derives the same address as the sealed import" \
    "BLeUXTx9thHGT7VJUtF9vHEmfMDgW1nnKZ9UVer2CoLX" \
    spectra --json wallet show "Open SOL"
check "exports with no password asked"      $OK \
    spectra wallet export "Open SOL" --yes
contains "and the phrase is the one imported" \
    '"seedPhrase":"legal winner thank year wave sausage worth useful legal winner thank yellow"' \
    spectra --json wallet export "Open SOL" --yes
# A password and no password are different states, not the same one with a
# blank field: the sealed wallet still demands its password.
check "the sealed wallet still wants its password" $REJECTED \
    with_password wrong spectra wallet export "Acceptance SOL" --yes
check "and --no-password refuses to also take a password file" $USAGE \
    spectra wallet import --chain Solana --name Nope --no-password --password-file /dev/null
check "deletes the unsealed wallet"         $OK \
    spectra wallet delete "Open SOL" --yes

# ── Addresses per network ───────────────────────────────────────────────────
#
# A wallet on a family with testnets holds one address per network, derived
# once at import. The app used to re-derive the testnet address from the seed
# on every read — so nothing outside the app could see it, and a
# password-sealed wallet, which has no seed to read, showed the mainnet address
# on testnet instead.

section "addresses per network"
contains "a Bitcoin wallet stores its testnet4 address too" '"Bitcoin Testnet4"' \
    spectra --json wallet show "Multi 1"
contains "and its signet one" '"Bitcoin Signet"' spectra --json wallet show "Multi 1"
contains "the mainnet address is the primary" 'bc1q' spectra --json wallet show "Multi 1"
# One key, two encodings: a testnet address is not the mainnet one.
check "the testnet address differs from the mainnet address" $OK \
    bash -c '"$1" --data-dir "$2" --json wallet show "Multi 1" | grep -q "tb1"' _ "$BIN" "$DATA_DIR"
# The EVM family shares one address, and Ethereum Classic has a slot of its own
# holding the same key — so an Ethereum wallet answers on both.
contains "an EVM wallet fills the Ethereum Classic slot too" '"Ethereum Classic"' \
    spectra --json wallet show "Multi 2"
# A chain the wallet was never imported for has no address, and no seed is read
# to invent one.
check "a Solana wallet holds no Bitcoin address" $OK \
    bash -c '! "$1" --data-dir "$2" --json wallet show "Multi 3" | grep -q "bc1q"' _ "$BIN" "$DATA_DIR"

section "public-child receive derivation"
check "imports an unsealed BTC wallet" $OK \
    with_seed "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about" \
    spectra wallet import --chain Bitcoin --name "Open BTC" --no-password
contains "derives the reserved receive address offline" '"address":"bc1q' \
    spectra --json pool next "Open BTC"
contains "the receive index remains reserved on reopen" '"index":1' \
    spectra --json pool next "Open BTC"
check "deletes the temporary BTC wallet" $OK spectra wallet delete "Open BTC" --yes

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
# The twelve codes are core's. They were a Swift enum and nothing else, so this
# command stored whatever string it was handed — and every amount then rendered
# unconverted with that code beside it.
check "refuses a code nothing quotes"       $REJECTED spectra currency ZZZ
check "and one that is not a code at all"   $REJECTED spectra currency bitcoin
contains "the refusal changed nothing"  '"currency":"CHF"' spectra --json currency
# Cross-rates are core state now, not a blob one front end kept: this reads the
# same store the app does. Fetching them needs network, so what is offline is
# the empty answer.
contains "no rates stored until one is fetched" '"count":0' \
    spectra --json currency --rates

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
    spectra token track --chain Ethereum USDC
check "refuses tracking the same token twice"      $REJECTED \
    spectra token track --chain Ethereum USDC
contains "the known token survives a new process" '"symbol":"USDC"' \
    spectra --json token list
check "untracks"                                   $OK spectra token untrack USDC
check "refuses untracking what is not tracked"     $REJECTED spectra token untrack USDC

# ── Amount display ──────────────────────────────────────────────────────────
#
# Decimal places follow the amount, not a per-chain setting. There used to be
# one setting per chain and one per token — 137 steppers — because a fixed count
# cannot serve both a large balance and a small one: at the old default of three
# places, 0.00042 BTC read "<0.001 BTC", and the only cure was to find Bitcoin in
# a list of forty-six and tap "+" five times.
#
# Six significant digits, counted from the first non-zero digit, capped by what
# the asset has and by eight places.

section "amount display"
contains "a small balance keeps its digits" '"shows":"0.00042"' \
    spectra --json token format 0.00042 --chain Bitcoin
contains "and is not marked as below a threshold" '"belowThreshold":false' \
    spectra --json token format 0.00042 --chain Bitcoin
contains "an eighteen-decimal chain does the same" '"shows":"0.000015"' \
    spectra --json token format 0.000015 --chain Ethereum
contains "a large balance spends its budget on the integer" '"shows":"1234.57"' \
    spectra --json token format 1234.5678 --chain Ethereum
contains "trailing zeros are trimmed, not padded" '"shows":"12.5"' \
    spectra --json token format 12.5 --chain Ethereum --symbol USDC
contains "a token never shows more places than it has" '"assetDecimals":6' \
    spectra --json token format 12.5 --chain Ethereum --symbol USDC
contains "one wei is dust and says so" '"belowThreshold":true' \
    spectra --json token format 0.000000000000000001 --chain Ethereum
contains "and the marker is the eight-place floor" '"shows":"<0.00000001"' \
    spectra --json token format 0.000000000000000001 --chain Ethereum
check "refuses a token the chain does not have"    $REJECTED \
    spectra token format 1 --chain Bitcoin --symbol USDC

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

# ── Signing without broadcasting ────────────────────────────────────────────
#
# The send path's last unproven step is the broadcast itself. `--sign-only`
# runs everything before it — stored identity, amount, fees, live nonce, the
# built and signed payload — and stops, so the path can be exercised without
# moving funds. Signing needs the network, so what is checked here is the
# refusal: a chain whose builder cannot stop before broadcasting must say so
# rather than broadcast a caller's dry run.

section "sign without broadcasting"
check "refuses sign-only where the builder cannot stop" $REJECTED \
    spectra send broadcast --from "Multi 3" --to "BLeUXTx9thHGT7VJUtF9vHEmfMDgW1nnKZ9UVer2CoLX" \
    --amount 0.001 --sign-only
contains "and says which chain" "Solana" \
    spectra send broadcast --from "Multi 3" --to "BLeUXTx9thHGT7VJUtF9vHEmfMDgW1nnKZ9UVer2CoLX" \
    --amount 0.001 --sign-only
# A broadcast still takes --yes; signing does not, because it moves nothing.
check "a broadcast without --yes is refused" $USAGE \
    spectra send broadcast --from "Multi 1" --to bc1qgkju4yvvtuz0s8vqn837q396jezu2h8ex7gk98 --amount 0.001

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

# Which pending sends can still be replaced is core's rule, over core's own
# records. Recording one needs a broadcast, so what is offline is the empty
# answer and the wallet filter; the rule itself is covered by
# `cargo test -p spectra_core replaceable` and the Swift bridge tests.
section "replaceable sends"
check "lists nothing to replace" $OK spectra txs --replaceable
contains "answers as an empty list" '"replaceable":[]' spectra --json txs --replaceable
contains "scopes to one wallet" '"count":0' spectra --json txs --replaceable --wallet "Multi 2"
check "reports an unknown wallet" 1 spectra txs --replaceable --wallet "no such wallet"

section "EVM manual nonce"
contains "parses whitespace and leading zeros" '"nonce":12' spectra --json send overrides --nonce ' 0012 '
contains "accepts nonce above Int32" '"nonce":2147483648' spectra --json send overrides --nonce 2147483648
contains "accepts signed FFI maximum nonce" '"nonce":9223372036854775807' spectra --json send overrides --nonce 9223372036854775807
for nonce in '' ' ' '+1' '1.0' '1e2' '0x10' '1 2' '１２' '9223372036854775808'; do
    check "refuses invalid manual nonce [$nonce]" $REJECTED spectra send overrides --nonce "$nonce"
done

section "EVM overrides"
check "default overrides are valid" $OK spectra send overrides
contains "keeps a zero nonce" '"nonce":0' spectra --json send overrides --nonce 0
check "refuses negative nonce" $REJECTED spectra send overrides --nonce -1
check "refuses zero gas" $REJECTED spectra send overrides --gas-limit 0
check "refuses negative gas" $REJECTED spectra send overrides --gas-limit -1
check "refuses EVM overrides on Bitcoin" $REJECTED spectra send overrides --chain Bitcoin
check "refuses incomplete calldata bytes" $REJECTED \
    spectra send overrides --gas-limit 50000 --calldata 0x0
check "refuses invalid calldata hex" $REJECTED \
    spectra send overrides --gas-limit 50000 --calldata 0xzz
check "custom calldata needs explicit gas" $REJECTED \
    spectra send overrides --calldata 0x0102
contains "keeps calldata bytes" '"calldataBytes":3' \
    spectra --json send overrides --gas-limit 50000 --calldata 0x0102ff
contains "explicit empty calldata stays explicit" '"calldataBytes":0' \
    spectra --json send overrides --gas-limit 21000 --calldata 0x
check "refuses a non-array access list" $REJECTED \
    spectra send overrides --gas-limit 50000 --access-list '{}'
check "refuses a malformed access-list address" $REJECTED \
    spectra send overrides --gas-limit 50000 --access-list '[{"address":"0x11","storageKeys":[]}]'
check "refuses short storage keys" $REJECTED \
    spectra send overrides --gas-limit 50000 --access-list '[{"address":"0x1111111111111111111111111111111111111111","storageKeys":["0x01"]}]'
contains "empty access list needs no custom gas" '"accessListEntries":0' \
    spectra --json send overrides --access-list '[]'
access_list_fixture='[{"address":"0x1111111111111111111111111111111111111111","storageKeys":["0x2222222222222222222222222222222222222222222222222222222222222222"]}]'
contains "keeps access-list entries" '"accessListEntries":1' \
    spectra --json send overrides --gas-limit 50000 --access-list "$access_list_fixture"
contains "keeps access-list storage keys" '"storageKeys":1' \
    spectra --json send overrides --gas-limit 50000 --access-list "$access_list_fixture"
check "non-empty access list needs explicit gas" $REJECTED \
    spectra send overrides --access-list "$access_list_fixture"
contains "keeps sign-only intent" '"signOnly":true' spectra --json send overrides --sign-only

section "custom EVM fees"
contains "returns parsed fees from core" '"maxFeePerGasGwei":30.25' \
    spectra --json send fees --max-fee ' 30.25 ' --priority-fee 1
check "accepts a one-wei priority fee" $OK \
    spectra send fees --max-fee 1 --priority-fee 0.000000001
check "accepts equal max and priority fees" $OK \
    spectra send fees --max-fee 2 --priority-fee 2
check "refuses priority above max" $REJECTED \
    spectra send fees --max-fee 1 --priority-fee 2
for bad_fee in inf NaN -1 0 1e-10 1e100; do
    check "refuses max fee $bad_fee" $REJECTED \
        spectra send fees --max-fee "$bad_fee" --priority-fee 1
    check "refuses priority fee $bad_fee" $REJECTED \
        spectra send fees --max-fee 30 --priority-fee "$bad_fee"
done

section "send affordability"
# The fee half of "can this send land". `route_send_asset` already refuses
# amount > balance; this is the part that was in Swift, where four callers each
# decided whether the asset was the chain's own — spelled `== "TRX"`, `== "SOL"`,
# a literal `true`, and a preflight field.
contains "counts the fee against a native balance" '"verdict":"amountPlusFeeExceedsBalance"' \
    spectra --json send affordability --chain Bitcoin --symbol BTC --amount 1 --fee 0.5 --balance 1.2
contains "and quotes it to the chain's own decimals" '"required":"1.50000000"' \
    spectra --json send affordability --chain Bitcoin --symbol BTC --amount 1 --fee 0.5 --balance 1.2
# Arbitrum charges gas in ETH, not ARB. A caller that took the governance token
# for the native asset would check the fee against the wrong balance.
contains "a governance token is not the gas asset" '"verdict":"feeExceedsGasBalance"' \
    spectra --json send affordability --chain Arbitrum --symbol ARB --amount 1 --fee 0.5 \
        --balance 1.2 --gas-balance 0.1
contains "and the fee is named in what gas is paid in" '"gasSymbol":"ETH"' \
    spectra --json send affordability --chain Arbitrum --symbol ARB --amount 1 --fee 0.5 \
        --balance 1.2 --gas-balance 0.1
contains "a token over its own balance is refused first" '"verdict":"amountExceedsBalance"' \
    spectra --json send affordability --chain Ethereum --symbol USDC --amount 5 --fee 0.5 \
        --balance 1.2 --gas-balance 0.1
contains "and both fitting is affordable" '"verdict":"affordable"' \
    spectra --json send affordability --chain Ethereum --symbol USDC --amount 1 --fee 0.5 \
        --balance 1.2 --gas-balance 2

section "send destination probe"
# The recipient check the composer runs. Swift held it as four chain arms that
# fetched different things and worded the answer three ways; core answers with
# two booleans and the front end supplies the sentence. Only the offline half
# is assertable here — the verdict itself is a balance and a history read.
check "refuses a chain the registry does not know" $USAGE \
    spectra send probe --chain NotAChain --address $EVM_ADDR
check "refuses half a token description"           $USAGE \
    spectra send probe --chain Base --address $EVM_ADDR --contract $EVM_ADDR
check "refuses a token description with no contract" $USAGE \
    spectra send probe --chain Base --address $EVM_ADDR --symbol USDC --decimals 6

# ── Network selection ───────────────────────────────────────────────────────
#
# Which `Chain` of a family the user is on. This had no command until now, and
# that is how "reset to defaults" came to reset three families where the
# registry has twenty-nine — the axis was reachable only from the iOS picker,
# so nothing here could see it.

section "network selection"
check "lists the families that have a choice" $OK spectra network list
contains "and defaults to mainnet"          '"family":"bitcoin","isTestnet":false,"selected":"bitcoin"' \
    spectra --json network list
check "puts a family on a testnet"          $OK spectra network set solana-devnet
contains "and reads it back"                '"selected":"solana-devnet"' \
    spectra --json network list
check "and another, on a different family"  $OK spectra network set bitcoin-signet
check "refuses an id the registry does not know" $REJECTED \
    spectra network set nonsuch
# The bug this axis hid: the reset named bitcoin, ethereum and dogecoin.
check "resetting settings clears every family" $OK spectra settings reset --yes
contains "including the two just moved"     '"family":"solana","isTestnet":false,"selected":"solana"' \
    spectra --json network list
contains "and the other one"                '"family":"bitcoin","isTestnet":false,"selected":"bitcoin"' \
    spectra --json network list

# ── Token discovery ─────────────────────────────────────────────────────────
#
# The complement of the known-token list: ask the chain what an address holds
# rather than asking it about a list the caller already has. Decimals come from
# the chain, an unlisted token still appears, and one call replaces one per
# known token.
#
# Five chains have a node that answers "what does this address hold?" — Solana,
# Tron, Sui, Aptos and TON. The EVM family and NEAR do not: a token contract
# only answers about a holder you name, so listing holdings there needs an
# indexer. Those must say so rather than return an empty list, which would read
# as "holds nothing".

section "token discovery"
# Bitcoin and Ethereum refuse from the registry flag alone, so these assert
# without touching a network. The enumerable chains' paths are real RPC calls
# and cannot be asserted here.
check "refuses a chain with no token program" 1 \
    spectra token discover --wallet "Renamed BTC"
contains "and says so rather than reporting an empty wallet" "cannot enumerate holdings" \
    spectra token discover --wallet "Renamed BTC"

# ── FFI surface ─────────────────────────────────────────────────────────────
#
# An export nothing calls still costs: it is generated into the bindings, it
# has to keep compiling, and it reads as API. Three had been unreachable long
# enough that two of them were only kept alive by their own tests.

section "ffi surface"
check "no export is unreachable from both front ends" $OK \
    "$(cd "$(dirname "$0")" && pwd)/unreachable-exports.sh"

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
# The same keyed shape, for the setting that decides which node a chain talks
# to. It was one `ethereum_rpc_endpoint` string, read through an accessor that
# was `chainName == "Ethereum" ? … : nil`, so twenty-two EVM mainnets could not
# be pointed at a private node from any front end.
check "points a second EVM chain at a private node" $OK \
    spectra settings set rpc-endpoint.Base https://base.internal.example
contains "and reads it back"                '"value":"https://base.internal.example"' \
    spectra --json settings get rpc-endpoint.Base
contains "a chain never set falls back to the catalog" '"value":""' \
    spectra --json settings get rpc-endpoint.Polygon
contains "an empty value clears the override" '"value":""' \
    spectra --json settings set rpc-endpoint.Base ""
check "refuses a chain the registry does not know" $REJECTED \
    spectra settings set rpc-endpoint.Nonsuch https://x.example
# Resetting was iOS-only, and it reset settings by assigning each mirror a
# literal it believed was the default — nineteen of them across two files, none
# checkable against `AppSettings::default()`. It is a command now.
check "refuses to reset without --yes"      $USAGE \
    spectra settings reset
check "resets every setting"                $OK \
    spectra settings reset --yes
contains "a changed number is back at its default" '"value":"10"' \
    spectra --json settings get bitcoin-stop-gap
contains "and a per-chain override is gone" '"value":""' \
    spectra --json settings get rpc-endpoint.Base
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

# ── Tor routing ─────────────────────────────────────────────────────────────
#
# The four Tor settings were `UserDefaults` keys in one front end, so no other
# front end, no test and no script could read or set them. They are settings
# like any other now. The kill switch is enforced in core's HTTP layer, which
# is why the address is validated here rather than handed to a proxy builder
# that fails closed and says nothing.

section "tor routing"
check "turns Tor on"                        $OK spectra settings set tor-enabled true
contains "and a second process reads it back" '"value":"true"' \
    spectra --json settings get tor-enabled
check "selects a custom proxy"              $OK spectra settings set tor-custom-proxy true
check "refuses an address with no scheme"   $REJECTED \
    spectra settings set tor-proxy-address 127.0.0.1:9150
check "refuses a scheme that is not socks5" $REJECTED \
    spectra settings set tor-proxy-address http://127.0.0.1:9150
check "refuses an address with no port"     $REJECTED \
    spectra settings set tor-proxy-address socks5://127.0.0.1
check "refuses port zero"                   $REJECTED \
    spectra settings set tor-proxy-address socks5://127.0.0.1:0
contains "the refusals stored nothing"      '"value":"socks5://127.0.0.1:9150"' \
    spectra --json settings get tor-proxy-address
contains "accepts a remote-DNS proxy"       '"value":"socks5h://10.0.0.2:9050"' \
    spectra --json settings set tor-proxy-address socks5h://10.0.0.2:9050
contains "an empty value restores the default" '"value":"socks5://127.0.0.1:9150"' \
    spectra --json settings set tor-proxy-address ""
check "arms the kill switch"                $OK spectra settings set tor-kill-switch true
# Turned off again so the rest of this run is not behind a kill switch with Tor
# stopped: core refuses outbound requests in exactly that state, which is the
# point of the setting.
check "turns Tor off again"                 $OK spectra settings set tor-enabled false
contains "the switch is still armed"        '"value":"true"' \
    spectra --json settings get tor-kill-switch

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
# Core derives the address from the key on the commit now, the way it already
# did from a seed phrase — neither front end derives an import address itself
# any more. The CLI still asks core the same question before sealing, because
# a refusal after sealing leaves a key stored under an id no wallet references.
SECRETS_BEFORE="$(find "$DATA_DIR/secrets" -type f 2>/dev/null | wc -l | tr -d ' ')"
check "refuses a chain that cannot derive from a key" $REJECTED \
    with_password "correct horse" spectra wallet import --chain Cardano \
        --name "No PK" --private-key-file "$DATA_DIR/pk.hex"
if [[ "$(find "$DATA_DIR/secrets" -type f 2>/dev/null | wc -l | tr -d ' ')" == "$SECRETS_BEFORE" ]]; then
    PASSED=$((PASSED + 1))
    printf '  \033[32m✓\033[0m and seals no key on the way to refusing\n'
else
    FAILED=$((FAILED + 1))
    printf '  \033[31m✗\033[0m and seals no key on the way to refusing\n'
fi
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
contains "a chain that derives says so in the catalog" '"name":"Polygon"' \
    spectra --json chains --filter Polygon
contains "and one that does not says that"    '"privateKeyImport":false' \
    spectra --json chains --filter Solana
check "private-key sender resolves without a seed or derivation path" $OK \
    with_password "correct horse" spectra send identity --from "PK Wallet"
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
contains "the catalog says which chains stake" '"staking":true' \
    spectra --json chains --filter Polkadot
contains "and which do not"                   '"staking":false' \
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
