# Scripts

This directory contains 31 scripts and tool configuration files. Use the Makefile
for routine work:

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
| `cli-acceptance.sh` | Main entry point: builds or uses the supplied `spectra` binary, checks command output, exit codes and invalid inputs, then runs the Python suites below except `cli-monero-regtest.py`. |
| `cli-wallets.py` | Wallet imports, mnemonic and passphrase handling, invalid derivation inputs, automatic names, receive addresses and password validation. |
| `cli-portfolio.py` | Balance refresh and preservation after failed reads; network/token identity; valuation with missing quotes or exchange rates; persisted portfolio inclusion and its effect on totals; price and portfolio movement alerts. |
| `cli-history.py` | Complete Bitcoin history pagination and duplicate-free repeated refreshes; stored history paging, search, sorting, deduplication and source labels; corrupt-record refusal; individual transaction status rechecks. |
| `cli-send.py` | Send previews, self-send confirmation, fee refusal, cancellation and replacement drafts, and network mismatch refusal; wrong passwords never broadcast, while a correct password broadcasts once and saves the matching transaction. |
| `cli-diagnostics.py` | Offline refresh outcomes, background maintenance policy, failure/recovery logs, diagnostics against the selected network, wrong-chain node refusal, and validator queries through the configured node. |
| `cli-transport.py` | A stored Tor or custom-proxy policy routes a fresh CLI process through the selected SOCKS proxy. |
| `cli-endpoints.py` | Typed endpoint persistence, source filters and API selection. |
| `cli-token-preferences.py` | Token-wide choices and editable price-source metadata survive process restarts. |
| `cli-send-stages.py` | Durable build, sign and explicit-node submission stages against loopback nodes. |
| `cli-send-icp-zcash.py` | ICP and Zcash send stages against loopback providers. |
| `cli-send-monero.py` | Monero ownership and network guards; the signature fixture runs in Rust. |
| `cli-monero-regtest.py` | Optional: a real `monerod` behind a loopback proxy, run by hand with `--monerod /path/to/monerod`. Not part of `make verify`. |
| `cli-assertions.sh` | Shared shell assertions: checks exit codes and output, and counts passes and failures. Sourced by other scripts. |
| `test-cli-assertions.sh` | Tests the assertion helpers so failed commands cannot be reported as passing. |

The wallets, portfolio, history, send, diagnostics and transport suites use the
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
| `build-ios.sh` | Compiles Rust libraries for iPhone devices and simulators, merging simulator architectures. Accepts `--release`. |
| `build-android.sh` | Compiles Rust libraries for Android architectures and copies them to `jniLibs`. Requires the Android NDK and cargo-ndk. Accepts `--release`. |
| `bindgen-ios.sh` | Generates Swift bindings from the compiled Rust library and applies the project's generator fixes. Do not edit the generated output by hand. |
| `bindgen-android.sh` | Generates Kotlin bindings from the compiled Rust library. |
| `ios-rust-build-env.sh` | Sourced by build scripts to set a consistent minimum iOS version and clear incompatible iOS build caches when that version changes. |

## Code and resource checks

These source scans help identify problems. Review findings for dynamic calls and
other cases a scan cannot resolve. Not all of these checks are part of
`make verify`; run them directly as needed.

| File | Purpose |
|---|---|
| `count-exports.sh` | Counts Rust functions and methods exposed to Swift. |
| `unreachable-exports.sh` | Finds exported Rust interfaces with no detected Swift or CLI callers. |
| `uncalled-core-fns.sh` | Finds public Rust core functions with no detected callers. |
| `unused-strings.sh` | Finds unused text and inconsistent translation keys across locales. |
| `swift-shell-literals.sh` | Finds hard-coded chain names and amount precision in Swift, where domain rules should come from core. |
| `check-design-tokens.sh` | Finds corner radii, opacity values and other style literals that bypass shared UI design tokens. |

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
| `generate-protocol-vectors.cjs` | Generates protocol reference data using the TON and NEAR SDKs. |
| `generate-send-audit-vectors.cjs` | Generates address, transaction and signature reference data using Sui, Aptos, Solana, Tron and related SDKs. |

## Former test files

Thirteen separate fixture files, previously named after development stages and
follow-up batches, were consolidated into five feature suites. Older plans and
change records refer to the paths used at the time; this table locates their
current coverage.

| Former file | Current location |
|---|---|
| `cli-stage3.sh` | Recovery checks in `cli-diagnostics.py`, balance refresh in `cli-portfolio.py`, and missing-transaction rebroadcast refusal in the main acceptance script. Empty-wallet shape assertions were removed. |
| `cli-stage3-followup.sh` | Persisted rename in the main acceptance script, portfolio inclusion in `cli-portfolio.py`, maintenance policy in `cli-diagnostics.py`, and transaction recheck/send checks in their respective suites. Separate empty quote-cache and maintenance-scope checks were removed. |
| `cli-shell-ownership.py` | Naming and receive addresses in wallets; movement alerts in portfolio; staking node queries in diagnostics. |
| `cli-shell-boundary.py` | Import parameters in wallets; offline refresh in diagnostics. |
| `cli-shell-five-fixes.py` | Password validation in wallets; network diagnostics in diagnostics; password-protected broadcasts in send. |
| `cli-projection-boundary.py` | Valuation in portfolio; history paging, search and deduplication in history. |
| `cli-balance-refresh.py`, `cli-network-token-identity.py` | `cli-portfolio.py`. |
| `cli-bitcoin-history.py`, `cli-history-corruption.py`, `cli-history-source.py`, `cli-transaction-recheck.py` | `cli-history.py`. |
| `cli-owned-send.py` | Send scenarios in `cli-send.py`; price alerts in `cli-portfolio.py`. |

CLI integration tests check behavior across processes and persisted results.
Rust unit tests cover pure-function details. Group new scenarios by feature,
rather than creating a file for each development batch; assertion counts are not
a measure of coverage quality.
