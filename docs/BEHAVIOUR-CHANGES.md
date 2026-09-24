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

## 2026-09-23 — Chains are named by id across the boundary

- **Before:** wallets, holdings, transactions, address-book entries, token
  commands, fee-priority settings, diagnostics, degraded banners, wiki places,
  funds-finder candidates and endpoint probes carried a chain's display name
  (`"Bitcoin Testnet4"`), and Swift turned it back into a `Chain` with
  `Chain(displayName:)`. `WalletState` stored the name twice (`chain_name` and
  the id), `WalletView` carried a `family_name`, and the CLI's `--json` output
  printed names under `"chain"`.
- **After:** every record and command carries `chain_id`. A display name is
  derived only where text is drawn (`Chain::display_name_for_id` in core,
  `Chain.displayName(forId:)` in Swift). A wallet's family is
  `WalletState::family()`, the mainnet counterpart of its network.
  `Chain(displayName:)`, `CoreTokenHostingChain`, `evm_seed_derivation_chain`
  and the unused identity fields (`token_hosting_chain`, `send_execution_shape`,
  `rpc_health_method`, `pending_status_poll`, `seed_derivation_chain`) are
  gone; `Chain::hosts_tokens()` replaces the hosting-chain list. The wallet
  detail screen shows the address for the network the wallet is on; it showed
  the mainnet slot, so a testnet wallet displayed its mainnet address.
- **Why:** a display name is presentation, and it was also the key. Renaming a
  chain or translating it would have broken identity, and every front end kept
  its own name-to-chain parse.
- **CLI check:** `spectra --json portfolio --stored` shows `"chainId":"bitcoin"`
  and no `"chainName"`; `spectra --json wallet show <wallet>` lists network ids
  such as `"bitcoin-signet"`; `spectra --json settings` keys fee priorities by id.
- **Verification:** the `make verify` gates, run individually: rustfmt and Clippy with
  `-D warnings` clean, 841 core tests plus the transport test, 443 CLI
  acceptance checks, and 132 iOS tests on an iPhone 17 Pro simulator
  (`build-for-testing`, then `test-without-building`). The
  `unreachable-exports`, `uncalled-core-fns` and `unused-strings` checks report
  none.

## 2026-09-23 — Wallet signing material is one typed value

- **Before:** `WalletState` stored `is_watch_only`, and Swift asked the Keychain
  (`wallet_secret_state`) on the render path whether a wallet had a seed, a
  private key and a password. `CoreWalletRustSecretMaterialDescriptor` and
  `SecretMaterialDescriptor` described the same thing and were read by nothing.
- **After:** `WalletState.signing` is `WalletSigning` — `watchOnly`,
  `seedPhrase { passwordProtected }` or `privateKey { passwordProtected }` —
  written by core at import. Revealing a phrase is
  `reveal_seed_phrase(wallet_id, password)`, which answers a `SeedPhraseReveal`
  (`phrase`, `notStored`, `passwordRequired`, `incorrectPassword`,
  `passwordNotRequired`). The descriptors, `wallet_secret_state` and
  `wallet_seed_phrase` are gone.
- **Why:** whether a wallet can sign is domain state core decides at import;
  reading it from the Keychain per frame duplicated that and cost a secure
  store read per row.
- **CLI check:** `spectra --json wallet show <wallet>` shows
  `"signing":{"kind":"seedPhrase","passwordProtected":false}` for a seed import
  without a password; `spectra --json wallet export <wallet> --yes` prints the
  phrase through `reveal_seed_phrase` and names the typed reason when it cannot.
- **Verification:** the `make verify` gates, run individually: rustfmt and Clippy with
  `-D warnings` clean, 841 core tests plus the transport test, 443 CLI
  acceptance checks, and 132 iOS tests on an iPhone 17 Pro simulator
  (`build-for-testing`, then `test-without-building`). The
  `unreachable-exports`, `uncalled-core-fns` and `unused-strings` checks report
  none.

## 2026-09-23 — Amounts are exact decimals and display cuts, never rounds up

- **Before:** holdings, transaction amounts, receipt and confirmed fees,
  preview details and send affordability were `f64`. Swift summed holdings for
  a dashboard row, multiplied amount by price for every fiat figure, parsed the
  send field with `Double(...)`, and rounded compact amounts to nearest, so a
  balance of `0.999999999` BTC read `1`. `send_amount_shortcut` took a float
  balance.
- **After:** amounts are canonical decimal strings (`core/src/decimal.rs`).
  Core sums (`CoreDashboardAssetGroup.total_amount`), compares affordability
  exactly, and `format_asset_amount` truncates to the displayed places with a
  `below_threshold` flag. `is_valid_amount_input` replaces `parse_amount_input`.
  `send_amount_shortcut` takes the exact balance; fee-adjusted preview maxima
  stay on the float path with one ULP reserved. Price-alert targets are typed
  text that core parses. `price_usd` on holdings and `fee_priority_raw` on
  records are removed.
- **Why:** money arithmetic in Swift was a second model of the same numbers,
  and a float that rounds up shows spendable funds that do not exist.
- **CLI check:** `spectra --json send shortcut --maximum 0.123456789 --decimals 8
  --percentage 10` prints `"amount":"0.01234567"`; `spectra --json token format
  1234.5678 --chain Ethereum` shows `"1234.56"`; `spectra alert add --chain
  bitcoin --target 0` is refused by core.
- **Verification:** the `make verify` gates, run individually: rustfmt and Clippy with
  `-D warnings` clean, 841 core tests plus the transport test, 443 CLI
  acceptance checks, and 132 iOS tests on an iPhone 17 Pro simulator
  (`build-for-testing`, then `test-without-building`). The
  `unreachable-exports`, `uncalled-core-fns` and `unused-strings` checks report
  none.

