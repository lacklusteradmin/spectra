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

## 2026-10-01 — The backdrop's colour clouds drift

- **Before:** `SpectraBackdrop` drew four blurred circles at fixed offsets, so
  every screen's wallpaper was a still image.
- **After:** each cloud wanders slowly around its old position on its own
  closed path (50–70 pt reach, 37–61 s per cycle, a slight swell in size).
  Position is a function of wall-clock time, so the backdrop of every tab and
  pushed screen agrees and a navigation transition does not jump. It holds
  still at its old layout under Reduce Motion, and freezes in Low Power Mode
  and in the background. The clouds are radial gradients shaped like the old
  blurred discs, so moving one moves a layer instead of re-running a large blur
  every frame; the timeline ticks at 30 fps.
- **Why:** the glass cards refract the backdrop, and a still one gives them
  nothing to show. The blur is gone so the motion stays cheap.
- **CLI check:** none applies; this is iOS rendering only. Compared against the
  previous build in the simulator.
- **Verification:** iOS build, `make check-ui`; `make lint`, `make test`,
  `make test-cli` and `make test-ios` not run (no core or test-covered code
  changed).

## 2026-10-01 — Aptos balances are read through view functions

- **Before:** every Aptos balance was read as a resource: APT as
  `0x1::coin::CoinStore<0x1::aptos_coin::AptosCoin>`, a token as
  `CoinStore<T>` with decimals from `CoinInfo<T>`. Every catalog Aptos token is
  a fungible asset (AIP-21) named by its metadata object's address, which has
  neither resource, so every catalog Aptos token read failed and was left out.
  An account that holds APT only as the migrated fungible asset — every account
  created since the migration — has no `CoinStore<AptosCoin>` either, so its
  APT read failed with `resource_not_found` and the wallet's whole refresh
  counted as an error. Aptos testnet tokens were refused as an unsupported
  chain.
- **After:** `AptosClient` reads every balance and decimal count through
  `POST /view`. An identifier without `::` is a fungible asset's metadata
  address, read with `0x1::primary_fungible_store::balance` and
  `0x1::fungible_asset::decimals` (type argument
  `0x1::fungible_asset::Metadata`); one with `::` is a coin type, read with
  `0x1::coin::balance<T>` and `0x1::coin::decimals<T>`. APT is read as the
  coin `0x1::aptos_coin::AptosCoin`. `coin::balance` adds the `CoinStore` and
  the paired fungible-asset store, so a migrated coin is counted wherever it
  sits, and an owner with neither reads as zero instead of failing.
  `aptos-testnet` tokens are read the same way. Aptos still does not claim
  `token-discovery`: a node cannot list fungible-asset stores.
- **Why:** the catalog's facts and the reads disagreed. The shape decides
  rather than the catalog's `standard`, because the network has one standard
  and a custom token takes it whichever shape it is; the two shapes are
  exactly the two `aptosTokenType` accepts. Reading a coin through
  `coin::balance` rather than its `CoinStore` is the same correction for the
  coin case, and keeps one request shape for every Aptos read.
- **CLI check:** `spectra wallet watch --chain aptos --address
  0x0000000000000000000000000000000000000000000000000000000000000001` then
  `spectra refresh` succeeds and stores APT, USDC and USDT (the account has no
  `CoinStore<AptosCoin>`). Offline: `cli-portfolio.py`
  `test_aptos_fungible_asset_refresh` (a loopback node that answers only
  `/view`) and
  `aptos_reads_a_fungible_asset_and_a_coin_through_view_functions`.
- **Verification:** `make lint`, `make test`, `make test-cli`; `make test-ios`
  not run.

## 2026-10-01 — A wallet's token balances come from one listing where a service can list them

- **Before:** a balance refresh read every enabled token separately. An EVM
  token was two requests (`balanceOf`, then `decimals`+`symbol`), and tokens
  were read **one after another**; Sui, Aptos and NEAR were two requests per
  token, Solana one per mint, TON one listing plus one `decimals` read per
  token. `discover_token_balances`, which lists holdings in one call, was
  only used by `spectra token discover`. Two of those reads were broken: TON
  parsed the indexer's `jetton` field as an object when it is an address
  string, so every jetton read failed, and Aptos claimed `token-discovery`
  although its node only lists legacy `CoinStore` coins, not the
  fungible-asset stores every catalog Aptos token lives in.
- **After:** a refresh first asks for the list of what the address holds —
  Solana and Sui nodes, TronGrid, TON Center v3, and now Blockscout/Routescan
  for EVM chains (`token-discovery` on every `blockscout` endpoint). A known
  token in the list takes the listed balance and decimals; one missing from
  it holds zero; a listed token the wallet does not know is ignored. A chain
  with no such service (EVM chains without an explorer, NEAR, Aptos) or a
  failed listing falls back to per-token reads, and EVM per-token reads go in
  JSON-RPC batches of 20 contracts, concurrently. TON matches jetton masters
  by raw address and takes decimals from the listing's metadata. Aptos no
  longer claims `token-discovery`. `enumerates_holdings` is gone from
  `chains.toml`: whether a chain can list holdings is whether its configured
  endpoints declare `token-discovery`. Under an explicit endpoint override
  (`spectra refresh --endpoint`), catalog indexers are not consulted, as
  TronGrid already was not. `spectra token discover` reports only catalog
  symbols, never an on-chain one.
- **Why:** with every known token read (see the next entry), per-token reads
  cost ~108 serial requests per Ethereum wallet refresh. One listing costs one
  request however many tokens the catalog has, and the per-token path and the
  listing path were two models of the same question.
- **CLI check:** `spectra wallet watch --chain ethereum --address 0xd8dA…6045`
  then `spectra refresh` stores the known tokens the explorer lists (USDC,
  USDT, DAI, …); the same on `avalanche` goes through Routescan, on `bnb`
  (no explorer) through batched `eth_call`s, and on `ton` reads USDT for
  `UQAj-peZGPH-cC25EAv4Q-h8cBXszTmkch6ba6wXC8BM40qt`. `spectra token discover
  --wallet <aptos wallet>` now says Aptos cannot enumerate holdings.
  Offline: `a_listing_answers_for_every_known_token`,
  `a_token_list_reads_both_dialects`, `jetton_wallets`.
- **Verification:** `make lint`, `make test`, `make test-cli`, `make test-ios`.

## 2026-10-01 — Every known token is read; the Known Tokens switch is gone

- **Before:** each catalog token had an "enabled" flag, true for a handful by
  default (157 deployments shipped off). The Known Tokens page drew a switch
  per token; `spectra token track`/`untrack` set it. Balance refresh, token
  history decoding, send preflight and the transfer builder all skipped a
  token that was off — so a token the user held but had not switched on was
  invisible and could not be sent.
- **After:** no flag. `enabled` is gone from `tokens.toml`,
  `testnet-tokens.toml`, `TokenDeploymentEntry` and
  `CoreTokenPreferenceEntry`; `StateCommand::SetTokenPreferencesEnabled`,
  `CoreTokenPreferenceKey`, `spectra token track`/`untrack` and the switch are
  removed. Every catalog token and every custom token is read, decoded and
  sendable. A catalog merge replaces the stored built-ins with the catalog's
  and keeps custom rows. The Advanced page no longer counts enabled tokens.
- **Why:** nobody wants a token they hold hidden, and the switch existed only
  to cap per-token request cost, which the listing in the entry above removes.
  A flag four paths had to remember to check was a second model of "which
  tokens does this wallet have".
- **CLI check:** `spectra --json token list` has no `isEnabled`;
  `spectra token untrack --chain Ethereum USDC` is a usage error.
- **Verification:** as above.

## 2026-10-01 — Each wallet-setup page is its own pushed screen

- **Before:** the whole setup flow was one pushed `SetupView` that swapped
  pages through a `@State` page. To keep "back" a step back rather than a
  jump out of the flow, it hid the system back button for a custom one, and
  hiding it also turned off the edge swipe — no setup page could be swiped
  back from.
- **After:** each page is pushed onto the navigation stack as its own
  `SetupView(page:)`. The system back button and the edge swipe step back one
  page and leave the flow from the first. The custom back button,
  `SetupFlow.previous(before:)` and the reset on a mode change are gone.
  Entering backup verification still draws its challenge first.
- **Why:** the navigation stack already models "pages pushed in order"; a
  second page model inside one screen had to fake the back button and lost
  the swipe doing it.
- **CLI check:** none applies — page navigation is view state. Covered by the
  iOS build and suite.

## 2026-10-01 — The import method is chosen before the chains

- **Before:** Add Wallet offered one "Import Wallet". Its chain page listed
  every chain, multi-select, and the secret page then offered Seed Phrase or
  Private Key. Picking Private Key after chains a key cannot derive on showed
  "Private key import is not available for: …" and kept the first chain only,
  so the user went back to re-pick. Watch-only listed every chain too and
  warned under the list when Monero was ticked. The chain rows drew
  multi-select circles in every mode, though watch-only and private-key
  imports keep one chain.
- **After:** Add Wallet offers "Import Seed Phrase" and "Import Private Key".
  The draft says which chains its mode `offers` — a private key lists only
  `derivesFromPrivateKey` chains, watch-only only `supportsWatchOnlyImport`
  ones — and `selectedChains` drops any it does not. Single-chain modes title
  the page "Choose a Chain", say why the list is short, draw no empty circles,
  have no "Selected" filter, and close the full list on a pick. The secret page
  has no method picker; `WalletSecretImportMode`, the unsupported-chain
  message, the Monero warning and an editing note no page reached are gone.
- **Why:** the method decides which chains are possible, so asking it after
  the chains made a contradiction the page then had to explain. Filtering the
  list removes the contradiction instead of reporting it.
- **CLI check:** none applies — the CLI takes the method as `--seed-env` or
  `--private-key-env`/`--private-key-file` and the chain as `--chain`, and
  already refuses a chain the key cannot derive on. The registry flags the
  picker filters on are covered by `only_monero_is_excluded_from_watch_only_import`
  and the private-key import checks in `scripts/cli-acceptance.sh`;
  `ImportMethodTests` covers the filter.
- **Verification:** `make lint`, `make test` (887 core tests), `make test-cli`
  (473 passed) and `make test-ios` (103 tests in 23 suites, all passing).

## 2026-10-01 — Cardano and Substrate read a phrase's entropy in its own wordlist

