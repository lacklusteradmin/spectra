# Behaviour changed on purpose

The log Rule 0 requires. [PLAN.md](PLAN.md) holds the rules and the work
still open; this file holds what has already changed and why, so a decision can
be found later without reading the plan around it.

Rule 0 licenses changing behaviour, not changing it silently. One entry per
change, newest first, and each says what it was, what it is, why that side, and
how to check it without the app:

- **Before / After** — the behaviour on each side, concretely enough to tell
  whether you are looking at the old one.
- **Why** — which rule this serves. "Rule 0" alone is not a reason; name the
  inconsistency, the dead feature, or the second model being collapsed.
- **CLI check** — the `spectra` invocation that shows it, or an honest note
  that none applies and what covers it instead.
- **Verification** — the three suites at the time of the change.

## 2026-09-23 — Toggle token discovery directly from the list

- **Before:** changing discovery required opening the token detail.
- **After:** each Known Tokens row has its discovery switch beside the detail
  link. The detail no longer repeats the switch. Failures are visible on the
  list, and the accessible switch label identifies the token and its global
  network scope. Network/source filters do not narrow the switch's effect.
- **Why:** common token management should not require entering a detail page.
- **CLI check:** `python3 scripts/cli-token-preferences.py target/debug/spectra`
  verifies the unchanged core-wide toggle. Placement is covered by the existing
  iOS real-window screen rendering check in `make verify`.

## 2026-09-23 — Known Tokens filters move into the toolbar

- **Before:** a permanent Filters section occupied the top of the token list.
- **After:** the top-right filter button opens the network/source pickers and
  Clear Filters action. Its filled icon indicates an active filter; the list
  starts with tokens. New Token remains available beside it.
- **Why:** keep filtering accessible without taking space from the token list.
- **CLI check:** none for toolbar placement; this is Swift-only presentation.
  `make verify` includes the existing real-window token screen rendering check.

## 2026-09-23 — Remove the obsolete provider lookup from core tests

- **Before:** production pricing used independent provider IDs, but test-only
  types, an extra catalog projection and a CoinGecko-keyed CoinPaprika lookup
  still reproduced the deleted inference path and documented it as current.
- **After:** those helpers and comments are deleted. Catalog checks now inspect
  token identities and each provider's own IDs directly, including uniqueness
  for both providers and CoinPaprika agreement across deployments. The pricing
  regression still proves that a blank or conflicting CoinGecko ID cannot
  determine the CoinPaprika quote. Runtime behaviour is unchanged.
- **CLI check:** `python3 scripts/cli-token-preferences.py target/debug/spectra`
  still verifies independent provider configuration; `cargo test -p spectra_core
  provider_identity_tests` checks the catalog without a provider-to-provider map.

## 2026-09-23 — Token-wide discovery and complete token management

- **Before:** tracking was per deployment, and a Swift group toggle depended on
  callers enumerating networks. New deployments could reset a saved choice.
  Known Tokens used `Contract / Mint` everywhere and exposed only CoinGecko.
  Custom tokens could only change precision; CoinPaprika quotes were inferred
  through the built-in CoinGecko mapping.
- **After:** toggling any deployment applies to every known deployment with the
  same token ID. Catalog refresh carries that choice to new networks. A custom
  token keeps its own identity: matching names, symbols or provider IDs never
  link it to another token. This enables discovery on supported wallet networks;
  it does not create wallets or invent unknown token deployments.
- **After:** the list shows names, symbols, networks and custom provenance. The
  detail has one discovery switch, read-only network deployments, and both price
  sources. The custom form adds/edits names, symbols, precision and independent,
  optional CoinGecko/CoinPaprika IDs. Network and identifier cannot be edited.
  Provider IDs are validated before storage; URLs are refused. Changing metadata
  clears that deployment's cached price. Built-ins stay read-only.