## 2026-09-23 — Core values everything in the display currency

- **Before:** Swift held `livePrices` and `fiatRatesFromUSD`, converted USD
  figures itself, and priced holdings by symbol. Price-alert and
  large-movement notifications printed USD figures with the selected
  currency's symbol. The send screens computed the fee's and amount's value
  from their own copies.
- **After:** `PortfolioValuation` carries per-holding values, unit prices and
  alert targets in the display currency; dashboard groups carry `total_value`
  and `price`; `OwnedSendPreview` carries `network_fee`, `network_fee_value`,
  `amount_value` and the `amount` it quoted, and the review screen shows a
  value only for the amount on screen. Notifications carry their `currency`.
  `refresh_app` returns the price-alert and movement notifications it
  evaluated, so Swift no longer calls `evaluate_price_alerts` or
  `evaluate_portfolio_movement` separately. `unpriced_chain_ids` and
  `refresh_owned_prices` wrappers are gone; a testnet holding has no value.
- **Why:** one valuation, in core. The duplicated conversions disagreed on
  rounding and currency.
- **CLI check:** `spectra --json portfolio --stored` reports dashboard groups
  with `totalAmount`, `totalValue` and `price` in the selected currency;
  `python3 scripts/cli-portfolio.py target/debug/spectra` covers alert targets
  and totals.
- **Verification:** the `make verify` gates, run individually: rustfmt and Clippy with
  `-D warnings` clean, 841 core tests plus the transport test, 443 CLI
  acceptance checks, and 132 iOS tests on an iPhone 17 Pro simulator
  (`build-for-testing`, then `test-without-building`). The
  `unreachable-exports`, `uncalled-core-fns` and `unused-strings` checks report
  none.

## 2026-09-23 — The send composer holds one quote

- **Before:** `SendPreviewStore` kept a slot per chain name, a
  `preparingChains` set, and a sixteen-arm switch reading each preview's fee;
  the self-send check had two implementations — review folded case and ignored
  a `pending` confirmation it was always given as `None`, preflight compared
  normalized addresses.
- **After:** the store holds the latest `OwnedSendPreview`, valid only for the
  wallet, holding and network it names. `is_own_address` is the one ownership
  check, compared in the chain's normal form (all-caps bech32 is lowercased by
  `normalize_address`). `self_send_confirmation` and its request, plan and
  pending records are removed; `spectra send self-check` reports
  `{"ownAddress": bool}`.
- **Why:** per-chain slots modelled several concurrent quotes the composer
  never has, and two ownership rules could disagree.
- **CLI check:** `spectra --json send self-check --wallet W --holding
  ethereum:native --destination <own address>` prints `"ownAddress":true`.
- **Verification:** the `make verify` gates, run individually: rustfmt and Clippy with
  `-D warnings` clean, 841 core tests plus the transport test, 443 CLI
  acceptance checks, and 132 iOS tests on an iPhone 17 Pro simulator
  (`build-for-testing`, then `test-without-building`). The
  `unreachable-exports`, `uncalled-core-fns` and `unused-strings` checks report
  none.

## 2026-09-23 — Typed failure and degradation reasons

- **Before:** a failed transaction stored an English `failureReason` string
  (`FAILURE_REASON_STUCK` among them) and a degraded chain stored an English
  detail that Swift matched against known sentences to translate.
- **After:** `TransactionFailure` (`stuckAfterRetries`,
  `submissionOutcomeUnknown`, `rebroadcastOutcomeUnknown`,
  `reported { message }`) and `ChainDegradation` (`historyRefreshFailed`,
  `historyPartiallyLoaded`, `failed { message }`). Swift localizes by case.
  `core/src/diagnostics/degraded.rs` and its sentence table are deleted.
- **Why:** matching on English text was a second, lossy encoding of a closed
  set of reasons.
- **CLI check:** `spectra --json txs --record <id>` prints
  `"failureReason":{"kind":"submissionOutcomeUnknown"}` for an unconfirmed
  broadcast; `spectra --json diagnostics state --command
  '{"Degraded":{"chain_id":"solana","reason":{"kind":"historyRefreshFailed"}}}'`
  stores a typed reason.
- **Verification:** the `make verify` gates, run individually: rustfmt and Clippy with
  `-D warnings` clean, 841 core tests plus the transport test, 443 CLI
  acceptance checks, and 132 iOS tests on an iPhone 17 Pro simulator
  (`build-for-testing`, then `test-without-building`). The
  `unreachable-exports`, `uncalled-core-fns` and `unused-strings` checks report
  none.

## 2026-09-23 — Address book ids and duplicates are core's

- **Before:** Swift minted each contact's UUID and rejected a duplicate
  case-insensitively before core saw it; core compared case-insensitively too,
  which would merge two distinct base58 addresses.
- **After:** core mints the id (`AddressBookEntryAdded` names it) and a
  duplicate is exact equality of normalized addresses, where normalization
  lowercases an all-caps bech32 or CashAddr address.
- **Why:** identity and acceptance are core decisions; case-folding is only
  right for encodings that are case-insensitive.
- **CLI check:** `spectra address book add` of an all-caps bech32 address after
  its lowercase form is refused as a duplicate; `cargo test -p spectra_core
  store::tests::address_book` covers base58 addresses that differ only in case.
- **Verification:** the `make verify` gates, run individually: rustfmt and Clippy with
  `-D warnings` clean, 841 core tests plus the transport test, 443 CLI
  acceptance checks, and 132 iOS tests on an iPhone 17 Pro simulator
  (`build-for-testing`, then `test-without-building`). The
  `unreachable-exports`, `uncalled-core-fns` and `unused-strings` checks report
  none.

