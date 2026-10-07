# Scripts

This directory contains build, verification and reference-data tools. Use the
Makefile for routine work:

```sh
make verify       # Formatting, Rust lint/tests, CLI tests and iOS simulator tests
make test-cli     # CLI acceptance tests only
make check-ui     # UI design tokens and SVG formatting
```

## CLI tests

These tests exercise the `spectra` command-line program using temporary databases.
When node responses are needed, they start mock servers on `127.0.0.1`; they do
not broadcast transactions to real chains. Databases and servers are cleaned up
when tests finish. The environment must allow binding local ports.

| File | Purpose |
|---|---|
| `cli-acceptance.sh` | Main entry point: builds or uses the supplied `spectra` binary, checks what only the binary shows (state across processes, the SQLite store and sealed secret files, exit codes, confirmation gates, the loopback network guard), then runs the Python suites below except `cli-monero-regtest.py`. Rules themselves are tested in core. |
| `cli-wallets.py` | Custom EVM paths and signing identity after restart, passphrase handling and invalid derivation inputs, and per-network chain IDs in signed EVM bytes. |
| `cli-portfolio.py` | Balance refresh and preservation after failed reads; network/token identity; valuation with missing quotes or exchange rates; persisted portfolio inclusion and its effect on totals; price alerts. |
| `cli-history.py` | Complete Bitcoin history pagination and duplicate-free repeated refreshes; stored history paging, search, sorting, deduplication and source labels; corrupt-record refusal; provider cursors across restarts. |
| `cli-send.py` | Send previews, self-send confirmation, fee refusal, cancellation and replacement drafts, who holds each transfer end and a not-yet-sent address (a wallet or a contact), and network mismatch refusal; wrong passwords never broadcast, while a correct password broadcasts once and saves the matching transaction. |
| `cli-transport.py` | A stored Tor or custom-proxy policy routes a fresh CLI process through the selected SOCKS proxy. |
| `cli-endpoints.py` | Every catalog network/API pair persists as a custom endpoint; source filters, capability routing and the health summary for networks without providers. |
| `cli-token-preferences.py` | Token-wide choices and editable price-source metadata survive process restarts. |
| `cli-send-stages.py` | Durable build, sign and explicit-node submission stages against loopback nodes. |
| `cli-send-polkadot.py` | Polkadot and Bittensor metadata-driven signing, reviewed fees and finalized success/failure events. |
| `cli-send-cardano.py` | Independent extended-key witness bytes, pure-ADA input selection and refusal of incomplete or changed UTXO facts. |
| `cli-finality-account-chains.py` | Exact XRP, Stellar and Tron execution results, failed transactions and pending unknown results across process restarts. |
| `cli-send-local-digests.py` | Sui/Aptos local transaction digests reject a mismatched submission reply and retain the same signed payload for a fresh-process retry. |
| `cli-send-tokens.py` | Sui, Aptos and TON token stages, metadata and stale-state refusal, plus ERC-20 on Ethereum Classic and HyperEVM. |
| `cli-receipt-fees.py` | Complete historical OP Stack actual fees for confirmed and reverted transactions; missing components remain unknown after reopening. |
| `cli-chain-coverage.py` | Network-correct raw private-key imports, signer identities and testnet watch imports. |
| `cli-send-icp-zcash.py` | ICP and Zcash send stages against loopback providers. |
| `cli-trc10.py` | Mainnet/Nile TRC-10 metadata, discovery/history, exact token signing and durable success/failure; wrong identity/precision/funds refuse early. |
| `cli-staking.py` | Four account chains and DOT nomination pools, ownership/funds checks, fresh-process receipts, same-payload retries and finalized outcomes. |
| `cli-icp-staking.py` | Controller-only neurons, explicit lock/fee review, independent Candid responses and refusal of forged execution certificates before funding. |
| `cli-send-near.py` | Mainnet/testnet native and NEP-141 protocol fee budgets, implicit-account costs, storage reserves and fee/funds changes before signing or pending retries. |
| `cli-send-monero.py` | Monero ownership and network guards; the signature fixture runs in Rust. |
| `cli-monero-regtest.py` | Optional: a real `monerod` behind a loopback proxy, run by hand with `--monerod /path/to/monerod`. Not part of `make verify`. |
| `cli-assertions.sh` | Shared shell assertions: checks exit codes and output, and counts passes and failures. Sourced by other scripts. |
| `test-cli-assertions.sh` | Tests the assertion helpers so failed commands cannot be reported as passing. |

The wallets, portfolio, history, send and transport suites use the
standard-library `unittest` runner. Each scenario reports its own result, and a
failure does not stop the remaining scenarios in that file. No third-party
Python packages are required. Run a whole suite or an individual scenario:

```sh
python3 scripts/cli-portfolio.py
python3 scripts/cli-history.py target/debug/spectra
python3 scripts/cli-send.py target/debug/spectra SendTests.test_password_protected_broadcast
```

Do not use `python -O` or `PYTHONOPTIMIZE`: the suites rely on assertions and
refuse to run when assertions are disabled. Mock nodes verify requests, error
handling and persistence; they do not establish acceptance by a real chain.

## Builds and binding generation

