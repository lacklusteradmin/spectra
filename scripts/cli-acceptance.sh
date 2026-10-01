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
# No external network. State, crypto and validation run offline; the Bitcoin
# pagination and service checks use isolated loopback fixtures. Balance,
# history and send workflows use these local providers; no live chain is needed.
# Core enforces this: `SPECTRA_LOOPBACK_ONLY` refuses and journals every
# request to a non-loopback host, and any journaled request fails the run.
#
# Usage:  scripts/cli-acceptance.sh [path/to/spectra]

set -uo pipefail

BIN="${1:-}"
if [[ -z "$BIN" ]]; then
    cargo build -p spectra_cli --quiet || exit 1
    BIN="$(cd "$(dirname "$0")/.." && pwd)/target/debug/spectra"
fi

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
source "$(dirname "$0")/cli-assertions.sh"

check "transparent transaction stages" 0 python3 "$(dirname "$0")/cli-send-stages.py" "$BIN"


# Shortcuts are floored in core over the exact balance, never through a float.
contains "MAX is the exact balance" '"amount":"0.99999"' spectra --json send shortcut --maximum 0.99999 --decimals 8
contains "half is exact" '"amount":"0.5"' spectra --json send shortcut --maximum 1 --decimals 8 --percentage 50
contains "a percentage floors at precision" '"amount":"0.01234567"' spectra --json send shortcut --maximum 0.123456789 --decimals 8 --percentage 10
check "shortcut refuses percentages over 100" 1 spectra send shortcut --maximum 1 --decimals 8 --percentage 101

section() { printf '\n\033[1m%s\033[0m\n' "$1"; }

# Exit codes are part of the interface: 0 done, 2 the caller asked wrongly,
# 3 core considered it and said no.
readonly OK=0 USAGE=2 REJECTED=3

# One well-formed EVM address, used wherever a check needs a real one.
readonly EVM_ADDR=0x742d35Cc6634C0532925a3b844Bc454e4438f44e

# ── Registry ────────────────────────────────────────────────────────────────

check "assertions reject failed commands and wrong output" $OK bash "$(dirname "$0")/test-cli-assertions.sh"

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

section "TON address integrity"
check "TON valid friendly address" $OK spectra address validate --chain TON EQDKbjIcfM6ezt8KjKJJLshZJJSqX7XOA4ff-W72r5gqPrHF
check "TON refuses arbitrary 48 characters" $REJECTED spectra address validate --chain TON AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
check "TON refuses checksum typo" $REJECTED spectra address validate --chain TON EQDKbjIcfM6ezt8KjKJJLshZJJSqX7XOA4ff-W72r5gqPrHA
check "TON mainnet refuses test-only address" $REJECTED spectra address validate --chain TON kQDKbjIcfM6ezt8KjKJJLshZJJSqX7XOA4ff-W72r5gqPgpP
check "TON testnet accepts test-only address" $OK spectra address validate --chain ton-testnet kQDKbjIcfM6ezt8KjKJJLshZJJSqX7XOA4ff-W72r5gqPgpP

section "chain registry"
check "lists chains"                        $OK spectra chains
contains "resolves a network by name"  '"nativeSymbol":"BTC"' \
    spectra --json chains --filter bitcoin
contains "hides testnets by default"   '"chains":[]' \
    spectra --json chains --filter "bitcoin testnet"

# ── Address validation ──────────────────────────────────────────────────────
#
# The rule that every chain's import address is validated. Both halves matter:
# an invalid address must be refused, and a valid address must come back
# normalised by core rather than as typed.

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
check "accepts a correct EIP-55 checksum"   $OK \
    spectra address validate --chain Ethereum 0x742d35Cc6634C0532925a3b844Bc454e4438f44e
check "refuses a broken EIP-55 checksum"    $REJECTED \
    spectra address validate --chain Ethereum 0x742d35cC6634C0532925a3b844Bc454e4438f44e
check "accepts the unchecksummed lower-case form" $OK \
    spectra address validate --chain Ethereum 0x742d35cc6634c0532925a3b844bc454e4438f44e

# The send composer's QR scanner: a payment URI reduces to a validated address.
section "scanned payment payloads"
contains "reads the address out of a BIP-21 URI" '"address":"bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"' \
    spectra --json send scan --chain Bitcoin 'bitcoin:bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4?amount=0.1&label=Shop'
contains "reads an EIP-681 URI and returns the stored form" '"address":"0x742d35cc6634c0532925a3b844bc454e4438f44e"' \
    spectra --json send scan --chain Ethereum 'ethereum:0x742d35Cc6634C0532925a3b844Bc454e4438f44e@1/transfer?value=1'
check "refuses an address for another chain" $REJECTED \
    spectra send scan --chain Ethereum bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4
check "refuses a payload with no address"    $REJECTED \
    spectra send scan --chain Bitcoin 'https://example.com/pay?to=someone'

# ── Wallet lifecycle ────────────────────────────────────────────────────────

section "wallet lifecycle"
check "creates a wallet"                    $OK \
    spectra wallet new --chain Bitcoin --name "Acceptance BTC"
contains "stores the catalog derivation path" "m/84'/0'/0'/0/0" \
    spectra --json wallet show "Acceptance BTC"
# The five BIP-39 lengths are core's list, and core refuses any other count.
check "creates a wallet at a non-default BIP-39 length" $OK \
    spectra wallet new --chain Bitcoin --name "Eighteen Words" --words 18
contains_exit $USAGE "refuses a length BIP-39 does not define, naming the five" \
    "12, 15, 18, 21 or 24" \
    spectra wallet new --chain Bitcoin --name Bad --words 13
check "imports a known mnemonic"            $OK \
    with_seed "legal winner thank year wave sausage worth useful legal winner thank yellow" \
    spectra wallet import --chain Solana --name "Acceptance SOL"
contains "derives the documented address for that mnemonic" \
    "BLeUXTx9thHGT7VJUtF9vHEmfMDgW1nnKZ9UVer2CoLX" \
    spectra --json wallet show "Acceptance SOL"
# One seed, several chains, in one command. Core derives the addresses, so the
# multi-chain rule — every EVM chain derives from Ethereum's path — lives with
# the registry rather than in a front end.
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
# One verdict decides a seed phrase, and it says which of the two things is
# wrong: words that are in no wordlist are named, and only a phrase built
# entirely of real words is worth checksumming.
contains_exit 3 "names the words that are in no wordlist" "BIP-39 word list: not, a, at" \
    with_seed "not a real seed phrase at all here" \
    spectra wallet import --chain Solana --name Bad