## 2026-09-23 — Smaller presentation fixes found while shrinking the shell

- **Before:** a holding's colour and a pin option's colour came from its
  ticker; the transaction detail showed the raw signed payload only for hex;
  self-test and rescan log categories were per-chain strings; the detail sheet
  worked out which side of a transfer was the user's.
- **After:** colour and artwork follow the deployment id; the raw payload is
  shown for any format; categories are "Self-Tests" and "Rescan" with the chain
  in the log's `chainId`; `transaction_endpoints(id)` returns each side and
  whether it is the user's. Unread projections and exports are removed:
  `resolved_addresses_by_wallet_id`, `merge_built_in_token_preferences`,
  `endpoint_tag`, `WalletHoldingRef`, `GroupedPortfolioHolding`.
- **Why:** a custom token calling itself `ETH` is not Ether, and an export or
  projection nobody reads is a second copy going stale.
- **CLI check:** `spectra --json txs --endpoints <id>` shows `from`/`to` with
  `isMine`.
- **Verification:** the `make verify` gates, run individually: rustfmt and Clippy with
  `-D warnings` clean, 841 core tests plus the transport test, 443 CLI
  acceptance checks, and 132 iOS tests on an iPhone 17 Pro simulator
  (`build-for-testing`, then `test-without-building`). The
  `unreachable-exports`, `uncalled-core-fns` and `unused-strings` checks report
  none.

## 2026-09-23 — Remove failed providers and probe actual endpoint reads

- **Before:** the directory retained 14 failed provider records, Tron PublicNode
  used the wrong path, and health checks could test a different host or only an
  EVM chain ID. Monero and Blockstream suffered false failures. The default CLI
  sweep omitted testnets and swallowed probe errors. EVM history metadata named
  sources outside the directory used by actual reads, including unsupported
  Berachain Routescan, and testnets inherited mainnet metadata.
- **After:** remove Blockchair BSV's three records, SoChain LTC, BlockCypher DOGE
  testnet, TronGrid `.pro`/`.network`, HappyStaking Koios, Monero stagenet Exan,
  Cloudflare Ethereum, OnFinality Hyperliquid, and Trezor ZEC/BTG/DASH. Tron
  PublicNode uses `/jsonrpc`. Networks without providers have empty configured
  lists and explicit CLI diagnostics; supported custom APIs remain addable even
  without a built-in provider. Verified EVM history sources are in the shared
  directory, with only implemented history capabilities; Berachain has none.
  Routescan requests use its actual `/etherscan` path, and testnets never inherit
  mainnet history. Read probes validate JSON/API responses, EVM identity plus
  block/balance reads, history errors, and the Monero/Esplora protocol paths.
  All built-in APIs have probes; browser links are explicitly excluded. The
  default CLI checks every concrete network and propagates internal errors.
- **Why:** an unavailable feature is preferable to a known-broken fallback or a
  misleading green health indicator. Routing, metadata and diagnostics must use
  one directory, and removal must not prohibit a valid user-provided replacement.
- **CLI check:** `spectra --json endpoints`; `spectra --json endpoints --chain
  zcash` reports no configured API; `spectra --json send configured-endpoints
  dash` returns an empty list. `python3 scripts/cli-endpoints.py target/debug/spectra`
  checks empty defaults, custom replacements, testnet coverage and a loopback
  node whose chain ID succeeds but block reads fail. `spectra --json endpoints
  --catalog --chain base` includes the same history source used by requests.
- **Verification:** the `make verify` Rust/CLI gates passed: rustfmt, Clippy
  with `-D warnings`, 844 core tests plus the transport test, and 442 CLI checks.
  After correcting the corresponding old iOS capability assertions, the full
  `make test-ios` rerun passed all 133 tests, including endpoint screen rendering
  and Ethereum testnet contexts. The updated CLI live sweep covered 78 networks
  at 2026-09-23 15:46 UTC: all 111 remaining API records passed their read checks,
  with zero unreachable or unchecked APIs. This does not claim broadcast success.
  Verification used temporary stores; no transaction was broadcast.

## 2026-09-23 — Finish native shell ownership and presentation boundaries

- **Before:** dismissing a composer discarded successful broadcast completions
  before application projections and post-send refresh ran. Live Activities and
  resumed send details treated the recent/pending summary as complete history.
  One refresh reread history twice and alert adoption queued another portfolio
  read. Row artwork serialized holdings into Rust on each render, and number
  presentation required the whole AppState.
- **After:** successful broadcasts return their committed result even after the
  originating form closes; application handling uses that result's transaction
  ID without replacing a newer form. Activities and send details resolve stored
  IDs directly; read failures preserve activities for retry. Refresh evaluates
  alerts/movement first and then adopts each final projection once. Native
  presentation receives explicit projection values, owns its formatter cache,
  and uses cached core-derived identities and immutable catalog artwork. The
  redundant whole-holding artwork export is removed. Exact signing review is
  unchanged and still renders the durable artifact's amount.
- **Why:** transient UI lifetime must not discard a committed operation. A bounded
  summary is not a database. Rendering should neither require application services
  nor repeatedly serialize domain records merely to look up static metadata.
- **CLI check:** `python3 scripts/cli-history.py target/debug/spectra
  HistoryTests.test_stored_pages` proves an old confirmed record absent from the
  50-row summary remains queryable after reopening. `spectra --json token artwork
  --deployment-id base:native` checks identity-based artwork. Existing staged-send
  CLI coverage proves durable broadcast results and immutable-payload retries.
  Native dismissal, ActivityKit reconciliation and formatter ownership have no
  CLI lifecycle equivalent; SendSessionTests, ShellBoundaryTests,
  AmountPresentationTests and CoinBadgeArtworkTests cover those boundaries.