- **Before:** with no wordlist named, Cardano's CIP-3 root parsed the phrase
  as English, so a valid non-English phrase was refused ("Could not derive an
  address from this secret for any selected chain.") although the same phrase
  imported for Bitcoin. Substrate (Polkadot) did detect the language, but
  `bip39`'s `Mnemonic::to_entropy` re-detects it from the words and panics on
  a phrase valid in both Chinese lists, so `wallet import --chain Polkadot`
  with such a phrase aborted with exit 101.
- **After:** both read the phrase in its own wordlist (`parse_mnemonic`, the
  path every other chain already took) and take the entropy from the parsed
  word indices (`mnemonic_entropy`), never re-detecting. A named wordlist is
  still strict: a Chinese phrase under `english` or an unknown name is
  refused. `resolve_bip39_language` lost its `None` → English default, the
  trap that produced the bug. Ambiguity rule: the first language in
  `Language::ALL` that parses wins. Every word the Simplified and Traditional
  Chinese lists share sits at the same index in both (1,275 words, checked by
  a core test), so either reading gives the same entropy and the same
  address. English and French share 100 words at different indices; a
  phrase of only those that checksums in both stays English, its reading
  before detection, and naming `french` picks French.
- **Why:** CIP-3 and Substrate root on the entropy, so the wordlist decides
  the key; assuming English was a second, wrong answer to a question core
  already answers from the words. A phrase for the all-zero entropy now lands
  on the same Cardano address in Chinese and English, as CIP-3 requires.
- **CLI check:** `SPECTRA_SEED="的 的 的 的 的 的 的 的 的 的 的 在" spectra wallet
  import --chain Cardano --name C` imports
  `addr1vy8ac7qqy0vtulyl7wntmsxc6wex80gvcyjy33qffrhm7ss7lxrqp`, the address
  of `abandon … about`; the same phrase with `--chain Polkadot` imports.
- **Verification:** `make lint`, `make test` (887 core tests) and `make test-cli`
  (473 passed); `make test-ios` not run (no Swift or FFI change).

## 2026-10-01 — A seed phrase's length and wordlist are read from its words

- **Before:** the import page asked for a length (five tiles plus a custom
  field) and a wordlist (a ten-entry menu written in Swift, defaulting to
  English) before the first word. The grid had exactly that many slots, and a
  paste of more words than slots silently dropped the rest: 24 words pasted
  with 12 selected kept the first 12. With no language given, core counted a
  word valid if *any* wordlist held it. The word fields used an ASCII
  keyboard, so a Japanese, Korean or Chinese wordlist could not be typed, and
  Paste read `UIPasteboard`, which raised iOS's paste prompt. Setup had two
  backs: the navigation bar's left the whole flow, the bottom bar's went back
  one step.
- **After:** `SeedPhraseCheck` takes `word_count: Option<u32>` and
  `language: Option<String>`; `None` asks core. Core judges the phrase at the
  shortest BIP-39 length holding every filled slot (at least 12), detects the
  wordlist holding the most words (a tie goes to the one the checksum
  confirms, then to English first), and names words off *that* list. More than
  24 words is named, not cut. The verdict carries `word_count`, `language` and
  whether each was inferred; `seed_phrase_languages()` lists the wordlists.
  The page shows the method, then a grid that grows to fit what is typed or
  pasted (a space in the last slot or "More words" adds the next standard
  length), a status line ("Valid phrase · English · 24 words"), and one
  Advanced row. Length and wordlist overrides moved into the Advanced sheet,
  where a fixed length makes core refuse a longer phrase. Fields use the
  default keyboard, 44pt rows and `privacySensitive()`; Paste is
  `PasteButton`. Setup has one back, in the navigation bar, stepping through
  the flow and leaving it from the first page. `spectra wallet check-seed`
  prints the verdict; `wallet import` names an unfinished length ("13 words")
  and the detected list in its refusal.
- **Why:** both questions had answers in the words, and asking them first
  pushed the entry below the fold and made the silent truncation possible.
  The wordlist was a caller-owned list of a core fact. Two backs that meant
  different things was one too many.
- **CLI check:** `SPECTRA_SEED="<23×abandon> art" spectra --json wallet
  check-seed` shows `"wordCount":24` and `"language":"en"`; adding
  `--words 12` gives `"error":"Seed phrase must be 12 words."`.
- **Verification:** `make lint`, `make test` (884 core tests), `make test-cli`
  (471 passed) and `make test-ios` (98 tests in 22 suites, all passing).

## 2026-10-01 — The chain picker is one ranked list with tag filters

- **Before:** `chain-ui.toml` gave each network a `category` (`bitcoin-family`,
  `evm-l1`, `evm-l2`, `other`) and eight mainnets a `popular_rank`. The setup
  page drew those eight as a two-column grid of cards; "Browse all" opened a
  differently styled list grouped into Bitcoin Family, EVM Chains, EVM L2s,
  Other Chains and Testnets, in catalog order inside each section, with
  testnets mixed into the default view. `spectra chains` listed in catalog
  order, and `popularRank` was `null` for every chain off the short list.
- **After:** `ChainCategory` is gone. Every mainnet has a `popular_rank`,
  1..=46 without gaps (market cap, with Base moved ahead of Arbitrum and the
  tokenless L2s ahead of the tail), and `tags` from a closed `ChainTag` set.
  `layer-2`, `utxo`, `eutxo`, `move`, `substrate`, `pow`, `privacy` and
  `payments` are written in the catalog; `layer-1`, `evm` and `testnet` are
  derived, and the catalog refuses a row that writes one. A testnet row writes
  neither rank nor tags: it takes its mainnet's and adds `testnet`. Both pickers
  draw one row style (`ChainSelectionRow`): the setup page lists the top six
  mainnets, and the full list orders by popularity or name, filters by a
  single-line scrolling row of tags (plus "Selected" in a multi-select), and
  shows testnets only under their own filter, as a selection, or as a search
  match. `spectra chains` lists in popular order, prints `tags`, and takes
  `--tag`.
- **Why:** the category was a second, coarser model of what tags now say, and
  it could only group, not filter; ranking eight chains left the other
  thirty-eight in an order nobody chose. The Utxo and Substrate tags are
  written rather than derived only because the registry facts they mirror
  read the catalog; `tags_agree_with_the_registry` holds them to those facts.
- **CLI check:** `spectra --json chains --tag move` lists Sui and Aptos;
  `spectra --json chains --testnets --filter "Ethereum Sepolia"` shows
  `"popularRank":2` and `"tags":["layer-1","evm","testnet"]`;
  `spectra chains --tag sidechain` exits 2.
- **Verification:** `make lint`, `make test` (877 core tests), `make test-cli`
  (463 passed) and `make test-ios` (94 tests in 21 suites, all passing).

## 2026-10-01 — Launch reads once; balance ticks no longer re-run unrelated work

- **Before:** launch read the transaction projection twice, re-opened the
  state database after `ready()` had opened it, and ran a foreground refresh
  alongside the launch sweep the engine's first tick already performs. Every
  mid-sweep portfolio snapshot reassigned every wallet, cache, quote and
  dashboard field, so all of their views re-rendered each 300 ms; every
  balance change bumped `walletsRevision`, which re-ran the history page query
  and the transaction-detail reads. `AppLocalization` rebuilt its cache key
  from the preferred languages on every string it returned. Several views found
  a wallet by scanning `store.wallets`.
- **After:** launch opens the database once, reads core's state with
  `appState()`, reads the transaction projection once, and only settles
  leftover Live Activities on its first activation; later activations refresh
  as before. Snapshot fields are assigned only when they differ
  (`WalletDerivedCache` is `Equatable`). `walletIdentityRevision` changes only
  when a wallet is added, removed or changes in anything but its balances, and
  the price-alert picker watches the alertable holdings instead. The string
  tables are cached until `NSLocale.currentLocaleDidChangeNotification`. Views
  look wallets up with `wallet(for:)`.
- **Why:** the same reads ran twice at launch, and a balance landing every few
  hundred milliseconds re-ran work that does not depend on balances.
- **CLI check:** none applies — launch order, observation and view refresh are
  the platform's.
- **Verification:** `make test-ios` runs 95 tests in 21 suites with this and
  every 2026-09-30 Swift entry above applied: 94 pass. The one failure,
  `testnetSymbolsKeepTheirLowercasePrefix`, predates these changes — Beta
  Commit 193 gave Bitcoin Signet the symbol `sBTC`. `scripts/unused-strings.sh`
  passes.

## 2026-09-30 — A blank wallet password is refused, not read as "no password"

- **Before:** `store_seed_phrase`, `store_private_key` and `load_material`
  in `core/src/store/wallet_secrets.rs` read a password with
  `password.map(str::trim).filter(|p| !p.is_empty())`. `Some("   ")`, a
  caller asking for a password, became `None`, which is the explicit choice of
  none. `SPECTRA_PASSWORD="   " spectra wallet import` or an empty
  `--password-file` stored the phrase unsealed, exited 0, and
  `wallet export --yes` then printed it without a password.
  `WalletImportCommit::signing` recorded such a wallet as not
  password-protected. `validate_wallet_password` accepted a whitespace-only
  password and confirmation as "no password", and the iOS import form then
  sent that `Some("   ")` to core.
- **After:**
  - Only `None` stores material unsealed. A `Some` password that is blank
    after trimming fails with `WalletSecretError::EmptyPassword` when storing
    and when reading.
  - `import_wallets` refuses a blank password as `InvalidInput`, before
    planning, deriving or storing. The CLI exits 3 and stores no wallet and
    no secret.
  - `signing()` reports password protection as `password.is_some()`.
  - `validate_wallet_password` treats only two empty fields as "no password".
    A whitespace-only one is `TooShort`, so the iOS form blocks it and sends
    `nil` only for an empty field.
  - `--no-password` is the only way the CLI asks for an unsealed wallet.
- **Why:** funds and keys take the stricter side. Collapsing "a password
  that is blank" into "no password" turned a malformed request for a seal
  into plaintext key storage. `seal` already refused an empty password, and
  the store functions bypassed that refusal.
- **CLI check:**
  `SPECTRA_SEED="abandon … about" SPECTRA_PASSWORD="   " spectra --data-dir "$(mktemp -d)" --json wallet import --chain bitcoin`
  exits 3 with `"password cannot be empty"`, and `wallet list` in that data
  directory is empty. `scripts/cli-acceptance.sh` asserts this for a seed
  import, `wallet new --password-file /dev/null` and a private-key import,
  and checks that no secret file is written. `scripts/cli-wallets.py` asserts
  `check-password` rejects a whitespace-only password as `tooShort`. Unit
  tests:
  - `store::wallet_secrets::tests::a_blank_password_stores_nothing_rather_than_storing_unsealed`
  - `store::wallet_secrets::tests::a_blank_password_is_refused_when_reading`
  - `store::tests::wallet_import::a_blank_password_is_refused_rather_than_stored_unsealed`
  - `validation::password_verdict_tests`
- **Verification:** `make verify`. `cargo fmt --check` and clippy at
  `-D warnings` are clean. `cargo test --workspace` passes (873 core tests).
  `scripts/cli-acceptance.sh` reports 457 passed, including
  `cli-wallets.py`. `xcodebuild test` ran 95 tests with 1 failure:
  `PresentationCatalogTests.testTestnetSymbolsKeepTheirLowercasePrefix`
  expects `tBTC` for Bitcoin Signet, but `core/data/chain-ui.toml` gives
  `sBTC`. That failure comes from existing data and is unrelated to this
  change.

## 2026-09-30 — Core errors are typed per layer; a bare string no longer becomes a bridge error

- **Before:** about 500 core functions returned `Result<_, String>`.
  `SpectraBridgeError` had `From<String>` and `From<&str>`, both mapping to
  `Failure`, so a timeout, a malformed provider answer, a node refusing a
  broadcast and a bad derivation path all reached Swift and the CLI as the
  same variant. Examples:
  - A UTXO shortfall surfaced as the literal key `utxo.insufficientFunds`,
    which nothing localized.
  - Sealing a wallet with an empty password reported "stored secret is
    corrupt".
  - `api/http.rs` carried two request paths: `HttpClient` returned `String`,
    and `http_request`/`http_post_json` returned an `HttpError`. The second
    was left over from an FFI export that no longer existed.
  - The endpoint catalog's load error was cached and returned to every
    caller, including a settings screen that displayed it.
- **After:**
  - Each layer has its own error: `ApiError`, `DerivationError`, `SendError`,
    `DbError`, `RegistryError`, `EnvelopeError` and `WalletSecretError`.
  - Each converts into the `SpectraBridgeError` variant that fits it. For
    example, a transport failure becomes `Network`, a shape mismatch becomes
    `Decode`, and a refused path, amount or address becomes `InvalidInput`.
  - `From<String>` and `From<&str>` are gone.
  - A UTXO shortfall is `SendError::InsufficientFunds` ("Insufficient funds
    for the amount plus the network fee."). An empty password is
    `WalletSecretError::EmptyPassword`.
  - HTTP has one retry loop:
    - A non-2xx answer other than 429/5xx is `ApiError::Status`, displayed as
      "HTTP 404: body" instead of "HTTP 404 Not Found: body".
    - The Koios submit and the diagnostics EVM probe use `HttpClient`. The
      probe now goes through `EvmClient`, so an RPC error object fails it.
  - Registry lookups name the chain ("ethereum is not a Monero network").
  - The endpoint catalog panics on first use when the embedded file is
    broken, like the chain and token catalogs.
  - FFI renames: `chain_endpoints` → `endpoint_settings` (no longer throws);
    `AppCoreChainEndpoints` → `ChainEndpointSettings` (its field
    `grouped_settings` → `groups`); `AppCoreGroupedSettingsEntry` →
    `EndpointSettingsGroup`; `AppCoreEndpointRecord` → `EndpointRecord`.
  - `HttpError`, `HttpHeader`, `HttpResponse`, `HttpTextResponse` and
    `HttpRetryProfile` are no longer generated.
  - CLI errors from these layers take their exit code from the same
    classification: `InvalidInput` exits 3, the rest exit 1.
- **Why:** callers could not tell "retry later" from "the user typed
  something wrong" without matching message text, and one of those texts was
  an untranslated key. Two HTTP stacks answered the same question
  differently.
- **CLI check:** none shows every category offline. `scripts/cli-acceptance.sh`
  runs the retyped paths. The classification is pinned by unit tests:
  - `api::http::tests::a_client_error_status_is_returned_not_retried`
  - `send::bitcoin::tests::insufficient_funds_is_reported_the_same_way_pinned_or_selected`
  - `store::seed_envelope` tamper and nonce tests
  - `send::icp_stages::tests::signed_envelope_bytes_are_pinned`, which also
    covers the CBOR library change (`serde_cbor`, unmaintained per
    RUSTSEC-2021-0127, replaced by `ciborium` with keys in canonical order,
    byte-identical output)
- **Verification:** `cargo fmt --check` and
  `cargo clippy --workspace --all-targets -D warnings` clean;
  `cargo test --workspace` 871 passed; `scripts/cli-acceptance.sh` 451 passed;
  `make test-ios` 94 of 95 passed. The one failure,
  `testTestnetSymbolsKeepTheirLowercasePrefix`, was already failing: Bitcoin
  Signet's catalog symbol is `sBTC`, and neither the catalog nor the test is
  touched here.

## 2026-09-30 — One display locale for words and figures; counted strings agree in number

- **Before:** words came from the shipped table for the reader's language, but
  `AppLocalization.locale` was that bare language (`en`, `zh-Hans`) with no
  region, while decimal separators and currency formatters used
  `Locale.current`. The two could disagree in one sentence. Currency names in
  the picker were twelve hand-kept strings per language. Counted strings always
  read as plural in English ("1 confirmations", "1 assets"), one built the
  plural by appending "es" only when the locale identifier started with `en`,
  and "%lld confirmations" had no Chinese translation at all.
- **After:** `AppLocalization.locale` is the shipped language with the reader's
  region, calendar and numbering; `AmountPresentation`, fiat and gas formatters,
  relative times and percentages all use it. Currency names come from that
  locale (`localizedString(forCurrencyCode:)`) with the ISO code beside them.
  `AppLocalization.format(_:count:_:)` reads `<key>#one` for a count of 1,
  from the same table as `<key>`, so Chinese never borrows the English
  singular; the counted strings carry both forms and "%lld confirmations" is
  translated. `scripts/unused-strings.sh` accepts `#one` keys and fails on one
  whose key is gone.
- **Why:** a figure and the sentence around it followed different settings, a
  language table repeated what the system already names, and English grammar
  was decided by string surgery in one view.
- **CLI check:** none applies — display formatting is the platform's. The
  string gate (`scripts/unused-strings.sh`) covers the tables.
- **Verification:** `scripts/unused-strings.sh` passes; app and test sources
  type-check with `swiftc` under Swift 6. `make test-ios` could not run: the
  working tree's `core/src/api/` does not compile (`ApiError` unresolved),
  independent of this change.

## 2026-09-30 — Refresh events are adopted in the order core sends them

- **Before:** `WalletRefreshObserver` turned every callback into its own
  unstructured `Task { @MainActor }`. Swift does not order those, so a Tor
  status could be overwritten by an older one, and a refresh result could land
  after a later one. The observer held `weak var store` and was
  `@unchecked Sendable`.
- **After:** the observer yields each callback onto an `AsyncStream`; one
  `AppState` task drains it on the main actor, one event at a time. The
  observer is plainly `Sendable`, and the draining task holds only the stream,
  so neither it nor core's engine keeps `AppState` alive.
- **Why:** the last status core reported must be the one shown.
- **CLI check:** none applies — the observer is the platform's end of the
  engine. `testCoreRefreshEngineDoesNotKeepAppStateAlive` covers the lifetime.
- **Verification:** as the entry above.

## 2026-09-30 — A notification the system refuses is logged

- **Before:** price-alert and transaction-status notifications were added with
  no completion handler, so a refusal vanished; the portfolio-movement one,
  written separately, logged it.
- **After:** all three go through one `postNotification` that awaits the add
  and logs a failure under "Notifications".
- **Why:** two copies of one platform call had drifted.
- **CLI check:** none applies — notifications are the platform's.
- **Verification:** as the first entry above.

## 2026-09-30 — App lock covers sheets and alerts, survives relaunch, and ignores `.inactive`

- **Before:** locking swapped `MainTabView` between two branches of an
  `if`/`else`, so every lock and unlock destroyed and rebuilt the whole app
  tree — navigation, scroll positions and an open send were lost. The lock
  card and the app-switcher cover were overlays inside that tree, so a sheet,
  alert or confirmation dialog open at the time sat above them (in the
  snapshot, and after the tree was rebuilt behind a still-presented alert).
  `.inactive` counted as leaving the app: with Face ID and Auto Lock on, the
  system's own Face ID sheet for signing, deleting or revealing a seed locked
  the app behind it, and so did Control Center. A launch never locked, so
  closing the app from the switcher and reopening it got past Auto Lock.
- **After:** both covers are shown in a window of their own above the app's
  (`sceneCover`), so they hide every presentation and the app's tree is never
  rebuilt; the lock card sits on the Spectra backdrop rather than a blur of the
  app. Only `.background` locks and reports the app inactive to core's engine;
  the snapshot cover still appears whenever the scene is not active. A launch
  starts locked when Face ID and Auto Lock are both on.
- **Why:** the lock must cover what the app is showing, whatever presented it,
  and hiding it must not cost the user their place. `.inactive` is not leaving
  the app, and a relaunch is.
- **CLI check:** none applies — scene phases, windows and the lock are
  platform presentation with no core state. Checked by type-checking the app
  target; the behaviour needs a device or simulator run.
- **Verification:** `scripts/unused-strings.sh` passes; the app sources
  type-check with `swiftc` under Swift 6. `make test-ios` could not run: the
  working tree's `core/src/api/` does not compile (`ApiError` unresolved),
  independent of this change.

## 2026-09-30 — Pasted seed phrases split on any whitespace

- **Before:** a phrase pasted into one word field of the import grid was split
  on spaces only, and the funds finder on spaces and tabs, so a phrase copied
  one word per line landed in a single field.
- **After:** both split on any whitespace, newlines and the ideographic space
  included.
- **Why:** the separator a phrase was copied with is not part of the phrase.
- **CLI check:** none applies — the grid is platform input; core's
  `check_seed_phrase` and the CLI take the phrase already split.
- **Verification:** as the entry above.

## 2026-09-30 — The history diagnostic has no 20-second UI deadline

- **Before:** the chain history diagnostic raced the refresh against a
  20-second timer in a task group, swallowing the timeout error.
- **After:** it awaits the refresh; core's HTTP timeouts (10 s connect, 30 s
  request) bound it.
- **Why:** the deadline never held. A task group waits for every child, and a
  UniFFI call does not stop when its Swift task is cancelled, so the action
  returned when the refresh did either way. Its two strings are gone.
- **CLI check:** `spectra` runs history refresh without a UI deadline already;
  nothing changes there.
- **Verification:** as the first entry above.

## 2026-09-30 — Token-2022 mints send when their extensions leave the transfer intact

- **Before:** the Solana send refused any Token-2022 mint that had a single
  extension, whatever it was. PYUSD, on mainnet and Devnet, carries eight, so
  it showed a balance and could not be sent.
- **After:** extensions are admitted by name, each because a plain transfer by
  the owner still means what it says: metadata, group and member records,
  mint close authority, permanent delegate, confidential-transfer configs, a
  transfer hook with no program, and a transfer fee that charges nothing in
  both its current and its scheduled config. A mint with that fee extension
  is sent with `TransferCheckedWithFee` stating a zero fee instead of
  `TransferChecked`. Everything else is refused, now naming the extension: a
  hook program, a fee that can withhold, non-transferable, default account
  state, interest-bearing or scaled amounts, pausable, and anything unlisted.
- **Why:** the blanket refusal was right about the risk and wrong about its
  scope. Two extensions can change what the recipient gets — a fee withholds
  part of the amount, a hook runs another program — and the rest describe the
  mint or act only outside this transfer. Stating the fee turns the chain into
  the check: Token-2022 recomputes it when the transaction lands and fails on a
  mismatch, so a fee switched on after review fails the send rather than
  quietly withholding. Fees that do charge stay refused until review can show
  the amount that arrives.
- **CLI check:** none offline — a send needs a funded Devnet key. Unit tests
  pin the admitted and refused extensions against PYUSD's Devnet mint and the
  two instruction layouts. On Devnet, `prepare_transfer` for 1 PYUSD from a
  holder built a transaction that `simulateTransaction` ran as
  `CreateIdempotent` then `TransferCheckedWithFee`; the same transaction
  stating a fee of 1 failed with "Calculated fee does not match expected fee".
- **Verification:** `make lint`, `cargo test --workspace`, `make test-cli`
  and `make test-ios` pass.

## 2026-09-30 — Testnets host tokens; testnet USDC ships in the catalog

- **Before:** every testnet had an empty `token_standard` in `chains.toml`, and
  `Chain::hosts_tokens` refused testnets outright, so storage rejected any
  protocol token on them and `testnet-tokens.toml` could hold only faucet
  coins. Testing a token send meant a mainnet and real funds.
- **After:** a testnet carries its mainnet's standard (Sepolia and Hoodi
  ERC-20, BNB Testnet BEP-20, Fuji ARC-20, Nile TRC-20, Devnet SPL, and so
  on) and `hosts_tokens` follows the standard alone. `testnet-tokens.toml` adds
  `usd-coin-testnet` (USDC, no market identity) on Ethereum, Arbitrum and Base
  Sepolia, Avalanche Fuji, Polygon Amoy, Hyperliquid Testnet, Solana Devnet,
  Sui, Aptos and NEAR testnets. The asset wiki skips testnet deployments, as it
  already skipped testnet coins. Stellar has no token standard on mainnet
  either, so its testnet USDC is not listed. `paypal-usd-testnet` (PYUSD) sits
  beside it on Ethereum and Arbitrum Sepolia, Polygon Amoy, X Layer Testnet and
  Solana Devnet. The Devnet mint is Token-2022; sending it is the next entry.