contains_exit 3 "and blames the checksum when the words are real" "checksum" \
    with_seed "legal winner thank year wave sausage worth useful legal winner thank legal" \
    spectra wallet import --chain Solana --name Bad
# The import page asks for neither a length nor a wordlist: core reads both
# from the words, and refuses rather than cuts a phrase longer than one fixed.
readonly ZERO_24="abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art"
contains "a 24-word phrase is read as 24 words"          '"wordCount":24' \
    with_seed "$ZERO_24" spectra --json wallet check-seed
contains "and its wordlist is detected"                   '"language":"en"' \
    with_seed "$ZERO_24" spectra --json wallet check-seed
contains "a Chinese phrase is detected as Chinese"        '"language":"zh-hans"' \
    with_seed "的 的 的 的 的 的 的 的 的 的 的 在" spectra --json wallet check-seed
contains "a typo leaves the length unfinished, not wrong" '"invalidWordCount":1' \
    with_seed "abandon abandn abandon" spectra --json wallet check-seed
contains "a fixed length refuses a longer phrase"         '"error":"Seed phrase must be 12 words."' \
    with_seed "$ZERO_24" spectra --json wallet check-seed --words 12
contains "and more than 24 words is named"                'That is 25 words' \
    with_seed "$ZERO_24 abandon" spectra --json wallet check-seed
check "an unknown wordlist is a usage error"              $USAGE \
    with_seed "$ZERO_24" spectra wallet check-seed --language klingon
contains_exit 3 "an import names an unfinished length"   "13 words" \
    with_seed "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon" \
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
# Cardano (CIP-3) roots on the entropy instead, so the phrase must be read in
# its own wordlist — it used to be assumed English and refused — and the same
# entropy must land on the English phrase's address. Substrate roots on the
# entropy too, and panicked on a phrase valid in both Chinese lists.
contains "imports a Chinese mnemonic for Cardano" \
    "addr1vy8ac7qqy0vtulyl7wntmsxc6wex80gvcyjy33qffrhm7ss7lxrqp" \
    with_seed "的 的 的 的 的 的 的 的 的 的 的 在" \
    spectra wallet import --chain Cardano --name "Chinese ADA"
check "and for Polkadot" $OK \
    with_seed "的 的 的 的 的 的 的 的 的 的 的 在" \
    spectra wallet import --chain Polkadot --name "Chinese DOT"

# Incompatible stored wallet records must refuse loading without changing the bytes.
if command -v sqlite3 >/dev/null 2>&1; then
    stale_row() {
        sqlite3 "$DATA_DIR/spectra.sqlite" "UPDATE wallets SET payload = REPLACE(payload,
            '\"derivationOverrides\":{\"passphrase\":null,\"hmacKey\":null}',
            '\"derivationOverrides\":{\"passphrase\":null,\"mnemonicWordlist\":null,\"hmacKey\":null}')
            WHERE name = 'Chinese';"
    }
    fresh_row() {
        sqlite3 "$DATA_DIR/spectra.sqlite" \
            "UPDATE wallets SET payload = REPLACE(payload, '\"mnemonicWordlist\":null,', '')
             WHERE name = 'Chinese';"
    }
    stale_row
    contains_exit 1 "an incompatible wallet refuses loading" "wallet_load_all decode" spectra wallet list
    fresh_row
    contains "failed loading leaves the stored wallet intact" \
        "Chinese" spectra wallet list
else
    printf '  \033[33m-\033[0m %s\n' "skipped (no sqlite3): undecodable wallet row"
fi
check "renames through the reducer"         $OK \
    spectra wallet rename "Acceptance BTC" "Renamed BTC"
contains "renamed wallet survives reopening" '"name":"Renamed BTC"' spectra --json wallet list
check "refuses an empty name"               $REJECTED \
    spectra wallet rename "Renamed BTC" "   "
check "reports an unknown wallet"           1 spectra wallet show "no such wallet"

# Each malformed metadata case uses a copy; refusal must preserve it exactly.
check "stored metadata strictly requires the current format" 0 python3 - "$BIN" "$DATA_DIR" <<'PYSTORED'
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
contains_exit 3 "names the address it refused" "definitely-not-an-address" \
    spectra wallet watch --chain Solana --address definitely-not-an-address
check "watch-only sender cannot resolve signing identity" $REJECTED \
    spectra send identity --from "Acceptance Watch"
check "refuses to export a watch-only wallet" $REJECTED \
    spectra wallet export "Acceptance Watch" --yes
# The watch-addresses picker in the app is this flag. Ethereum Classic has its
# own address slot inside the EVM family, and every EVM mainnet takes a watch
# address.
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
# EVM membership comes from the registry; display ranks join from chain-ui.toml.
contains "Sepolia exposes core EVM membership" '"isEvm":true' \
    spectra --json chains --testnets --filter "Ethereum Sepolia"
contains "Bitcoin remains non-EVM with separate UI metadata" '"isEvm":false' \
    spectra --json chains --filter Bitcoin
# The picker's order and filters are catalog facts, not lists in a view.
contains "the catalog ranks every chain for the picker" '"popularRank":1' \
    spectra --json chains --filter Bitcoin
contains "including one off the old short list"         '"popularRank":27' \
    spectra --json chains --filter Polygon
contains "a testnet shares its mainnet's rank"          '"popularRank":2' \
    spectra --json chains --testnets --filter "Ethereum Sepolia"
contains "tags carry the derived and authored filters"  '"tags":["layer-1","utxo","pow"]' \
    spectra --json chains --filter Bitcoin
contains "a testnet adds its own tag"                   '"tags":["layer-1","evm","testnet"]' \
    spectra --json chains --testnets --filter "Ethereum Sepolia"
contains "the tag filter lists the Move chains"          '"name":"Aptos"' \
    spectra --json chains --tag move
lacks "and nothing else"                                 '"name":"Bitcoin"' \
    spectra --json chains --tag move
check "an unknown tag is a usage error"                 $USAGE \
    spectra chains --tag sidechain
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
# Settings exposes the complete catalog, including auxiliary APIs.
check "settings includes every Bitcoin endpoint" 0 python3 - "$BIN" "$DATA_DIR" <<'PYSETTINGS'
import json, subprocess, sys
for chain in ("bitcoin", "ethereum", "ethereum-sepolia", "monero"):
    data = json.loads(subprocess.check_output([
        sys.argv[1], "--data-dir", sys.argv[2], "--json", "endpoints", "--catalog", "--chain", chain
    ]))
    group = next(g for g in data["settingsGroups"] if g["chainId"] == chain)
    assert group["endpoints"] == list(dict.fromkeys(r["endpoint"] for r in data["endpoints"]))