- **Verification:** `make verify` passed: rustfmt, Clippy with `-D warnings`,
  838 core tests plus the transport test, 442 CLI acceptance checks and 133 iOS
  simulator tests, including the real-window signed-send rendering check. Xcode
  regenerated UniFFI bindings. Design-token, project-file and diff checks passed;
  the callable FFI surface is 138 with zero unreachable export candidates.
  Local mock servers required running verification outside the network sandbox.

## 2026-09-23 — Show pinned assets first in the pin picker

- **Before:** Pinned Assets mixed pinned and unpinned assets in symbol order,
  making existing pins hard to find when turning them off.
- **After:** core returns pinned assets first, then unpinned assets; each group
  remains ordered by symbol and token identity. The existing Swift picker and
  its search results use this order, refreshed after pin, unpin and reset.
- **Why:** managing existing pins should not require searching the full catalog.
  Keeping the ordering in the shared projection gives CLI and app the same list.
- **CLI check:** `spectra --json portfolio --pin-options`; automated coverage:
  `python3 scripts/cli-portfolio.py target/debug/spectra
  PortfolioTests.test_pin_options_put_pinned_assets_first` covers defaults,
  selected pins, unpinning, reopening, no pins and reset.
- **Verification:** the focused CLI regression, 442 CLI acceptance checks and
  123 iOS simulator tests passed. Clippy (`-D warnings`), rustfmt and scoped diff
  checks passed. Full Rust tests ran with 832 passing and two failures in
  `service::history_derived` (`derived_views_use_the_rows_unix_timestamp` and
  `the_store_answers_and_a_reopened_service_answers_the_same`) while separate
  history changes were in progress in the shared workspace. No interactive
  visual check was performed.

## 2026-09-23 — Stop DOGE polling at confirmation

- **Before:** DOGE alone automatically re-polled confirmed sends up to a shared
  12-confirmation threshold, with a five-minute interval and a finality event.
  Its in-memory stop flag was lost on restart, so old confirmed sends became
  eligible again and their cumulative confirmation counts were refreshed.
- **After:** automatic status maintenance selects only pending transactions on
  every chain. DOGE stops at the first reported confirmation, including after
  reopening storage. The depth flag, threshold, confirmed polling interval and
  finality event/binding/localizations are removed. Provider-reported counts
  remain optional metadata on status reads for UTXO chains; they do not control
  polling. Explicit Recheck still works for eligible failed/confirmed records
  and resumes pending polling if the provider reports a reorg. Unresolved reads
  schedule another pending check instead of being treated as complete.
- **Why:** a display-driven DOGE exception is not a cross-chain finality policy.
  Persisted transaction status already determines whether maintenance is needed;
  a confirmation count must not imply guaranteed finality.
- **CLI check:** `python3 scripts/cli-history.py target/debug/spectra
  HistoryTests.test_confirmed_doge_never_needs_automatic_polling` seeds counts of
  1, 12 and 100001, checks `txs --maintenance` and `txs --refresh-pending` in new
  processes without network access, retains Recheck availability and verifies
  pending records remain eligible. Core mock-node tests cover first confirmation,
  restart, manual recheck and reorg recovery.
- **Verification:** `make verify` passed: rustfmt, Clippy with `-D warnings`,
  834 core tests plus the transport test, 442 CLI acceptance checks and 122 iOS
  simulator tests. The Xcode build regenerated the Swift bindings. Design-token,
  runtime-catalog JSON and diff checks also passed.

## 2026-09-23 — Keep transaction history rows focused on status

- **Before:** history rows displayed the stored confirmation count without a
  limit and a standalone Recheck button for every eligible transaction, even
  though Recheck was also available in the row's context menu.
- **After:** history rows omit confirmation counts and the standalone button.
  The status badge, confirmation count in transaction details and core-authorized
  Recheck in the context menu remain available on all supported chains.
- **Why:** a cumulative confirmation count adds noise to the history summary;
  occasional status recovery does not warrant a repeated primary-sized control.
- **CLI check:** no CLI behavior changes; this is presentation only. Inspect
  `spectra txs --record ID --json` for the retained confirmation count and
  `actions`. Swift parsing and the design-token check cover the edited UI;
  inspect History and its long-press menu in the app for the visual change.
- **Initial DOGE assessment (implemented in the entry above):** retire the DOGE-only
  post-confirmation polling policy in a separate core change. The registry
  explicitly motivates it by displaying confirmation depth. It marks a send
  confirmed on its first inclusion, then polls every 300 seconds until the
  shared threshold of 12, producing a finality log. It can notice a reorg during
  that interval, but is not a consistent cross-chain finality policy. Its stop
  tracker is in memory; a new service queries confirmed DOGE sends again and
  can store a much larger current count. Keep pending-transaction polling,
  explicit status rechecks and observed confirmation data. If confirmation-depth
  monitoring is a product requirement, give it an explicit per-chain policy
  and restart-stable stopping behavior rather than retaining this display-driven
  exception. Removing the exception needs core/CLI reorg and restart coverage.
- **Verification:** iOS simulator Debug build, Swift parsing, design-token and
  diff checks passed. Full suites were not run for this localized UI removal;
  no core, storage or FFI changes. No interactive visual check was performed.

## 2026-09-23 — Enter the large movement dollar threshold directly

- **Before:** the USD minimum used a stepper with $5 increments.
- **After:** a labeled numeric field accepts a directly entered USD amount,
  with a decimal keyboard and Done action. The percentage keeps its stepper;
  there are no amount presets. Core still owns persistence and the existing
  $1–$100,000 bounds.
- **Why:** large portfolio thresholds should not require repeated $5 taps.
- **CLI check:** in a temporary data directory, `spectra settings set
  large-movement-usd 12345.67` followed by `spectra settings get
  large-movement-usd` retains 12345.67 across processes. Also checked 5000,
  zero (bounded to 1) and 100001 (bounded to 100000).
