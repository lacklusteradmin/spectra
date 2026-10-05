# Open items

Engineering work that remains. Rule 0 in [AGENTS.md](../AGENTS.md) applies.
Delete an item once it is done; what changed belongs in
[BEHAVIOUR-CHANGES.md](BEHAVIOUR-CHANGES.md).

## Tasks

- [ ] **Audit endpoints across all supported chains and networks.** Review
  the endpoint catalog, default selection and custom endpoint routing. Verify
  each endpoint's concrete network identity, `EndpointApi`, advertised
  capabilities, access requirements and availability; confirm built-in defaults
  need no API key. Cover reads, history/token discovery, fees, transaction status
  and broadcast routing where advertised.
  Check URL validation, health probes, failure reporting and fallback behaviour
  against the same core rules. Record dated, reproducible evidence per endpoint,
  remove or correct unsupported entries, and prove routing/refusal paths through
  offline CLI fixtures. Keep live probes read-only; any resulting behaviour
  changes need `BEHAVIOUR-CHANGES.md` entries and `make verify`.

- [ ] **Detect FFI record fields with no production writer.** A syntactic scan
  cannot reliably distinguish unwritten fields from serde or multi-line writes.
  A useful gate needs type-aware analysis before it can reject unused fields.

- [ ] **Name FFI types once, in Rust, so Swift needs no typealiases.** Swift
  renames nine UniFFI types with `typealias`, so each has two names and both
  appear in code: `Coin = AssetHolding`, `TransactionRecord =
  CorePersistedTransactionRecord`, `TransactionStatus = CoreTransactionStatus`,
  `PriceAlertRule = PriceAlertEvaluationAlert`, `PriceAlertCondition =
  CorePriceAlertCondition`, `TokenPreferenceEntry = CoreTokenPreferenceEntry`,
  `SeedDerivationPaths = CoreSeedDerivationPaths`, `DashboardAssetGroup =
  CoreDashboardAssetGroup` and `DashboardPinOption = CoreDashboardPinOption`
  (the last two in `swift/views/DashboardViews.swift`, the rest in
  `CoreModels.swift`, `ChainTypes.swift` and `RegistryModels.swift`). Pick the
  one name each type should have — drop the `Core` prefix, which says where a
  type lives rather than what it is, and settle `Coin` versus `AssetHolding` —
  and rename the Rust type (or set its UniFFI name) so the binding carries it.
  Apply the same rule to the remaining `Core*` exports Swift uses unaliased
  (`CoreSeedDerivationPreset`, `CoreWalletDerivationOverrides`,
  `CoreTokenPreferenceKey`, `CoreAppState`). Rename in CLI and Kotlin call
  sites in the same change, regenerate the bindings, delete every alias, and
  pass `make verify`. No behaviour changes; nothing to record beyond the
  commit.
- [ ] **Give `AppState`'s domains their own observable state.** `AppState` is
  one `@Observable` class whose methods are spread over 30
  `AppState+<Domain>.swift` extensions, a third of them under 40 lines. The
  extensions share every stored property, so the split hides line count but
  not coupling, and any view that reads one property is in the same
  invalidation scope as the rest. `sendFlow`, `receiveFlow`, `walletImport`,
  `preferences` and `diagnostics` already show the target shape: a small
  `@MainActor @Observable` type that `AppState` owns, holding that domain's
  view state and exposing its actions. Move the remaining domains the same way
  — address book, token preferences, price alerts, Tor, history paging,
  send execution and preview, notifications and Live
  Activities — one domain per change, each taking its properties out of
  `AppState` and its views reading the new object rather than the store.
  Merge the tiny extensions that are only adapters (`Diagnostics`,
  `Persistence`, `TorLifecycle`) into the domain that owns them
  rather than giving each a type. Keep core as the owner of domain state: the
  new types hold projections and view state only, per AGENTS.md. Each step
  needs the iOS suite green and no user-visible change; record nothing unless
  behaviour moves.

## Supported chain scope