PYSETTINGS

# Monero's spend and view keys come from the seed, so its catalog row carries
# `derivation_path = []`. "No default path" is an answer, not an error.
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

section "endpoint APIs and capabilities"
check "API routing excludes incompatible wire formats" 0 python3 - "$BIN" "$DATA_DIR" <<'PYAPI'
import json, subprocess, sys
binary, directory = sys.argv[1:]
catalog = json.loads(subprocess.check_output([binary, "--data-dir", directory, "--json", "endpoints", "--catalog"]))
records = catalog["endpoints"]
assert all("kind" not in r for r in records)
assert all(r["api"] is not None or not r["capabilities"] for r in records)
configured = {r["chainId"]: r["endpoints"] for r in catalog["configured"]}
btc = configured["bitcoin"]
# Bitcoin speaks Esplora and BlockCypher alike, and uses both.
assert btc and all(any(r["endpoint"] == url and r["api"] in ("esplora", "blockcypher") for r in records) for url in btc)
assert "https://api.blockcypher.com/v1/btc/main" in btc
assert "https://blockchain.info/multiaddr" not in btc
assert configured["ton"] == ["https://toncenter.com/api/v2"]
# A transport list holds every API a chain's own client speaks; secondary
# services are found by API.
assert all(":" not in chain for chain in configured)
assert [r["endpoint"] for r in records if r["chainId"] == "ton" and r["api"] == "toncenter-v3"] == ["https://toncenter.com/api/v3"]
assert configured["tron"] == ["https://api.trongrid.io", "https://tron-rpc.publicnode.com"]
assert not any(r["api"] in ("taostats", "subscan", "ethplorer") for r in records)
# Litecoin speaks Blockbook, Esplora and BlockCypher alike, and uses all three.
assert configured["litecoin"] == ["https://litecoinspace.org/api", "https://api.blockcypher.com/v1/ltc/main", "https://blockbook.ltc.zelcore.io"]
assert configured["bitcoin-cash"] == ["https://rest.bch.actorforth.org/v2", "https://blockbook.bch.zelcore.io"]
assert configured["monero"]
assert all(any(r["endpoint"] == url and r["api"] == "monero-daemon-rpc" for r in records) for url in configured["monero"])
PYAPI
lacks "endpoint catalog omits unused provider metadata" '"providerID"' \
    spectra --json endpoints --catalog
# An endpoint declares what it is (its API) separately from what it is used
# for (its capabilities). No EVM node serves `history`, because
# `eth_getTransactionsByAddress` is not a method.
contains "an EVM node declares its API"        '"api":"evm-json-rpc"' \
    spectra --json endpoints --catalog --chain Ethereum
contains "and does not claim address history" '"capabilities":["balance","fee","broadcast","token-balance","verification"]' \
    spectra --json endpoints --catalog --chain Ethereum

lacks "the old native-history capability is gone" '"native-history"' \
    spectra --json endpoints --catalog
contains "Bitcoin exposes native history" '"history"' \
    spectra --json endpoints --catalog --chain Bitcoin
lacks "Bitcoin does not claim token balances" '"token-balance"' \
    spectra --json endpoints --catalog --chain Bitcoin
contains "an explorer lists history, transfers and holdings" '"capabilities":["history","token-discovery","token-history"]' \
    spectra --json endpoints --catalog --chain Ethereum
contains "Solana nodes enumerate tokens and expose token transfers" '"capabilities":["balance","history","fee","broadcast","token-balance","token-discovery","token-history","verification","staking"]' \
    spectra --json endpoints --catalog --chain Solana
contains "TON v2 only claims native history" '"capabilities":["balance","history","fee","broadcast","verification","token-balance"]' \
    spectra --json endpoints --catalog --chain TON
contains "TON v3 claims only what its adapter reads: jetton balances and discovery" '"capabilities":["token-balance","token-discovery"]' \
    spectra --json endpoints --catalog --chain TON

section "transaction explorers"
# Explorer pages are links, not endpoints: they live in explorers.toml, and the
# endpoint catalog is APIs only.
check "every catalog endpoint declares an API and a use" 0 python3 - "$BIN" "$DATA_DIR" <<'PYEXPLORERS'
import json, subprocess, sys
binary, directory = sys.argv[1:]
run = lambda *a: json.loads(subprocess.check_output([binary, "--data-dir", directory, "--json", *a]))
records = run("endpoints", "--catalog")["endpoints"]
assert all(r["api"] and r["capabilities"] for r in records)
explorers = run("explorers")["explorers"]
assert len({e["chainId"] for e in explorers}) == len(explorers)
assert all(e["txUrl"].startswith("https://") and e["txUrl"].count("{hash}") == 1 for e in explorers)
pages = {e["txUrl"].split("{hash}")[0] for e in explorers}
assert not any(r["endpoint"].startswith(page) for r in records for page in pages)
PYEXPLORERS
contains "an explorer link puts the hash where its page wants it" \
    '"url":"https://explorer.aptoslabs.com/txn/0xabc?network=mainnet"' \
    spectra --json explorers --chain Aptos --tx 0xabc
contains "and names the explorer" '"name":"Etherscan"' \
    spectra --json explorers --chain Ethereum
contains "a testnet links to its own network's explorer" \
    '"url":"https://sepolia.etherscan.io/tx/0xabc"' \
    spectra --json explorers --chain ethereum-sepolia --tx 0xabc
contains "Solana Devnet links to Solscan's devnet cluster" \
    '"url":"https://solscan.io/tx/abc?cluster=devnet"' \
    spectra --json explorers --chain solana-devnet --tx abc
check "a network without an explorer has no link" 1 \
    spectra --json explorers --chain kaspa-testnet --tx abc
check "a blank hash is refused" $USAGE \
    spectra --json explorers --chain Ethereum --tx " "
check "--tx needs a chain" $USAGE \
    spectra --json explorers --tx abc

check "an unknown capability is refused before it is saved" $USAGE \
    spectra endpoints --chain base --api evm-json-rpc --capabilities native-history --add https://base.example

contains "donation addresses come from core's validated catalog" \
    '"address":"0xefa039ed09c3fe6aeceb89b365b2740e4050365c","chainId":"ethereum"' \
    spectra --json donations

section "endpoint network identity"
contains "Sepolia endpoints carry their concrete network ID" '"chainId":"ethereum-sepolia"' \
    spectra --json endpoints --catalog --chain ethereum-sepolia
lacks "Sepolia never returns mainnet ownership" '"chainId":"ethereum"' \
    spectra --json endpoints --catalog --chain ethereum-sepolia
