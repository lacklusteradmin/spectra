# Tests

This directory holds the CLI acceptance suites in [`acceptance/`](acceptance/)
and the independent SDK generators for the reference data in
`core/tests/fixtures/` in [`vectors/`](vectors/). Build and source-check tools
live in [`scripts/`](../scripts/README.md). Run from the repository root:

```sh
make test-cli     # Build and run the offline CLI acceptance suite
```

## CLI acceptance

These tests exercise the `spectra` command-line program using temporary databases.
When node responses are needed, they start mock servers on `127.0.0.1`; they do
not broadcast transactions to real chains. Databases and servers are cleaned up
when tests finish. The environment must allow binding local ports.

`python3 -B tests/acceptance [path/to/spectra]` (what `make test-cli` runs)
builds the CLI and the Zcash and MWEB fixture programs unless given a binary,
starts every suite below at once — each owns its data directories and loopback
nodes — and meanwhile checks what only the binary shows: state across
processes, the SQLite store and sealed secret files, exit codes, confirmation
gates and the loopback network guard. `SPECTRA_LOOPBACK_ONLY` makes core refuse
and journal any request beyond loopback; a journaled request fails the run.
Rules themselves are tested in core.

Every suite builds on [`acceptance/harness.py`](acceptance/harness.py): the
binary under test, `Spectra` (one data directory and the environment its
processes see, with `run`, `fail` and `process`), `serve` (a loopback node for
a block's length) with the `Handler` and `JsonRpc` bases, `fixture` for
`core/tests/fixtures/`, `companion` and `ChainFixture` for the fixture
programs, the multisig helpers `Spectra.sign` and `signed_by`, and `suite` for
a node another suite defines. Suites check with `assert`, so the harness refuses
to load under `python -O` or `PYTHONOPTIMIZE`.

| Suite | Purpose |
|---|---|
| `send.py` | A password-protected owned send: `send owned-broadcast` is refused without `--yes`; with it and the password from the environment it broadcasts exactly once. |
| `endpoints.py` | The listing's built-in/custom source filter and the health verdict the CLI aggregates (networks without APIs, unreachable endpoints) against a loopback Blockbook node. |
| `send_stages.py` | An EVM send built, signed and broadcast in separate processes: nothing submitted before `broadcast-signed --yes`, an identical retry, and two processes racing to sign at one nonce, one refused as reserved. |
| `send_polkadot.py` | A Polkadot Asset Hub send across processes: a fee changed at broadcast submits nothing, one broadcast, an unchanged rebroadcast, polled finality, recovery after a lost status write, and a failed outcome with its cursor kept. |
| `send_cardano.py` | A Cardano send built, signed and broadcast once, exactly as signed. |
| `send_tokens.py` | A TON jetton send broadcast once, then an incomplete trace, an aborted one and a lost status write recovered to its final outcome across processes. |
| `send_icp_zcash.py` | ICP and transparent Zcash sends built, signed and broadcast in separate processes against loopback providers, with an identical retry. |
| `trc10.py` | A mainnet TRC-10 send broadcast once with its `TransferAssetContract` body, polled to confirmed and recorded with its token and amount. |
| `staking.py` | One stake per account chain and a DOT pool stake across processes: the stored artifact reopened exactly, a broadcast refusal, a same-bytes retry, and exact-hash and finalized recovery. |
| `nfts.py` | ERC-721 and ERC-1155 transfers signed and broadcast, exactly the ethers.js bytes submitted. |
| `zcash_shielded.py` | Zcash shielded funds against `spectra-zcash-fixture`, a loopback lightwalletd whose synthetic chain checks each transaction as a node would and journals what it paid whom: scanning from the restore height, shielding, a shielded payment with a memo only its recipient reads, paying a transparent address, recovery into a second data directory, a restore height past the tip refused; the Sapling parameters the one request beyond loopback, refused. |
| `litecoin_mweb.py` | Litecoin MWEB funds against `spectra-litecoin-mweb-fixture`, a loopback Litecoin node and Esplora indexer whose synthetic chain — independent of core's MWEB, its transactions decoded by the `litecoin` crate — proves every light-client answer, checks each transaction as a node would and journals what it paid whom: a scan of forty thousand outputs in batches, a payment inside MWEB, a peg-out, a peg-in to the wallet, recovery at another path, and a node on another network refused. |
| `multisig_psbt.py` | A 2-of-3 P2WSH multisig on Bitcoin across three data directories against a loopback Esplora: receive, discovery and balance kept across processes, a PSBT signed by two cosigners in their own data directories, joined and broadcast once; submit and discard ask for `--yes`. |
| `multisig_safe.py` | A Safe's policy read from a loopback node, two owners signing in their own data directories and one executing once; a second submission refused as already submitted. |
| `multisig_tron.py` | A Tron account's active permission multi-signed in two data directories, joined and broadcast once. |
| `multisig_xrp.py` | An XRP signer-list payment with a destination tag signed in two data directories, joined, finalized and submitted once against a loopback rippled. |
| `multisig_stellar.py` | A Stellar memo payment signed by two signers in two data directories, joined and submitted once against a loopback Horizon. |
| `multisig_aptos.py` | An Aptos MultiKey transfer signed in two data directories, joined and submitted once against a loopback node. |
| `multisig_cardano.py` | A Cardano native-script spend witnessed by two CIP-1854 keys in two data directories, joined and submitted once against a loopback Koios. |
| `multisig_substrate.py` | A pallet-multisig transfer on Asset Hub against a node that keeps the pallet's state: approved by hash in one data directory and with its call in another, which executes it. |
| `multisig_ton.py` | A TON multisig v2 against a toncenter that keeps the multisig's and its orders' state: one signer's W5 wallet proposes an order, another's approves it in a second data directory and executes it. |
| `issued_assets.py` | XRP Ledger and Stellar trust lines opened, paid through and removed, each signed once and broadcast once as signed, and recorded in history by kind. |
| `wallet_operations.py` | Operations a wallet's page builds, against loopback nodes: closing XRP and Stellar accounts, deleting a NEAR access key, refunding NEAR token storage, merging Sui coins and closing Solana token accounts, each built, signed, broadcast once with the exact signed bytes and recorded in history; a refused closing stores nothing. |
| `icp_staking.py` | An ICP neuron stake signed and reopened, then refused before funding on a forged certificate; recheck and repair leave the stored payload unchanged. |
| `send_near.py` | A NEAR send across processes: a refused broadcast submits nothing, an uncertain one is retried with the same bytes, and the final status is recovered before the nonce, expiry or fee could change. |
| `monero_regtest.py` | Optional: a real `monerod` behind a loopback proxy, run by hand with `--monerod /path/to/monerod`. Not part of `make verify`. |

Each suite exercises only what the binary alone shows; the rules behind it are
cargo tests in core. Some suites use the standard-library `unittest` runner, so
each scenario reports its own result and a failure does not stop the others. No
third-party Python packages are required. Run a whole suite or one scenario:

```sh
python3 -B tests/acceptance/send_stages.py target/debug/spectra
python3 -B tests/acceptance/send.py target/debug/spectra SendTests.test_password_protected_broadcast
```

A suite run on its own is confined to loopback as the driver's are: the
harness journals what core refuses and fails the run when anything was. Mock
nodes verify requests, error handling and persistence; they do not establish
acceptance by a real chain.

## Test reference data generation

These scripts use independent SDKs to generate reference results for Rust and
CLI tests in [`core/tests/fixtures/`](../core/tests/fixtures/): addresses, keys,
transaction bytes, hashes and signatures. Keeping the generators makes the
reference data reproducible and inspectable without using core's implementation
to compute its own expected results. Every `.cjs` generator has fixture data
consumed by tests.

The `.cjs` extension selects Node.js CommonJS, matching the scripts' `require()`
imports. These are developer tools; the app, builds, CI and normal test runs use
the checked-in fixtures and do not run the generators. Regenerate only when
changing or extending the reference cases.

The generators live in [`vectors/`](vectors/) and share one install:
`package.json` pins one version of every SDK they load, and `package-lock.json`
the rest of the tree. Install it once (into the ignored
`tests/vectors/node_modules/`), then run a generator from the repository root
with the line in its header, which names its fixture as an output redirection
or an argument when it does not write the file itself:

```sh
npm ci --ignore-scripts --prefix tests/vectors
node tests/vectors/generate-account-key-vectors.cjs > core/tests/fixtures/account-keys.json
```

Two pins are held back on purpose: `@noble/hashes` stays on 1.x because the
DFINITY 3.4.3 packages require it as a peer, and `overrides` keeps bip322-js
2.0.0 on bitcoinjs-lib 6.1.8: from 6.1.9 its fee check rejects the UTXO-less
`to_spend` transaction bip322-js builds. Some fixtures hold randomized
values (Taproot BIP-322, sr25519, Kaspa and Monero message signatures, and new
TON mnemonics), which change on every run. The Python generators and
`polyseed-vectors.c` are not part of the install: their headers give their own
setup, and the Monero phrase generator's Polyseed build steps use that helper
source.

| File | Purpose |
|---|---|
| `generate-trc10-send-vectors.cjs` | Independent TronWeb 6.0.4 TRC-10 protobuf, digest and signature references. |
| `generate-account-staking-vectors.cjs` | Independent Solana, Sui, Aptos and NEAR staking transaction references. |
| `generate-polkadot-staking-vectors.cjs` | Independent @polkadot/types 17.0.2 metadata, storage values and nomination-pool call references. |
| `generate-icp-staking-vectors.cjs` | DFINITY 3.4.3 Candid, ingress IDs and secp256k1 update/read-state signatures; owned and queued-maturity responses. |
| `generate-protocol-vectors.cjs` | Generates protocol reference data using the TON and NEAR SDKs. |
| `generate-send-audit-vectors.cjs` | Generates address, transaction and signature reference data using Sui, Aptos, Solana, Tron and related SDKs. |
| `generate-token-send-vectors.cjs` | Sui, Aptos and TON (v4R2 jetton) token-transfer references from the official SDKs. |
| `generate-xrp-payment-vector.cjs` | XRPL payment signing reference from the official XRPL packages. |
| `generate-payment-memo-vectors.cjs` | XRP destination tags and Stellar text/ID memos on payments and account closing, from each network's SDK. |
| `generate-message-signature-vectors.cjs` | Legacy Bitcoin-family, BIP-322, EVM, Tron, Solana, Sui and Substrate message signatures from independent libraries. |
| `generate-more-message-signature-vectors.cjs` | Cardano CIP-8 data signatures, Kaspa personal messages and Monero `SigV2`, from each network's SDK. |
| `generate-stellar-message-vectors.py` | Stellar SEP-53 message signatures from the Stellar SDK for Python. |
| `generate-near-key-deletion-vector.cjs` | A NEAR `DeleteKey` transaction from @near-js/transactions. |
| `generate-near-storage-unregister-vector.cjs` | A NEP-145 `storage_unregister` call, empty arguments and one yoctoNEAR, from @near-js/transactions. |
| `generate-near-token-registration-vector.cjs` | A NEAR token recipient's storage registration and transfer in one signed transaction, from @near-js/transactions. |
| `generate-cardano-stake-address-vectors.cjs` | The CIP-19 stake address behind a phrase's base address and behind every base-address credential mix, and none behind enterprise and pointer addresses, from the Cardano Serialization Library. |
| `generate-sui-merge-vectors.cjs` | Sui coin merges, a token type's and SUI's own, from @mysten/sui. |
| `generate-solana-close-accounts-vector.cjs` | Closing SPL and Token-2022 accounts, compiled and signed by @solana/web3.js. |
| `generate-cardano-asset-vectors.cjs` | Cardano transfers carrying native assets, with each output's minimum ADA and the minimum fee, from the Cardano Serialization Library. |
| `generate-nft-transfer-vectors.cjs` | ERC-721 and ERC-1155 `safeTransferFrom`, `ownerOf`, `balanceOf` and ERC-165 calls for token ids up to 2^256 − 1, and two transfers signed as EIP-1559 transactions, from ethers.js. |
| `generate-issued-asset-vectors.cjs` | XRP Ledger issued-currency payments (with SendMax) and TrustSet, rippled's amount encoding, and Stellar credit-asset payments and ChangeTrust, from ripple-binary-codec and the Stellar SDK. |
| `generate-solana-token-2022-vectors.cjs` | Token-2022 `TransferCheckedWithFee` and a transfer hook's extra accounts, every seed kind, resolved by @solana/spl-token. |
| `generate-account-closing-vectors.cjs` | XRP AccountDelete and Stellar AccountMerge, built and signed by the official XRPL packages and the Stellar SDK. |
| `generate-derivation-profile-vectors.cjs` | Addresses for every registry derivation profile at accounts 0 and 1, from independent libraries. |
| `generate-account-key-vectors.cjs` | Account extended public keys in each UTXO network's encoding, from bip39, bip32 and tiny-secp256k1. |
| `generate-account-utxo-vectors.cjs` | Signed transactions spending several account addresses with separate input keys, from bitcoinjs-lib and kaspa-wasm. |
| `generate-substrate-path-vectors.cjs` | Substrate hard/soft derivation paths and passphrases, with sr25519 keys and SS58 addresses, from polkadot.js. |
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
| `generate-monero-subaddress-vectors.cjs` | Monero mainnet and stagenet subaddresses from wallet2 compiled to WebAssembly by monero-ts. |
| `generate-monero-phrase-vectors.py` | Monero 25-word seeds and Polyseeds from two independent implementations (with `polyseed-vectors.c`). |
| `polyseed-vectors.c` | Polyseed reference-implementation helper for `generate-monero-phrase-vectors.py`; build instructions are in that generator's header. |

## Test organisation

CLI integration tests check behavior across processes and persisted results.
Rust unit tests cover pure-function details. Group new scenarios by feature,
rather than creating a file for each development batch; assertion counts are not
a measure of coverage quality.