[CHAIN-SUPPORT.md](CHAIN-SUPPORT.md) defines supported protocols, import formats,
provider coverage, staking actions and explicit exclusions. The [follow-up asset
audit](audits/token-verification-2026-10-04/README.md) reactivates 58 deployments
and all associated identity/wiki records without changing original fields.

The following gaps were identified in the 2026-10-04 code review. An absent
feature is not a supported capability; add its registry capability only when
core owns the complete operation and the CLI can prove it. Protocol and
funds/key changes require independent vectors, CLI acceptance, the relevant
refusal paths and `make verify`, with behaviour changes recorded separately.

### Transfer correctness

- [ ] **Fix Decred destination script construction first.**
  `decode_dcr_address` accepts both `Ds…` P2PKH and `Dc…` P2SH addresses but
  returns only the hash; `send/decred.rs` always creates a P2PKH recipient
  output. Retain the decoded network and script type and build the matching
  output, or explicitly refuse unsupported destinations before provider reads.
  Prove P2PKH/P2SH recipient scripts and network refusal with independent
  vectors and CLI inspection of the signed transaction. See
  [address decoding](../core/src/derivation/decred.rs) and
  [transfer construction](../core/src/send/decred.rs).

### Asset and privacy protocols

- [ ] **Implement complete Litecoin MWEB support.** Address/key derivation,
  owned-output scanning, balances, recovery, MWEB transfers and peg-in/peg-out
  need a complete protocol adapter. The former incomplete peg-in builder was
  removed; MWEB destinations are currently refused. Verify protocol bytes and
  proofs independently and exercise receive, build/sign, recovery and refusal
  paths through the CLI before advertising support.
- [ ] **Implement Zcash shielded wallets and transfers.** Only transparent
  addresses and transactions are supported today. Add the supported shielded
  address/key formats, note scanning and durable recovery, balances, proof
  generation and signing as one coherent core model. Prove recovery and
  transparent/shielded transfer boundaries with independent vectors and CLI
  acceptance; do not advertise shielded support from address validation alone.
- [ ] **Support XRP and Stellar issued assets.** Both are outside the tracked
  asset interface today. Add issuer-qualified asset identity, metadata,
  balances, history and transfers, including each chain's ownership and account
  prerequisites. CLI checks must distinguish assets with the same display code
  but different issuers and prove signing and refusal paths.
- [ ] **Support Cardano native assets and token-bearing ADA inputs.** The
  current ADA-only builder skips UTXOs containing native assets, so sufficient
  total ADA does not imply an ADA payment can be made. Add asset identity,
  balances, history, transfer and change handling; ADA payments using mixed
  inputs must return every unspent asset correctly. Prove asset conservation,
  output minimums, fees and signing with independent vectors and CLI checks.
- [ ] **Extend Solana Token-2022 transfers.** Actual transfer fees, transfer
  hooks with a program and other extensions that change transfer semantics are
  refused. Zero-fee configurations and hooks without a program already work.
  Implement each supported extension's exact recipient amount, required
  accounts and review/signing semantics; explicitly refuse remaining unknown
  extensions. Prove both active-extension transfers and refusal paths in the
  CLI. See [mint validation](../core/src/api/solana_json_rpc.rs).
- [ ] **Add EVM ERC-721/ERC-1155 NFT support.** Current asset discovery and
  transfer assembly cover fungible ERC-20-style assets, not NFTs. Add a model
  with contract and token-ID identity, ownership/quantity reads, discovery,
  history and the appropriate transfer builders. Prove that token IDs and
  quantities cannot be interpreted as fungible decimal balances.

### Wallet import and address recovery

- [ ] **Add owned Monero accounts and subaddresses.** The wallet derives its
  primary address and scans without registering owned subaddresses. Add core
  account/index derivation, receive rotation and durable scan targets. Prove
  recovery of outputs sent to non-primary accounts/subaddresses and continued
  scanning after restart. Sending to a subaddress recipient is a separate,
  already parsed destination capability.