- **Why:** discovery is a token choice, not a per-network setting. Provider IDs
  identify quotes, not assets; both providers must work independently for custom
  tokens. Swift renders and forwards core's decisions.
- **CLI check:** `python3 scripts/cli-token-preferences.py target/debug/spectra`
  exercises cross-network toggles, reopening, independent custom identity,
  editing/clearing both provider IDs, invalid-ID refusal and deletion offline.
  `spectra token edit` accepts the same fields as `token add`, including
  `--coinpaprika-id`; `spectra --json token list` exposes the identities, choices
  and both provider IDs. Core tests cover new-network choice inheritance and
  CoinPaprika-only quote resolution without CoinGecko inference.

## 2026-09-22 — Resolve token history labels from deployment identity

- **Before:** Solana RPC history used the SPL mint address as its symbol, and
  normalization preserved that address as both ticker and asset name.
- **After:** history normalization resolves catalog token names and tickers by
  network and mint/contract identity. Unknown contracts retain their address;
  a matching ticker or an address on another network cannot borrow a catalog
  asset's name. Existing provider records receive corrected labels on refresh.
- **Why:** display metadata must follow the actual token deployment, and the
  same core result must serve history rows, details and searchable storage.
- **CLI check:** `python3 scripts/cli-history.py target/debug/spectra
  HistoryTests.test_solana_token_history_labels` exercises a loopback Solana RPC,
  USDC/USDT/unknown mints, persistence, refresh without duplicates and ticker
  search. Rust coverage also checks case-sensitive mints and devnet isolation.
- **Verification:** `make verify` passed (formatting, Clippy, workspace Rust
  tests, CLI acceptance including the new history regression, and iPhone
  simulator tests).

## 2026-09-22 — Endpoint history label matches the catalog

- **Before:** endpoint capability labels displayed “Native history” (and the
  equivalent native-coin qualifier in Chinese).
- **After:** the English label is “History”; simplified and traditional Chinese
  use “交易历史” and “交易歷史”.
- **Why:** the label should match the `History` capability in `endpoints.toml`.
- **CLI check:** no CLI behavior changes; inspect the `endpointCapability.history`
  values with `rg 'endpointCapability.history' resources/strings/RuntimeStrings.*.json`.
- **Verification:** `make verify` passed (formatting, Clippy, workspace Rust
  tests, CLI acceptance and iPhone simulator tests). Run outside the sandbox
  so mock servers could bind local ports and Xcode could access the simulator.

## 2026-09-22 — Durable send review and native session isolation

- **Before:** Swift composed temporary warning strings from a quote, then passed
  the quote's request back to core to build. Resuming a prepared send showed only
  amount/address/network and lost recipient, first-use and self-send warnings.
  **After:** `build_owned_send` owns quote-to-build orchestration. Every artifact
  stores typed build-time advisories included in its review digest; direct CLI
  builds derive them too. Inspect/list and reopening return the same review.
  Signing still checks freshness and never rebuilds the payload. Warnings describe
  the build-time evaluation, not a new live recipient probe on resume.
  **Why:** the transaction being reviewed and its advisories must survive together.
  **CLI check:** `scripts/cli-send.py target/debug/spectra` exercises
  `send build-owned` and persisted self-send warnings; `scripts/cli-send-stages.py
  target/debug/spectra` checks reopening, exact sub-float amounts, and rejection
  when a stored advisory is altered.
- **Before:** delayed restore/build/authentication callbacks could update another
  send composer, and completing authentication could sign after closing it.
  **After:** Swift owns one transient send session; closing, switching assets,
  returning to edits or resuming another artifact invalidates the old session.
  Artifact and endpoint projections are adopted together. Authentication must
  return to the same session before signing. Already-dispatched core operations
  remain durable and inspectable; closing does not undo a signature or broadcast.
  **Why:** page lifetime belongs to the native UI, transaction lifetime to core.
  **Check:** iOS `SendSessionTests` covers delayed endpoints, authentication,
  failure and broadcast completions. CLI stage checks above verify that durable
  artifacts remain independently inspectable without that UI session.