- **After, networks:** seven EVM testnets join the registry, each with its
  faucet coin and USDC: Linea Sepolia (59141), Celo Sepolia (11142220), Cronos
  Testnet (338), zkSync Era Sepolia (300), Sonic Testnet (14601), Ink Sepolia
  (763373) and X Layer Testnet (1952). Every chain id was read back with
  `eth_chainId`, and every USDC contract answered `symbol()` `USDC` and
  `decimals()` 6. RPC rows: Linea's PublicNode (the official
  `rpc.sepolia.linea.build` answers every `eth_getBalance` with -32603), Celo
  Forno, Cronos `evm-t3`, zkSync, Sonic Labs, Ink's Gelato and PublicNode, and
  two X Layer hosts. Blockscout history for Celo Sepolia, zkSync Sepolia and
  Ink Sepolia; Routescan serves none of the seven, and the other four have no
  keyless indexer, so they have no history, as BNB Testnet and Amoy have none.
- **Why:** a testnet is where a token path should be exercised first; the rule
  kept it untestable anywhere but mainnet. Valuation already prices testnet
  holdings at zero, so hosting tokens there adds no priced asset. A test now
  pins each testnet's standard to its mainnet's.
- **CLI check:** `spectra token catalog --chain base-sepolia` lists USDC at
  `0x036cbd53842c5426634e7929541ec2318f3dcf7e`; `spectra token track --chain
  base-sepolia USDC` reports it already tracked. `spectra endpoints --chain
  ink-sepolia` probes three answering endpoints; after `spectra network set
  zksync-era-sepolia`, `spectra refresh` reads the Sepolia balance in tETH.
- **Verification:** `make lint`, `cargo test --workspace`, `make test-cli`
  and `make test-ios` pass.

## 2026-09-30 — No float on the send path: fees, maxima and history amounts are exact

- **Before:** every send preview carried its fee, spendable balance and
  maximum as `f64`, built by dividing integer units by a float factor
  (satoshis by `1e8`, wei by `1e18`, a TRX balance by `1e6`). The quote turned
  that float back into a decimal for the affordability check, into fee units
  for signing, and into amount shortcuts — the last through a helper that
  subtracted one ULP to cover the rounding it had just introduced, so a
  maximum of 4.2 offered 4.199999. The registry's fallback fees, NEAR's token
  gas reserve, the request's Sui gas budget, Cardano fee and fee rate, EVM
  custom fees in gwei, the replacement fee bump (`× 1.2` printed with
  `{:.3}`), EVM receipt costs and history amounts were all `f64`. Core's own
  preview JSON carried floats between two of its functions. Five
  intermediate preview records and three Dogecoin duplicates
  (`spendableBalanceDoge`, `maxSendableDoge`, `requestedAmountDoge`) crossed
  the FFI unread. `estimated_fee_rate_per_kb` and
  `ResolvedPendingStatus.confirmed_network_fee` had no writer.
- **After:** those values are exact decimal strings, computed in the chain's
  smallest unit and converted once with `decimal::from_units`. A maximum is
  the fee subtracted exactly (`decimal::sub_or_zero`), a shortcut is a share
  of those units, and 4.2 offers 4.2. EVM fees are whole wei: a custom fee
  finer than a wei, zero, or past u64 is refused rather than rounded; the
  replacement bump is `ceil(wei × 1.2)`, at least 0.1 gwei. Core's preview
  JSON carries decimal strings and wei integers, and a float there is
  refused. History amounts are exact where the source gave units or a
  decimal string, and the shortest spelling of what a float-only source said
  otherwise; multi-address netting sums the legs exactly. The unread records,
  duplicates and writerless fields are deleted. Floats remain only for
  quantities that are approximate by nature — prices, display-currency
  values, portfolio totals, timestamps, the large-send percentage, and a
  node's fractional sat/vB estimate, which the Bitcoin builder rounds up to
  whole satoshis.
- **Why:** a fee, a balance and a maximum are funds. Each float conversion
  rounded somewhere, and the code had grown compensation for its own
  rounding (the ULP reservation, `{:.3}`) instead of not rounding.
- **CLI check:** `spectra --json send fees --max-fee ' 30.25 ' --priority-fee
  1` prints `"maxFeePerGasGwei":"30.25"`; `spectra send fees --max-fee
  1.0000000001 --priority-fee 1` is refused; `spectra --json history …`
  prints amounts as strings (`"amount":"2.5"`). Offline coverage:
  `send::preview_decode::tests`, `decimal::tests`,
  `send::flow::…evm_bump_scales_existing_to_the_wei`.
- **Verification:** `make verify` — rustfmt and clippy clean, 864 core
  tests, 453 CLI acceptance checks, iOS test suite passed. The working tree
  also held other sessions' uncommitted changes, which the run included.

## 2026-09-30 — A chain crosses every boundary as `Chain`, not as its id string

- **Before:** about 160 record fields and most service parameters carried a
  chain as `chain_id: String`, and core re-parsed it with `Chain::from_str_id`
  roughly 150 times, each with its own "unknown chain" branch. The FFI took
  and returned strings, so Swift built ids with `chain.id` and turned them
  back with `Chain(id:)` / `Chain.displayName(forId:)`. The family → network
  selection was a `String → String` map. Maps keyed by chain (endpoint index,
  keypool owned addresses, diagnostics, watch-only entries) were keyed by
  string. The foreign-address send warning matched the exact ids `tron`,
  `solana`, `xrp` and `monero`, so their testnets never raised it. EVM
  recipient warnings on iOS printed the chain *id* ("ethereum") where the
  sentence wanted its name.
- **After:** records, parameters, map keys and warning payloads carry
  `registry::Chain`; the family selection is `Chain → Chain`. `Chain`
  serializes, stores in SQLite and prints as its catalog id, so stored JSON,
  columns and CLI output keep the same spelling. A row naming an id the catalog
  does not know now fails where it is read instead of travelling as a string.
  String parsing remains only where text arrives: CLI arguments, the catalog
  TOML files and database columns. The foreign-address warning applies to a
  family's testnets too, and iOS words EVM recipient and high-risk warnings
  with the chain's display name. Tests that fed a made-up chain id to check
  the "unknown chain" branches are gone with the branches. The chain catalog's
  ids are read on their own, so the token catalog can name chains while the
  chain catalog is still being built.
- **Why:** one chain identity instead of two spellings of it. A misspelt id
  was a runtime refusal (or a silent miss in a string comparison, as with the
  testnet warnings); now it does not compile. Storage refuses early rather
  than carrying an id nothing can resolve, the stricter side for funds and
  addresses.
- **CLI check:** `spectra --json wallet list` and `spectra --json send review
  …` print `"chain":"bitcoin"`-style ids exactly as before;
  `scripts/cli-acceptance.sh` passes unchanged apart from a comment.
- **Verification:** `make verify` — rustfmt and clippy clean, 869 core
  tests, 453 CLI acceptance checks, iOS test suite passed.

## 2026-09-30 — Token balances are stored exactly; one decimal formatter

- **Before:** sixteen chain clients each formatted smallest units to a decimal
  string by hand, and several capped the result at six fractional digits.
  EVM ERC-20, SPL and TRC-20 balances went through the capped formatter and
  were stored on the holding truncated, so an 18-decimal token lost its
  dust: the stored balance, the "exceeds the available balance" check and a
  send-max all read the truncated figure. A malformed amount from an indexer
  parsed as `0`. Native balance structs carried `*_display` strings nothing
  read.
- **After:** `decimal::from_units` / `from_unit_digits` are the only
  conversions, exact at any size; balances are stored with every digit. A
  malformed indexer amount is an error, not a zero transfer. The unread
  display fields and the per-chain formatters are deleted. Display rounding
  is the front end's job.
- **Why:** a balance is funds; truncating it is guessing. Sixteen copies of
  one conversion had already drifted (capped and uncapped).
- **CLI check:** none offline — the affected path is a live balance read.
  Covered by `decimal::tests::unit_digits_of_any_size` and the adapter tests.
- **Verification:** as the entry above.

## 2026-09-30 — One rule decides whether a holding can be sent

- **Before:** two rules. The send button used `SendRule` (EVM: native or a
  tracked token; ETC and Hyperliquid native only; everything else any asset,
  so Sui, Aptos and TON tokens showed a send button). The preflight used
  `route_send_asset`, which matched chain *display names* to string
  "submit kinds" nobody read, let an untracked EVM or Tron token through with
  no contract, and returned a `preview_kind` always equal to `submit_kind`.
  The route was exported (`send_asset_routing`) and printed by
  `spectra send review`.
- **After:** `send::SendAsset` answers for both: every chain sends its native
  asset; a token only when the user tracks it and `Chain::sends_tokens()`
  (EVM except ETC and Hyperliquid, Solana, Tron, NEAR — the chains whose
  builder has a token transfer). `SendRule`, the routing record, its export
  and the unread `SendPreflight` fields are gone; the preflight carries
  `chain` and the exact `amount` string instead of an `f64`.
  `spectra send review` prints only `{"preflight": …}`.
- **Why:** two models of one question disagreed in both directions. An
  untracked token has no contract or scale to send with, so it is refused.
- **CLI check:** `spectra --json send review --wallet <w> --holding <h>
  --amount 1 --destination <addr>` on an untracked token answers
  "… transfers are not enabled yet."; covered offline by
  `send::tests::an_untracked_token_is_refused` and
  `send::transfer::tests`.
- **Verification:** as the first entry.

## 2026-09-30 — Zcash, Bitcoin Gold and fifteen testnets have providers

- **Before:** Zcash and Bitcoin Gold had no endpoint, because their Trezor
  Blockbook rows had died. Fifteen testnets had none either: Litecoin, BSV,
  Kaspa, Hyperliquid, Tron Nile, Solana Devnet, XRP, Stellar, Aptos, TON, NEAR,
  Westend and Monero stagenet, plus ETC Mordor (history only, no node) and
  Avalanche Fuji (a node, no history). Dogecoin, Litecoin, Bitcoin Cash and
  Dash spoke Blockbook but listed no Blockbook instance.