- [ ] **Add Monero view-only wallet import and scanning.** There is no current
  watch-only import path for a Monero scan-key wallet. Model public spend and
  private view keys explicitly, validate the derived address before storing,
  and allow scanning without a spend key. Report any limits on spent-output
  knowledge; prove import, scan, restart and refusal to sign through the CLI.
  An address alone does not contain the keys needed for Monero scanning.
- [ ] **Add account address recovery for ZEC, BTG, DCR, KAS and DASH.** Each
  currently uses one owned address per network. Add supported receive/change
  derivation, account-wide gap scanning, receive rotation and balances across
  owned sources together with signing for every discovered spendable source.
  CLI recovery must find used child addresses beyond the initial address and
  remain correct after reopening the database.
- [ ] **Support Polkadot/Bittensor junction derivation.** `//hard` and `/soft`
  derivation is unsupported; only the root sr25519 derivation is exposed.
  The primitive rejects explicit paths, but ordinary chain dispatch does not
  forward the path to these adapters. First ensure every entry point refuses
  unsupported paths rather than silently deriving the root key. Then implement
  explicit path parsing and derivation against independent vectors, validate
  imported identity before storing, and prove CLI import and signing for
  supported paths.
- [ ] **Add Bitcoin-family multisig and PSBT workflows.** Wallets currently
  model single-key scripts. Core must own the signing policy, cosigner/script
  identity, partial signatures, transaction review and finalization. Prove
  interoperability with independent PSBT fixtures and refuse foreign inputs,
  mismatched scripts or changed outputs before signing.
- [ ] **Extend external account xpub import beyond Bitcoin.** Bitcoin mainnet
  and its test networks are the only accepted account-xpub imports. Account
  discovery on other UTXO chains does not provide this import capability.
  Add each supported chain's network/path/script validation, owned public
  derivation and durable discovery; CLI checks must prove watch-only recovery
  and reject incompatible networks or account formats before storing.

### History and provider coverage

- [ ] **Add built-in EVM history and token-discovery providers.** BNB,
  HyperEVM, Linea, Sei, Cronos, opBNB, Sonic, Berachain, X Layer and Monad
  currently require a custom Blockscout-compatible address indexer. Add only
  independently verified keyless sources for the concrete network, including
  the corresponding test networks where coverage is absent. Verify network
  identity, paginated native/token history, discovery and failure reporting;
  an ordinary RPC does not supply complete address history.
- [ ] **Implement Polkadot/Westend and Bittensor address-history adapters.**
  Their current Substrate RPC path supports balances, transfers and submitted
  finality, but no complete address history. A custom RPC cannot fill the gap:
  add an independently verified indexer API, registry capabilities, pagination
  and durable merge/continuation handling. Prove received and externally
  submitted transactions through CLI fixtures, not only local submissions.
- [ ] **Restore verified default providers for BCH, DOGE, ZEC, DCR and DASH
  testnets.** These networks currently require custom endpoints before reads
  or transfers. Verify the concrete network and every advertised capability,
  including balance, UTXOs, history, fee/status reads and submission routing.
  CLI checks must distinguish missing configuration from provider failure;
  retain no failed endpoint as a fallback.

### Staking coverage

- [ ] **Extend staking beyond the six supported mainnets and enable supported
  testnets.** Only SOL, SUI, APT, NEAR, DOT and ICP mainnets currently have
  adapters; all testnets are refused. Add other staking integrations one
  protocol at a time, with owned positions, exact action prerequisites,
  preparation/signing, explicit broadcast, final execution proof and durable
  recovery. Prove the whole action lifecycle and wrong-owner/network refusal
  through the CLI before enabling a network in the registry.
- [ ] **Implement Peercoin minting if it is brought into product scope.**
  Minting is explicitly excluded today. It needs an online node and its own
  authority, key-access, process-lifetime and recovery model; a staking catalog
  entry alone is not an implementation. Resolve those requirements in core
  and prove the lifecycle through the CLI before exposing the action.