- **Verification:** Swift parsing, design-token and diff checks passed.
  Runtime catalog check reports only the existing unused `Provider Notes` key.
  iOS simulator Debug build and JSON catalog checks passed. Full suites were
  not run for this localized control change.

## 2026-09-23 — Remove the large movement footer

- **Before:** Large Movement Alerts ended with a paragraph explaining that
  controls tune notifications during portfolio balance refreshes.
- **After:** the paragraph and its containing section are removed, along with
  all three runtime translations.
- **Why:** remove the unwanted footer at the user's request.
- **CLI check:** no domain behaviour changes; inspect the Swift view and runtime
  catalogs to confirm the footer string is absent.
- **Verification:** targeted Swift parsing, JSON parsing and `git diff --check`;
  full suites are not required for this copy-only change.

## 2026-09-23 — Remove the large movement toggle description

- **Before:** Large Portfolio Movement Alerts showed an explanatory sentence
  when enabled and an off-status sentence when disabled beneath its toggle.
- **After:** neither sentence appears; their translations are removed from
  all three runtime catalogs.
- **Why:** remove the unwanted toggle description at the user's request.
- **CLI check:** no domain behaviour changes; inspect the Swift view and runtime
  catalogs to confirm both strings are absent.
- **Verification:** targeted Swift parsing, JSON parsing and `git diff --check`;
  full suites are not required for this copy-only change.

## 2026-09-23 — Remove Price Alerts explanatory copy

- **Before:** Price Alerts opened with a long introduction and displayed a
  paragraph below Enable Price Alerts.
- **After:** the page starts with Notifications and its toggle, with both
  explanatory paragraphs and their English/Simplified Chinese/Traditional
  Chinese translations removed.
- **Why:** simplify the page at the user's request.
- **CLI check:** no domain behaviour changes; check the Swift view and runtime
  catalogs for absence of the two removed strings.
- **Verification:** targeted Swift parsing, JSON parsing and `git diff --check`;
  full suites are not required for this copy-only change.

## 2026-09-23 — One typed custom endpoint directory

- **Before:** Endpoints embedded separate EVM RPC, Bitcoin Esplora and Monero
  backend editors. Their settings had different storage shapes; the Esplora
  and Monero values were displayed without being used by the service's node
  selection. Diagnostics offered another set of these editors.
- **After:** a top-right plus opens Network / Type / URL. Types use the exact
  `api` names from `endpoints.toml` for that network. The existing network
  groups and endpoint details remain, with Built-In / Custom labels and a
  source filter. Diagnostics links to this directory instead of maintaining
  separate editors. Core stores typed custom endpoints, rejects invalid and
  duplicate URLs, and supplies them to the existing matching API clients and
  health probes. Newer custom URLs are tried before older custom URLs and
  built-in nodes; concrete networks remain isolated. Blockscout history also
  accepts custom sources; TronGrid token discovery uses its own typed account
  URLs. This does not add implementations for operations
  an API's existing adapter does not implement.
- **Why:** one persisted directory and the same API adapter for custom and
  built-in URLs, instead of three inconsistent, partly ineffective settings.
  The old fields and CLI settings are removed directly, without migration.
- **CLI check:** `spectra endpoints --chain solana --api solana-json-rpc
  --add https://node.example`; `spectra --json endpoints --catalog --source
  custom`; `spectra --json send configured-endpoints solana`. Offline acceptance
  checks every catalog network/API pair, reopens storage, rejects duplicates
  and incompatible types, and verifies source filtering and network isolation.
- **Verification:** formatting/clippy, 829 core tests and the transport test
  passed. All 122 iOS tests passed, including endpoint binding and real-window
  rendering tests; exported screenshots were visually inspected. CLI acceptance
  passed 441 checks, including all 67 catalog network/API pairs, with its sole
  failure being the pre-existing unused `Provider Notes` translation from a
  separate Pricing edit. Therefore `make verify` is not fully green; iOS was
  run separately because that CLI gate stops the aggregate command. Swift and
  Kotlin bindings were regenerated (optional `ktlint` is unavailable). Mock
  tests cover persisted Solana/WhatsOnChain reads and separate Blockscout and
  TronGrid indexer requests. JSON parsing and `git diff --check` passed.

## 2026-09-23 — Remove the Report a Problem introduction

- **Before:** Report a Problem opened with explanatory placeholder text.
- **After:** the page starts with Support Link; the introductory section,
  catalog field and all three translations are removed.
- **Why:** remove the unwanted introduction at the user's request.
- **CLI check:** no domain behaviour changed. Confirm `reportProblemDescription`
  has no references in `swift` or `resources/strings`.
- **Verification:** localized JSON parsing, reference scan and diff checks
  passed; no full suite for this localized copy removal.

## 2026-09-23 — Remove the unused Strict RPC Only setting

- **Before:** Advanced settings offered “Strict RPC Only (Disable Ledger
  Fallback)”. Core persisted the flag and CLI exposed `strict-rpc-only`, but
  no balance or networking operation consumed it.
- **After:** remove the toggle, explanatory translations, stored field, update
  variant, CLI mapping and generated binding wiring. No compatibility shim;
  balance and networking behaviour is unchanged.
- **Why:** remove an ineffective setting that promised a policy it did not enforce.
- **CLI check:** `spectra settings get strict-rpc-only` and
  `spectra settings set strict-rpc-only true` reject the unknown setting;
  `spectra --json settings list` omits the removed setting.
  The existing invalid-boolean acceptance check now uses `price-alerts`.