lacks "Ethereum does not absorb testnet endpoints" 'ethereum-sepolia' \
    spectra --json endpoints --catalog --chain ethereum
contains "Bitcoin Testnet4 is independent of its display title" '"chainId":"bitcoin-testnet-4"' \
    spectra --json endpoints --catalog --chain bitcoin-testnet-4
lacks "catalog identity carries no grouping title" '"groupTitle"' \
    spectra --json endpoints --catalog
check "unknown endpoint network is refused" $USAGE \
    spectra --json endpoints --catalog --chain unknown-network

section "keyless provider policy"
# Substrate balances are the node's own `System.Account` storage; reading one
# needs the network, so offline the check is that the catalog declares it.
# History has no keyless source and refuses before any request.
for chain in Polkadot Bittensor; do
    check "$chain offline wallet creation" $OK spectra wallet new --chain "$chain" --name "Keyless $chain" --no-password
    contains "$chain nodes declare balance" '"balance"' spectra --json endpoints --catalog --chain "$chain"
    contains_exit 1 "$chain history has no source" "no keyless history source" spectra history "Keyless $chain"
    check "$chain fixture wallet cleanup" $OK spectra wallet delete "Keyless $chain" --yes
done
contains_exit 3 "Cardano staking refuses without network access" "Staking queries are unavailable for Cardano" spectra staking validators --chain Cardano
contains "Cardano has no staking query implementation" '"staking":false' spectra --json chains --filter Cardano

section "evm history source"
# No API-key provider is configured; missing history is explicit.
for chain in "BNB Chain" Sonic opBNB Sei Linea Hyperliquid Cronos "X Layer"; do
    contains "$chain has no history source" '"historySource":"none"' \
        spectra --json chains --filter "$chain"
done
contains "Ethereum retains its keyless history source" 'https://eth.blockscout.com' \
    spectra --json chains --filter Ethereum

section "utxo address discovery"
# The derive-and-probe walk the app runs on every UTXO refresh. Core reads the
# seed, the derivation path, the keypool bound, the balance and the history.
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
# A wallet whose material is stored without a password, in the same key layout
# core uses for sealed wallets.
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
# Nor is a blank password the choice of none: it asks for a seal it cannot
# make. Refused before anything is stored, rather than stored in the clear.
SECRETS_BEFORE="$(find "$DATA_DIR/secrets" -type f 2>/dev/null | wc -l | tr -d ' ')"
check "a whitespace-only password is refused" $REJECTED \
    with_password "   " with_seed "legal winner thank year wave sausage worth useful legal winner thank yellow" \
    spectra wallet import --chain Solana --name "Blank SOL"
check "and so is an empty password file"    $REJECTED \
    spectra wallet new --chain Solana --name "Blank SOL" --password-file /dev/null
lacks "and neither stored a wallet"         '"Blank SOL"' \
    spectra --json wallet list
if [[ "$(find "$DATA_DIR/secrets" -type f 2>/dev/null | wc -l | tr -d ' ')" == "$SECRETS_BEFORE" ]]; then
    PASSED=$((PASSED + 1))
    printf '  \033[32m✓\033[0m and stored no secret\n'
else
    FAILED=$((FAILED + 1))
    printf '  \033[31m✗\033[0m and stored no secret\n'
fi
check "deletes the unsealed wallet"         $OK \
    spectra wallet delete "Open SOL" --yes

# ── Addresses per network ───────────────────────────────────────────────────
#
# A wallet on a family with testnets holds one address per network, derived
# once at import, so a password-sealed wallet — which has no seed to read —
# still shows the right address on a testnet.

section "addresses per network"
contains "a Bitcoin wallet stores its testnet4 address too" '"bitcoin-testnet-4"' \
    spectra --json wallet show "Multi 1"
contains "and its signet one" '"bitcoin-signet"' spectra --json wallet show "Multi 1"
contains "the mainnet address is the primary" 'bc1q' spectra --json wallet show "Multi 1"
# One key, two encodings: a testnet address is not the mainnet one.
check "the testnet address differs from the mainnet address" $OK \
    bash -c '"$1" --data-dir "$2" --json wallet show "Multi 1" | grep -q "tb1"' _ "$BIN" "$DATA_DIR"
# The EVM family shares one address, and Ethereum Classic has a slot of its own
# holding the same key — so an Ethereum wallet answers on both.
contains "an EVM wallet fills the Ethereum Classic slot too" '"ethereum-classic"' \
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
# One producer of receive addresses, and it is core. `wallet receive` printed
# the wallet's stored account address — the record's index-0 one — while the
# app's receive screen asked core, so the two named different addresses under
# the same word and the CLI's was one core does not watch.
contains "hands out the reserved address, not the stored one" \
    '"address":"bc1qnjg0jd8228aq7egyzacy8cys3knf9xvrerkf9g"' \
    spectra --json wallet receive "Open BTC"
lacks "and never the index-0 address the wallet record holds" \
    "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu" \
    spectra --json wallet receive "Open BTC"
# Idempotent: opening the receive screen twice must not walk the keypool
# forward and leave the address already shown unwatched.
contains "asking again keeps the reserved index" \
    '"address":"bc1qnjg0jd8228aq7egyzacy8cys3knf9xvrerkf9g"' \
    spectra --json wallet receive "Open BTC"
# The diagnostics row reports the reservation as it was recorded when handed
# out.
contains "shows the path at the reserved index" "/0/1\"" \
    spectra --json pool show "Open BTC"
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
# The setting the app reads from the same store.

section "display currency"
contains "defaults to USD"  '"currency":"USD"' spectra --json currency
check "sets a currency"                     $OK spectra currency CHF
contains "reads it back from the store" '"currency":"CHF"' spectra --json currency
# The twelve codes are core's, and a code nothing quotes is refused.
check "refuses a code nothing quotes"       $REJECTED spectra currency ZZZ
check "and one that is not a code at all"   $REJECTED spectra currency bitcoin
contains "the refusal changed nothing"  '"currency":"CHF"' spectra --json currency
# Cross-rates are core state: this reads the same store the app does. Fetching
# them needs network, so what is offline is the empty answer.
contains "no rates stored until one is fetched" '"count":0' \
    spectra --json currency --rates

# ── Price alerts ────────────────────────────────────────────────────────────
#
# The rules live in `CoreAppState`. Every check here is a separate process, so
# this is also the persistence test.

section "price alerts"
check "adds an alert"                       $OK \
    spectra alert add --chain Bitcoin --target 1 --above
