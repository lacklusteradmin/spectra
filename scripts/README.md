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
| `cli-send.py` | A password-protected owned send: `send owned-broadcast` is refused without `--yes`; with it and the password from the environment it broadcasts exactly once. |
| `cli-endpoints.py` | The listing's built-in/custom source filter and the health verdict the CLI aggregates (networks without APIs, unreachable endpoints) against a loopback Blockbook node. |
| `cli-send-stages.py` | An EVM send built, signed and broadcast in separate processes: nothing submitted before `broadcast-signed --yes`, an identical retry, and two processes racing to sign at one nonce, one refused as reserved. |
| `cli-send-polkadot.py` | A Polkadot Asset Hub send across processes: a fee changed at broadcast submits nothing, one broadcast, an unchanged rebroadcast, polled finality, recovery after a lost status write, and a failed outcome with its cursor kept. |
| `cli-send-cardano.py` | A Cardano send built, signed and broadcast once, exactly as signed. |
| `cli-send-tokens.py` | A TON jetton send broadcast once, then an incomplete trace, an aborted one and a lost status write recovered to its final outcome across processes. |
| `cli-send-icp-zcash.py` | ICP and transparent Zcash sends built, signed and broadcast in separate processes against loopback providers, with an identical retry. |
| `cli-trc10.py` | A mainnet TRC-10 send broadcast once with its `TransferAssetContract` body, polled to confirmed and recorded with its token and amount. |
| `cli-staking.py` | One stake per account chain and a DOT pool stake across processes: the stored artifact reopened exactly, a broadcast refusal, a same-bytes retry, and exact-hash and finalized recovery. |
| `cli-nfts.py` | ERC-721 and ERC-1155 transfers signed and broadcast, exactly the ethers.js bytes submitted. |
| `cli-zcash-shielded.py` | Zcash shielded funds against `spectra-zcash-fixture`, a loopback lightwalletd whose synthetic chain checks each transaction as a node would and journals what it paid whom: scanning from the restore height, shielding, a shielded payment with a memo only its recipient reads, paying a transparent address, recovery into a second data directory, a restore height past the tip refused; the Sapling parameters the one request beyond loopback, refused. |
| `cli-litecoin-mweb.py` | Litecoin MWEB funds against `spectra-litecoin-mweb-fixture`, a loopback Litecoin node and Esplora indexer whose synthetic chain — independent of core's MWEB, its transactions decoded by the `litecoin` crate — proves every light-client answer, checks each transaction as a node would and journals what it paid whom: a scan of forty thousand outputs in batches, a payment inside MWEB, a peg-out, a peg-in to the wallet, recovery at another path, and a node on another network refused. |
| `cli-multisig-psbt.py` | A 2-of-3 P2WSH multisig on Bitcoin across three data directories against a loopback Esplora: receive, discovery and balance kept across processes, a PSBT signed by two cosigners in their own data directories, joined and broadcast once; submit and discard ask for `--yes`. |
| `cli-multisig-safe.py` | A Safe's policy read from a loopback node, two owners signing in their own data directories and one executing once; a second submission refused as already submitted. |
| `cli-multisig-tron.py` | A Tron account's active permission multi-signed in two data directories, joined and broadcast once. |
| `cli-multisig-xrp.py` | An XRP signer-list payment with a destination tag signed in two data directories, joined, finalized and submitted once against a loopback rippled. |
| `cli-multisig-stellar.py` | A Stellar memo payment signed by two signers in two data directories, joined and submitted once against a loopback Horizon. |
| `cli-multisig-aptos.py` | An Aptos MultiKey transfer signed in two data directories, joined and submitted once against a loopback node. |
| `cli-multisig-cardano.py` | A Cardano native-script spend witnessed by two CIP-1854 keys in two data directories, joined and submitted once against a loopback Koios. |
| `cli-multisig-substrate.py` | A pallet-multisig transfer on Asset Hub against a node that keeps the pallet's state: approved by hash in one data directory and with its call in another, which executes it. |
| `cli-multisig-ton.py` | A TON multisig v2 against a toncenter that keeps the multisig's and its orders' state: one signer's W5 wallet proposes an order, another's approves it in a second data directory and executes it. |
| `cli-issued-assets.py` | XRP Ledger and Stellar trust lines opened, paid through and removed, each signed once and broadcast once as signed, and recorded in history by kind. |
| `cli-wallet-operations.py` | Operations a wallet's page builds, against loopback nodes: closing XRP and Stellar accounts, deleting a NEAR access key, refunding NEAR token storage, merging Sui coins and closing Solana token accounts, each built, signed, broadcast once with the exact signed bytes and recorded in history; a refused closing stores nothing. |
| `cli-icp-staking.py` | An ICP neuron stake signed and reopened, then refused before funding on a forged certificate; recheck and repair leave the stored payload unchanged. |
| `cli-send-near.py` | A NEAR send across processes: a refused broadcast submits nothing, an uncertain one is retried with the same bytes, and the final status is recovered before the nonce, expiry or fee could change. |
| `cli-monero-regtest.py` | Optional: a real `monerod` behind a loopback proxy, run by hand with `--monerod /path/to/monerod`. Not part of `make verify`. |
| `cli-assertions.sh` | Shared shell assertions: checks exit codes and output, and counts passes and failures. Sourced by other scripts. |
| `test-cli-assertions.sh` | Tests the assertion helpers so failed commands cannot be reported as passing. |