- **Verification:** Swift and Kotlin bindings regenerated; formatting/clippy,
  829 core tests plus the transport test, and `make test-ios` passed. CLI
  acceptance passed 443 checks but failed its unused-string gate on the
  pre-existing `Provider Notes` translation left by a separate Pricing edit,
  so `make verify` is not fully green. Direct CLI get/set rejection and list
  omission checks passed; runtime translation JSON and `git diff --check`
  passed. Kotlin generation succeeded without optional `ktlint` formatting.

## 2026-09-23 — Restore the Send recipient icon

- **Before:** the recipient page header and composer step referenced the missing
  SF Symbol `person.crop.circle.badge.arrow.forward.fill`, leaving no icon.
- **After:** both use `person.crop.circle.fill`.
- **Why:** use an available system symbol so the recipient icon renders.
- **CLI check:** no domain behaviour changed. A local Swift/AppKit symbol lookup
  returned nil for the old name and an image for the replacement.
- **Verification:** symbol lookup and diff checks only; no full suite or iOS
  simulator run for this localized presentation fix.

## 2026-09-23 — Remove Pricing and Endpoints explanatory copy

- **Before:** Pricing displayed a pricing introduction and a Provider Notes
  section; Endpoints displayed an introductory paragraph.
- **After:** those three explanations and their localized catalog fields are
  removed. Pricing starts with Display Currency; Endpoints starts with its
  endpoint list unless a loading error needs to be shown.
- **Why:** remove explanatory text at the user's request and avoid empty sections.
- **CLI check:** no domain behaviour changed; inspect the Swift view diff and
  confirm removed catalog keys have no references with
  `rg 'pricingIntro|publicProviderNote|copy\.intro' swift resources/strings`.
- **Verification:** JSON parsing and diff checks only; `make verify` skipped at
  the user's request.

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

## 2026-09-23 — Flatten ETHFI icon artwork

- **Before:** ETHFI kept source-size paths, a transformed clipping definition,
  nested scaling and a duplicate translucent white path. The center artwork was
  reported partially clipped in the iOS asset rendering.
- **After:** One compound path and explicitly scaled gradient coordinates draw
  directly in the 64×64 viewBox, without clipping or transforms. The white
  overlay is folded into the gradient stops; the background now fills the
  required radius-32 disc, with the existing inset gradient ring retained.
- **Why:** Remove unnecessary SVG coordinate and clipping dependencies while
  preserving the recognizable geometry and gradient treatment.
- **CLI check:** `scripts/normalize-icons.sh --check` and
  `cmp icons/crypto/ethfi.svg swift/Assets.xcassets/crypto/ethfi.imageset/ethfi.svg`.
  Normalization and export passed; macOS Quick Look renders the complete mark.
  iOS simulator rendering has not been checked for this resource-only change.

## 2026-09-23 — Use a purple circle for INK artwork

- **Before:** A white background circle sat behind a purple compound path whose
  outer contour duplicated the circle and whose inner contour cut out the mark.
- **After:** A purple radius-32 circle and a white path draw the same artwork;
  the internal contour is unchanged. No visible behavior change is intended.
- **Why:** Express the background directly and remove the redundant circle
  contour and even-odd cutout dependency.
- **CLI check:** `scripts/normalize-icons.sh --check` and
  `cmp icons/crypto/ink.svg swift/Assets.xcassets/crypto/ink.imageset/ink.svg`
  passed after normalization and export.

## 2026-09-23 — Enlarge LEO's central artwork

- **Before:** LEO's eight central paths used source coordinates and scale(.128)
  transforms, leaving a relatively small mark inside the background disc.
- **After:** The mark is 20% larger about the disc center. Path and gradient
  coordinates are written directly in the 64×64 coordinate system, with no
  transforms; the background disc is unchanged.
- **Why:** Improve the mark's prominence while keeping the SVG geometry explicit.
- **CLI check:** `scripts/normalize-icons.sh --check` and
  `cmp icons/crypto/leo.svg swift/Assets.xcassets/crypto/leo.imageset/leo.svg`
  passed. macOS Quick Look confirms a complete mark with space around it;
  iOS simulator rendering was not run for this resource-only change.

## 2026-09-23 — Give ONDO artwork more padding

- **Before:** ONDO's black mark extended almost to the white background edge.
- **After:** All three artwork paths are 10% smaller about (32, 32), leaving
  approximately 3.2 points of padding. The white circle is unchanged, and the
  final SVG contains direct path coordinates without transforms.
- **Why:** Give the central mark slightly more breathing room.
- **CLI check:** `scripts/normalize-icons.sh --check` and
  `cmp icons/crypto/ondo.svg swift/Assets.xcassets/crypto/ondo.imageset/ondo.svg`
  passed after normalization and export. XML inspection confirmed no transforms
  remain and the background circle is unchanged. No iOS simulator test was run.

## 2026-09-23 — Custom endpoint capabilities are explicit and enforced

**Before:** Adding a custom endpoint stored only its network, API and URL. The
UI copied the first matching built-in provider's entire record, including its
capabilities and probe metadata. Requests generally shared a slot's unfiltered
URL list; the broadcast capability guard inspected only built-in providers.

**After:** Every custom endpoint stores its own nonempty, canonical capability
set. Swift provides unchecked capability toggles with descriptions; CLI additions
require `--capabilities`. Core rejects unknown capabilities and operations for
which the selected adapter has no implementation. Adapter options describe what
Spectra can call, not what an individual provider supports. Unsupported catalog
reference APIs can still be inspected but cannot be added as usable custom APIs.
The add form lists only network/API combinations with implemented operations.
Custom directory records no longer inherit any provider metadata. Health probes
report reachability separately and never grant capabilities.