- **Before:** an unreachable legacy Sent page and unused review/destination
  fields coexisted with the durable stages. A state command also initiated two
  portfolio reads. **After:** node outcomes, history details and recipient-save
  actions stay on the stage page; obsolete fields/result navigation are removed.
  Awaited state commands and startup adopt state without spawning a second read.
  **Why:** one visible send flow and one refresh path avoid contradictory state.
  **Check:** `make verify`; native stage rendering and session tests, existing
  state/projection tests, and CLI send/owned-state acceptance.
- Amount shortening remains an optional shared display utility. Send review uses
  the exact decimal string from the durable artifact, never the six-significant-
  digit balance formatter; fee/payload details remain inspectable. The CLI stage
  check proves exact 18-decimal amounts, and the native rendering test uses
  `1.000000000000000001`.
- **Verification:** Final `make verify` passed: formatting, Clippy, 827 Rust
  unit tests plus 1 integration test, 443 CLI acceptance checks, and 117 iPhone
  simulator tests. The native stage screenshot was also inspected.

## 2026-09-22 — Persistent, separately actionable send stages

- **Before:** the app's review expired in memory after 120 seconds and its next
  action signed and submitted. Only Bitcoin and EVM exposed sign-only paths.
- **After:** the app builds a persistent, secret-free typed artifact, signs the
  reviewed fingerprint with a separate user action, then submits the saved
  payload only to explicitly selected configured endpoints. The CLI exposes
  `send build`, `send inspect`, `send list`, `send sign --review-digest`,
  `send broadcast-signed --endpoint ... --yes`, and `send configured-endpoints`.
  Closing the composer does not discard a prepared or signed transaction;
  the send screen can reopen core's artifacts. Signing does not submit.
- **After:** signing checks nonce/sequence, selected inputs/objects, and protocol
  expiry before storing signed content. SQLite compare-and-swap and input
  reservations prevent conflicting concurrent signing. Retries use the same
  signed bytes and retain a separate result per selected endpoint. A lost or
  malformed response remains uncertain. Node acceptance is separate from the
  existing on-chain history status. The history rebroadcast action reuses the
  artifact's last explicit endpoint selection.
- **After:** staged preparation refuses Monero (its current wallet-RPC builder
  signs during construction), ICP (local verification of construction payload
  content is missing), and Zcash (current consensus-upgrade validation is
  missing), with explicit reasons owned by `registry::Chain`. These are
  deliberately disabled send capabilities under Rule 0; re-enabling them requires
  a locally verifiable, independently reviewable build/sign implementation.
  Custom broadcast endpoints currently require a verifiable EVM or Aptos
  network identity; other adapters require matching network/API/broadcast
  capability in the core catalog. No unselected fallback is used.
- **Why:** the data inspected by a user must be the data signed; signing is not
  permission to submit, and uncertain submission is not permission to create a
  replacement transaction. Private keys and passwords never enter artifacts.
  The new Stellar signer uses its registry-owned network passphrase, including
  testnet. Fixed-fee UTXO review includes dust absorbed into the actual fee.
- **CLI check:** `python3 scripts/cli-send-stages.py target/debug/spectra` uses
  only loopback nodes and separate CLI processes. It covers persistence,
  independent build/sign/broadcast, competing nonce reservations, stale and
  altered content, incompatible destinations, partial success, and identical
  payload retries. `spectra --json send configured-endpoints ethereum` is an
  offline view of the configured endpoint table.
- **After:** `execute_send` now composes the same Build/Sign/Broadcast operations.
  Dead combined protocol wrappers and the former parameter/submission/result
  dispatchers are removed. Artifacts expose the exact native symbol or token
  contract identity. Signing and broadcast outcomes are durable across restart;
  closing the UI only discards its projection. Expiry is checked again before
  submission, and a confirmed history record cannot be reset by rebroadcasting.