# Core names the reason as a code and the front end words it.
contains_exit $REJECTED "refuses an alert that cannot fire, saying why" "positive number" \
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
# engine refuses an empty run rather than reporting a successful no-op.

section "refresh"
# Against its *own* empty directory: by this point the shared one has wallets,
# and `spectra refresh` there would sweep them over the network — which this
# script promises not to do. The first version of this check did exactly that.
check "refuses a refresh with no wallets"   $REJECTED \
    "$BIN" --data-dir "$(mktemp -d)" refresh

# ── Diagnostics ─────────────────────────────────────────────────────────────
#
# Core's own self-tests, which need no network and no device.

section "diagnostics"
check "every chain's self-tests pass"       $OK spectra diagnostics self-test
contains "reports a check count"      '"failed":0' \
    spectra --json diagnostics self-test
check "self-tests one chain"                $OK spectra diagnostics self-test --chain Bitcoin
check "refuses self-tests for an unknown chain" $USAGE \
    spectra diagnostics self-test --chain Nope
contains "builds a diagnostics document"  '"endpoints"' \
    spectra diagnostics show --chain Bitcoin
contains "the document names the network it describes" '"network":"bitcoin"' \
    spectra --json diagnostics show --chain Bitcoin
contains "core builds the diagnostics bundle" '"chainDiagnosticsJson"' \
    spectra diagnostics bundle
contains "the bundle's header counts core's wallets" '"walletCount"' \
    spectra diagnostics bundle

# ── Known tokens ────────────────────────────────────────────────────────────
#
# A token cannot display more places than it has, and the list has to survive a
# reopen — every command here is a separate process, so this section is also
# the persistence test.

section "token and deployment references"
contains "Ethereum deployment resolves its token identity" '"token_id":"ethereum"' \
    spectra --json token catalog --chain ethereum
contains "Arbitrum deployment shares the Ethereum token identity" '"token_id":"ethereum"' \
    spectra --json token catalog --chain arbitrum
contains "Sepolia deployment resolves a separate testnet token" '"token_id":"ethereum-sepolia"' \
    spectra --json token catalog --chain ethereum-sepolia
contains "Sepolia keeps its derived deployment ID" '"deployment_id":"ethereum-sepolia:native"' \
    spectra --json token catalog --chain ethereum-sepolia
contains "testnet token has no market identity" '"coingecko_id":""' \
    spectra --json token catalog --chain ethereum-sepolia
contains "Bitcoin Testnet4 resolves from the flat testnet tables" '"deployment_id":"bitcoin-testnet-4:native"' \
    spectra --json token catalog --chain bitcoin-testnet-4
contains "a testnet coin is named as a test coin" '"name":"Test Bitcoin"' \
    spectra --json token catalog --chain bitcoin-testnet-4
contains "and so is a testnet token"              '"name":"Test USD Coin"' \
    spectra --json token catalog --chain ethereum-sepolia

section "known tokens"
check "editable price sources survive reopening" $OK python3 "$(dirname "$0")/cli-token-preferences.py" "$BIN"
# Every catalog token is known: opening the store seeds the catalog's rows, and
# there is no switch to turn one off.
contains "lists the built-in catalog" '"symbol":"USDC"' \
    spectra --json token catalog --chain Ethereum
contains "opening seeds the catalog's own rows" '"symbol":"USDC"' \
    spectra --json token list
check "has no switch to stop tracking one"         $USAGE \
    spectra token untrack --chain Ethereum USDC

section "custom tokens"
# Every rule is the reducer's: the symbol, the contract judged by the chain that
# would host it, the duplicate, and the precision. The composer held all four
# and this command held none of them, so a Solana mint went into the Base list.
contains "adds one, upper-casing the symbol" '"symbol":"MOON"' \
    spectra --json token add --chain Base --symbol " moon " --name "Moon Coin" \
        --contract $EVM_ADDR --decimals 18
check "refuses the same contract in another case"  $REJECTED \
    spectra token add --chain Base --symbol SUN --name Sun \
        --contract 0x742D35CC6634C0532925A3B844BC454E4438F44E --decimals 18
check "refuses a contract from another family"     $REJECTED \
    spectra token add --chain Base --symbol SOL2 --name Sol \
        --contract EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v --decimals 6
check "and the other way round"                    $REJECTED \
    spectra token add --chain Solana --symbol EVM2 --name Evm \
        --contract $EVM_ADDR --decimals 6
check "refuses a pasted name as a symbol"          $REJECTED \
    spectra token add --chain Base --symbol "Moonbeam Network Token" --name Moon \
        --contract 0x1111111111111111111111111111111111111111 --decimals 18
check "refuses a precision no token has"           $REJECTED \
    spectra token add --chain Base --symbol DEEP --name Deep \
        --contract 0x1111111111111111111111111111111111111111 --decimals 31
check "refuses a chain that hosts no tokens"       $REJECTED \
    spectra token add --chain Bitcoin --symbol NOPE --name Nope \
        --contract $EVM_ADDR --decimals 8
check "rescales the one it accepted"               $OK \
    spectra token decimals --chain Base --contract $EVM_ADDR --decimals 9
check "but not past what a token has"              $REJECTED \
    spectra token decimals --chain Base --contract $EVM_ADDR --decimals 31
check "removes it"                                 $OK \
    spectra token remove --chain Base --contract $EVM_ADDR
check "and will not remove it twice"               $REJECTED \
    spectra token remove --chain Base --contract $EVM_ADDR
check "will not reset without --yes"               $USAGE spectra token reset
check "resets to the catalog"                      $OK spectra token reset --yes

# ── Amount display ──────────────────────────────────────────────────────────
#
# Decimal places follow the amount, not a per-chain setting: six significant
# digits, counted from the first non-zero digit, capped by what the asset has
# and by eight places.

section "amount display"
contains "a small balance keeps its digits" '"shows":"0.00042"' \
    spectra --json token format 0.00042 --chain Bitcoin
contains "and is not marked as below a threshold" '"belowThreshold":false' \
    spectra --json token format 0.00042 --chain Bitcoin
contains "an eighteen-decimal chain does the same" '"shows":"0.000015"' \
    spectra --json token format 0.000015 --chain Ethereum
contains "a large balance spends its budget on the integer, cut rather than rounded up" '"shows":"1234.56"' \
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
# Offline half only: "which chains stake" is the part worth asserting without
# a network.

# ── EVM send assembly ───────────────────────────────────────────────────────
#
# `prepare_evm_send_assembly` builds the transaction the send sheet estimates
# gas against. Assembling takes no key, no network and no store, so it belongs
# here.