Each suite exercises only what the binary alone shows; the rules behind it are
cargo tests in core. Some suites use the standard-library `unittest` runner, so
each scenario reports its own result and a failure does not stop the others. No
third-party Python packages are required. Run a whole suite or an individual
scenario:

```sh
python3 scripts/cli-send-stages.py target/debug/spectra
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
| `normalize-icons.sh` | Normalizes crypto SVGs to the project's format. `--check` reports drift without modifying files. |
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
| `generate-more-message-signature-vectors.cjs` | Cardano CIP-8 data signatures, Kaspa personal messages and Monero `SigV2`, from each network's SDK. |
| `generate-stellar-message-vectors.py` | Stellar SEP-53 message signatures from the Stellar SDK for Python. |
| `generate-near-key-deletion-vector.cjs` | A NEAR `DeleteKey` transaction from @near-js/transactions. |
| `generate-near-storage-unregister-vector.cjs` | A NEP-145 `storage_unregister` call, empty arguments and one yoctoNEAR, from @near-js/transactions. |
| `generate-cardano-stake-address-vectors.cjs` | The CIP-19 stake address behind a phrase's base address and behind every base-address credential mix, and none behind enterprise and pointer addresses, from the Cardano Serialization Library. |
| `generate-sui-merge-vectors.cjs` | Sui coin merges, a token type's and SUI's own, from @mysten/sui. |
| `generate-solana-close-accounts-vector.cjs` | Closing SPL and Token-2022 accounts, compiled and signed by @solana/web3.js. |
| `generate-cardano-asset-vectors.cjs` | Cardano transfers carrying native assets, with each output's minimum ADA and the minimum fee, from the Cardano Serialization Library. |
| `generate-nft-transfer-vectors.cjs` | ERC-721 and ERC-1155 `safeTransferFrom`, `ownerOf`, `balanceOf` and ERC-165 calls for token ids up to 2^256 − 1, and two transfers signed as EIP-1559 transactions, from ethers.js. |
| `generate-issued-asset-vectors.cjs` | XRP Ledger issued-currency payments (with SendMax) and TrustSet, rippled's amount encoding, and Stellar credit-asset payments and ChangeTrust, from ripple-binary-codec and the Stellar SDK. |
| `generate-solana-token-2022-vectors.cjs` | Token-2022 `TransferCheckedWithFee` and a transfer hook's extra accounts, every seed kind, resolved by @solana/spl-token. |
| `generate-account-closing-vectors.cjs` | XRP AccountDelete and Stellar AccountMerge, built and signed by the official XRPL packages and the Stellar SDK. |
| `generate-derivation-profile-vectors.cjs` | Addresses for every registry derivation profile at accounts 0 and 1, from independent libraries. |
| `generate-private-key-vectors.cjs` | Each chain's own private-key encodings, from the chains' SDKs. |
| `generate-ton-mnemonic-vectors.cjs` | ton-crypto 24-word mnemonics, their keys and v4R2 accounts. |
| `generate-multisig-psbt-vectors.cjs` | Bitcoin and Litecoin 2-of-3 P2WSH descriptors, addresses, PSBTs and finished transactions from bitcoinjs-lib. |
| `generate-p2sh-multisig-vectors.cjs` | Bitcoin Cash and Dogecoin 2-of-3 P2SH addresses, digests, signatures, BCHN PSBTs (ecash-lib), Dogecoin partially signed transactions and finished transactions (bitcoinjs-lib, bitcore-lib-cash). |
| `generate-safe-multisig-vectors.cjs` | Safe `SafeTx` hashes, owner signatures and `execTransaction` calldata from protocol-kit, with the official singleton deployments. |
| `generate-tron-multisig-vectors.cjs` | Tron transactions naming a permission, their ids and each key's signature from TronWeb. |
| `generate-xrp-multisig-vectors.cjs` | XRP multi-signed payments, each signer's signature and the combined blob from xrpl.js. |
| `generate-stellar-multisig-vectors.cjs` | Stellar payment envelopes with several signers and time bounds from stellar-base. |
| `generate-sui-multisig-vectors.cjs` | Sui multisig addresses and combined signatures over Ed25519, secp256k1 and secp256r1 keys from @mysten/sui. |
| `generate-aptos-multikey-vectors.cjs` | Aptos MultiKey authentication keys, signatures and signed transactions from the Aptos TS SDK. |
| `generate-cardano-multisig-vectors.cjs` | Cardano native scripts, their addresses, CIP-1854 keys and witnessed spends from cardano-serialization-lib. |
| `generate-substrate-multisig-vectors.cjs` | pallet-multisig account ids, approval calls and `Multisigs` storage on Asset Hub and Bittensor from polkadot.js. |
| `generate-ton-multisig-vectors.cjs` | TON multisig v2 data, order addresses, orders, proposals and approvals from @ton/core and the contract's build. |
| `generate-ton-w5-vectors.cjs` | @ton/ton W5 (wallet v5r1) accounts on both networks, transfers and jetton transfers. |
| `generate-monero-phrase-vectors.py` | Monero 25-word seeds and Polyseeds from two independent implementations (with `polyseed-vectors.c`). |
| `generate-monero-checkpoints.py` | Monero height/timestamp checkpoints for `core/data/monero-checkpoints.json`. |

## Test organisation

CLI integration tests check behavior across processes and persisted results.
Rust unit tests cover pure-function details. Group new scenarios by feature,
rather than creating a file for each development batch; assertion counts are not
a measure of coverage quality.