- **After:** 33 rows are added, each probed live on 2026-09-30.
  - Zelcore's Blockbook serves Zcash, Bitcoin Gold, Dash, Dogecoin, Litecoin
    and Bitcoin Cash, and Atomic Wallet's serves Dash. The Zcash row claims no
    `fee`: zcashd has removed `estimatefee`, and Zcash fees are ZIP-317,
    computed in core.
  - Testnet rows use the same APIs as their mainnets: litecoinspace testnet,
    WhatsOnChain `bsv/test`, Kaspa TN10, Hyperliquid's own RPC and dRPC, three
    Mordor RPCs, Routescan for Fuji history, TronGrid Nile, the Solana devnet
    RPC, XRPL altnet and XRPL Labs, Horizon testnet, the Aptos Labs testnet,
    toncenter testnet v2/v3, NEAR's RPC and FastNEAR with NearBlocks testnet,
    Westend's official RPC, and four stagenet daemons.
  - No testnet row claims `staking`, which only mainnets support.
  - Still without a provider: the Bitcoin Cash, Dogecoin, Zcash and Dash
    testnets (none found), and the Decred testnet (`testnet.dcrdata.org`
    answered 503). Dash testnet's Insight server would need Dash to speak
    Insight.
  - `cli-endpoints.py` exercised health against a keyless EVM testnet, but none
    remains. It now uses a Blockbook fixture on `zcash-testnet`; core's
    `evm_checks_reads_after_chain_identity_and_rejects_wrong_network` still
    covers EVM's identity-then-reads order.
- **Why:** a chain with no row cannot read a balance, estimate a fee or
  broadcast. Zcash and Bitcoin Gold were mainnets offered in the app with no
  working source at all.
- **CLI check:** `spectra endpoints --chain zcash` reports
  `https://blockbook.zec.zelcore.io` reachable; `spectra endpoints --chain
  monero-stagenet` reports four reachable daemons.
- **Verification:** run on the same `git archive HEAD` export as the entry
  below, for the same reason. 879 core tests and 453 CLI acceptance checks
  passed. No Rust changed in this entry, so clippy was not rerun. `test-ios`
  was not run.

## 2026-09-30 — Providers already in the catalog serve every chain they answer for

- **Before:** PublicNode, 1RPC, Blockscout, BlockCypher, mempool.emzy.de and
  Xray were each listed for only some of the chains they serve. Dash, Cardano
  Preprod, Sui Testnet and six EVM testnets (Arbitrum/Optimism/Base Sepolia,
  BNB Testnet, Avalanche Fuji, Polygon Amoy) had no endpoint at all. Sepolia and
  Hoodi had one RPC and no history. Sei, Celo, Cronos, opBNB, Sonic,
  Berachain, Unichain, Ink and Hyperliquid each depended on a single RPC.
  Bitcoin spoke BlockCypher but listed no BlockCypher row.