All service request selection now requires operation capabilities. Balance,
fees, token reads, history, UTXOs, verification and staking select eligible
providers; fallbacks keep the same requirements. EVM previews and builds split
balance, fee, nonce and token metadata reads; native/token indexer history selects
its respective sources independently. Protocol operations that internally need
several capabilities require their intersection, never their union. Signing
rechecks select UTXO/context providers. Built-in providers that were already used
for context verification, staking, Cardano UTXOs and TON token metadata now declare
those purposes explicitly. `staking` is a separate capability, not an implicit
permission granted by an unrelated read operation. Staking options also obey the
chain registry; ICP's static neuron directory needs no endpoint declaration.
WhatsOnChain's built-in API record now declares broadcast on its base URL;
redundant operation-path records are removed so the adapter does not append
`/tx/raw` twice after capability filtering. Automatic Bitcoin fee preview
failures no longer fabricate a 5 sat/vB quote.

Broadcast candidates require a broadcast declaration; the core checks it again
before network verification and submission. Explicit submissions cannot bypass a
saved endpoint's restriction or introduce an undeclared URL. Selected nodes still
must pass existing network/API checks; unsupported custom network verification
remains a refusal. Legacy stored-transaction rebroadcast validates each eligible
fallback target before sending. EVM builds validate the provider actually used
without probing every fallback in advance. Explicit transport configuration now
carries capability declarations for URLs absent from the directory; it cannot
widen known endpoint declarations. The endpoint settings screen refreshes when
committed custom endpoints change.

**Rationale:** An API protocol does not establish a provider's enabled operations.
The declaration must live with the endpoint and govern requests, not merely tags.
This directly changes the prelaunch storage/FFI shape: old custom endpoint records
without capabilities must be recreated; no compatibility shim or data deletion
was added.

**CLI checks:** Use a throwaway data directory and run:

```sh
spectra --data-dir /tmp/spectra-endpoint-check endpoints --chain ethereum \
  --api evm-json-rpc --add https://balance.example --capabilities balance
spectra --data-dir /tmp/spectra-endpoint-check endpoints --chain ethereum \
  --api evm-json-rpc --add https://broadcast.example --capabilities broadcast
spectra --data-dir /tmp/spectra-endpoint-check --json endpoints --catalog --source custom
spectra --data-dir /tmp/spectra-endpoint-check --json send configured-endpoints ethereum
```

The two custom rows retain different capability sets across processes; only the
broadcast URL is a send destination. Empty/unknown capabilities and EVM RPC
`history` declarations are rejected. `scripts/cli-endpoints.py` covers supported
adapter options, persistence, source filters and these restrictions. Core mock
regressions verify distinct providers for EVM balance/fee/context, actual
balance/broadcast request isolation, capability-preserving fallback, refusal
without requests when no eligible endpoint exists, and explicit-override guards.
Swift bridge tests cover the new field, empty-selection rejection and both screens
rendering in a real window.

## 2026-09-23 — Adjust Polkadot and Polygon mark sizes

- **Before:** Polkadot's white ellipses occupied more of the disc than desired;
  Polygon's white mark needed a small increase.
- **After:** Polkadot's mark is 8% smaller and Polygon's is 5% larger about
  (32, 32). Background discs are unchanged. Polkadot's rotated ellipses are
  expressed as elliptical arc paths; both SVGs have direct coordinates and no
  transforms.
- **Why:** Refine icon padding with restrained size changes.
- **CLI check:** `scripts/normalize-icons.sh --check` passed. XML and byte
  comparisons confirmed unchanged background circles, no transforms, and exact
  matches with the exported Swift assets. No iOS simulator test was run.

## 2026-09-23 — Separate SEI, Sonic and Wrapped Bitcoin backgrounds

- **Before:** SEI and Sonic placed colored negative-space paths over white
  circles; Wrapped Bitcoin's dark outer border was a compound cutout path.
- **After:** SEI has a red circle and explicit white artwork; Sonic has a black
  circle and explicit white artwork. Wrapped Bitcoin has a dark circle and a
  white inner contour, retaining its orange Bitcoin mark and gray decoration.
  No visible redesign is intended; all background discs use radius 32 and no
  transforms are used.
- **Why:** Represent background colors directly, as with the INK cleanup.
- **CLI check:** `scripts/normalize-icons.sh --check` passed and byte comparisons
  verified all three Swift exports. Boolean path differences for SEI and Sonic
  matched the original white regions at 16,384 sampled points each before
  normalization. macOS Quick Look previews confirmed the complete artwork.
  iOS simulator tests were not run for this resource-only change.

## 2026-09-23 — Remove ETHFI's outer gradient ring

- **Before:** ETHFI had an inset gradient stroke around its background disc.
- **After:** The stroke and its unused gradient are removed. The background
  gradient and central artwork remain unchanged.
- **Why:** Use the requested borderless appearance.
- **CLI check:** `scripts/normalize-icons.sh --check` and
  `cmp icons/crypto/ethfi.svg swift/Assets.xcassets/crypto/ethfi.imageset/ethfi.svg`
  passed after normalization and export. No iOS simulator test was run.

## 2026-09-23 — Use a solid ETHFI background

- **Before:** ETHFI's disc used a subtle dark-purple gradient.
- **After:** The disc uses solid #302a92, the original gradient's first color.
  The central artwork retains its gradient.
- **Why:** Remove the background gradient as requested.
- **CLI check:** `scripts/normalize-icons.sh --check` and
  `cmp icons/crypto/ethfi.svg swift/Assets.xcassets/crypto/ethfi.imageset/ethfi.svg`
  passed after normalization and export. No iOS simulator test was run.

## 2026-09-23 — Remove residual white rims on SEI and Sonic

- **Before:** Converting negative-space artwork with a circle difference also
  retained thin white gaps between the original approximate perimeter and the
  standard background circle. These appeared as unwanted rim fragments.