# ── Signing without broadcasting ────────────────────────────────────────────
#
# The send path's last unproven step is the broadcast itself. `--sign-only`
# runs everything before it — stored identity, amount, fees, live nonce, the
# built and signed payload and stops. The loopback staged-send suite and
# multi-protocol core audit cover signing without any submission.
section "sign without broadcasting"
contains "Solana exposes separate signing capability" '"supportsSeparateSigning":true' spectra --json chains --filter solana
# A broadcast still takes --yes; signing does not, because it moves nothing.
check "a broadcast without --yes is refused" $USAGE \
    spectra send broadcast --from "Multi 1" --to bc1qgkju4yvvtuz0s8vqn837q396jezu2h8ex7gk98 --amount 0.001

section "EVM send assembly"
# Every EVM mainnet assembles, Base included.
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
# The asset is its contract, not its ticker: a token calling itself ETH is
# still an ERC-20 transfer to that contract, never a value transfer of ETH.
contains "a borrowed gas ticker does not make a token native" '"isNative":false' \
    spectra --json send assemble --chain Ethereum --from $EVM_ADDR --to $EVM_ADDR \
        --amount 1 --symbol ETH \
        --contract 0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48 --decimals 6
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
# `cargo test -p spectra_core replaceable`.
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

section "custom EVM fees"
contains "returns parsed fees from core" '"maxFeePerGasGwei":"30.25"' \
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
# The fee half of "can this send land". The send preflight already refuses
# amount > balance; this counts the fee against the chain's own asset.
contains "counts the fee against a native balance" '"verdict":"amountPlusFeeExceedsBalance"' \
    spectra --json send affordability --chain Bitcoin --symbol BTC --amount 1 --fee 0.5 --balance 1.2
contains "and states the exact total required" '"required":"1.5"' \
    spectra --json send affordability --chain Bitcoin --symbol BTC --amount 1 --fee 0.5 --balance 1.2
# Arbitrum charges gas in ETH, not ARB. A caller that took the governance token
# for the native asset would check the fee against the wrong balance.
contains "a governance token is not the gas asset" '"verdict":"feeExceedsGasBalance"' \
    spectra --json send affordability --chain Arbitrum --symbol ARB --deployment arbitrum:erc-20:0x912ce59144191c1204e64559fe8253a0e49e6548 --amount 1 --fee 0.5 \
        --balance 1.2 --gas-balance 0.1
contains "and the fee is named in what gas is paid in" '"gasSymbol":"ETH"' \
    spectra --json send affordability --chain Arbitrum --symbol ARB --deployment arbitrum:erc-20:0x912ce59144191c1204e64559fe8253a0e49e6548 --amount 1 --fee 0.5 \
        --balance 1.2 --gas-balance 0.1
contains "a token over its own balance is refused first" '"verdict":"amountExceedsBalance"' \
    spectra --json send affordability --chain Ethereum --symbol USDC --deployment ethereum:erc-20:0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48 --amount 5 --fee 0.5 \
        --balance 1.2 --gas-balance 0.1
contains "and both fitting is affordable" '"verdict":"affordable"' \
    spectra --json send affordability --chain Ethereum --symbol USDC --deployment ethereum:erc-20:0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48 --amount 1 --fee 0.5 \
        --balance 1.2 --gas-balance 2

section "send destination probe"
# The recipient check the composer runs: core answers with two booleans and the
# front end supplies the sentence.
#
# Named by wallet and asset, not by a token descriptor the caller builds: which
# contract an asset is on a chain is a catalog question. Only the offline half
# is assertable here — the verdict itself is a balance and a history read. An
# import holds its network's native asset from the start, so the refusal names
# a token.
check "refuses a wallet that is not there"         1 \
    spectra send probe --wallet "no such wallet" --to $EVM_ADDR
check "refuses an asset the wallet does not hold"  $REJECTED \
    spectra send probe --wallet "Multi 2" --asset USDC --to $EVM_ADDR
check "refuses a chain the registry does not know" $USAGE \
    spectra send probe --wallet "Multi 2" --asset ETH --chain NotAChain --to $EVM_ADDR

section "send destination resolution"
# What the composer does with the destination field. Whether a `.eth` name is
# looked up is `Chain::resolves_ens_names`, so the refusals are assertable
# offline — a name off Ethereum never reaches the network.
contains "a typed address comes back in the chain's own form" \
    '"address":"0x742d35cc6634c0532925a3b844bc454e4438f44e"' \
    spectra --json send destination --chain Base --to $EVM_ADDR
contains "a typed address is not a name lookup" '"usedEns":false' \
    spectra --json send destination --chain Bitcoin \
        --to bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq
check "refuses an empty destination"                 $REJECTED \
    spectra send destination --chain Ethereum --to "   "
check "refuses an address from another family"       $REJECTED \
    spectra send destination --chain Bitcoin --to $EVM_ADDR
for ens_chain in Arbitrum Base Polygon Bitcoin; do
    check "refuses a .eth name on $ens_chain"        $REJECTED \
        spectra send destination --chain $ens_chain --to vitalik.eth
done

# ── Network selection ───────────────────────────────────────────────────────
#
# Which `Chain` of a family the user is on, for every family the registry
# offers a choice in.

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
# A switch has to take the family's derivation state with it: reserved keypool
# indices and discovered addresses belong to the network they were derived on.
contains "clears the family's derivation state with the switch" \
    '"clearedDerivationState":["solana","solana-devnet"]' \
    spectra --json network set solana-devnet
contains "and names both sides of a bitcoin switch" '"clearedDerivationState":[' \
    spectra --json network set bitcoin-signet
check "clearing a chain nothing derived on still succeeds" $OK \
    spectra network set solana
# The bug this axis hid: the reset named bitcoin, ethereum and dogecoin.
check "resetting settings clears every family" $OK spectra settings reset --yes
contains "including the two just moved"     '"family":"solana","isTestnet":false,"selected":"solana"' \
    spectra --json network list
contains "and the other one"                '"family":"bitcoin","isTestnet":false,"selected":"bitcoin"' \
    spectra --json network list

section "testnet derivation identity"
check "selects Bitcoin Testnet4 for import" $OK spectra network set bitcoin-testnet-4
check "imports a testnet wallet with network-local paths" $OK \
    with_seed "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about" \
    spectra wallet import --chain Bitcoin --name "Testnet Paths"
contains "stored testnet path uses coin type one" "m/84'/1'/0'/0/0" \
    spectra --json wallet show "Testnet Paths"
contains "wallet summary shows the active testnet address" '"address":"tb1' \
    spectra --json wallet show "Testnet Paths"
