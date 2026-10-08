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
  its derived address, or its account public key, matches a watch-only
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
- [x] **Offer TON's wallet contract version.** TON derived only v4R2. W5
  (wallet v5r1) now stands beside it — address, state init, external message
  and signing, checked against @ton/ton vectors — and is the default. TON's
  phrase and key imports ask for the version as a wallet version rather than
  a derivation profile: TON has no path, and the version is a fact of the
  stored address, which a send reads back with the key. Finding used
  accounts reads both versions. CLI import, send and refusal checks cover
  it.

## Wallet details

The details page shows a total, holdings and an address; everything else is
a level down under Advanced or elsewhere in the app. Staking is its own tab
with a wallet picker, and Monero's sync sits inside Send. Make the page the
wallet's home: what any wallet can do and what its network adds, each with a
short note on what it does. The first two items are the foundation; the rest
build on them one at a time and need not land together. Advertise an action
only once core owns it end to end and the CLI proves it; a watch-only wallet
lists only what needs no key. An action that signs or reveals a secret needs
independent vectors, CLI acceptance of its refusal paths and `make verify`.
Customisation stays functional: no colours, icons or other decoration.

- [x] **Derive the page's actions in core.** One per-wallet action
  descriptor, modelled on the setup descriptor, lists what the wallet offers
  from its network, signing kind and live state, with each action's note.
  The CLI prints it (`wallet actions <wallet>`) and Swift renders it
  generically; no network-specific action is decided in Swift. It starts
  with what exists: send and receive from this wallet, this wallet's history,
  rename, revealing the phrase and deletion. The notes reuse the registry's
  capability and limit summary the setup page shows: account reserves, a
  single address, transparent-only, a custom indexer. A core test keeps the
  descriptor honest, as the setup descriptor's does.
- [x] **Start actions from the wallet; delete the Staking tab.** A wallet on
  a network with a staking adapter shows its positions on its page and starts
  every staking action there. Delete the Staking tab with its network list
  and wallet picker (`StakingView`, `ChainStakingDetailView`,
  `MainAppTab.staking`), so staking has one way in: its wallet. Record the
  removal in BEHAVIOUR-CHANGES.md. Monero's sync and its restore height move
  from Send to the wallet's page.
- [x] **Add this wallet's key to another network.** A wallet stays on one
  network, but its secret often belongs on several, most of all the EVM
  networks that share one address. From the wallet's page, after Face ID and
  the wallet password, core reads the sealed secret and runs the target
  network's ordinary import with it; the secret never crosses to Swift. The
  new wallet seals its own copy and stands alone, as every setup does. Core
  lists the targets that read the same secret: a BIP-39 phrase on any
  BIP-39 network, along that network's profile and with the source's
  passphrase; a Monero or TON phrase only within its family; a key on the
  networks with its curve and key form. A watch-only wallet offers the
  networks that read its address, such as an EVM address on other EVM
  networks. Each target previews its address before anything is sealed, and
  duplicate refusal and the watch-only upgrade apply as in any setup. Prove
  through the CLI that the copy signs, that deleting either wallet leaves the
  other intact, and that a target with another phrase format or curve is
  refused.
- [x] **Upgrade a watched wallet from its page.** Core already upgrades a
  watch-only wallet in place when a signing setup derives its address, but
  the only way in is an ordinary Add Wallet. Give the watched wallet's page
  the entry: it opens its network's phrase or key method bound to that
  wallet, and the preview shows whether the input derives the watched address.
  A secret that derives another address is refused rather than added as a new
  wallet. Prove the refusal through the CLI, and that the wallet stays watched
  after it.
- [x] **Open the address in an explorer.** `core/data/explorers.toml` holds
  only transaction templates. Add an address template per network, test
  networks included, checked against each explorer's live address page, and
  link it from the page.
- [x] **Export keys in the network's own formats.** A wallet can reveal only
  its phrase. Add the private key in the encoding its network's import reads
  (WIF, Solana's base58 keypair, Stellar's `S…` seed and the others; Monero's
  spend and view keys), and what another wallet needs to watch it: Bitcoin's
  account xpub and Monero's address with its private view key. The xpub and
  the view key expose the whole history, so every export sits behind Face ID
  and the wallet password. Copy uses the phrase's local-only, expiring
  pasteboard item. Core tests round-trip each export through Spectra's import
  and through vectors from a reference wallet.
- [x] **Sign a message to prove an address.** Sign a plain-text message with
  the wallet's key in its network's scheme: BIP-322 on Bitcoin-family SegWit
  and Taproot addresses and the legacy signed message on P2PKH, EIP-191
  `personal_sign` on EVM, Solana's off-chain message, and the native forms on
  the other networks that define one. Verify a signature against an address
  as well. Refuse anything that is not a message: EIP-712 typed data, since a
  permit authorises spending, and any payload that parses as a transaction on
  the network. Prove signing, verification and refusal against reference
  vectors and through the CLI.