- **After:** an explicit EVM same-nonce replacement must increase both fee caps
  by at least 10% (and at least one wei). Core atomically transfers the nonce
  reservation while retaining both signed artifacts. Ordinary competing sign
  attempts cannot overwrite the reservation. Wallet deletion removes artifacts
  and reservations with the wallet's other owned data.
- **Coverage:** core audits exercise stored-wallet Build/Sign for Solana native,
  SPL and Token-2022, Sui, Aptos and Tron native/TRC-20, including signature
  verification and wire submission adapters. Unregistered non-EVM/non-Aptos
  mock endpoints are refused by the public broadcast operation. The CLI suite
  covers exact native/token amounts beyond floating-point precision, metadata
  failure/mismatch, simultaneous processes competing for a nonce, response loss,
  explicit fee-bumped replacement, and artifact cleanup on wallet deletion.
- **Verification:** `make verify` passed. The final host rerun (`make lint test`)
  passes with 822 core unit tests and the workspace integration test; CLI
  acceptance passes all 441 checks. The expanded staged CLI fixture also passes
  independently. All 112 iOS tests pass after replacing the blank offscreen
  renderer with a real-window snapshot and an assertion rejecting empty images.
  The exported send-stage screenshot was inspected: asset/network, addresses,
  separate stage status, inspectable payload disclosures and explicit endpoint
  selection are visible. UniFFI bindings were regenerated by the build script.

## 2026-09-21 — Complete the Swift shell boundary review

### Committed state versions replace caller request epochs

- **Before:** Swift assigned request epochs; a failed later request could suppress
  a successful earlier response, and command responses had no core state version.
- **After:** core assigns a session-local revision only when publishing committed
  resident state. No-op commands and failed writes retain the version. All state
  responses carry it, including portfolio snapshots; Swift rejects older state
  without advancing anything on failure. Network-selection intents use the same
  awaitable command lane as wallet edits. Versions are not persisted.
- **Why:** the owner knows commit order; caller launch/completion order does not.
- **CLI check:** `spectra currency EUR` followed by `spectra currency` checks
  persistence. Concurrent response ordering needs a shared session: Rust test
  `committed_versions_order_reads_and_failed_writes_do_not_advance_them` and the
  Swift stale-response tests exercise it, including a failed write.

### Watch-only inputs name chains, not storage slots

- **Before:** clients grouped watch addresses by internal address slots, merging
  EVM chains before core received the input.
- **After:** `WalletImportWatchOnlyEntries.byChainId` preserves each chain's
  identity. Core validates its addresses and constructs storage slots. A family
  id uses core's selected network; a concrete testnet id stays that network.
  Unknown/unsupported chain inputs are rejected with the refused addresses.
- **Why:** storage layout and shared EVM slots are core implementation details.
- **CLI check:** `spectra wallet watch --chain arbitrum --address
  0x742d35cc6634c0532925a3b844bc454e4438f44e`; repeat with `--chain ethereum`.
  The import acceptance suite covers network validation; Rust test
  `watch_only_chain_identity_is_not_an_evm_storage_slot` checks distinct inputs.

### Transaction actions come from execution checks

- **Before:** Swift reconstructed recheck/rebroadcast eligibility from kind,
  status, registry fields and nonempty payload strings.
- **After:** history pages, transaction snapshots and record reads include
  core-derived availability reasons. The same checks run again before execution;
  reasons are never persisted or accepted as authorization. Malformed hashes or
  incompatible payload formats no longer offer actions. Confirmed UTXO records
  may be rechecked, matching core's existing reorg-handling operation; confirmed
  sends still cannot be rebroadcast.
- **Why:** a UI projection must not define a second version of execution rules.
- **CLI check:** `spectra txs --record ID --json` and `spectra txs --page --json`
  expose `actions`. `python3 scripts/cli-history.py target/debug/spectra
  HistoryTests.test_action_projection_and_unix_timestamp` tests the decision
  matrix offline, and the recheck test covers confirmation/reorg behavior.