check "reopened testnet wallet resolves its signer" $OK \
    spectra send identity --from "Testnet Paths" --chain bitcoin-testnet-4
check "switches the wallet back to mainnet" $OK spectra network set bitcoin
contains "mainnet keeps its own path" "m/84'/0'/0'/0/0" \
    spectra --json wallet show "Testnet Paths"
check "mainnet signer still matches after the switch" $OK \
    spectra send identity --from "Testnet Paths" --chain bitcoin
check "cleans up testnet derivation wallet" $OK spectra wallet delete "Testnet Paths" --yes

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
contains_exit 1 "and says so rather than reporting an empty wallet" "cannot enumerate holdings" \
    spectra token discover --wallet "Renamed BTC"

# ── Dead weight ─────────────────────────────────────────────────────────────
#
# An export nothing calls still costs: it is generated into the bindings, it
# has to keep compiling, and it reads as API.
#
# Below the FFI surface `dead_code` treats a `pub fn` in a lib crate as API and
# never fires, so a function can lose its last caller and keep compiling.
# Shipped copy is the third shape — `resources/` ships whether or not anything
# reads it.

section "dead weight"
check "no export is unreachable from both front ends" $OK \
    "$(cd "$(dirname "$0")" && pwd)/unreachable-exports.sh"
check "no public core function is uncalled" $OK \
    "$(cd "$(dirname "$0")" && pwd)/uncalled-core-fns.sh"
check "no shipped string is unread" $OK \
    "$(cd "$(dirname "$0")" && pwd)/unused-strings.sh"
check "the app names no chain by spelling and fixes no amount precision" $OK \
    "$(cd "$(dirname "$0")" && pwd)/swift-shell-literals.sh"

section "settings"
check "backend key setting is removed" $REJECTED spectra settings set monero-backend-api-key KEY
check "Etherscan key setting is removed" $REJECTED spectra settings set etherscan-api-key KEY
check "lists the settings core owns"        $OK spectra settings list
check "automatic refresh has no manual interval setting" $REJECTED \
    spectra settings set refresh-frequency-minutes 30
check "adds a typed custom endpoint" $OK \
    spectra endpoints --chain monero --api monero-daemon-rpc --capabilities fee,broadcast,verification --add https://wallet.example
contains "a second process reads the custom endpoint" '"endpoint":"https://wallet.example"' \
    spectra --json endpoints --catalog --source custom --chain monero
# Monero scans and signs on device; there is no wallet-RPC adapter to point at.
check "Monero wallet RPC is not an endpoint API" $REJECTED \
    spectra endpoints --chain monero --api monero-wallet-rpc --capabilities fee --add https://wallet-rpc.example
# Fee priority was stored per chain and spent by no send path; it is gone
# rather than kept as a choice that changes nothing.
check "fee priority is not a setting"      $REJECTED \
    spectra settings set fee-priority.Dogecoin economy
# Custom nodes are keyed by chain, so every EVM network can have its own. It was one `ethereum_rpc_endpoint` string, read through an accessor that
# was `chainName == "Ethereum" ? … : nil`, so twenty-two EVM mainnets could not
# be pointed at a private node from any front end.
check "adds an EVM endpoint" $OK \
    spectra endpoints --chain base --api evm-json-rpc --capabilities balance,fee,broadcast,verification,token-balance --add https://base.internal.example
check "rejects an incompatible API" $REJECTED \
    spectra endpoints --chain base --api esplora --capabilities balance,utxo,fee,broadcast,verification --add https://wrong.example
check "rejects a malformed endpoint" $REJECTED \
    spectra endpoints --chain bitcoin --api esplora --capabilities balance,utxo,fee,broadcast,verification --add "https://a.example,nope"
check "rejects a duplicate endpoint" $REJECTED \
    spectra endpoints --chain base --api evm-json-rpc --capabilities balance,fee,broadcast,verification,token-balance --add https://base.internal.example

# checkable against `AppSettings::default()`. It is a command now.
check "refuses to reset without --yes"      $USAGE \
    spectra settings reset
check "resets every setting"                $OK \
    spectra settings reset --yes
contains "a changed number is back at its default" '"value":"10"' \
    spectra --json settings get bitcoin-stop-gap
contains "custom endpoints are reset" '"total":0' \
    spectra --json endpoints --catalog --source custom
# The bound is core's. A stop gap of zero finds no addresses.
check "bounds a number instead of storing it" $OK \
    spectra settings set bitcoin-stop-gap 9999
contains "clamped to the top of the range"  '"value":"200"' \
    spectra --json settings get bitcoin-stop-gap
contains "trims a pasted endpoint" '"endpoint":"https://wallet.example"' \
    spectra --json endpoints --chain monero --api monero-daemon-rpc --capabilities fee,broadcast,verification --add "  https://wallet.example  "
check "refuses a setting that does not exist" $REJECTED spectra settings set nope 1
check "refuses a value of the wrong kind"   $REJECTED \
    spectra settings set price-alerts maybe

# ── Tor routing ─────────────────────────────────────────────────────────────
#
# The Tor settings are core settings like any other. The kill switch is
# enforced in core's HTTP layer, which is why the address is validated here
# rather than handed to a proxy builder that fails closed and says nothing.

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
# private-key derivation by chain since `derive_from_private_key`; what
# was missing was the command.
printf '4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318\n' > "$DATA_DIR/pk.hex"
check "imports a wallet from a private key"  $OK \
    with_password "correct horse" spectra wallet import --chain Ethereum \
        --name "PK Wallet" --private-key-file "$DATA_DIR/pk.hex"
contains "and derives the right address"     '0x2c7536e3605d9c16a7a3d7b1898e529396a65c23' \
    spectra --json wallet show "PK Wallet"
check "refuses a key sealed under a blank password" $REJECTED \
    with_password "   " spectra wallet import --chain Polygon \
        --name "Blank PK" --private-key-file "$DATA_DIR/pk.hex"
lacks "and stores no wallet for it"          '"Blank PK"' \
    spectra --json wallet list
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
# Which chains a private key covers is one registry fact, so the app's picker
# and the CLI cannot disagree about it. Polygon derives the same EVM address as
# Ethereum, and Decred derives too.
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
# Self-tests are keyed by the registry id every caller resolves its input to.
contains "runs a chain's self-tests"       '"chain":"xrp"' \
    spectra --json diagnostics self-test --chain "XRP Ledger"
contains "and the symbol resolves to it"   '"chain":"xrp"' \
    spectra --json diagnostics self-test --chain XRP
contains "with no failures"                '"failed":0' \
    spectra --json diagnostics self-test --chain "XRP Ledger"