| File | Purpose |
|---|---|
| `build-ios.sh` | Run by the Xcode “Build Rust Core” phase: compiles the Rust library for the device or the arm64 simulator and generates the Swift bindings from it. |
| `build-android.sh` | Compiles Rust libraries for Android architectures and copies them to `jniLibs`. Requires the Android NDK and cargo-ndk. Accepts `--release`. |
| `bindgen-android.sh` | Generates Kotlin bindings from the compiled Rust library. |
| `ios-rust-build-env.sh` | Sourced by build scripts to set a consistent minimum iOS version and clear incompatible iOS build caches when that version changes. |

## Code and resource checks

These source scans help identify problems. Review findings for dynamic calls and
other cases a scan cannot resolve. `make lint` and CI run every scan below except
`check-design-tokens.sh`, which `make check-ui` runs.

| File | Purpose |
|---|---|
| `unreachable-exports.sh` | Finds exported Rust interfaces with no detected Swift or CLI callers. |
| `uncalled-core-fns.sh` | Finds public Rust core functions with no detected callers. |
| `unwritten-record-fields.sh` | Finds `uniffi::Record` fields no production code writes. Builds core and the CLI with `--emit=mir` into `target/mir/` and runs `record_writers.py` on the result. |
| `record_writers.py` | Reads rustc's MIR, where every place carries its type, to find each record field's writes; its docstring says what counts as one. |
| `test-record-writers.py` | Compiles a fixture with the pinned rustc and checks what the MIR reader reports, so a toolchain that prints MIR differently fails there first. |
| `unused-strings.sh` | Finds unused text and inconsistent translation keys across locales. |
| `source_scan.py` | Shared production-source extraction for Rust and foreign source scans; test fixtures do not count as runtime callers or copy producers. |
| `test-source-scan.py` | Tests production/test source selection and Rust module paths. |
| `swift-shell-literals.sh` | Finds hard-coded chain names and amount precision in Swift, where domain rules should come from core. |
| `check-design-tokens.sh` | Finds corner radii, opacity values and other style literals that bypass shared UI design tokens. |

The function scan matches names rather than resolving receiver types or building
a call graph. An unrelated method with the same name can hide an unused method;
review these candidates manually even when the scan passes. The record-field
scan does resolve types — it reads them from the compiler — but it does not
follow a value through a function call, so a field set from a call's result
counts as written whatever the call returns.

## Icons

| File | Purpose |
|---|---|
| `normalize-icons.sh` | Normalizes crypto and fiat SVGs to the project's format. `--check` reports drift without modifying files. |
| `svgo.config.mjs` | SVG normalization rules used by the script above; this is a configuration file. |
| `export-swift-icons.sh` | Converts and synchronizes sources from `icons/` into the Xcode asset catalog, rendering app icons as PNGs. |

After editing icon sources, run:

```sh
scripts/normalize-icons.sh && scripts/export-swift-icons.sh
```

## Test reference data generation

These scripts use independent SDKs to generate reference results for Rust tests.
Normal test runs do not require regeneration. Each file's header lists the pinned
SDK versions and installation/run commands.

| File | Purpose |
|---|---|
| `generate-trc10-send-vectors.cjs` | Independent TronWeb 6.0.4 TRC-10 protobuf, digest and signature references. |
| `generate-account-staking-vectors.cjs` | Independent Solana, Sui, Aptos and NEAR staking transaction references. |
| `generate-polkadot-staking-vectors.cjs` | Independent @polkadot/types 17.0.2 metadata, storage values and nomination-pool call references. |
| `generate-icp-staking-vectors.cjs` | DFINITY 3.4.3 Candid, ingress IDs and update/read-state signatures; owned and queued-maturity responses. |
| `generate-protocol-vectors.cjs` | Generates protocol reference data using the TON and NEAR SDKs. |
| `generate-send-audit-vectors.cjs` | Generates address, transaction and signature reference data using Sui, Aptos, Solana, Tron and related SDKs. |
| `generate-token-send-vectors.cjs` | Sui, Aptos and TON (v4R2 jetton) token-transfer references from the official SDKs. |
| `generate-xrp-payment-vector.cjs` | XRPL payment signing reference from the official XRPL packages. |
| `generate-derivation-profile-vectors.cjs` | Addresses for every registry derivation profile at accounts 0 and 1, from independent libraries. |
| `generate-private-key-vectors.cjs` | Each chain's own private-key encodings, from the chains' SDKs. |
| `generate-ton-mnemonic-vectors.cjs` | ton-crypto 24-word mnemonics, their keys and v4R2 accounts. |
| `generate-ton-w5-vectors.cjs` | @ton/ton W5 (wallet v5r1) accounts on both networks, transfers and jetton transfers. |
| `generate-monero-phrase-vectors.py` | Monero 25-word seeds and Polyseeds from two independent implementations (with `polyseed-vectors.c`). |
| `generate-monero-checkpoints.py` | Monero height/timestamp checkpoints for `core/data/monero-checkpoints.json`. |

## Test organisation

CLI integration tests check behavior across processes and persisted results.
Rust unit tests cover pure-function details. Group new scenarios by feature,
rather than creating a file for each development batch; assertion counts are not
a measure of coverage quality.