### One Unix timestamp throughout transaction storage and display

- **Before:** payload `createdAt` used the Swift 2001 epoch while indexes and
  provider records used Unix time, requiring silent floating-point conversions.
- **After:** payload/FFI `createdAtUnix`, indexes and provider records all use
  Unix seconds, preserving fractional seconds. Swift constructs dates with
  `timeIntervalSince1970`. This directly changes the prelaunch storage shape;
  there is no legacy decoder or migration.
- **Why:** one cross-platform meaning eliminates 31-year interpretation errors.
- **CLI check:** the history action/timestamp test above reopens the database
  and checks exact `1700000000.125` in record and summary output. The full
  history acceptance suite checks sorting, paging and provider updates.

### Static currency metadata and native maintenance lifetime

- **Before:** currency-code reads called FFI for a full formatting record;
  each AppState also cached those same rules. The maintenance loop strengthened
  `self` before its infinite loop, retaining AppState throughout every sleep.
- **After:** one immutable currency catalog supplies identities and display
  metadata, with no per-AppState rule cache or per-render currency FFI call.
  Shared significant-digit/dust rules remain in core because CLI also uses
  them; locale formatting and formatter caches stay native. Maintenance owns
  AppState only while executing a tick, releasing it before sleep.
- **Why:** remove incidental bridging and retain native lifecycle ownership.
- **CLI check:** existing `spectra token format 0.00042 --chain Bitcoin` acceptance exercises
  shared amount rules. Native lifetime has no CLI equivalent: Swift test
  `testMaintenanceSleepDoesNotKeepAppStateAlive` verifies release during sleep;
  `testFiatCatalogSuppliesStableIdentityAndDisplayMetadata` verifies the catalog.

## 2026-09-21 — Remove authenticated Maestro and TronScan endpoints

- **Before:** The endpoint directory included Maestro's Bitcoin Esplora API in
  Bitcoin's default fallback list and TronScan's mainnet `accountv2` API as a
  catalog entry (not selected by the default transport).
- **After:** Both entries are removed. Bitcoin retains Blockstream, Mempool and
  Mempool Emzy. TronGrid and the separately configured TronScan history URL
  remain unchanged.
- **Why:** Maestro documents a required project API key and its hostname failed
  DNS resolution during the audit. TronScan `accountv2` returned HTTP 401 to an
  anonymous read. Neither belongs in the app's keyless endpoint directory.
- **CLI check:** `spectra --json endpoints --catalog --chain bitcoin` and
  `spectra --json endpoints --catalog --chain tron` omit `gomaestro-api.org`
  and `apilist.tronscanapi.com/api/accountv2`, respectively, from both catalog
  entries and configured transports.
- **Verification:** `make verify` passed (formatting, Clippy, workspace Rust
  tests, CLI acceptance and iPhone simulator tests). The catalog commands above
  also confirmed both URLs are absent. Tests were run outside the filesystem
  sandbox so local mock servers could bind ports and Xcode could use the simulator.

## 2026-09-22 — Local staged sends for ICP, Zcash and Monero

- **Before:** These three chains refused new staged sends. The older ICP
  derivation used a non-ledger address digest; Zcash used a fixed NU5 branch and
  an incorrect ZIP-244 signature preimage; Monero assumed a remote wallet RPC.
- **After:** ICP derives CRC32/SHA224 ledger account identifiers from the DER
  Ed25519 self-authenticating principal. It constructs protobuf ledger arguments,
  signs IC call/read-state envelopes and computes the ledger hash locally.
  Rosetta supplies network/fee metadata and submits the already signed envelope.
  Sealed prelaunch ICP wallets made with the old derivation must be reimported;
  invalid old addresses are refused, with no compatibility decoder.