- **After:** 40 rows are added. PublicNode adds RPCs for BNB and its testnet,
  opBNB, Sonic, Berachain, Unichain, Celo, Cronos, Sei, Ink, Hyperliquid,
  Polkadot, Sui Testnet and the six EVM testnets. 1RPC adds BNB, opBNB,
  zkSync Era, Celo, Cronos, Sonic, Unichain, Hyperliquid, Sui, Sepolia and
  Hoodi. Blockscout adds history for Sepolia, Hoodi, Arbitrum/Base/Optimism
  Sepolia and ETC Mordor. BlockCypher adds Bitcoin and Dash mainnet;
  mempool.emzy.de adds Signet and Testnet4; Xray's Koios adds Cardano Preprod.
  Each was probed live on 2026-09-30. Left out: 1RPC Mantle and Aptos (Mantle
  answers `eth_chainId` and refuses every read without a paid plan; the Aptos
  node is pruned), BlockCypher `btc/test3` (about 150,000 blocks behind), and
  every BlockPI public RPC except Sui (521, "Payment Required" or "Apikey not
  found"). Sui Testnet claims no `staking`, which the adapter refuses there. A
  testnet may now name its own explorer; it still never inherits its mainnet's.
- **Why:** a provider already trusted for one chain costs nothing to use on
  another, and a chain with no row has no way to read, fee or broadcast. The
  "testnets have no history" assertion recorded what the catalog held, not a
  rule. The rule is the one from 2026-09-23: a testnet never inherits mainnet
  history.
- **CLI check:** `spectra endpoints --chain dash` reports
  `https://api.blockcypher.com/v1/dash/main` reachable; `spectra endpoints
  --chain arbitrum-sepolia` reports the PublicNode RPC and the Blockscout
  explorer reachable.
- **Verification:** run on a `git archive HEAD` export with these changes
  applied, because the working tree held an unrelated refactor in progress
  that did not compile. rustfmt and clippy were clean; 879 core tests and 453
  CLI acceptance checks passed. `test-ios` was not run: the same refactor
  blocks the build. The Sepolia and Hoodi expectations in
  `testEthereumTestNetworksExposeExpectedContextsAndEndpoints` were updated
  but have not been run.

## 2026-09-29 — CLI acceptance is confined to loopback

- **Before:** `scripts/cli-acceptance.sh` claimed "no external network", but
  nothing enforced it. A fixture endpoint was raced against the chain's
  built-in providers, so `cli-send.py` read Ethereum mainnet through
  publicnode and 1rpc on every run. Its password test validated those nodes and
  sent them the signed transaction as well (an owned send validates and
  submits to every configured broadcast endpoint), and passed only because
  they answered. `cli-diagnostics.py` read Solana validators from the public
  RPCs the same way. A catalog change could turn an offline failure into a
  live read without anyone noticing, as Polkadot's balance rows did.
- **After:** when `SPECTRA_LOOPBACK_ONLY` names a journal file, core confines
  the process to loopback hosts. Endpoint selection (`chain_endpoints`,
  `api_endpoints`) leaves remote endpoints out, so a chain is served by its
  loopback fixtures alone. The listings (`endpoints --catalog`, `configured`)
  are unchanged. Every other remote request is routed to a dead proxy and never
  sent, and so is an embedded Tor bootstrap. Each is journaled with its
  host and command line. The acceptance script exports the variable for the
  whole run, including the Python suites. The exception is `cli-transport.py`:
  its local SOCKS proxy carries requests that name remote hosts. The script
  fails if the journal is not empty, and a canary check first proves that a
  health probe of the built-in Bitcoin providers is caught. `cli-endpoints.py`
  now lists selection with loopback URLs that are never called. The variable
  is unset in the app and in `cargo test`, where nothing changes.
- **Why:** an acceptance check must pass or fail on core's rules, not on the
  day's chain state or a provider's uptime, and it must never broadcast.
  Refusing silently would still let a check pass on an offline error it did
  not mean to test, so every refusal fails the run and names the command.
- **CLI check:** `SPECTRA_LOOPBACK_ONLY=/tmp/journal spectra --json endpoints
  --chain bitcoin` reports every provider unreachable, and `/tmp/journal` lists
  `https://blockstream.info/`, `https://mempool.space/` and
  `https://mempool.emzy.de/`. `scripts/cli-acceptance.sh` ends with "no
  command reached beyond loopback".
- **Verification:** `make lint test test-cli`: rustfmt and clippy clean, 879
  core tests plus the transport test, 453 CLI checks. `test-ios` not run: no
  Swift or FFI surface changed.

## 2026-09-29 — Polkadot and Bittensor balances; Bitcoin Cash has an indexer

- **Before:** Polkadot and Bittensor balance reads failed with "no keyless
  balance source configured". Bitcoin Cash had no catalog endpoint at all: its
  Blockbook rows had died and the BCH REST v2 rows were removed with the
  adapter-less APIs, so balance, history, UTXOs and broadcast all failed. The
  BCH/Litecoin fee preview refused with "No fee endpoint configured" when no
  endpoint quoted one. Tron had one node HTTP endpoint.
- **After:** `SubstrateClient::fetch_balance` reads the `System.Account`
  record with `state_getStorage` and decodes it at the chain's
  `Chain::substrate_balance_bytes` (Polkadot `u128`, subtensor `u64`); a record
  of any other length is refused. The balance shown is the transferable one:
  `free` less what `frozen` holds beyond `reserved`, so staked funds are not
  offered to a send. The catalog's Substrate rows declare `balance`. A new
  `bch-rest-v2` adapter serves Bitcoin Cash through `UtxoClient` (balance,
  history, UTXOs, status, broadcast; no fee quote), with
  `https://rest.bch.actorforth.org/v2` in the catalog. Its inputs report BCH in
  a field named `valueSat`; the adapter reads them as BCH. The Litecoin/BCH
  preview uses a live fee where one answers and 1 sat/B otherwise. Tron gains
  `https://tron-rpc.publicnode.com` as a second node HTTP endpoint. OnFinality's
  public Hyperliquid RPC was probed and left out: it answers "Too Many
  Requests" after two calls and stays refused.
- **Why:** Substrate nodes hold account balances as plain storage; no indexer
  or key was ever needed. Bitcoin Cash had no working source at all, and the
  one keyless indexer still answering needed only an adapter.
- **CLI check:** watch `13UVJyLnbVp9RBZYFwFGyDvVd1y27Tt8tkntv6Q7JVPhFsTB` on
  Polkadot and `spectra --json balance <wallet>` reports its DOT; watch a
  Bitcoin Cash address and `spectra --json history <wallet>` lists it.
  `spectra --json endpoints --catalog` lists the BCH REST row under
  `configured` for `bitcoin-cash`.
- **Verification:** all four suites passed: rustfmt/clippy, 877 core tests plus
  the transport test, 450 CLI acceptance checks and 95 iPhone simulator tests;
  the CLI check above was also run against the live networks.

## 2026-09-29 — One adapter per API; `send` only builds and signs

- **Before:** `fetch/` had one file per chain, and several held two APIs:
  `evm.rs` the node and Blockscout, `ton.rs` TON Center v2 and v3 behind one
  client with two endpoint lists, `tron.rs` the node and TronGrid v1, `near.rs`
  the node and Nearblocks. `polkadot.rs` and `bittensor.rs` were the same
  Substrate client twice. Eighteen `send/` files added methods to fetch
  clients, so an API's wire code was split between `fetch/` and `send/` —
  BlockCypher's broken broadcast sat in `send/dogecoin.rs`. NEAR history asked
  only the first Nearblocks endpoint.
- **After:** `core/src/api/` holds one module per `EndpointApi`, named after its
  `as_str()`, with that API's requests, parsing and submission, whichever chain
  uses it: `esplora`, `whatsonchain`, `insight`, `koios`, `horizon`,
  `substrate_json_rpc` (one `SubstrateClient`), `blockscout`, `toncenter_v3`,
  `trongrid_v1`, `nearblocks` and `monero_daemon_rpc` among them. `api` is a
  root module with its transport (`http`, `json_rpc`), the shared answer types
  and provider timestamp parsing, and depends on none of `fetch`, `send`,
  `staking` or `service`, which all call it. `send/` builds and signs; its
  fetch-then-build flows are free functions taking a client.
  NEAR history races every Nearblocks endpoint. Polkadot and Bittensor balance
  and history refuse in the service instead of in two stub clients.
- **Why:** an adapter is a wire contract, not a chain; naming files after
  chains hid which code spoke which API and let one API's code live in two
  directories.
- **CLI check:** none shows file layout; `spectra --json history` on a NEAR
  wallet reads Nearblocks as before. `make test` covers the moved adapters,
  including `substrate_balance_and_history_are_errors_not_empty_wallets`.
- **Verification:** all four suites passed: rustfmt/clippy, 871 core tests plus
  the transport test, 450 CLI acceptance checks and 95 iPhone simulator tests.

## 2026-09-29 — Every endpoint is asked at once, in every API a chain speaks

- **Before:** `with_fallback` tried a chain's endpoints top to bottom with
  180 ms between failures, so catalog order decided which provider served
  every request and a dead one cost a timeout on each call. Each chain had one
  `primary_api()`, and a URL speaking any other API was dropped from its list:
  Litecoin's primary was Blockbook, whose rows were removed in September, so
  its Esplora and BlockCypher rows were never used and Litecoin had no working
  endpoint. BlockCypher broadcasts went to Blockbook's `/api/v2/sendtx/` and
  could not succeed. The four UTXO adapters disagreed on meaning: WhatsOnChain's
  balance included unconfirmed funds, Esplora's `utxo_count` was a transaction
  count, BlockCypher's UTXO list left out mempool outputs, and history came in
  three shapes (`net_sats`, `amount_sat`, `amount_koin`). A user's broadcast to
  several selected endpoints went to one after another.
- **After:** `race` asks every endpoint at once and answers with the first
  success; only when all fail is it an error. `Chain::endpoint_apis()` lists
  every API a chain's own client speaks — Bitcoin Esplora and BlockCypher;
  Litecoin Blockbook, Esplora and BlockCypher; Dogecoin BlockCypher and
  Blockbook; Dash Blockbook and BlockCypher — and every URL in any of them is
  used. `api::utxo::UtxoClient` serves the UTXO family over all of them in
  one set of types: confirmed balance plus mempool delta, UTXOs including the
  mempool's, one history shape, newest first. Only Esplora can continue
  history past the first page; the others refuse a cursor and the race
  answers from Esplora. BlockCypher broadcasts to `/txs/push` and quotes fees.
  Broadcasts, the user's selected ones included, go to every endpoint
  together and each runs to its end. `NativeBalanceSummary.utxo_count` and
  `utxoCount` in `spectra balance` are gone. A configured URL outside the
  directory is still read as the chain's `default_api()`.
- **Why:** order had become an unstated preference between providers, and a
  second API on a chain was a silent dead row rather than redundancy — the
  `primary_api` filter hid that Litecoin had nothing left. One client answering
  in one meaning is what lets endpoints of different APIs be interchangeable.
- **CLI check:** `spectra --json endpoints --catalog` lists both
  `https://litecoinspace.org/api` and `https://api.blockcypher.com/v1/ltc/main`
  under `configured` for `litecoin`, where it listed nothing.
- **Verification:** all four suites passed: rustfmt/clippy, 872 core tests plus
  the transport test, 450 CLI acceptance checks and 95 iPhone simulator tests.

## 2026-09-29 — Donation addresses are core's, checked when they load

- **Before:** `resources/Donations.json` held the Donate screen's five
  addresses. Swift decoded it, and only a Swift test checked that each was a
  valid address on the chain it named.
- **After:** `core/data/donations.toml` holds them. Loading refuses an unknown
  or test network, a second address for one network, and any address that is
  not already its chain's valid, normalized form. Swift reads
  `donation_destinations()`; `spectra donations` prints the same list. The
  JSON resource and the Swift test are gone.
- **Why:** these are funds destinations; AGENTS.md asks for them to be
  validated by core before anything shows them, not trusted from a platform
  resource.
- **CLI check:** `spectra --json donations` lists the five addresses.
- **Verification:** all four suites passed: rustfmt/clippy, 867 core tests plus
  the transport test, 450 CLI acceptance checks and 95 iPhone simulator tests.

## 2026-09-29 — The endpoint catalog claims only what an adapter can do

- **Before:** 14 of 111 catalog rows claimed capabilities their API's adapter
  does not implement. Eight APIs (Blockchair, blockchain.info, BCH REST v2,
  SoChain v2, XRPScan, Substrate Sidecar, Tron JSON-RPC, Tronscan) had no
  adapter at all; their 12 rows only appeared on the Endpoints screen and in
  health checks. TON v3 claimed balance and history, TronGrid v1 balance and
  token balance. `probe_url` sat on 57 rows and was read for seven, all
  Blockchair or BCH REST.
- **After:** loading refuses a row that claims a capability outside
  `endpoint_capability_options` for its API on that network. The 12
  adapter-less rows, the eight `EndpointApi` variants, their health checks
  and `probe_url` are removed. TON v3 declares token balance and discovery;
  TronGrid v1 declares history, token history and token discovery. Bitcoin
  Cash is left with no catalog endpoint, which is what its client already
  saw.
- **Why:** a capability is documented as a claim that has to be true, and the
  Endpoints screen showed these as working services. An API nothing can call
  is not an endpoint.
- **CLI check:** `spectra --json endpoints --catalog` has no row whose
  `capabilities` exceed its `supportedCapabilities`, and no `probeURL`.
- **Verification:** all four suites passed: rustfmt/clippy, 867 core tests plus
  the transport test, 450 CLI acceptance checks and 95 iPhone simulator tests.

## 2026-09-29 — Tron history reads TronGrid

- **Before:** Tron history needed a Tronscan endpoint, and the keyless
  catalog had none after 2026-09-21, so every Tron history read failed with
  "No Tron history indexer configured" before any request. The Tronscan
  client also turned a failed read into an empty history.
- **After:** `TronClient::fetch_history` reads TronGrid's v1 account API
  (`/transactions` and `/transactions/trc20`, confirmed only), the one the
  catalog already lists for token discovery. It keeps successful TRX
  transfers and TRC-20 `Transfer` events, converts node hex addresses to
  base58check, formats token amounts exactly, and returns an error when a
  read fails. The account bases come from `tron_account_endpoints`, shared
  with token discovery: catalog and custom `trongrid-v1` rows, or each primary
  node's own `/v1/accounts` under an explicit override. The Tronscan client is
  removed.
- **Why:** a chain with a keyless indexer in the catalog should not have
  broken history, and an empty history is not an honest answer to a failed
  read.
- **CLI check:** `spectra history <tron wallet>` lists transfers;
  `scripts/cli-history.py` covers it against a loopback TronGrid.
- **Verification:** all four suites passed: rustfmt/clippy, 867 core tests plus
  the transport test, 450 CLI acceptance checks and 95 iPhone simulator tests.

## 2026-09-28 — Endpoint slots and capability strings are gone

- **Before:** services beside a chain's own client were reached through
  `EndpointSlot`: `Chain::endpoint_api(slot)` hard-coded TON v3 as the
  Secondary slot and Nearblocks and XRPScan as the Explorer slot, and their
  lists were keyed by strings such as `"ton:secondary"` and `"near:explorer"`,
  parsed back with `split_once(':')` in three places. Blockscout and TronGrid
  were already asked for by API through `api_endpoints`, so routing had two
  models. Nothing read the XRP explorer list, and Tron history read
  `"tron:explorer"`, a slot with no API. A transport override with an unknown
  chain id was accepted and ignored. Capabilities were strings everywhere:
  checked against a list when the catalog loaded, packed into
  `ENDPOINT_CAPABILITY_*` bitmasks (staking an unnamed `1 << 14`) for
  filtering, and compared by literal at about 300 call sites. Custom
  endpoints stored them sorted alphabetically.
- **After:** `Chain::primary_api()` names the one API a chain's client
  speaks. `endpoints_for` is that API's list; every other service is
  `api_endpoints(chain, api, capabilities)`, custom endpoints first and then
  the catalog, whether or not the transport was overridden. `ChainEndpoints`
  is a chain's primary list and refuses an unknown network, and
  `spectra endpoints --catalog` reports one `configured` row per chain.
  `EndpointCapability` is an enum with the same kebab-case names, used by the
  catalog, records, custom endpoints, probes, overrides, `AddCustomEndpoint`
  and `spectra endpoints --capabilities`, so a misspelt capability fails to
  parse. Filters take a capability slice, the masks are gone, custom
  endpoints store capabilities in declaration order, and Swift localizes a
  capability through `endpoint_capability_id`.
- **Why:** a slot was a second name for an API, with string keys to carry
  it, and it hid that Tron history had no route. A capability string could be
  misspelt anywhere except the catalog file.
- **Not fixed:** Tron history still fails with "No Tron history indexer
  configured": it needs a Tronscan endpoint, and the keyless catalog has had
  none since 2026-09-21.
- **CLI check:** `spectra --json endpoints --catalog` has no `configured`
  key containing `:`; `spectra endpoints --chain base --api evm-json-rpc
  --capabilities native-history --add https://base.example` exits 2.
- **Verification:** all four suites passed: rustfmt/clippy, 862 core tests plus
  the transport test, 449 CLI acceptance checks and 96 iPhone simulator tests.

## 2026-09-28 — Transaction explorers moved out of the endpoint catalog

- **Before:** `endpoints.toml` held 44 transaction-explorer pages beside the
  APIs, as rows with no `api`, `capabilities = []`, an `explorer_label` such
  as "Open In Etherscan" and, for Aptos only, a `tx_suffix`. Loading had to
  refuse capabilities on a link and link fields on an API, and every consumer
  of `AppCoreEndpointRecord.api` handled `None`. The explorer URLs, plus an
  unused `hyperliquid.explorer.web` link, were listed on the Endpoints screen,
  in `spectra endpoints --catalog` and in the settings groups, and
  `spectra endpoints` counted each one as an endpoint "with no probe". Swift
  built the transaction URL by concatenating prefix, hash and suffix, and
  showed the English label untranslated.
- **After:** `explorers.toml` holds one row per network: `chain_id`, the
  explorer's `name` and a `tx_url` template with `{hash}`, checked at load
  (known network, one row each, HTTPS, exactly one `{hash}`).
  `Chain::transaction_explorer`, `transaction_explorers()` and
  `transaction_explorer_link()` (name and finished URL) replace
  `AppCoreChainEndpoints.transaction_explorer`, `AppCoreExplorerEntry` and
  the record's `explorer_label`/`tx_suffix`. Every endpoint row must declare
  an `api` and at least one capability, so `AppCoreEndpointRecord.api` and
  `EndpointProbe.api` are no longer optional. The Endpoints screen lists APIs
  only; a new Settings → Explorers screen lists the explorers, and the
  button reads a localized "Open In %@". The Hyperliquid web link is gone.
- **Why:** an explorer is a page the app opens, never a service it
  requests. Keeping two models in one table cost two cross-field rules, an
  optional API everywhere and a suffix field for one chain; separated, the
  types say it.
- **CLI check:** `spectra --json explorers --chain Aptos --tx 0xabc` prints
  `https://explorer.aptoslabs.com/txn/0xabc?network=mainnet`;
  `spectra --json endpoints --catalog` has no row without an `api`.
- **Verification:** all four suites passed: rustfmt/clippy, 863 core tests plus
  the transport test, 448 CLI acceptance checks and 96 iPhone simulator tests.

## 2026-09-28 — Monero's wallet-RPC remnants are gone

- **Before:** `SendBroadcastMode` crossed the FFI on every `ChainIdentity`,
  `spectra --json chains` printed it as `sendBroadcastMode`, and the send
  network card switched over it. Its `PreparesWithBackend` variant, documented
  as "Monero has no in-process wallet to sign with", was no longer constructed
  once Monero began signing locally, so `send_broadcast_mode()` returned
  `SignsAndBroadcasts` for every chain. `EndpointApi` still carried
  `monero-wallet-rpc` and `monero-light-wallet`, and balance, history and
  broadcast kept `MoneroWalletRpc` branches backed by `fetch/monero.rs` and
  `send/monero.rs`; Monero routes through `monero-daemon-rpc`, so none was
  reachable.
- **After:** the enum, the `ChainIdentity.send_broadcast_mode` field, the CLI
  `sendBroadcastMode` key and the backend-preparation sentence with its three
  translations are removed; the card always says Spectra signs and broadcasts
  the chain's transfers in-app. Both Monero API variants, their dispatch arms
  and the wallet-RPC client are removed, so a custom endpoint declaring either
  API is refused.
- **Why:** a one-variant choice is a constant, and every removed piece
  described a server-side Monero wallet that the local scan/sign engine
  replaced on 2026-09-22.
- **CLI check:** `spectra --json chains --filter monero` has no
  `sendBroadcastMode` key; `spectra endpoints --chain monero --api
  monero-wallet-rpc --capabilities fee --add URL` exits 3.
- **Verification:** all four suites passed: rustfmt/clippy, 862 core tests plus
  the transport test, 442 CLI acceptance checks and 96 iPhone simulator tests.

## 2026-09-28 — Fee priority is gone: no send path spent it

- **Before:** every non-EVM send page and the Bitcoin-family diagnostics
  screen offered an Economy / Normal / Priority picker, stored per chain in
  `AppSettings.fee_priority_by_chain` and settable as
  `spectra settings set fee-priority.<chain>`. The UTXO page said Spectra
  "applies it to live send previews". Core read it in one place: the Dogecoin
  preview, which echoed it back as a `feePriority` label. No build, sign or
  fee estimate on any chain used it.
- **After:** the setting, its reducer case, the `FeePriority` enum, the CLI
  `fee-priority.*` keys, the Dogecoin preview's `feePriority` field and every
  picker are removed. A stored `feePriorityByChain` is an unknown setting and
  is refused like any other.
- **Why:** a funds-affecting choice that changes nothing tells the user a fee
  was chosen when it was not. Making it real is a new feature; keeping it was
  a false one. The diagnostics screen gated its copy of the picker on the
  endpoint catalog having Esplora bases, a chain fact read from the wrong
  place, so `AppCoreChainEndpoints.bitcoin_esplora` went with it.
- **CLI check:** `spectra settings set fee-priority.Dogecoin economy` is
  refused; `spectra --json settings list` has no `fee-priority` key.
- **Verification:** all four suites passed: rustfmt/clippy, 863 core tests plus the
  transport test, 443 CLI acceptance checks and 96 iPhone simulator tests.

## 2026-09-28 — Core owns chain diagnostics and builds the bundle

- **Before:** a chain's diagnostics screen kept the last endpoint check and
  the last history-run time in Swift, for this launch only, and passed them
  back to `diagnostics_json` to be serialized. The bundle was assembled in
  Swift from those values. `spectra diagnostics show` passed empty endpoints
  and no times, so the CLI and the app produced different documents for the
  same chain. The endpoint check probed the family's mainnet even when a
  testnet was selected; the self-test results were stored and never shown;
  before any check the screen listed catalog endpoints as "Not checked yet".
- **After:** core records history rows, the history-run time, endpoint probes
  and the check time in its diagnostics registry. `WalletService.chain_diagnostics`
  answers for a family on its selected network — counts, sources by use,
  endpoints and the document — and `diagnostics_bundle` builds the whole
  bundle (schema 2) from core state plus the platform's version, OS, locale
  and time zone. The document names `chainId` and `network`, gives times as
  `null` until a run happens, and marks endpoints nothing can probe as
  `checked: false`. The screen re-reads after every run, so self-test
  outcomes appear in its operational events; it lists endpoints only once
  checked (the endpoint catalog screen lists them before that).
- **Why:** the export was Swift reading session state only to hand it back
  for a decision, and the CLI could not reproduce it. One owner removes the
  split, the forwarding `StandardChainDiagnosticsDispatch` and the fields
  nothing read (`lastRunAt` for self-tests and rescans, the last imported
  bundle).
- **CLI check:** `spectra --json diagnostics show --chain Bitcoin` prints
  `"network":"bitcoin"`; `spectra diagnostics bundle` prints the bundle with
  `chainDiagnosticsJson` and `walletCount`.
- **Verification:** all four suites passed: rustfmt/clippy, 863 core tests plus the
  transport test, 443 CLI acceptance checks and 96 iPhone simulator tests.

## 2026-09-28 — "Last checked" is core's clock; balances land per wallet

- **Before:** Swift stamped its own "last checked" time whenever a refresh
  returned pending results, failures included. During a balance sweep core
  collected every wallet before reporting any, and the app ignored per-wallet
  reports anyway, so every balance waited for the slowest chain.
- **After:** `AppRefreshResult.pending_checked_at_unix` carries core's
  `refresh_clock.pending_transactions_at`, stamped only by a failure-free
  check, and the app shows that. The engine reports each wallet as its
  balances are committed, and the app re-reads the portfolio at most every
  300 ms while a sweep is landing, then once more when it completes.
- **Why:** core already owned the clock; the Swift copy disagreed with it on
  failure. Holding every balance for the slowest provider was an artefact of
  collecting the stream, not a rule.
- **CLI check:** `spectra --json diagnostics refresh --intent … --conditions …`
  prints the result with `pending_checked_at_unix`. Per-wallet timing has no
  CLI check; the observer contract documents it.
- **Verification:** all four suites passed: rustfmt/clippy, 863 core tests plus the
  transport test, 443 CLI acceptance checks and 96 iPhone simulator tests.

## 2026-09-28 — Generic secrets move to the `com.spectra.wallet` Keychain service

- **Before:** sealed-wallet salts, password verifiers and Monero view keys were
  stored under the Keychain service `com.spectra.pricing`, a name left from an
  earlier use.
- **After:** they are stored under `com.spectra.wallet`. Prelaunch, so nothing
  migrates; an install from before this change loses those items.
- **Why:** AGENTS: change keychain keys directly rather than keep a
  misleading name for compatibility.
- **CLI check:** none applies — the CLI uses its file-backed secret store.
  `SecureSeedStoreTests.testGenericSecretsUseTheWalletService` covers it.
- **Verification:** all four suites passed: rustfmt/clippy, 863 core tests plus the
  transport test, 443 CLI acceptance checks and 96 iPhone simulator tests.

## 2026-09-28 — `send overrides` no longer echoes `--sign-only`

- **Before:** `spectra send overrides --sign-only` accepted the flag and
  printed `"signOnly":true`. The value went into `EvmSendOverrides.sign_only`,
  which nothing read: sign-only sends are decided by
  `SendExecutionRequest::wants_sign_only`, from the request's own inputs.
- **After:** `send overrides` has no `--sign-only` flag and no `signOnly`
  field. It still validates nonce, gas limit, calldata and access list.
  `send broadcast --sign-only` is unchanged.
- **Why:** the flag reported an intent that went nowhere. It was the only
  reader of a dead field, removed with the rest of the unwired send options
  (Bitcoin extra outputs, coin-selection strategy and both `sign_only`
  fields).
- **CLI check:** `spectra --json send overrides --gas-limit 50000` prints no
  `signOnly`; `spectra send overrides --sign-only` is refused as an unknown
  argument.
- **Verification:** `make verify` passed: rustfmt/clippy, 865 core tests plus the
  transport test, 444 CLI acceptance checks and 133 iPhone
  simulator tests.

## 2026-09-28 — Deleting a wallet asks once

- **Before:** confirming Delete Wallet on the wallet's Advanced page stored
  the wallet in `walletPendingDeletion` and then asked for Face ID. That same
  field presented a second "Delete Wallet?" alert from the dashboard, so a
  refused or cancelled Face ID left the deletion pending and the dashboard
  asked again — with a message that told a private-key wallet to note its
  seed phrase. The watch-only message read "…until you still have this
  address."
- **After:** `deleteWallet(_:)` takes the confirmed wallet directly; nothing
  is held between the confirmation and the authentication, and the dashboard
  alert is gone. The watch-only message reads "…unless you still have this
  address."
- **Why:** one confirmation, on the page that owns it; the pending field was
  a second path to the same action with its own, wrong, copy.
- **CLI check:** none applies — the prompt is the app's; `spectra wallet
  delete` asks its own confirmation once and removes the wallet the same way.
- **Verification:** `make test-ios`.

## 2026-09-28 — Holdings and Chain Breakdown no longer count their rows

- **Before:** the wallet detail's Holdings header and the asset detail's
  Chain Breakdown header each carried a tinted capsule with the number of
  rows listed beneath it.
- **After:** each header is its title alone.
- **Why:** the count restated the list directly under it; a badge reads as
  something to act on, and there was nothing to act on.
- **CLI check:** none applies — presentation only.
- **Verification:** `make test-ios`.

## 2026-09-27 — Receive pages say the network once and have no page header

- **Before:** under the navigation title ("Wallet" / "Address") each receive
  page repeated itself in a large header — "Choose Wallet" with "Pick where
  the incoming transfer should land.", "Receive Address" with "Scan the code,
  or copy the address to share it." The address card named the network three
  times: a tinted network label, the "Receive only … on this network" warning,
  and the `symbol · network` line under the wallet name.
- **After:** no page header on either page, and both navigation titles read
  "Receive" — the word on the button that opened the flow — instead of
  "Wallet" and "Address", which alone did not say what the page was for.
  The address card opens with the warning; the wallet line under the code
  reads the wallet name over the network name alone (`Ethereum Sepolia`, not
  `tETH · Ethereum Sepolia`) beside the chain's mark rather than the gas
  token's. The three header strings are gone from every locale.
- **Why:** the header restated the title, and its subtitle described what the
  wallet list and the QR code and Copy button already make plain; the
  network label repeated the sentence directly under it. The gas token's
  symbol and mark suggested the address takes only that token, when it takes
  any asset on the chain.
- **CLI check:** none applies — presentation only.
- **Verification:** simulator build, address page before/after screenshots;
  `scripts/unused-strings.sh` clean. Suites not run.

## 2026-09-27 — Receive has no bottom action bar or step indicator

- **Before:** the receive flow pinned a bottom bar under its content. On the
  wallet step its primary button read "Continue" and only moved on once a
  wallet row had been ticked; on the address step it read "Copy Address",
  beside a back chevron. A glass "Wallet / Address" step indicator sat above
  both steps. Opening receive with a single wallet preselected it but still
  stopped on the wallet step.
- **After:** no bottom bar and no step indicator; the navigation title names
  the step. Tapping a wallet row opens its address; "Copy Address" is the
  first button in the address page's action card, above Share and Save. The
  address is pushed onto the navigation stack, so back or an edge swipe
  returns to the wallet list. With a single wallet, receive opens on its address and back leaves
  the flow.
- **Why:** the bar and the indicator repeated what the page and navigation
  bar already offered — a wallet row is the choice, so a second tap on
  Continue was ceremony, and copy sat apart from the other address actions.
  The address is a pushed page, so the system back button and edge swipe
  return to the list.
- **CLI check:** none applies; this is Swift navigation only. Receive address
  resolution is unchanged and covered by `spectra` receive-address acceptance.
- **Verification:** simulator build and walk-through (single wallet opens on
  the address; copy fills the pasteboard; list → address → back → list →
  back → home; X closes from the address step). Suites not run.

## 2026-09-27 — Unconfirmed transactions sort first; Dogecoin nets its legs

- **Before:** an unconfirmed transaction the chain had not dated was stored
  under the unknown sentinel and so sorted as the oldest row, at the bottom
  of the history, in the "Older" group. The unknown sentinel also counted as
  a wallet's earliest transaction. Dogecoin's client kept only the first
  BlockCypher ref of each transaction — one ref per input and per output —
  so a send read as its spent input alone or as its change, and it ignored
  `unconfirmed_txrefs`.
- **After:** the history index sorts an undated pending record after every
  dated one (as 9999-12-31), so it is first newest-first and last
  oldest-first; its stored time stays unknown and it still reads "Unknown
  date", and once confirmed the dated record replaces the key. The history
  page groups these rows under "Unconfirmed", above "Today". A wallet's
  earliest transaction ignores undated rows. Dogecoin nets each
  transaction's refs into one entry and includes mempool refs as undated
  pending entries.
- **Why:** a transaction still waiting for a block is the newest thing in the
  wallet; and a transaction's amount is what it moved for the address, not
  one of its legs.
- **CLI check:** `spectra txs --page` lists an undated pending record first
  (`--oldest-first` last): `wallet_db::tests::undated_pending_transactions_sort_as_the_newest`.
  `spectra history <dogecoin wallet>`;
  `dogecoin::history_tests::a_transactions_legs_net_into_one_entry`.
- **Verification:** `make verify`.

## 2026-09-27 — An undated transaction says so; a misread date is an error

- **Before:** a history client that found no time put 0 in its place, and the
  history showed the Unix epoch — 31 December 1969 west of UTC. That covered
  three different things: a transaction not yet in a block, which has no
  time; a Solana block whose `blockTime` the node does not have; and a
  provider field the client read wrongly (Sui without `timestampMs`, Kaspa's
  camelCase fields). Normalization also turned an unreadable time string into
  0, and Dogecoin had its own RFC 3339 parser that answered 0 on failure.
- **After:** each client reports an unconfirmed transaction's time as absent —
  Blockbook, Bitcoin SV, Decred and Dogecoin by block height, Sui by whether
  the transaction is in a checkpoint — and a Solana block without
  `blockTime` likewise. A confirmed transaction without a readable time fails
  the fetch, naming it (`history: confirmed transaction … has no time`);
  sources that list only confirmed transactions (EVM explorers, Tron, XRP,
  Stellar, Cardano, Kaspa, Aptos, NEAR, ICP, Monero) require one.
  Normalization reads null as undated and refuses anything else that is not
  a positive number; Stellar's and Dogecoin's RFC 3339 times are parsed in
  their clients with the shared strict parser. An undated record is stored
  with the unknown sentinel the EVM, UTXO and Bitcoin paths already used, and
  the history shows "Unknown date" for it.
- **Why:** a missing time is either not known yet or a bug. Showing 1969
  hid both; now the first says so and the second is reported where it
  happens.
- **CLI check:** `spectra history <wallet>` shows no 1970 dates for the
  chains in the previous entry. Core tests:
  `normalize_chain_history_tests::only_null_is_an_unknown_time`,
  `sui::history_tests::only_an_uncheckpointed_block_is_undated`,
  `history_refresh::tests::an_undated_entry_is_stored_as_unknown`. Rows
  written by earlier builds keep their stored time; a History & Cache reset
  clears them.
- **Verification:** `make verify`.

## 2026-09-27 — History can hide small amounts

- **Before:** the history page showed every stored transfer. A large watched
  address filled it with zero-value and dust transfers — EVM contract calls
  (0 ETH), ERC-20/TRC-20 address-poisoning transfers of 0 USDT, 1-nanoton
  TON notifications — with no way to set them aside.
- **After:** `HistoryQuery` has `hideSmallAmounts`, which leaves out
  transfers below `HISTORY_SMALL_AMOUNT_THRESHOLD` (0.00001 of the row's own
  asset); the threshold itself is kept. A cursor carries the setting and is
  refused by a query with the other one. The history filter menu has a "Hide
  small amounts" toggle, off by default, that names the threshold core
  exports as `historySmallAmountThreshold()`.
- **Why:** these transfers are real chain activity, so the history keeps
  them; the user decides whether to see them. One threshold for every asset
  keeps it a filter of dust rather than a valuation.
- **CLI check:** `spectra txs --page --hide-small-amounts`;
  `scripts/cli-history.py HistoryTests.test_hide_small_amounts` stores 0,
  0.000009, 0.00001 and 1 and expects the last two.
- **Verification:** `make verify`.

## 2026-09-27 — Chain history no longer invents zero or wrong amounts

Each history client now reports what the transaction moved for the address,
fee excluded on account chains, and leaves out a transaction that moved
nothing. What each did before:

- **Sui:** every row was a receipt of 0 SUI — the client hard-coded
  `is_incoming: true, amount_mist: 0`. It now queries as sender and as
  recipient and reads the address's SUI balance change, adding back the gas
  it paid.
- **TON:** a wallet's own send starts with an external message (no source,
  no value), which was stored as a receipt of 0 TON beside every send.
  External messages in and out are no longer entries.
- **XRP:** an issued-currency payment read as 0 XRP, a partial payment as its
  `Amount` ceiling rather than what was delivered, and a failed payment as a
  transfer. The amount is now the account's own `AccountRoot` balance change
  from the metadata; only successful payments count.
- **Stellar:** `create_account` read as a 0 XLM send between empty addresses
  (its fields are `funder`, `account`, `starting_balance`), and a payment of
  an issued asset was shown under XLM's name. Both are read correctly or left
  out.
- **NEAR:** the indexer path `/accounts/{id}/activity` answers 404, so there
  was no NEAR history at all. It now reads Nearblocks'
  `/account/{id}/txns`: successful receipts that attach a deposit between the
  account and another; gas refunds from `system` are fees, not transfers.
- **Solana:** a version-0 transaction that loads the address from a lookup
  table read as 0 SOL, a fee-only transaction as a 0 SOL send, amounts below
  0.000001 SOL were cut to 0, and a token account closed by the transaction
  lost its outgoing transfer. The dead `fetch_history` is removed.
- **Aptos:** any entry function named `*transfer*` counted as APT, so token
  transfers were filed as APT and fungible-asset transfers as 0 APT to
  nobody. Only the framework's APT transfers count, and only when successful.
- **ICP:** mints, burns and approvals read as 0 ICP sends. The amount is now
  the account's own change across transfer, mint and burn operations.
- **Cardano:** every transaction was a receipt of its total output — every
  party's, change included. It is now the address's own outputs less its
  inputs.
- **Kaspa:** the API's snake_case fields were read as camelCase, so every
  field defaulted and each transaction was a receipt of 0 KAS (and a pending
  send never confirmed); inputs were also fetched without their addresses.
  Kaspa and Decred had no normalization shape, so neither chain's history
  reached the store.
- **Bitcoin SV:** WhatsOnChain's inputs carry no address or value, so no
  input was recognized: a send with change read as receiving the change, one
  without as 0, and an unreadable transaction as a 0 BSV send. Inputs are
  now matched against the address's own outputs in the same history, and an
  unreadable transaction fails the refresh.
- **Blockbook:** the amount was the transaction's total output; it is now
  the address's net.
- **All normalized chains:** an unreadable amount was stored as 0; the row is
  refused. A whole number of base units converts exactly (10^23 yoctoNEAR is
  0.1 NEAR, not 0.09999999999999999). EVM native ends are compared
  lowercased, as token ends already were.

- **Why:** a 0 or a wrong amount is a false record of the user's funds. The
  zero-value transfers that remain are ones the chain actually carried; the
  entry above lets the user hide them.
- **CLI check:** `spectra history <wallet>` on a watched Sui, TON, XRP,
  Stellar, NEAR, Kaspa or Cardano address shows no 0-amount rows. Unit tests
  per client (`history_tests` in each `fetch/*.rs`) hold the shapes the
  providers return; `scripts/cli-history.py` covers Blockbook's net amount.
- **Verification:** `make verify`.

## 2026-09-27 — The portfolio card no longer counts wallets

- **Before:** under the portfolio total, a footnote read `Across 3 wallets`
  (the wallets included in the total).
- **After:** the card shows the label and the total only; the
  `dashboard.portfolio.walletCount.*` strings are gone from every locale.
- **Why:** the Wallets card directly below lists the wallets, so the count
  repeated it; which wallets the total includes is what the card opens to
  choose.
- **CLI check:** none applies — presentation only.
- **Verification:** `make test-ios`.

## 2026-09-27 — A partial total no longer says what it leaves out

- **Before:** a total with unpriced holdings read `$5,400.00 · 1 without a
  price` in the figure itself — the portfolio header's large title, each
  wallet card's value and the wallet detail's total — so the headline number
  shrank to fit a sentence.
- **After:** the figure is the amount alone, and the `%lld without a price`
  string is gone from every locale.
- **Why:** each unpriced holding already shows "—" as its value on its own
  row, which says the same thing where it applies; repeating it in the
  headline made the total the least legible thing on the card.
- **CLI check:** none applies — presentation only; core's `unpricedCount` is
  unchanged. `testUnvaluedFiguresAreUnavailableAndPartialTotalsShowTheFigureAlone`
  covers it.
- **Verification:** `make test-ios`.

## 2026-09-27 — The send review names the recipient and shows its address whole

- **Before:** the pre-build review printed the typed amount as entered
  (`0.001`, never localized) and the recipient as one monospaced run that
  wrapped wherever it ran out of room. The post-build review printed
  `artifact.amount artifact.asset` — for a token send, the amount followed by
  the contract address — and the sender and recipient as bare monospaced
  runs. Neither said whose address it was.
- **After:** both reviews show the amount with the locale's decimal separator
  and the asset's symbol (a token's from the wallet's holding of that
  contract, falling back to what core stored). Each address — the recipient
  on the pre-build review, sender and recipient after the build — is shown in
  full, in groups of four with the first and last groups emphasized, wrapping
  only between groups, with a Copy context menu; beside it is core's
  `addressHolder` answer: the wallet's own name, another of the user's
  wallets on the network, or a saved contact.
- **Why:** the review is the last screen before signing; it has to show every
  character of the destination in a form a person can check, and "Alice" or
  "Savings" says more than forty hex digits. A contract address is not a
  unit.
- **CLI check:** `spectra address holder --wallet Source <address>` prints
  the same holder; `scripts/cli-send.py` asserts the wallet, another wallet
  (in a different case), a contact and an unknown address.
- **Verification:** see "Transfer ends name who holds them" below.

## 2026-09-27 — Transfer ends name who holds them

- **Before:** `transactionEndpoints` gave each end an address and `isMine`,
  true only for the transacting wallet. An address of another of the user's
  wallets, or of a saved contact, looked like a stranger's.
- **After:** each `TransactionEndpoint` carries `holder`, an
  `EndpointHolder` — `wallet { name }` or `contact { name }` — or none. Core
  looks it up in order: the transacting wallet (by its known addresses), the
  user's other wallets on the record's network (their addresses and keypool
  rows), then address-book entries for that network, all compared in the
  chain's normal form. A wallet on another network is not consulted even
  when it shares the address. The new `addressHolder(walletId, chainId,
  address)` asks the same question for an address not yet sent to.
  `isMine` keeps its meaning. `spectra txs --endpoints` prints the holder,
  and `spectra address holder` is new. The transaction detail shows the
  holder beside each address, and "Mine" only when there is no holder.
- **Why:** who is on the other end is a decision about the user's own data,
  so it is core's, not a Swift join of the address book against a history
  row.
- **CLI check:** `python3 scripts/cli-send.py target/debug/spectra
  SendTests.test_review_and_replacement` — `txs --endpoints` names another
  wallet (`Other`) and a contact (`Alice`), and `address holder` answers for
  four addresses. Core unit tests in `history_query::endpoint_tests` cover
  precedence, normal-form matching and the JSON shape.
- **Verification:** `make verify` (see the final report of this change).

## 2026-09-27 — History rows use the shared status badge

- **Before:** a history row drew its status as a capsule filled with the
  status colour at 85% and primary text on it — green on black in light mode
  — while the transaction detail and the send card used
  `TransactionStatusBadge` (tinted fill, coloured text).
- **After:** the history row uses `TransactionStatusBadge` too.
- **Why:** one status, one look; the filled capsule had the weakest contrast
  of the three.
- **CLI check:** none applies; presentation only.
- **Verification:** `make ios`; checked in the simulator.

## 2026-09-27 — Transaction detail shows each fact once

- **Before:** the transaction detail screen stacked a header, a timeline, an
  Overview card of up to 22 rows, the mempool actions, an address card, a
  hash card and a raw-transaction card. Status appeared three times, the
  amount, time, block and confirmations twice each, and the source address
  repeated the From address. Derivation paths, gas and payload format sat in
  the same card as the network and fee, in the same style. The Speed Up /
  Cancel actions came after the Overview card. Values were secondary text;
  the status chip had a third style of its own; "Mine" was green. Long
  hashes and the raw payload wrapped with system hyphens that read as part
  of the value (`…183e2f-` / `b3666e88c52-`).
- **After:** hero (signed, coloured amount and the shared
  `TransactionStatusBadge`) → Speed Up / Cancel when core says the send is
  replaceable → timeline, the only place status, time, block and
  confirmations appear → addresses, one line each, cut in the middle with
  their ends emphasized, tap to copy → Details (network, fee, hash, explorer
  link) → Technical Details, folded by default, holding gas, fee rate,
  change output, payload format, paths, a source address only when it
  differs from From, the change address and the raw transaction. Key/value
  rows follow docs/IOS-UI.md (accent symbol, secondary label, primary value)
  and put the value under the label when both do not fit on one line.
  Identifiers never wrap; the raw payload wraps at any character without a
  hyphen, and copying goes through the original string. "Mine" is a neutral
  inset pill. The empty Addresses card is no longer drawn.
- **Why:** the screen answered "what happened" three times and "what is
  the fee" once, below twenty rows of internals; a hyphen inserted into a
  hash is a wrong value on screen.
- **CLI check:** none applies; the rows read the same `TransactionRecord`
  and `transactionEndpoints` core already returns. Checked by hand in the
  simulator on a confirmed Sepolia send, including that copying the hash
  and the raw transaction yields the exact stored strings.
- **Verification:** `make ios` Debug simulator build, `make check-ui`,
  `scripts/unused-strings.sh` ("Confirmations" removed, "Technical Details"
  added in all three locales).

## 2026-09-27 — The app is covered whenever it is not active

- **Before:** only the app lock hid the app, and only when both Face ID and
  Auto Lock were on; it blurred the tab view by 8pt under the unlock card.
  With either setting off, the app switcher snapshot showed the portfolio
  total, balances and addresses in full.
- **After:** whenever the scene is not active (app switcher, Control Centre,
  Notification Centre, a system prompt, background) an opaque cover — the
  backdrop and the Spectra logo — sits over everything, whatever the lock
  settings. It lifts as soon as the scene is active again. The app lock is
  unchanged and independent of it.
- **Why:** hiding balances from the snapshot is privacy every user needs, not
  a side effect of opting into Face ID; the lock's blur was also never meant
  to be the snapshot guard.
- **CLI check:** none applies; this is Swift view state driven by
  `scenePhase`. Check by opening the app switcher in the simulator with Face
  ID off.
- **Verification:** `make ios` Debug simulator build.

## 2026-09-27 — History keeps its loaded rows across a reload

- **Before:** History reloaded its first 20 rows whenever the transaction or
  wallet revision changed, and after Load more fetched older on-chain history.
  A reader who had paged down to row 60 was cut back to 20 by any refresh, and
  the on-chain rows Load more had just fetched were beyond the cut.
- **After:** a reload re-reads at least as many rows as were on screen, and
  after an on-chain fetch one page more, walking core's cursor in queries of
  at most 200 rows (core's limit). Changing the wallet, type, sort or search
  still starts again from 20. The on-chain fetch bumps the transaction
  revision, so the view's own reload races the Load more button's; the extra
  page is view state both read, not an argument to one of them, so whichever
  lands last keeps it.
- **Why:** a refresh should update the list, not discard the reader's place in
  it; Load more that ends with fewer rows than it started with is broken.
- **CLI check:** none applies; this is Swift paging state over core's
  unchanged `historyPage` query.
- **Verification:** `make ios` Debug simulator build succeeded. Not exercised
  by hand: the simulator wallet has no history to page through.

## 2026-09-27 — Asset detail shows its value once

- **Before:** the asset detail screen showed the fiat total twice: under the
  name in the hero card, and again as "Total Value" in a stats card below it,
  whose only other row was "Total Amount".
- **After:** the stats card is gone. The hero card shows the fiat total with
  the held amount (for example `0.5 BTC`) beneath it; the chain breakdown
  follows directly. The "Total Amount" and "Total Value" strings are removed.
- **Why:** the second row repeated the hero card, and a card left holding one
  row reads as a leftover rather than a section.
- **CLI check:** none applies; this is Swift presentation only.
  `scripts/unused-strings.sh` reports 0.
- **Verification:** `make ios` Debug simulator build succeeded;
  `scripts/check-design-tokens.sh` ok. Rust, CLI and iOS test suites were not
  run because no logic changed.

## 2026-09-26 — Exported async methods run on core's own runtime

- **Before:** UniFFI polled every exported async future on the calling
  thread. On iOS that is a Swift cooperative-pool thread with a 512 KiB stack,
  and in a Debug build Build Transaction overflowed it inside the EVM nonce
  read (`EXC_BAD_ACCESS`, stack guard) before any request left the device. The amount
  page asked core for a 10% shortcut that core never computed, so that button
  was always disabled; a share shortcut filled in 18-digit wei amounts. The
  recipient check failed whenever the history read failed, so on every chain
  without an explorer (every testnet) it said "Unable to verify this address's
  activity" even for a funded address. A self-send was flagged at review both
  as "a new destination with no prior history" and as "belongs to your
  wallet".
- **After:** all 73 exported async methods hand their body to
  `core::worker::run`, which spawns it on a core-owned tokio runtime with 8 MiB
  worker stacks; the caller only awaits the join, and dropping it aborts the
  body, so cancellation is unchanged. `WalletService`, `RefreshEngine` and
  `FundsScan` are cheap `Clone`s for this. Build, Sign and Broadcast stay
  three explicit actions, with broadcast nodes chosen on the signed
  transaction (a one-action Send was tried in Beta Commit 177 and reverted: it
  broadcast to core's defaults and skipped that choice, against the
  transparent-stages design in PLAN.md). Core
  exports the shortcut list (`send_amount_shortcut_percentages`, 25/50/75/100);
  shares are cut to display precision, the maximum stays exact. A funded
  destination is known used without a history read. The preview's
  `RecipientCheck` is a record carrying `is_own_address`, and a self-send's
  review drops the `NewAddress` warning.
- **Why:** how much stack core needs is core's to decide, not the calling
  platform's. The other changes each removed a message or control that stated
  something false.
- **CLI check:** `spectra --json send preview` shows `shortcuts` for 25/50/75/100
  and `recipient.isOwnAddress`; `spectra send build-owned` to one of the
  wallet's own addresses lists no `new_address` warning. The CLI already
  polls on its own runtime, so the crash never reproduced there; the new
  `worker` tests cover the runtime, cancellation and panic propagation.
- **Verification:** `make verify` passed: lint, 853 Rust tests, 445 CLI
  acceptance checks and 133 iPhone simulator tests. By hand in a Debug build
  on an Ethereum Sepolia wallet: the composer reaches the Send confirmation
  (the build that used to crash), then was cancelled; nothing was signed or
  broadcast.

## 2026-09-26 — One wallet setup, with advanced options on the seed step

- **Before:** Add Wallet opened with a Simple / Advanced segmented picker. It
  was stored on the draft as `SetupModeChoice`, and its only effect was
  whether the seed step showed an orange Advanced card. The card routed to an
  `.advanced` side page inside the setup flow, which every navigation switch
  had to special-case (blank primary title, disabled primary action, custom
  back route, hidden button bar). Nothing on the seed step said whether a path,
  passphrase or HMAC key had been changed.
- **After:** there is no mode. Create and import seed flows always show a
  quiet "Advanced Options" row under the seed phrase. It opens a sheet with
  the same per-chain paths and overrides (`WalletDerivationOptionsView`). Once
  any of these differs from its default, the row turns orange and names them
  ("Customized: Derivation Paths and Passphrase"). Private-key and watch-only
  imports have no row, as before. `WalletSetupPage.advanced` and its special
  cases are gone. The picker's strings and the leftover
  "Choose Setup Type" are deleted.
- **Why:** the picker asked a question before the user had context for it, and
  it gated one button. Keeping the side route in the linear page enum was a
  second navigation model inside `SetupFlow`. The options stay hidden, but
  they change which addresses the seed derives, so the row must show when
  they differ from their defaults.
- **CLI check:** none applies; this is Swift presentation only, and derivation
  inputs reach core unchanged. `scripts/unused-strings.sh` reports 0.
- **Verification:** `make test-ios` passed 133 iPhone simulator tests. The flow
  was checked by hand in the simulator: no picker, the row under the grid, the
  sheet, and the summary after editing a path and a passphrase. Rust and CLI
  suites were not run because no Rust code changed.

## 2026-09-25 — Remove dead code the scans missed, and stop the scans reading bindings

- **Before:** core exported four holding-merge types (`HoldingMergeExistingInput`,
  `HoldingMergeIncomingInput`, `HoldingMergeAppendPayload`, `HoldingMergeAction`)
  that no function took or returned, so they appeared only in the generated
  bindings. Swift kept `SettingTextField` (its last callers went in Beta Commit
  167), `SpectraLoadingCard`, `AppLocalization.preferredLocalizationIdentifiers()`
  with the `Tables.identifiers` field only it read, and a `TransactionKind`
  alias only a test used. The runtime tables kept `" Last good sync: %@."`,
  superseded by `diagnostics.degradedLastGoodSyncFormat`. The CLI depended on
  `uuid` and the FFI crate on `uniffi`, neither used. `uncalled-core-fns.sh`
  and `unused-strings.sh` walked their directories, so they read the ignored
  Kotlin bindings (and, for strings, `swift/generated/`) once bindgen had run;
  generated code calls every export and quotes every doc comment, so both
  checks passed on any machine that had built bindings. The stale Kotlin
  bindings were what kept the Last-good-sync key "reachable".
- **After:** all of the above is deleted, with no replacement. Both scans read
  only files git tracks or would track (`git ls-files --cached --others
  --exclude-standard`), so ignored output no longer counts as a caller.
  `scripts/README.md` lists every CLI suite and the current file count.
- **Why:** dead exports read as API and keep compiling; a scan that ignored
  output can satisfy is not a gate.
- **CLI check:** none changes; no removed item was reachable from `spectra`.
  `scripts/uncalled-core-fns.sh`, `scripts/unreachable-exports.sh` and
  `scripts/unused-strings.sh` report 0 with Kotlin bindings present, and
  `cargo machete` reports no unused dependencies.
- **Verification:** `make verify` passed: fmt and clippy clean, 847 Rust tests,
  445 CLI acceptance checks and 133 iPhone simulator tests. Swift bindings
  regenerated without the `uniffi` dependency in `ffi/`.

## 2026-09-24 — History, merges and EVM assembly identify assets by deployment

- **Before:** four places decided which asset a thing was by its ticker. A
  Solana or Tron history row without a contract was the native coin only if its
  ticker matched, and a token row's name was looked up by ticker
  (`token_name_on_chain`). A history entry with no deployment was filed as the
  native coin when its ticker matched. The transaction merge compared tickers on
  top of deployment ids, with a registry flag (`merge_identity_includes_symbol`)
  for Tron. EVM send assembly treated a token-less input as a value transfer
  only when its ticker equalled the gas asset's. An SPL balance row with no mint
  flowed on with an empty ticker.
- **After:** identity is the deployment id — network, standard, contract. A
  history row's contract (or mint) gives its deployment; a row without one is
  the network's own coin, which the adapters guarantee (Tron token rows require
  a contract, Solana SPL rows without a mint are skipped); a contract that does
  not normalize refuses the row. A token row is named from the catalog entry for
  its deployment, or shown as its contract. Merges compare deployment ids only.
  `EvmSendAssemblyInput` carries `deployment_id` instead of `symbol`: the
  network's native deployment is a value transfer and must carry no contract;
  any other must carry the contract that derives exactly that deployment.
  `history_deployment` is now `tokens::deployment_id_for`.
- **Why:** a ticker is display text; any token can borrow one (PLAN: never
  infer identity from ticker). On the funds path the assembly now refuses a
  mismatch between the stated asset and its contract instead of trusting either.
- **CLI check:** `spectra --json send assemble --chain Ethereum --from … --to …
  --amount 1 --symbol ETH --contract 0xa0b8…eb48 --decimals 6` answers
  `"isNative":false`. Core tests cover a borrowed ticker in history, two tokens
  sharing a ticker in one transaction, and every assembly mismatch.
- **Verification:** `make verify` passed: rustfmt/clippy, 846 core tests plus
  the transport test, 445 offline CLI checks and 133 iPhone simulator tests.

## 2026-09-24 — Crypto wiki prose is keyed by token id

- **Before:** the wiki grouped deployments by `token_id`, but looked up each
  coin's description and supply model in `crypto-wiki.toml` by ticker
  (`asset = "ETH"`). A new token reusing a ticker would silently have shown
  another coin's text; the file's header relied on a uniqueness test that no
  longer existed.
- **After:** every row is `token_id = "…"`, the lookup and the coverage test use
  the id, and the file rejects unknown keys. The prose shown is unchanged.
- **Why:** identity is the catalog id, never the ticker (PLAN boundary rules).
- **CLI check:** none — the wiki is not a CLI command; `cargo test -p
  spectra_core wiki` checks that every coin has exactly its own row.

## 2026-09-24 — Thinner Swift shell: one command queue, core-run refresh, one string table

### Device authentication errors stay in the flow that asked

- **Before:** any failed device authentication — unlock, delete wallet, reset,
  rebroadcast, Monero sync — wrote both `sendFlow.error` and `appLockError`, so
  one failed unlock showed a "Send Error" and a "Security Notice" on the home
  screen. Rebroadcast reported `sendFlow.error` as its own failure. The replace
  and cancel composers wrote "Replacement context loaded…" into the send error
  field, where the home screen showed it as an error.
- **After:** `authenticate(_:reason:)` returns the failure reason; each caller
  shows it in its own flow (unlock → lock screen, delete → `commandError`,
  reset → the reset sheet, which stays open, sign → the send session,
  rebroadcast/Monero sync → their own result). The "context loaded" line is
  removed: the filled composer and the pending-send line already say it.
- **Before:** revealing a seed phrase required biometrics only, with no
  passcode fallback, whatever the Face ID preference said; a passcode-only
  device could never reveal one.
- **After:** `.revealSeedPhrase` is a `DeviceAuthenticationAction` that always
  requires device-owner authentication (biometrics or passcode), and no
  preference turns it off.
- **Why:** an error belongs to the operation that failed; key material is never
  shown unauthenticated, and a user without Face ID must still be able to see
  their own backup.
- **CLI check:** none applies — device authentication is a platform operation.
  `DeviceAuthenticationTests` covers the policy table and `SendSessionTests`
  the signing session's handling of a returned failure.

### One state-command queue

- **Before:** seven ways to send a `StateCommand`: settings (queued, optimistic),
  contacts (own queue), wallet fields (third queue), pins and fiat currency
  (unqueued, errors swallowed), token preferences (unqueued), custom endpoints
  (called the bridge from the view). Some adopted the state and rebuilt the
  portfolio in a detached task, some awaited the rebuild.
- **After:** `enqueueStateCommand` is the only path. Commands run in the order
  the user issued them, the committed state and portfolio snapshot are adopted
  inside the queue, and `awaitPendingStateCommands` waits for all of them.
  Failures that have no field of their own (pins, fiat currency, wallet fields,
  settings) appear as an "Action Failed" notice instead of being dropped or
  only logged. `applyCoreState` no longer spawns a portfolio read.
- **Why:** one writer and one ordering rule for core-owned state; a dropped
  error is a silent failure.
- **CLI check:** none needed — the CLI already calls `apply_state_command`
  directly; `AppStateTests` covers ordering, stale reads and persistence.

### Core runs the maintenance loop and writes its own event log

- **Before:** Swift ran a `while` loop that asked core for a plan, called
  `refresh_app(Scheduled)` and slept for the cadence core returned; core's
  balance engine ran a second loop. After each sweep Swift called
  `refresh_app(BalancesUpdated)` itself. Swift wrote the operational log lines
  for events core performed — confirmed/failed status changes, broadcast
  accepted, rescan started/completed, self-test results, refresh failures —
  some localized, some English.
- **After:** `BalanceRefreshEngine`/`BalanceObserver` are `RefreshEngine`/
  `RefreshObserver`. The platform reports `DeviceConditions` (foreground,
  network path, visible tab); while the app is active and a wallet has something
  to fetch, core runs both the balance sweep and the maintenance tick, runs the
  post-sweep judgement, and hands each `AppRefreshResult` to
  `on_refresh_complete`. Swift adopts projections and delivers notifications.
  Core records status changes, each broadcast attempt's outcome, rescan
  outcomes, self-test results, refresh and pending failures and failed
  rechecks, in English, with `source: core`. Swift logs only failures on its
  own side (a platform API, or a call into core that threw).
- **Why:** one owner for refresh cadence and for the log of what core did; the
  CLI now gets the same log lines as the app.
- **CLI check:** `python3 scripts/cli-diagnostics.py target/debug/spectra
  DiagnosticsTests.test_offline_refresh` asserts core-sourced `Rescan`,
  `Refresh` and `Self-Tests` lines after an offline rescan and a configured
  self-test. Core tests cover the engine loop lifecycle and the logged recheck.

### Core pushes what changed instead of the shell re-reading

- **Before:** Swift polled `tor_status()` every second. Each wallet's balance
  update made Swift re-read the whole portfolio snapshot; each completed
  refresh re-read the portfolio, the transaction snapshot and diagnostics
  whether or not they changed. "Load more history" asked core one synchronous
  cursor question per wallet on every render. The send composer made two core
  calls, a quote and a separate recipient check.
- **After:** core publishes every Tor state change and bootstrap step; the
  refresh engine forwards them through `on_tor_status_changed`. The app reads
  the portfolio once per sweep, and `AppRefreshResult` says whether
  `transactions_changed` or `diagnostics_changed`, so the shell re-reads only
  those. `TransactionSnapshot.wallets_with_more_history` names the wallets with
  pages left; `history_cursor` is no longer exported. `preview_owned_send`
  checks the recipient beside the quote and returns it as `recipient`
  (`checked` with the activity, or `unavailable`); a failed recipient read
  leaves the quote standing.
- **Why:** each answer comes from its owner once, with what changed attached,
  rather than the shell asking again to find out.
- **CLI check:** `spectra --json send preview …` prints the `recipient` field;
  `python3 scripts/cli-diagnostics.py target/debug/spectra` covers the refresh
  flags through `diagnostics refresh`. Core tests cover the engine's Tor
  forwarding.

### The Swift bridge is the service, not a copy of its API

- **Before:** `WalletServiceBridge` restated about fifty core methods as
  one-line forwards, some with different labels, plus stale comments.
- **After:** it owns only the database path, the lazy service with its secret
  store, the open-state binding and the refresh engine; callers use core's API
  as generated (`try await bridge.ready().portfolioSnapshot()`).
- **Why:** a forwarding layer that only renames is a second API to keep in step.
- **CLI check:** none applies; no behaviour changes.

### Registry fact for account xpubs; tests that restated core

- `Chain::accepts_account_xpub` (Bitcoin only) replaces `chain == .bitcoin` in
  the watch-only form and in core's import planner.
- iOS tests that asserted core rules now assert only that the result crosses
  the binding: contact refusal wording (rules in `address_book.rs`), log
  appends (trim and the 800 cap, now tested in core), and the testnet4 import
  derivation (covered by `store/tests/wallet_import.rs`).

### The seed envelope's master key is wrapped by the Secure Enclave

- **Before:** signing material was sealed under a master key stored in the
  Keychain as raw bytes, in the same access class as the sealed items, so any
  reader of one Keychain item could read the other and the envelope protected
  nothing Keychain did not already.
- **After:** the master key is stored only as an ECIES blob encrypted to a
  P-256 key created in the Secure Enclave (`.privateKeyUsage`,
  `WhenPasscodeSetThisDeviceOnly`). A copied Keychain opens nothing without
  this device's enclave. No user presence is required, so background
  derivation is unaffected. A wrapped key whose wrapping key is gone is
  unreadable, never replaced. The simulator has no enclave and uses a software
  wrapping key. Seeds sealed under the old raw key are not migrated
  (prelaunch): re-import them.
- **Why:** the user chose to make the layer real rather than keep or delete a
  layer that added no protection; for keys, the stricter side.
- **CLI check:** none applies — the Keychain and enclave are platform storage;
  the CLI's file secret store is unchanged. `SecureSeedStoreTests` checks the
  stored shape and round trips; the device branch compiles for `generic/platform=iOS`.

### Smaller removals

- The `providerState` reset scope is removed: core cleared nothing for it and
  Swift cleared `URLCache`/cookies/credentials that nothing uses (all network
  traffic is core's). `settings reset --scope providerState` is now rejected.
- `CoreSeedDerivationPaths.is_custom_enabled` is removed: it was stored and
  never read. `normalized_send_address` is no longer exported (no caller).
- The chain catalog's `category` is a typed `ChainCategory`; a misspelt section
  fails at load instead of dropping the chain from the picker.
- The recipient activity check runs beside the send preview under one request
  token instead of before it; the preview no longer waits on it.
- The Tor status poll runs only while Tor is enabled or winding down.
- Dead Swift code removed: the wrapper debounce (`DebouncedAction`), the
  maintenance-plan wrapper, unused formatter/flags/wrappers, the duplicated
  edit-mode flag, and the Swift copy of the history filter enum. The two
  byte-identical Keychain store types are one `SealedSigningStore`; the seed
  envelope and its Keychain services are unchanged.

### One localized string table

- **Before:** two localization systems: `RuntimeStrings.<locale>.json` looked up
  through a manifest plus an `.lproj` fallback that no bundle had, and six
  `*Content.<locale>.json` files decoded into structs. Donation addresses were
  copied into each locale's file.
- **After:** `RuntimeStrings.<locale>.json` is the only localized copy; screen
  copy structs are typed names for `namespace.key` entries. Donation addresses
  live once in `resources/Donations.json`, titled by chain name, and a test
  checks each is a valid address for its chain. `scripts/unused-strings.sh`
  now also checks dotted keys, both ways; it removed 21 strings nothing read.
- **Why:** one table per language, and funds destinations written once.
- **CLI check:** `scripts/unused-strings.sh`; `PresentationCatalogTests`.

- **Verification:** `make verify` passed: rustfmt/clippy, 844 core tests plus
  the transport test, 444 offline CLI checks and 133 iPhone simulator tests,
  including `testEthereumTestNetworksExposeExpectedContextsAndEndpoints`.

## 2026-09-24 — The CLI's live portfolio and spot price are core's valuation

- **Before:** `spectra portfolio` fetched each wallet's native balance and a USD
  price itself, multiplied them as floats and converted with a rate it fetched
  on the spot; tokens were left out, and nothing it read was stored. When the
  rate lookup failed it quietly switched the whole report to USD. `spectra price
  <chain>` multiplied the same way, printed `0.00` for a testnet coin, and
  always said "via CoinGecko" whichever provider answered.
- **After:** live `portfolio` asks core to refresh each wallet's balances
  (`refresh_wallet_balances`), then prices and fiat rates when they are due,
  and renders `portfolio_snapshot().valuation`: every held asset, with its
  amount as core stores it and its value, the wallet totals and the total in
  the display currency, plus `unpricedCount`, the wallets whose balances could
  not be read (`unavailable`) and refresh `failures`. The refreshed balances and
  quotes are stored, as the app's are. `price <chain>` renders core's
  `native_spot_price`, which converts with the stored rate. A missing rate or
  a testnet coin gives no price (`null`, shown as `—`) rather than another
  currency's figure or a zero. The CLI-only `service::fetch_prices` and
  `service::fetch_fiat_rates` are removed, and so is the provider claim.
- **Why:** the last money arithmetic outside core. The CLI had its own
  valuation that disagreed with the app's: native assets only, float
  amounts, and its own currency fallback.
- **CLI check:** `python3 scripts/cli-portfolio.py target/debug/spectra
  PortfolioTests.test_live_portfolio_is_core_valuation` values a seeded wallet
  offline. Its unreadable balance is listed as unavailable, an unpriced token
  has a `null` value, and `price bitcoin-testnet-4` returns `"price":null`.
- **Verification:** rustfmt and Clippy with `-D warnings` clean; 841 core tests
  plus the transport test; 443 CLI acceptance checks; the export and
  uncalled-function checks report none. The regenerated Swift bindings are
  byte-identical, since no FFI export changed, so the iOS suite was not rerun.

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

## 2026-09-30 — Bitcoin Signet coins display sBTC

- **Before:** Bitcoin Signet's native coin displayed `tBTC`, the same symbol as
  Bitcoin Testnet and Testnet4, and chain search matched it on `tBTC`.
- **After:** Signet's native coin displays `sBTC` and chain search matches it on
  `sBTC`. Testnet and Testnet4 keep `tBTC`; token and deployment IDs are unchanged.
- **Why:** Signet coins are a separate network's coins; a distinct symbol keeps
  them from being mistaken for testnet coins in balances and history.
- **CLI check:** `spectra --json token catalog --chain bitcoin-signet` shows
  `"symbol":"sBTC"`.

## 2026-09-30 — Testnet stablecoins display tUSDC and tPYUSD

- **Before:** the testnet catalog's `usd-coin-testnet` and `paypal-usd-testnet`
  displayed `USDC` and `PYUSD`, the same symbols as their mainnet tokens.
- **After:** they display `tUSDC` and `tPYUSD`, like every other testnet asset's
  `t` prefix. Token IDs, deployments and contracts are unchanged; the contracts'
  own `symbol()` still answers `USDC` and `PYUSD`.
- **Why:** a testnet stablecoin has no value; a symbol matching the mainnet
  token invites mistaking test balances for real ones.
- **CLI check:** `spectra --json token catalog --chain ethereum-sepolia` shows
  `"symbol":"tUSDC"` and `"symbol":"tPYUSD"`.

## 2026-09-30 — Testnets, Solana and Sui transactions link to an explorer

- **Before:** `explorers.toml` listed mainnets only, and not Solana or Sui, so
  a transaction on any testnet or on those two mainnets had no "Open In"
  button, and Settings → Explorers listed none of them.
- **After:** 38 testnets have a row pointing at their own network's explorer
  (Sepolia at `sepolia.etherscan.io`, Aptos Testnet with `?network=testnet`,
  Signet at `mempool.space/signet`, Solana Devnet at Solscan with
  `?cluster=devnet`, Sui Testnet at `suiscan.xyz/testnet`, and so on).
  Solana links to Solscan and Sui to Suiscan. Kaspa Testnet is the only
  network left without one: its TN10 explorer did not answer.
- **Why:** a testnet is a network like any other; a transaction sent there is
  as worth checking as one on mainnet, and the explorer must be the testnet's,
  never the mainnet page that would show nothing. Solana and Sui were simply
  missing.
- **CLI check:** `spectra --json explorers --chain ethereum-sepolia --tx 0xabc`
  prints `https://sepolia.etherscan.io/tx/0xabc`;
  `spectra --json explorers --chain solana-devnet --tx abc` prints
  `https://solscan.io/tx/abc?cluster=devnet`.
