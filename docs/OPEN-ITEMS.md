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

## Single-wallet setup

Every Add Wallet path should work on one network and create one wallet:
creating a wallet, importing a phrase or a private key, watching addresses
and importing a Funds Finder result. The one exception is a watch-only import,
which can add several addresses on its network at once, one wallet each. A
wallet already belongs to one network
for good (`WalletView.chain_id`). Creating or importing a phrase across N
chains is a batch of N independent wallets, each sealing its own copy of the
phrase, refused together when any one fails and named "Name 1…N". The
private-key, watch-only and edit flows already take one chain. Once setup
is on one network, the network can be chosen first, and everything after it
can be specific to that network: the ways it can be added, the secret formats
they accept and the options they offer. The first two items are the
foundation; the rest build on them and need not land together.

- [x] **Collapse the setup request to one chain.** Replace
  `WalletImportRequest.selected_chain_ids` with one `chain` and delete what
  only a batch needs: the multi-chain branch of `check_import_shape`, the batch
  suffix in `wallet_display_name`, the per-chain id plans, and in Swift
  `selectedChainsStorage`, `allowsMultipleChainSelection`,
  `selectableDerivationChains` and the picker's clear-all. Creation makes one
  wallet like every import. A watch-only import still takes several
  addresses on its one chain, one wallet each, or one account xpub. `wallet
  new`, `wallet import` and `wallet watch` take exactly one `--chain`. Record
  the removal of multi-chain batches in BEHAVIOUR-CHANGES.md. Prove one
  network per setup and the refusal of a second chain through the CLI, and
  run `make verify`.