- **After:** Zcash transparent P2PKH inputs and P2PKH/P2SH destinations use local
  ZIP-244 signing and txids. Genesis, tip/next consensus branches, unspent inputs,
  ZIP-317 fees and expiry are checked. Known mainnet/testnet upgrades through
  NU6.2 are registered; unsupported future branches refuse. Shielded sends are
  not implemented. Testnet P2SH validation is corrected. The preview fee floor
  is 10,000 zatoshis. Blockbook broadcast now parses its JSON `result` txid.
- **After:** Monero uses on-device `monero-wallet` scanning, decoy selection and
  CLSAG/Bulletproof+ signing. Daemon endpoints receive public chain queries and
  explicitly submitted signed bytes, never spend/view keys. Core persists a network-scoped
  AES-GCM encrypted scan cache and frozen signing plan; the local view key lives
  in SecretStore and wallet deletion removes it. Signed artifacts reserve key
  images. Restart resumes scan progress; reorgs restart from the restore height.
  CLI `send sync-monero` and the iOS send screen authorize bounded scan batches,
  starting at zero unless a restore height is explicitly supplied. Local unlocked
  balance and scanned history replace remote wallet-RPC reads. Non-RingCT legacy
  outputs and subaddress discovery are outside this first local engine; HF16 is
  required and unknown hard forks refuse. Sync requires local key authorization.
- **Why:** Reviewable construction and signing must be independent of a server's
  wallet. Correct ledger identities, protocol digests and input ownership take
  precedence over the prelaunch implementation's behavior.
- **CLI checks:** `python3 scripts/cli-send-icp-zcash.py target/debug/spectra`
  checks local build/sign, immutable retries, restart, network/fee/expiry/spent
  input refusals. `spectra send sync-monero --from NAME [--restore-height H]
  [--once]`, then `send build`, `send sign` and `send broadcast-signed` exercise
  the same local Monero flow. `python3 scripts/cli-monero-regtest.py --monerod /path/to/monerod` starts an
  offline node and checks submission/reservations. Real monerod v0.18.5.1 accepted a locally
  signed regtest transaction; the public fixture and offline regression live
  under `core/tests/fixtures/monero-local.*`. ICP ledger-hash and Zcash signature
  tests additionally use independent official reference vectors.

- **Verification:** Final `make verify` passed: formatting, Clippy with warnings
  denied, 827 core tests plus the transport integration test, 443 CLI checks
  and 113 iPhone simulator tests. `cli-monero-regtest.py` also passed against
  official monerod. The default Monero daemon responded as synchronized mainnet;
  Zcash's default Trezor endpoint still returned HTTP 403 and remains the
  separately tracked external dependency. No real funds were sent.

## 2026-09-23 — Put maintenance actions beside chain diagnostics

- **Before:** Advanced listed a refresh button for every mainnet, all-endpoint
  checks and diagnostics bundle tools. Each chain's diagnostics split actions
  between the top of the page and a separate lower Chain Actions section.
- **After:** Each chain's top Actions section includes balance/history refresh,
  history diagnostics, endpoint checks, self-tests and supported rescans. Labels
  omit the chain name already shown in the network-aware page title. Diagnostics
  overview owns all-endpoint checks and bundle import/export, sharing and past
  exports. Advanced retains security, global refresh and global status.
- **Why:** Keep chain operations beside their results and prevent Advanced from
  growing with the chain catalog. Existing core operations and scope are reused.
- **CLI check:** Run `scripts/cli-acceptance.sh` for offline refresh refusal and
  diagnostics coverage, or `spectra diagnostics self-test --chain Bitcoin` and
  `spectra diagnostics show --chain Bitcoin`. Navigation is native-only; inspect
  Advanced, Diagnostics overview and a chain diagnostics page in the simulator.
- **Verification:** `make verify` passed: Rust formatting/Clippy and workspace
  tests, all 444 offline CLI checks, and iPhone simulator tests. Removed the
  nine obsolete localization keys identified by the unused-string gate.