- [x] **Sign messages on the remaining networks.** Stellar's SEP-53,
  Cardano's CIP-8 COSE signatures, Kaspa's and Monero's own message
  signatures, each checked against the network's own SDK. The dapp-bound
  formats (NEAR's NEP-413, Aptos's AIP-62, TON's proof) are not plain
  messages: each signs a dapp's domain and nonce, and without a dapp
  connection there is nothing to supply them. They belong with connecting
  to dapps, not with proving an address.
- [x] **Hide assets from a wallet's holdings.** Discovery lists whatever an
  indexer returns, spam airdrops included. Let a wallet hide a holding, kept
  in core with the wallet and listed again under hidden assets. A hidden
  asset leaves the wallet's total and the home page's aggregate but stays
  sendable. Prove persistence across reopening and the totals through the
  CLI.
- [x] **Bitcoin family: the addresses and coins behind the balance.** On the
  networks with account discovery (BTC, BCH, BSV, LTC, DOGE and PPC), list
  owned receive and change addresses with their use and balance, the next
  unused receive address, and the unspent outputs with their confirmations.
  Peercoin also shows the minting rewards still maturing, which its total
  includes and a send cannot spend. Read-only; freezing outputs belongs to
  coin selection in [FUTURE_PLANS.md](FUTURE_PLANS.md).
- [x] **EVM: token approvals and names.** List the ERC-20 allowances the
  address has granted, from an indexer's `Approval` events confirmed by a
  live `allowance` read, and revoke one with `approve(spender, 0)` through the
  ordinary send stages. A network without an indexer says so rather than
  showing an empty list. On Ethereum, show the address's ENS primary name
  only when the name resolves forward to the same address.
- [x] **Solana: close empty token accounts.** Each token account holds rent.
  List the wallet's empty SPL and Token-2022 accounts with the SOL each
  returns, and close them in one reviewed transaction. Refuse a non-empty
  account, one whose close authority is not the wallet, and a Token-2022
  account with withheld fees. Prove the instruction bytes and the refusals
  with vectors and through the CLI.
- [x] **Tron: account resources.** Show Bandwidth and Energy, what a TRX or
  TRC-20 transfer burns without them, and whether the account is activated.
  Stake 2.0 freezing and voting is a staking adapter under
  [Staking coverage](#staking-coverage).
- [x] **XRP and Stellar: reserves and closing the account.** Show the
  reserve split into the base reserve and what the account's objects or
  subentries hold, read from the network. Offer closing the account into
  another one to recover the reserve: XRP `AccountDelete` and Stellar
  `AccountMerge`. Both are irreversible: core checks every prerequisite the
  network enforces before signing, and the review names what is given up.
  Prove the transactions with vectors and each refusal through the CLI.
- [x] **TON: contract state.** Show the wallet contract version and the
  account's state (uninitialised, active or frozen), and explain that an
  uninitialised account deploys its contract on its first send.
- [x] **NEAR: access keys.** List the account's full-access and function-call
  keys, each function-call key with its receiver and allowance, marking the
  key Spectra signs with. Delete a function-call key through the send stages.
  Full-access keys are listed only, and deleting the wallet's own key is
  refused. Show the storage the account's balance must cover.
- [x] **NEAR: refund token storage deposits.** Each NEP-141 token contract
  that has registered the account holds its NEP-145 storage deposit, usually
  0.00125 NEAR. No node lists every contract an account registered with, so
  read `storage_balance_of` on the contracts the wallet's discovery and
  history name, list those that hold a deposit while the token balance is
  zero, with what each returns, and unregister them with
  `storage_unregister` through the send stages. Refuse a non-zero token
  balance and never pass `force`, which burns the tokens. Prove the
  transaction against @near-js vectors and the refusals through the CLI.
  Registering a recipient when sending is under
  [Transfer correctness](#transfer-correctness).
- [x] **Sui: merge coin objects.** A balance spread over many `Coin<T>`
  objects costs more gas to spend. Show each type's object count and merge a
  type's objects in one reviewed transaction, keeping the SUI gas coin
  separate. Prove the transaction with vectors and through the CLI.
- [x] **Polkadot and Bittensor: what the balance holds.** Split the balance
  into free, reserved and frozen, and show the existential deposit below
  which the account is reaped. Bittensor subnet staking is a staking adapter
  under [Staking coverage](#staking-coverage).
- [x] **ICP: principal and account identifier.** The stored address is the
  default ledger account identifier. Show the principal it derives from as
  well, and which form each kind of recipient expects: the ICP ledger, ICRC
  tokens and the NNS.
- [x] **Monero: prove a payment.** Keep each outgoing transfer's tx key with
  its history row and offer, from the transaction's detail, the proof that
  monero-wallet-cli's `check_tx_key` verifies. Owned subaddresses are under
  [Wallet import and address recovery](#wallet-import-and-address-recovery).
- [x] **Cardano: stake address and rewards.** A phrase wallet holds a base
  address, so its account has a stake key at `m/1852'/1815'/{account}'/2/0`.
  Show the stake address (CIP-19 type 14, `stake1…`/`stake_test1…`) and its
  reward balance and delegation as Koios reports them for that stake
  address; a raw-key wallet has neither. Withdrawing rewards and delegating
  are a staking adapter under [Staking coverage](#staking-coverage).
- [x] **Test networks: where to get coins.** A test network's page links its
  faucet from the registry, checked for the concrete network. Dogecoin
  testnet, Decred testnet and Kaspa TN10 have none yet: re-check
  faucet.decred.org, faucet.doge.toys and faucet-tn10.kaspanet.io and add
  them to `chains.toml` and the audit once they serve their network.

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

- [x] **Fix Decred destination script construction first.**
  `decode_dcr_address` accepts both `Ds…` P2PKH and `Dc…` P2SH addresses but
  returns only the hash; `send/decred.rs` always creates a P2PKH recipient
  output. Retain the decoded network and script type and build the matching
  output, or explicitly refuse unsupported destinations before provider reads.
  Prove P2PKH/P2SH recipient scripts and network refusal with independent
  vectors and CLI inspection of the signed transaction. See
  [address decoding](../core/src/derivation/decred.rs) and
  [transfer construction](../core/src/send/decred.rs).
- [x] **Derive Cardano base addresses.** Spectra derives a CIP-19
  enterprise address from the payment key at `m/1852'/1815'/{account}'/0/0`,
  with no stake credential. Mainstream Cardano wallets derive base addresses
  that add the stake key at `m/1852'/1815'/{account}'/2/0`, so a phrase
  imported from one of them lands on a different address and that wallet's
  funds go unseen. Derive the base address for phrase wallets, read balance,
  UTXOs and history there and send change back to it; spending still needs
  only the payment key's witness. A raw-key wallet holds no stake key and
  keeps its enterprise address. The used-account search reads base
  addresses too. Prove the address against independent vectors and against
  a reference wallet's restore of the same phrase, and prove transfers
  through the CLI. Record the address change in BEHAVIOUR-CHANGES.md and
  update CHAIN-SUPPORT.md. The wallet page can then show the stake address
  and its reward balance; delegation is a staking adapter under
  [Staking coverage](#staking-coverage). See
  [derivation](../core/src/derivation/cardano.rs).
- [x] **Send XRP destination tags and Stellar memos.** XRP payments carry no
  `DestinationTag` and Stellar payments no memo, so a send to an exchange's
  shared deposit address cannot say whose deposit it is. Take a tag or memo
  in the send flow, bind it into the review digest, and refuse a send to an
  account that requires one without it: XRP's `lsfRequireDestTag` and
  Stellar's SEP-29 `config.memo_required`. Account closing refuses those
  destinations today for the same reason. Prove the encodings against the
  networks' SDKs and the refusals through the CLI.
- [x] **Register a NEAR token's recipient before sending it.** A NEP-141
  send is `ft_transfer` alone ([send_near.rs](../core/src/service/send_near.rs)),
  and a standard token contract refuses a transfer to an account it has not
  registered, after the gas is spent. Read `storage_balance_of` for the
  recipient; when it is unregistered, read `storage_balance_bounds` and put
  `storage_deposit` with `registration_only` and the minimum deposit before
  `ft_transfer` in the same reviewed transaction, the deposit shown as part
  of the cost and bound into the review digest. Refuse a contract that
  answers neither method rather than sending blind. Prove the two-action
  transaction against @near-js vectors and the registered, unregistered and
  no-NEP-145 cases through the CLI. Refunding the wallet's own deposits is
  under [Wallet details](#wallet-details).

### Asset and privacy protocols

- [ ] **Support CIP-113 programmable tokens on Cardano.** CIP-0113
  (Proposed; merged on 2026-09-29 and launched by the Cardano Foundation on
  2026-10-07) holds a programmable token at the shared `programmableLogicBase`
  script address under its owner's stake credential, which a wallet sets to
  its payment key hash. An on-chain registry keyed by policy ID marks a
  policy programmable, and a transfer spends from the script address with the
  protocol-parameters UTxO and the registry's nodes as reference inputs,
  running the global and the token's own transfer logic as zero-amount
  withdrawals; the issuer's logic can also freeze, seize or deny holders.
  Spectra reads only the wallet's own address, so these tokens are invisible
  to it today, and no plain transaction moves one. Derive the wallet's
  programmable address from its payment key hash, read balances and history
  there, identify a token by the registry, and give it its own standard,
  `CIP-113`, beside `Cardano Native Token`. Build transfers with the script
  executions, their execution units, collateral and reference inputs; refuse a
  policy the registry does not list and a transfer its logic rejects, and say
  in the review what the issuer's logic can do. Prove the address, the
  registry proofs and the transaction bytes against the reference
  implementation (`cardano-foundation/cip113-programmable-tokens`), and the
  receive, transfer, refusal and recovery paths through the CLI. The standard
  is Proposed and its reference substandards are marked for testnets: build
  against what issuers deploy on mainnet.
- [x] **Implement complete Litecoin MWEB support.** Address/key derivation,
  owned-output scanning, balances, recovery, MWEB transfers and peg-in/peg-out
  need a complete protocol adapter. The former incomplete peg-in builder was
  removed; MWEB destinations are currently refused. Verify protocol bytes and
  proofs independently and exercise receive, build/sign, recovery and refusal
  paths through the CLI before advertising support.
- [x] **Implement Zcash shielded wallets and transfers.** Only transparent
  addresses and transactions are supported today. Add the supported shielded
  address/key formats, note scanning and durable recovery, balances, proof
  generation and signing as one coherent core model. Prove recovery and
  transparent/shielded transfer boundaries with independent vectors and CLI
  acceptance; do not advertise shielded support from address validation alone.
- [x] **Support XRP and Stellar issued assets.** Both are outside the tracked
  asset interface today. Add issuer-qualified asset identity, metadata,
  balances, history and transfers, including each chain's ownership and account
  prerequisites. CLI checks must distinguish assets with the same display code
  but different issuers and prove signing and refusal paths.
- [x] **Support Cardano native assets and token-bearing ADA inputs.** The
  current ADA-only builder skips UTXOs containing native assets, so sufficient
  total ADA does not imply an ADA payment can be made. Add asset identity,
  balances, history, transfer and change handling; ADA payments using mixed
  inputs must return every unspent asset correctly. Prove asset conservation,
  output minimums, fees and signing with independent vectors and CLI checks.
- [x] **Extend Solana Token-2022 transfers.** Actual transfer fees, transfer
  hooks with a program and other extensions that change transfer semantics are
  refused. Zero-fee configurations and hooks without a program already work.
  Implement each supported extension's exact recipient amount, required
  accounts and review/signing semantics; explicitly refuse remaining unknown
  extensions. Prove both active-extension transfers and refusal paths in the
  CLI. See [mint validation](../core/src/api/solana_json_rpc.rs).
- [x] **Add EVM ERC-721/ERC-1155 NFT support.** Current asset discovery and
  transfer assembly cover fungible ERC-20-style assets, not NFTs. Add a model
  with contract and token-ID identity, ownership/quantity reads, discovery,
  history and the appropriate transfer builders. Prove that token IDs and
  quantities cannot be interpreted as fungible decimal balances.

### Wallet import and address recovery

- [x] **Add owned Monero accounts and subaddresses.** The wallet derives its
  primary address and scans without registering owned subaddresses. Add core
  account/index derivation, receive rotation and durable scan targets. Prove
  recovery of outputs sent to non-primary accounts/subaddresses and continued
  scanning after restart. Sending to a subaddress recipient is a separate,
  already parsed destination capability.
- [x] **Add Monero view-only wallet import and scanning.** There is no current
  watch-only import path for a Monero scan-key wallet. Model public spend and
  private view keys explicitly, validate the derived address before storing,
  and allow scanning without a spend key. Report any limits on spent-output
  knowledge; prove import, scan, restart and refusal to sign through the CLI.
  An address alone does not contain the keys needed for Monero scanning.
- [x] **Add account address recovery for ZEC, BTG, DCR, KAS and DASH.** Each
  currently uses one owned address per network. Add supported receive/change
  derivation, account-wide gap scanning, receive rotation and balances across
  owned sources together with signing for every discovered spendable source.
  CLI recovery must find used child addresses beyond the initial address and
  remain correct after reopening the database.
- [x] **Support Polkadot/Bittensor junction derivation.** `//hard` and `/soft`
  derivation is unsupported; only the root sr25519 derivation is exposed.
  The primitive rejects explicit paths, but ordinary chain dispatch does not
  forward the path to these adapters. First ensure every entry point refuses
  unsupported paths rather than silently deriving the root key. Then implement
  explicit path parsing and derivation against independent vectors, validate
  imported identity before storing, and prove CLI import and signing for
  supported paths.
- [x] **Add Bitcoin-family multisig and PSBT workflows.** Wallets currently
  model single-key scripts. Core must own the signing policy, cosigner/script
  identity, partial signatures, transaction review and finalization. Prove
  interoperability with independent PSBT fixtures and refuse foreign inputs,
  mismatched scripts or changed outputs before signing.
- [x] **Extend external account xpub import beyond Bitcoin.** Bitcoin mainnet
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