section "staking"
contains_exit 3 "and says which chain, not which endpoint" "Staking queries are unavailable for Bitcoin" \
    spectra staking validators --chain Bitcoin
check "refuses staking on an unknown chain"            $USAGE \
    spectra staking validators --chain Nope
# Which chains stake is one registry column, and this is the column.
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

section "reviewed destinations and Aptos derivation"
check "accepts the reviewed destination" $OK spectra send destination --chain Ethereum --to 0x1111111111111111111111111111111111111111 --expected 0x1111111111111111111111111111111111111111
check "refuses a destination changed since review" $REJECTED spectra send destination --chain Ethereum --to 0x2222222222222222222222222222222222222222 --expected 0x1111111111111111111111111111111111111111
contains "Aptos address matches the official SDK" "0xeb663b681209e7087d681c5d3eed12aaa8e1915e7c87794542c3f96e94b3d3bf" with_seed "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about" spectra --json wallet import --chain Aptos --name "Audit Aptos"
check "removes the audit wallet" $OK spectra wallet delete "Audit Aptos" --yes

contains "Sui address matches the official SDK" "0x5e93a736d04fbb25737aa40bee40171ef79f65fae833749e3c089fe7cc2161f1" with_seed "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about" spectra --json wallet import --chain Sui --name "Audit Sui"
check "removes the Sui audit wallet" $OK spectra wallet delete "Audit Sui" --yes

section "manual status recheck"
check "recheck rejects a missing transaction" $REJECTED spectra txs --recheck missing
check "recheck rejects conflicting scope" $USAGE spectra txs --recheck missing --refresh-pending
check "recheck endpoint requires a transaction" $USAGE spectra txs --endpoint http://127.0.0.1:1

section "owned pending maintenance"
contains "empty maintenance completes without network" '"chains":[]' spectra --json txs --refresh-pending
check "maintenance rejects conflicting scope" $USAGE spectra txs --refresh-pending --poll-chain Ethereum

section "Dashboard, receive and reset"
closure_spectra() { "$BIN" --data-dir "$DATA_DIR/closure" "$@"; }
check "dashboard groups render from stored state" $OK closure_spectra --json portfolio --stored
# A pinned asset the user holds none of is built from the catalog rather than
# read from a wallet, so it never passes the canonicalize every stored holding
# does. Every holding, held or not, names its chain by registry id; display
# names are the front end's to render.
contains "a pinned asset nobody holds names its chain like a stored one" \
    '"chainId":"bitcoin"' closure_spectra --json portfolio --stored
lacks "and never by display name" \
    '"chainName"' closure_spectra --json portfolio --stored
# And it holds nothing, rather than a synthesized zero holding on some chain.
contains "and holds nothing at all" \
    '"holdings":[],"id":"bitcoin"' closure_spectra --json portfolio --stored
# Which assets a fresh dashboard pins is core's rule. Core answers per option,
# counting the default set.
check "a fresh dashboard pins bitcoin by default" $OK \
    bash -c '"$1" --data-dir "$2/closure" --json portfolio --pin-options | grep -q "\"is_pinned\":true,[^}]*\"token_id\":\"bitcoin\""' _ "$BIN" "$DATA_DIR"
check "unpinning one asset keeps the rest of the default set" $OK \
    bash -c '"$1" --data-dir "$2/closure" --json portfolio --unpin-token bitcoin --pin-options | grep -q "\"is_pinned\":true,[^}]*\"token_id\":\"ethereum\""' _ "$BIN" "$DATA_DIR"
check "and bitcoin is no longer pinned" $OK \
    bash -c '"$1" --data-dir "$2/closure" --json portfolio --pin-options | grep -q "\"is_pinned\":false,[^}]*\"token_id\":\"bitcoin\""' _ "$BIN" "$DATA_DIR"
check "all remaining default pins can be removed" $OK closure_spectra --json portfolio --unpin-token ethereum --unpin-token tether --unpin-token usd-coin --pin-options
lacks "no pins return after reopening" '"is_pinned":true' closure_spectra --json portfolio --pin-options
check "dashboard reset restores defaults explicitly" $OK closure_spectra settings reset --scope dashboardCustomization --yes
contains "reset pins bitcoin again" '"is_pinned":true' closure_spectra --json portfolio --pin-options
contains "empty chain discovery does not fetch" '"results":[]' closure_spectra --json pool discover-chain Bitcoin
check "reset rejects an unknown scope" $REJECTED closure_spectra settings reset --scope typo --yes
check "provider state is no longer a reset scope" $REJECTED closure_spectra settings reset --scope providerState --yes
check "imports closure watch wallet" $OK closure_spectra wallet watch --chain Ethereum --name "Closure Watch" --address 0x1111111111111111111111111111111111111111
contains "receive falls back to the stored address on an account chain" \
    '0x1111111111111111111111111111111111111111' closure_spectra --json wallet receive "Closure Watch"
check "stores closure alert" $OK closure_spectra alert add --chain Ethereum --target 100 --above
check "stored alert evaluation needs no network" $OK closure_spectra alert check --stored
check "resets wallets through the owned operation" $OK closure_spectra settings reset --scope walletsAndSecrets --scope alertsAndContacts --yes
contains "reset remains empty after reopening" '"wallets":[]' closure_spectra --json wallet list

section "Identity-based artwork"
contains "token identity resolves Ether artwork" '"artworkName":"ethereum"' spectra --json token artwork --token-id ethereum
contains "network identity resolves Base artwork" '"artworkName":"base"' spectra --json token artwork --chain-id base
contains "Base native deployment draws Ether" '"artworkName":"ethereum"' spectra --json token artwork --deployment-id base:native
contains "a ticker alone cannot claim artwork" '"artworkName":""' spectra --json token artwork --token-id USDC
contains "an unknown contract cannot claim artwork" '"artworkName":""' spectra --json token artwork --deployment-id ethereum:erc-20:0xdead
contains_exit 1 "missing transaction cannot be rebroadcast" 'transaction not found' \
    spectra --json send rebroadcast missing --yes

section "Offline integration suites"
for domain in wallets portfolio history send send-icp-zcash send-monero diagnostics endpoints; do
    check "$domain integration checks" $OK \
        python3 "$(dirname "$0")/cli-$domain.py" "$BIN"
done
# Its local SOCKS proxy is the point: requests name remote hosts and never
# leave loopback, so the host-based guard would refuse the very traffic it
# proves is proxied.
check "transport integration checks" $OK \
    env -u SPECTRA_LOOPBACK_ONLY python3 "$(dirname "$0")/cli-transport.py" "$BIN"

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