- **After:** White wave/fan boundaries end at the background disc; obsolete
  perimeter detours are removed. Background colors and interior marks remain.
- **Why:** Fix artifacts missed during the previous conversion's visual review.
- **CLI check:** `scripts/normalize-icons.sh --check` and byte comparisons with
  both Swift exports passed. A 256×256 point grid within radius 30.5 found zero
  interior fill differences against the original artwork for both icons.
  Inspected 512-pixel macOS Quick Look previews after cleanup. No iOS simulator
  test was run for this resource-only change.


## 2026-09-23 — Canonical history projections and cursor pagination

- **Before:** history pages selected one winner per wallet/chain/deployment/hash,
  but the pending snapshot, replacement list, count and earliest dates used raw
  rows. A superseded pending provider row could reappear beside its confirmed
  transaction, and rows without a current wallet could enter the summary.
- **After:** all these projections use the same current-wallet and identity-winner
  predicate. Counts describe visible transactions; superseded and orphan pending
  rows cannot offer replacement actions. Direct record lookup remains available.
- **Before:** stored pages accepted an integer offset. Inserts/deletes before that
  position could shift subsequent pages, and deep pages scanned preceding rows.
- **After:** `txs --page --cursor <nextCursor>` and the FFI use a core-produced
  cursor over indexed `(created_at, id)`. Cursor timestamps come from the SQL
  ordering columns, not the JSON payload. Equal timestamps retain ID order in
  both directions; deleting the anchor does not invalidate the continuation.
  Cursors are bound to wallet, filter, search and direction; changing those inputs
  requires a fresh first page. Limit may change between pages. The final page
  returns `nextCursor: null`. The old offset interface is removed directly.
  Pages read current data, not a frozen cross-request snapshot: new rows before
  the cursor appear on refresh; edits that move rows across the cursor require
  refresh. Swift restarts paging on its existing history/wallet revision signal.
- **Why:** one transaction must have one visible status; indexed continuation
  avoids position shifts without keeping database transactions open across UI
  requests.
- **CLI checks:** `python3 scripts/cli-history.py target/debug/spectra
  HistoryTests.test_stored_pages HistoryTests.test_cursor_changes_and_ties`
  covers duplicate pending/confirmed rows, counts, both sort directions, ties,
  anchor deletion, insertion and invalid/mismatched cursors across CLI restarts.

## 2026-09-23 — Scoped send preparation and atomic history batches

- **Before:** EVM nonce preparation loaded every history row and send artifact;
  Monero input selection loaded every artifact and repeatedly scanned outputs.
- **After:** indexed SQL selects pending history for the chain/sender and signed
  artifacts for the chain/sender or chain/wallet. Every selected artifact still
  passes full validation. Monero performs blocking artifact reads off the async
  executor and checks reserved key images with one set lookup per output.
  An unrelated undecodable artifact no longer blocks another sender's preparation;
  a selected invalid artifact still refuses the operation.
- **Before:** history batches repeatedly prepared SQL, with manual transaction
  cleanup that did not cover commit failure.
- **After:** each batch reuses a prepared statement and an immediate RAII
  transaction. Failed inserts, deletes and commits roll back; replacement still
  rejects duplicate IDs. No intended successful-write semantics change.
- **Checks:** `cargo test -p spectra_core --lib wallet_db` covers scoped indexes,
  invalid selected artifacts and partial-batch rollback/retry. Offline CLI paths:
  `python3 scripts/cli-send-stages.py target/debug/spectra` and
  `python3 scripts/cli-send-monero.py target/debug/spectra` exercise real preparation,
  signing, reservations and immutable-payload retries against local mock nodes.

## 2026-09-23 — Native flow isolation and core-owned precision projections

- **Before:** AppState held send, receive and import form fields directly.
  An import or rename returning after dismissal could reset a newer form,
  overwrite its error, close its navigation or clear its busy flag.
- **After:** dedicated native flow objects own these fields and reset behavior.
  Import/rename success, failure and cleanup are bound to a session identity.
  Navigation dismissal clears sensitive draft inputs and invalidates callbacks.
  Send/receive navigation bindings also invalidate their pending native work.
  Completed core writes still refresh wallet projections after dismissal.
- **Before:** Swift read cached custom-token decimals and returned them to an
  exported core helper to resolve display precision.
- **After:** the coherent portfolio snapshot supplies a precision catalog from
  core's stored preferences, keyed by concrete deployment. Disabled tokens keep
  their precision for history; removing a custom token removes its precision
  entry. Unknown history retains the core-provided 18-place display fallback.
  Before the first snapshot, dependent amount labels show `—`. The caller-fed
  precision FFI helper is removed. Compact number formatting remains an optional
  shared presentation utility; exact signing review amounts are unchanged.
- **Why:** native session lifetime belongs with the native form; persisted token
  metadata belongs with core. Neither change moves navigation into Rust.
- **CLI check:** `python3 scripts/cli-portfolio.py target/debug/spectra
  PortfolioTests.test_core_owned_asset_precision` checks native/custom precision,
  identical token identifiers on different networks, disabled tokens, edits,
  deletion and reopening. `WalletImportSessionTests` exercises delayed success,
  delayed failure, busy cleanup and dismissal; `AssetPrecisionBridgeTests`
  verifies the async FFI projection and rejects stale precision snapshots.
- **Verification:** all required gates passed after fixing the unused-string
  check and SwiftUI sub-object bindings: rustfmt/clippy, 838 core tests plus the
  transport test, 442 offline CLI checks and 127 iPhone simulator tests. The
  simulator suite includes the exact-amount send rendering check and
  `testEthereumTestNetworksExposeExpectedContextsAndEndpoints`.