- [x] **Choose the network first, then how to add it.** Add Wallet becomes
  the network picker. It lists mainnets, and a "Show test networks" switch
  adds the test networks as rows of the same list. The chosen network's page
  offers only the methods the network supports: create, phrase, private
  key, watched address, account xpub and, later, view key. Each method lists
  the formats it accepts, its derivation profiles, its extra fields and its
  limits. All of this is one per-chain setup descriptor on `registry::Chain`.
  The CLI prints it (`wallet methods --chain <chain>`) and Swift renders it
  generically. It replaces `AddWalletEntryView`'s fixed list, the
  `importsPrivateKey`/`isWatchOnlyMode` flags with `offers(_:)`, and the
  static `SetupFlow`s; pages follow from the chosen method instead. A core
  test keeps the descriptor honest: every advertised method and format
  imports, and every other is refused. As the items under
  [Wallet import and address recovery](#wallet-import-and-address-recovery)
  land, they become methods or fields on their network's page.
- [x] **Copy the phrase from a wallet's seed sheet.** One wallet per setup
  means the phrase is entered once per chain. Spectra will not model several
  wallets sharing one phrase. The wallet detail page already reveals the phrase
  after Face ID and its optional password, and the setup grid already has a
  paste button, so add a Copy button to the reveal sheet. Copy the phrase,
  both here and on the create page, as a local-only pasteboard item with a
  short expiration (`UIPasteboard` `.localOnly` and `.expirationDate`). It
  must not reach other devices through Universal Clipboard or stay on the
  pasteboard indefinitely. The create page's copy currently does neither;
  record that change in BEHAVIOUR-CHANGES.md.
- [x] **Use Monero's and TON's own phrase formats; the BIP-39 readings are
  wrong.** This is a correctness fix and comes first after the foundation.
  Both chains read phrases with schemes their own wallets do not use:
  - Monero reads anything other than 25 words as BIP-39 and takes the BIP-39
    seed's first 32 bytes as the spend key (`monero::derive_from_seed_phrase`).
    Monero's wallets do not restore phrases this way.
  - TON derives any phrase with the TON scheme (`derive_ton_seed`) without
    checking that it is a TON mnemonic. Tonkeeper refuses a BIP-39 phrase
    unless it happens to pass TON's own check.
  - Creation generates BIP-39 for both chains, so every Monero or TON backup
    Spectra creates restores only in Spectra.
  - `check_seed_phrase` judges only BIP-39, so the setup page refuses the
    formats that are right. Monero's 25 words fail as a non-standard length,
    and Tonkeeper's 24 words fail the checksum.

  The fix, per chain:
  - Monero: read the 25-word seed in every Monero wordlist, not only English,
    and the 16-word Polyseed, whose embedded birthday becomes the restore
    height. Create the 25-word seed, since every Monero wallet restores it.
    Delete the BIP-39 reading.
  - TON: validate phrases as ton-crypto's `mnemonicValidate` does, including
    password-protected phrases, and create them as its `mnemonicNew` does.
    Refuse a phrase that is not a TON mnemonic. A BIP-39 reading for TON, as
    some multi-chain wallets use, would be a separate derivation profile.
  - Both: pass the chain into the seed verdict, so the grid's length,
    wordlist and checksum follow the chain's format. Creation takes its
    format from the setup descriptor.

  Prove each format against vectors from the reference implementations
  (monero-wallet-cli, Polyseed, ton-crypto). Include restoring a
  Spectra-created backup in each of them. Prove through the CLI that both
  chains refuse a BIP-39 phrase. Record the removed readings in
  BEHAVIOUR-CHANGES.md, update CHAIN-SUPPORT.md, and run `make verify`.
- [x] **Accept each chain's native private-key encoding.** Only 32/64-byte
  hex is accepted (`standalone::private_key_hex`). Wallets export WIF on the
  Bitcoin family, base58 64-byte keypairs on Solana, `S…` secret seeds on
  Stellar, `suiprivkey1…` on Sui, `ed25519-priv-0x…` on Aptos and
  `ed25519:…` on NEAR. Parse each in core. Check embedded network bytes, and
  check that a keypair's public half matches its secret. Keep a WIF's
  compression flag: an uncompressed key owns a different P2PKH address and
  must not be silently compressed. Refuse a format on the wrong chain before
  storing, and have the editor name the format the chosen chain expects. Use
  independent vectors per format, with CLI import and refusal checks.
- [x] **Preview the stored address before committing.** Every method's page
  shows the address core will store and updates it as the inputs change. For
  a phrase or a key, that is the address derived for the chosen network, path
  and overrides; for a watch, it is the normalized watched address or the
  xpub's first receive address. The user can match it against their previous
  wallet before anything is sealed, and a created wallet shows it before
  backup verification. Use one core derivation for the preview and the commit
  so the two cannot disagree. No network access.
- [x] **Replace path presets with per-chain derivation profiles.**
  `SeedDerivationPreset` (Standard/Account 1/Account 2) moves every chain's
  account at once, a cross-chain setting a one-chain import does not need.
  Funds Finder hard-codes its own path matrix and labels. Put named profiles
  and an account index on `registry::Chain` for both callers to read, then
  delete the preset and the matrix. Examples are Bitcoin-family BIP-44/49/84/86
  script types, limited to those the chain's signer spends, and Solana's
  hardened and legacy paths. The raw path editor remains under Advanced.
  Profiles the deriver lacks, such as TON W5 beside v4R2, are separate work.
  Verify every profile against independent vectors before listing it.
- [x] **Find used accounts on request.** A network's page can scan that
  network's profiles and first account indexes when the user asks, reading
  balance and history through the `FundsScan` session, and import the
  account that has been used. The scan sends candidate addresses to a
  provider before any wallet exists, so it names the endpoint and never runs
  automatically. Funds Finder remains the cross-network search for a phrase
  whose network is unknown, reached from the picker. Both read the registry's
  profiles. Each funded result opens the single-wallet flow prefilled with
  network and path, instead of ending there. Prove ordering, refusal and
  endpoint failure reporting through loopback CLI fixtures.
- [x] **Take Monero's restore height at setup.** Today it can be set only
  on the sync screen, before the first sync. Store it with the wallet at
  import and delete the sync screen's field. A created wallet starts from a
  height core knows is earlier than its creation, not 0. Prove the stored
  height, the first scan's start and refusal of a change after the first sync
  through the CLI.
- [x] **Upgrade watched wallets and refuse other duplicates.** Nothing
  stops a second wallet with the same network and address; the only check is
  on minted ids. A signing setup is one from a phrase or a private key. When
  its derived address, or its Bitcoin account xpub, matches a watch-only
  wallet on the same network, it upgrades that wallet in place.
  Core seals the secret under the existing id, records the signing kind,
  paths and overrides, and keeps the name, history, labels and settings. The
  seal and the state change commit together or not at all. Every other match
  is refused and names the existing wallet: an address already held with
  keys, or an address watched again. In a multi-address watch, a line that is
  already held is reported among the rejected addresses, like an invalid one,
  and the other lines are imported. Record the upgrade in
  BEHAVIOUR-CHANGES.md. Through the CLI, prove that an upgraded wallet keeps
  its id and history, signs afterwards, survives reopening, and that each
  refusal holds.
- [x] **Show what the wallet will do before committing.** The final page
  summarises the chosen network's capabilities and limits from the registry,
  not from Swift copy. This covers history and token discovery, including
  whether a custom indexer is needed; staking; a single address on ZEC, BTG,
  DCR, KAS and DASH; account reserves on XRP and Stellar; Cardano's
  token-bearing inputs; Zcash being transparent-only; and testnets that need
  custom endpoints. It also names the endpoints the first refresh will
  contact, with a way to change them before the address is sent anywhere.
- [x] **Give each network's page its own options.** Once the descriptor
  exists, add network-specific fields one at a time. Core implements and
  proves each one before the descriptor advertises it. NEAR's page takes a
  named account, and XRP and Stellar show their activation reserve before
  the first receive; Polkadot and Bittensor junction paths wait on their own
  item below.
- [ ] **Offer TON's wallet contract version.** TON derives only v4R2. Implement
  W5 (wallet v5r1) beside it — address, state init, external message and
  signing — against ton-core vectors, then offer the version on TON's page as
  a derivation profile, with CLI import, send and refusal checks.

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
