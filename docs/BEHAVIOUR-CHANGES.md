# Behaviour changed on purpose

The log Rule 0 requires. [PLAN.md](../PLAN.md) holds the rules and the work
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

Split out of PLAN.md on 2026-09-15: it had reached 81 entries and 3171
of PLAN.md's 3585 lines, which left Rule 0 and the open work buried under the
history of work already done. Nothing was dropped in the move.

### Testnet tokens live in testnet-tokens.toml (2026-09-15)

- **Before:** `tokens.toml` held all 131 token identities, the 32 faucet coins
  among them — `bitcoin-testnet-native`, `ethereum-sepolia-native`,
  `polygon-amoy-native` and the rest, each a BTC/ETH/POL-shaped row with blank
  market ids, interleaved with the coins the wallet actually prices. **After:**
  `tokens.toml` holds the 99 mainnet identities and 236 deployments; the 32
  testnet identities and their 32 native deployments are `testnet-tokens.toml`,
  same shape, same order rule, embedded the same way. Core reads both and
  concatenates them, testnets last.
- **Why:** the two sets are read at different times by different people — one
  is the shipped catalog, the other is scaffolding for a developer pointing the
  app at a faucet — and mixing them made the shipped list a third longer than
  it is. Splitting also turns a convention into a checked fact: the file a
  token sits in is now a claim about its networks, and the catalog build
  asserts `network.environment` matches, so a mainnet coin cannot drift into
  the testnet file or the reverse. The rule that a testnet token carries no
  market identity is stated on that file and still enforced.
- **CLI check:** `spectra token catalog --chain ethereum-sepolia` still lists
  native ETH at 18 decimals, and `--chain ethereum` is unchanged — the split is
  invisible past the parser: same 131 tokens, same 268 deployment ids.
- **Verification:** `make verify` clean — 802 Rust, 357 CLI, 93 iOS.

### A deployment's id is derived, not declared (2026-09-15)

- **Before:** every one of the 268 deployments carried an `id` beside the facts
  it restates — `id = "ethereum:erc-20:0xa0b7…"` above `network`, `standard`
  and `contract` — and the catalog build asserted the two agreed, character by
  character, twice (once per kind). **After:** the field is gone from
  `tokens.toml` and core builds the same string from the deployment:
  `"{network}:native"`, or `"{network}:{standard-lowercased}:{contract}"`. The
  268 ids are byte-identical to the ones deleted, and `chains.toml` still names
  its `native_deployment` by id, still checked against the derived one.
- **Why:** an id that restates three fields beside them is a fourth place to
  get them wrong, and the assertions proving it did not were the file's own
  admission that it was derivable. Deriving it is the rule for funds, keys and
  addresses applied to identity: build it from what is true, do not trust the
  spelling next to it. What the assertions really guarded — that the contract
  is spelled the way every lookup spells it — is now a test of its own,
  `every_catalog_contract_is_already_normalized`, since the contract *is* the
  id and a mis-cased one would resolve to nothing.
- **CLI check:** `spectra token catalog --chain ethereum --json` still reports
  `"id": "ethereum:erc-20:0xa0b73e1ff0b80914ab6fe0444e65848c4c34450b"` for CRO,
  with no id in the file to read it from.
- **Verification:** `make verify` clean — 802 Rust, 357 CLI, 93 iOS.

### tokens.toml nests each deployment under the token it carries (2026-09-15)

- **Before:** the file was two flat arrays — 131 `[[tokens]]`, then 268
  `[[deployments]]` grouped by network, each repeating its owner as
  `token = "usd-coin"`. USDC's thirteen deployments sat in thirteen places, all
  of them away from the identity they belong to, and a token's own row said
  nothing about where it lives. **After:** every deployment is a
  `[[tokens.deployments]]` under its token and the back-reference is gone.
  Tokens with a native deployment lead the file in chain order (BTC, ETH,
  SOL…) and the protocol tokens follow by symbol; inside a token the native
  deployment precedes its contracts, which are in chain order too. Key order is one spelling now (`id` first) where it had drifted into two.
  Nothing else moved: same 131 tokens, same 268 deployments, same field values.
- **Why:** locality. The two-table shape asked a reader to hold a token id in
  their head and grep a 3.8k-line file for its other half, and the only thing
  it bought was the freedom to order deployments by network — which the nested
  form keeps within each token anyway. The nesting also deletes a failure
  rather than checking for it: a deployment naming a token the file does not
  define used to panic at catalog build, and can no longer be written down. The
  catalog it produces is token-major rather than network-major; every consumer
  reads it by id or sorts it (the wiki still leads ETH with Ethereum, since
  chain order is preserved within a token).
- **CLI check:** `spectra token catalog --chain cronos` still prints CRO as
  native at 18 decimals and `--chain ethereum` still prints it as an ERC-20 at
  8 — the two rows that used to sit a thousand lines apart in the file, and one
  block apart now.
- **Verification:** `make verify` clean — 801 Rust, 357 CLI, 93 iOS.

### Four faceted marks draw their facets in solid colour (2026-09-15)

- **Before:** `ethereum`, `ethereumclassic`, `steth` and `wsteth` each drew one
  glyph whose facets were separated by `fill-opacity` / `opacity` of `.6` and
  `.2`, letting the disc tint them. **After:** every facet names the colour that
  blend produced, and none of the four files contains an opacity attribute:

  | icon | disc | glyph | `.6` | `.2` |
  |---|---|---|---|---|
  | `ethereum` | `#627eea` | `#fff` | `#c0cbf7` | `#8198ee` |
  | `ethereumclassic` | `#328332` | `#fff` | `#adcdad` | `#5b9c5b` |
  | `steth` | `#fff` | `#00a3ff` | `#66c8ff` | `#ccedff` |
  | `wsteth` | `#00a3ff` | `#fff` | `#99daff` | `#33b5ff` |

- **The facets tile without overlapping, so this is the same artwork.** That was
  checked rather than assumed: each translucent shape was painted in a probe
  colour to get its visible coverage, the document was rendered again without
  it, and the backdrop was sampled through that coverage. All four came back
  with one colour under every facet. The renders differ by 0.4–0.8% RMSE at
  256 px across 230–500 of 65,536 pixels, all on the hairline seams between
  facets, where partial coverage on both sides of an edge used to let the disc
  through — the artifact that prompted this.
- **Why:** the transparency was decoration the renderer had to be trusted to
  compose, and on iOS it was not composing the way the source implied. A
  flattened fill is what the icon always meant, stated in a form that cannot be
  read two ways — the same reason the style bans `fill="none"` on the root and
  `clip-rule` outside `<clipPath>`: nothing in a shipped icon should depend on a
  renderer resolving a no-op.
- **The other three alpha users keep it, because theirs is not a no-op.** The
  same probe says `pumpfun`'s `.4` sheen lies over two colours (`#5fcb88` and
  the white disc), `dogecoin`'s `.8` highlight over four gold ramps, and
  `usd1`'s two `stop-opacity="0"` gradients fade over its concentric rings.
  Flattening those means splitting artwork along boolean intersections, which is
  a redraw, not a rewrite. `xcrun actool` compiles the catalog with no warning
  on any of them.
- **CLI check:** none applies; these are bundle resources, not domain state.
  `scripts/normalize-icons.sh --check` is the gate and passes on all 88 icons,
  and `CoinBadgeArtworkTests` still loads all four by name from the catalog.
- **Verification:** `make verify` clean.

### CoinLore is no longer a price provider (2026-09-15)

- **Before:** `PRICE_PROVIDERS` held CoinGecko, CoinPaprika and CoinLore, and
  every price refresh fetched all three concurrently and merged them in that
  order. **After:** it holds CoinGecko and CoinPaprika. CoinLore's ticker
  endpoint, its decoder, `coinlore_nameid_for`, the `coinlore_nameid` column on
  `AssetMarketIds` and the 67 `coinlore_nameid` lines in `tokens.toml` are gone.
- **Why:** it bought no coverage worth its cost. CoinLore was asked about the
  same assets the first two already price, so it only ever filled a key the
  other two had both missed — and it paid for that chance with a whole extra
  round trip per refresh, on an endpoint that returns the top 1000 tickers and
  is then discarded down to the handful asked about. Its id column was the
  worse half of the bargain: `coinlore_nameid = ""` meant "the CoinGecko id",
  so 124 of 131 rows carried a blank whose meaning lived in a resolver
  elsewhere, and the seven that were not blank were the only rows that said
  anything. A default nobody can read off the row is a rule, not data.
- **Consequence, stated plainly:** an asset CoinGecko and CoinPaprika both miss
  is now unpriced where CoinLore might have quoted it. Nothing in the catalog is
  in that position today — `only_deliberately_unlisted_assets_have_no_paprika_id`
  holds honey-3 as the single row without a CoinPaprika id, and it carries a
  CoinGecko one — so this costs coverage only for a future asset one of the two
  does not list, which is the point at which a third provider earns its place
  back.
- **CLI check:** `spectra market price BTC ETH SOL` and `spectra market
  portfolio` still quote, now over two providers; `spectra market currency` is
  untouched, since the four fiat-rate providers did not change. Acceptance runs
  without network, so it cannot see which providers answered — the catalog side
  is what the tests below pin.
- **Test:** `cargo test -p spectra_core market_id_tests` covers the catalog's
  remaining ids — `no_two_assets_claim_one_listing` now asks about CoinPaprika
  alone, and `coinlore_nameids_default_to_the_gecko_id` is deleted with the
  default it described. `merge_in_preference_order` is unchanged and its tests
  still cover the partial-failure and nobody-answered paths.
- **Verification:** `make lint test test-cli` clean (801 core tests);
  `make test-ios` unaffected — no Swift file named CoinLore.

### Six letter badges become marks, and Mantle's mark becomes the official one (2026-09-15)

- **Before:** STETH, WSTETH, AERO, LDO, ZRO and WLD each carried
  `artwork_name = ""`, so `CoinBadge` drew the first letters of their symbol.
  **After:** they name `steth`, `wsteth`, `aerodrome`, `lido`, `layerzero` and
  `worldcoin`, and those six SVGs are in `icons/crypto/` and the asset catalog.
  Mantle keeps its name and loses its old traced mark: `mantle.svg` is now
  Mantle's official brandmark, whose dashes are longer and whose hub is smaller
  (4.1% RMSE against the old artwork at the same size), and its disc goes from
  `#000` to Mantle's obsidian green `#092c24`.
- **The incoming files were exporter output, not library members.** Each was
  brought onto the house style rather than dropped in: the three Figma exports
  (`LDO`, `stETH`, `wstETH`) drew their disc as a `<rect rx="40">` inside a
  clip-path group, which is the shape `scaleToLibraryViewBox` cannot rescale and
  the shape the style forbids, so the disc became the `<circle>` the style asks
  for and the clip group went away. `LayerZero_emblem` and `[World] Logomark`
  arrived as bare glyphs with no disc at all, so each got one — black, with the
  emblem at 44/64 and World's logomark scaled so its own outer circle *is* the
  disc, since that circle is the mark's boundary. Mantle's brandmark is drawn
  full-bleed on its 800 canvas; it is inset to the 48/64 the old mark occupied,
  which is where the rest of the library sits (48–83% of the disc).
- **Aerodrome was 481 KB, and is 4.8 KB.** Its five swooshes were traced into
  ~2,600 micro-curves each — segments of 0.03 units on an 800 canvas, which at
  64 points is 0.0024 units, well under a pixel at any scale the app draws. The
  curves were flattened and refit to cubics at a 0.5-unit tolerance: 480,413
  characters of path data became 6,277, and the two renders differ by 0.3% RMSE.
  Its backdrop-blur filter and two clip-paths were dropped as no-ops — the
  render is identical without them.
- **Why:** an icon that nothing names is dead weight that still ships, because
  `export-swift-icons.sh` copies every SVG under `icons/crypto/` into the
  catalog whether or not a token points at it. The house style exists so the
  library is one set of 64×64 marks rather than a pile of exporter output, and
  a 481 KB icon for a 40-point badge is that failure with a size attached.
- **Test:** `CoinBadgeArtworkTests` covers this directly — every token whose
  `artworkName` is non-empty must load a real bundled image, so a name that does
  not match a file in the catalog fails rather than silently drawing a letter.
  `scripts/normalize-icons.sh --check` is the gate on the style itself; it
  passes on all 88 icons.
- **CLI check:** `spectra --json token artwork --token-id lido-dao` returns
  `{"artworkName":"lido"}`, and the same for `staked-ether`, `wrapped-steth`,
  `aerodrome-finance`, `layerzero` and `worldcoin-wld`; each returned `""`
  before.
- **Verification:** `cargo test --workspace` **802 passed**;
  `./scripts/cli-acceptance.sh` **357 passed**; iPhone 17 Pro **93 passed**

### Localized copy ships per locale and stops shipping an unread twin (2026-09-15)

- **Before:** six content files shipped four copies each —
  `CommonContent.{en,zh-Hans,zh-Hant}.json` plus an unsuffixed
  `CommonContent.json`, and the same for `Diagnostics`, `Donations`,
  `Endpoints`, `ImportFlow` and `Settings`. Each unsuffixed file was
  byte-identical to its `.en` twin and **none of them was ever read**:
  `StaticContentCatalog.candidateJSONURLs` maps the `Base` identifier onto the
  unsuffixed name, but `preferredLocalizationIdentifiers` appends the manifest's
  `sourceLanguage` — `en` — to the chain before `Base`, and `.en.json` always
  exists, so `Base` is never reached. `RuntimeStrings` already proved it by
  shipping no unsuffixed copy and working. **After:** the six are gone. Twelve
  fewer files in the bundle, and one spelling per string instead of two.
- **The fallback stays.** `AppLinks.json` and `BuyProviders.json` — URLs and
  provider lists, identical in every language — ship unsuffixed and *only*
  unsuffixed, and the trailing `append("\(baseName).json", …)` is how they are
  found. Removing it as "dead" would have broken both; the dead thing was the
  duplicate files, not the path that serves locale-independent data.
- **Why:** AGENTS.md's rule for `resources/` is that the file name is the only
  disambiguator, because the group copies flat. Two names for one string, one of
  them unreachable, is that rule's failure mode arriving by a different door:
  nothing would have caught `CommonContent.json` drifting from
  `CommonContent.en.json`, and the unread one is the one a search finds first.
- **Test:** `testLocalizedCopyShipsPerLocaleAndNothingElse` asserts the absence
  directly — no `<name>.json` in the bundle for the six localized files, and a
  present one for `AppLinks` and `BuyProviders`, so the two categories cannot be
  confused again. `testBundledCopyDecodesInEveryLocale` was the reason the
  duplicates looked required: it decoded a `"base"` variant, pinning the unread
  file in place as if it were the contract. It now reads `availableLocales` from
  `RuntimeStrings.manifest.json` instead of a hardcoded list — so a new locale
  cannot leave a file untested — and covers all six files rather than four.
  `WalletSetupPageCopyTests` reads `ImportFlowContent.current`, which goes
  through `loadRequiredResource` and `fatalError`s if nothing resolves; it
  passes, which is what proves the loader never needed the twins.
- **CLI check:** none applies; these are iOS bundle resources and core does not
  read them.
- **Verification:** `cargo test --workspace` **802 passed**;
  `./scripts/cli-acceptance.sh` **357 passed**; iPhone 17 Pro **93 passed**
  (92 + the new absence test). Changes remain uncommitted.

### A backend endpoint and an indexer stop sharing one role bit (2026-09-15)

- **Before:** `ENDPOINT_ROLE_BACKEND` was written `1 << 9`, the same shift as
  `ENDPOINT_ROLE_INDEXER` one line above it — under a doc comment that exists
  because `explorer` and `indexer` had already shared a bit once, and which did
  not stop the next constant from repeating the mistake. `catalog_endpoints`
  builds a non-EVM chain's primary list from `RPC | BALANCE | BACKEND`, so the
  mask also matched every `kind = "indexer"` row, including the sixteen that
  carry no `balance` capability. `with_fallback` tries a chain's endpoints top
  to bottom, so those became second and third choices for a balance read:

  | chain | primary list before | after |
  |---|---|---|
  | `bitcoin-cash` | 6 — `blockchair/bitcoin-cash`, **`…/push/transaction`**, **`…/dashboards/transaction/`**, `actorforth/v2`, **`…/sendRawTransaction/`**, **`…/transaction/details/`** | 2 |
  | `bitcoin-sv` | 7 — `whatsonchain/main`, **`…/chain/info`**, **`…/tx/raw`**, **`…/tx/hash/`**, `blockchair/bitcoin-sv`, **`…/push/transaction`**, **`…/dashboards/transaction/`** | 2 |
  | `xrp` | 5, including **`xrpscan.com/api/v1/account`** (`read`, `history`) | 4 |
  | `near` | 3, including **`api.nearblocks.io/v1`** (`read`, `history`) | 2 |
  | `bittensor` | 2, including **`api.taostats.io`** (`history`, `verification`) | 1 |

  The bolded URLs are not API bases. `…/push/transaction` accepts a broadcast,
  `…/dashboards/transaction/` and `…/tx/hash/` are URL prefixes a transaction id
  is appended to, and `…/chain/info` is a single verification document. BSV's
  *second* choice for a balance read was `…/chain/info`. **After:**
  `ENDPOINT_ROLE_BACKEND` is `1 << 10` and each list holds only endpoints that
  answer one of the three roles asked for. `bitcoin` (6) and `monero` (3) are
  unchanged: Bitcoin's indexers all carry `balance`, and Monero's three backends
  carry it too — so this never affected the chain the `backend` kind exists for.
- **Why:** Rule 0. The cost of the collision was not a wrong balance — a dead
  endpoint fails and the next one answers — but `endpoints.toml`'s own header
  states the price: "a dead one costs a timeout plus 180 ms on every call that
  reaches it", paid on every BCH and BSV read, twice. Two constants that must
  differ and are only ever read as a mask cannot be left to be checked by eye;
  this is the third time this file has been asked to and the second time it got
  it wrong.
- **Tests:** two, at the two altitudes the bug lived at, and each fails on the
  old constant with the bug named:
  - `every_role_name_owns_a_distinct_bit` walks all eleven role names through
    `endpoint_role_bit` and asserts each owns one bit nothing else owns
    (`"backend" and "indexer" share bit 0x200`). The two deliberate aliases
    (`rpc`/`rpc-node`, `explorer`/`web-link`) are asserted equal, and an unknown
    name is asserted to claim nothing rather than default onto a real bit.
  - `no_chain_is_offered_an_endpoint_that_answers_none_of_them` states the
    property over the catalog rather than the constants, so it survives the mask
    being rewritten: every URL in a primary list must be `rpc-node`, `backend`,
    or claim `balance` (`xrp lists https://xrpscan.com/api/v1/account as a
    primary endpoint, but it is a "indexer" claiming only ["read", "history"]`).
- **CLI check:** none applies directly, and that is itself the finding. Every
  `spectra` command builds its service through `WalletService::new_catalog()` →
  `catalog_endpoints()`, so this is precisely the CLI's own endpoint table — but
  no offline subcommand prints it. `spectra endpoints` probes the catalog rows
  rather than the configured table, and `diagnostics show` returns the durable
  document, which is empty without a refresh. The before/after above was
  measured by calling `catalog_endpoints()` directly under both constants.
  Recorded under "Known open items": the CLI cannot show the endpoint list it is
  about to use.
- **Verification:** `cargo test --workspace` **802 passed** (800 + the two
  above); `./scripts/cli-acceptance.sh` **357 passed**; iPhone 17 Pro **92
  passed**. Changes remain uncommitted.

### Three iOS surfaces stop showing what they cannot do (2026-09-15)

- **The Home filter button (fixed):** the trailing toolbar item was a `Menu`
  whose *entire* content sat behind `if dashboardPage == .assets`, so on the
  Wallets page it presented an empty menu — the button took the tap, drew no
  menu, and looked broken intermittently because whether it worked depended on
  which segment of the in-page Assets/Wallets picker was selected. The menu had
  been the home of the Dashboard-Section picker (Beta Commit 62) until that
  picker was promoted into the page body, leaving one assets-only action and an
  orphaned leading `Divider()`. **After:** the item exists only on the Assets
  page and is a plain `Button` with a `pin` glyph that opens `PinnedAssetsView`
  directly — one action needs no menu, and a tap that cannot do anything is now
  an absent control rather than a dead one. Costs one tap less to reach pinning
  and drops the `line.3.horizontal.decrease.circle` filter glyph, which named a
  filter this menu never offered. The two dead accessibility strings behind it
  ("Configure asset view"/"Configure wallet view") are gone from all three
  `RuntimeStrings` locales; they had been passed to `.accessibilityLabel` as raw
  literals, so their translations were never read.
- **The History "Activity" card (removed):** the summary card above the filter
  strip restated what the list below it already showed — a title, a subtitle and
  three counts (Visible/Pending/Sends) derived from `visibleTransactions`, the
  same rows the sections beneath it render with per-section counts. It cost the
  first screenful of a scroll view whose purpose is the log. Its private
  helpers (`historyMetric`, `sendVisibleCount`, `pendingVisibleCount`) went with
  it. `"Sends"` and `"Pending"` stay in the locales: `HistoryFilter.sends` and
  `TransactionStatus.pending` still use them.
- **The Staking section picker (unwrapped):** `detailSectionPicker` wrapped a
  system `.segmented` picker in `padding(6)` plus `spectraCardFill`, stacking
  Spectra glass on the control's own material — the one segmented picker in the
  app that did so, and against two rules in [iosUI.md](iosUI.md) ("avoid
  stacking glass on glass"; leave a standard control the behaviour it already
  provides). It also read as a content card beside the hero and stats cards it
  controls. **After:** `.pickerStyle(.segmented)` alone, sitting on
  `SpectraBackdrop`.
- **Why:** all three are the same failure — chrome that survived the thing it
  was built around (a menu's picker, a list's own counts, a card's missing
  backdrop). Rule 0 prefers deleting the residue to preserving the shape.
- **CLI check:** none applies; no domain decision moved, and core is untouched.
  The dashboard's pin set and the history projections these views read are
  already exercised by `spectra` in `scripts/cli-acceptance.sh`.
- **Verification:** `cargo test --workspace` **800 passed**;
  `./scripts/cli-acceptance.sh` **357 passed**; iPhone 17 Pro **92 passed**;
  `scripts/check-design-tokens.sh` ok. Changes remain uncommitted.

### A wallet row this build cannot decode costs that row, not the app (2026-09-15)

- **Before:** `wallet_load_all` decoded each row with `?`, so one row it could
  not read failed the whole load — and `app_state_load` failed with it, then
  `open_state`, which every call into core waits on. The install could no
  longer list a wallet, import one, or run its own reset; the only way out was
  deleting the app. This is not hypothetical and not rare: Beta Commit 144
  shrank `CoreWalletDerivationOverrides` from ten fields to two and added
  `deny_unknown_fields`, so every wallet row written before it carries a
  `mnemonicWordlist` this build refuses, and every install that had one came up
  bricked with `wallet_load_all decode …: unknown field 'mnemonicWordlist'`
  surfacing wherever the first failed bridge call happened to put it — on the
  import screen, and mislabelled "Fiat Rates Degraded Mode" in Notices.
  **After:** an undecodable row is skipped and logged with its id; the rows
  around it load and the app runs. Nothing is deleted to achieve that — no
  production write replaces the table wholesale (`app_state_save`, the only
  `AppStateChanges::between(None, …)`, is test-only), and a skipped id is
  absent from both sides of every real diff, so it is never pruned. Restoring
  a readable shape brings the wallet back with its address intact.
  `CoreWalletDerivationOverrides` **keeps** `deny_unknown_fields`: that guards
  the way in, where silently ignoring an override a caller *set* would derive
  a different address than the caller asked for. **Why:** Rule 0 makes stored
  shapes free to change and writes no migrations, so a loader that dies on the
  shapes it no longer knows contradicts the policy it serves — every future
  change would brick every existing install. Refusing protected nothing
  either: the row holds a name, a chain and cached balances, all re-derivable
  from the secret the Keychain still holds under the same wallet id, while
  taking the app down with it removed the very paths — import, reset — that
  recover from it. CLI check: `scripts/cli-acceptance.sh` rewrites one stored
  wallet's `derivationOverrides` to the pre-144 shape with `sqlite3`, then
  asserts `spectra wallet list` still lists the other wallets, that
  `spectra wallet import` still succeeds alongside the unreadable row, and
  that undoing the rewrite brings the wallet back — proving the bytes survived.
  In Rust, `an_unreadable_wallet_row_does_not_take_the_readable_ones_with_it`
  pins the same three properties.

### The shell's unread state is gone, and two FFI columns with it (2026-09-14)

- **Before:** `WalletDerivedCache` carried fifteen fields and five had no
  reader anywhere in the app: `signingMaterialWalletIDs`,
  `privateKeyBackedWalletIDs`, `passwordProtectedWalletIDs`,
  `secretDescriptorsByWalletID` — whose readers had moved to
  `WalletServiceBridge.walletSecretState` — and `includedPortfolioHoldings`.
  Two of them were refilled from core on every rebuild, two were carried
  across it by a `preserved…` dance whose comment named populating paths that
  no longer exist, and `clearWalletSecretIndex()` emptied all four on reset.
  `walletByID` and `walletByIDString` were the same dictionary built twice
  from the same expression. Beside them sat `activeEthereumSendWalletIDs`
  (declared and reset, never read or written), `normalizedHistoryRevision`
  (incremented, never observed), `lastObservedTransactions` (rewritten on every
  transaction-projection change, never read), `cachedResolvedTokenPreferences`
  and `cachedTokenPreferencesByChain` — the first an alias for
  `tokenPreferences` through a `let resolvedPreferences = tokenPreferences`
  no-op, the second written and never read while defaulting unknown chains to
  `.ethereum` — and an unused `clamped(to:)` left behind when the settings
  clamps moved into core. **After:** all of it is deleted; `WalletDerivedCache`
  is nine fields, each with a reader, and `resolvedTokenPreferences` is one
  name for one list. `WalletDerivedState` loses
  `signing_material_wallet_ids` and `private_key_backed_wallet_ids`, which were
  echoes of an input nothing on either side of the binding read; core still
  computes signing availability, now as a local it uses to decide send and
  receive enablement. **Why:** a projection nobody reads is not a cache, it is a
  second copy of core's answer going stale in the dark, and it is exactly what
  "Swift may hold view state, not authoritative domain state" is meant to keep
  out — the secret index mirrored *which wallets can sign*, a funds-path fact,
  for no reader at all. CLI check: `scripts/unreachable-exports.sh` stays at
  zero and `scripts/count-exports.sh` is unchanged — these were record fields,
  not exports — while `spectra wallet list` and `spectra send identity` still
  resolve signing material through core in `scripts/cli-acceptance.sh`.

### The Tron send-diagnostic notice never existed (2026-09-14)

- **Before:** `AppState` held `tronLastSendErrorDetails` and
  `tronLastSendErrorAt`; the dashboard rendered a notice from them, the
  diagnostics bundle exported `lastSendErrorAt`/`lastSendErrorDetails` for
  `chainName == "Tron"`, and `core_diagnostics_json` took both as parameters.
  Nothing anywhere assigned either field — the only writes were `nil` on reset
  — so the notice could not appear, the keys were always absent, and the app
  carried a per-chain special case in three files for a feature with no
  producer. **After:** the fields, the notice, the two chain-name tests and the
  two parameters are gone, along with `tronSendDiagnosticTitle` in
  `CommonContent` and its three locales. `core_diagnostics_json` now takes five
  arguments; only `extra_network_mode` remains conditional, and only for
  Bitcoin. **Why:** a chain name compared against a string literal in the shell
  is the shape Rule 2 exists to prevent, and this one guarded state that could
  never be non-nil — the worst version of it, since a reader could not tell the
  feature was dead. Send failures are already reported through `sendError` and
  the operational event log, which every chain shares. CLI check: `spectra
  --json diagnostics show --chain tron` builds its document and contains no
  `lastSendError` key; `core_diagnostics_json`'s every-chain test covers it.

### The setup picker's short list is a catalog rank (2026-09-14)

- **Before:** `SetupView` held `popularChainSelectionIDs`, eight chain ids
  typed into a Swift view and used three ways — the short list, its order, and
  the "extra selections" count. A chain added to `chains.toml` could not reach
  that list, and an id removed from the catalog would have matched nothing
  without saying so. Beside it, `SetupChainSelectionDescriptor` had both
  `symbol` and `gasTokenSymbol`, filled from one catalog column through a
  `gasToken` parameter whose only call site passed
  `chain.gasTokenSymbol == chain.gasTokenSymbol ? nil : chain.gasTokenSymbol`
  — a comparison of a value with itself, so always nil. **After:**
  `popular_rank` is a column of `chains.toml` (Bitcoin 1 through Litecoin 8),
  published on `ChainEntry`, and the view derives the list by sorting on it;
  the descriptor has one symbol field. The picker shows the same eight chains
  in the same order. **Why:** per-chain facts belong on the catalog, and a rank
  carries the ordering the array was silently also providing. A catalog test
  now states what the array could not: the ranks are `1..=n` with no gaps,
  duplicates or testnets. CLI check: `spectra --json chains --filter Bitcoin`
  reports `"popularRank":1` and `--filter Polygon` reports `"popularRank":null`;
  both are in `scripts/cli-acceptance.sh`.

### The performance-sample buffer and `swift-collections` are gone (2026-09-14)

- **Before:** `AppState` kept `recentPerformanceSamples`, a 120-entry
  `Deque<PerformanceSample>` that `recordPerformanceSample` prepended to from
  `refreshPendingTransactions` and `rebuildNormalizedHistoryIndex`. Nothing read
  it — no view, no log, no export, no test — so the timings were measured,
  boxed and dropped. **After:** the buffer, the recorder, the
  `PerformanceSample` record and both call sites are deleted, and with them the
  `swift-collections` package: `DequeModule` was the only thing it supplied, so
  the dependency existed to back a collection with no reader. `KeychainAccess`
  stays; `swift/SecureStores.swift` is built on it. **Why:** instrumentation
  that nobody can read is not instrumentation, and a package dependency is a
  build input, a resolution step and a supply-chain surface. Timing worth
  keeping belongs where it can be seen, which on this boundary means core, not
  a Swift-side ring buffer. No CLI check is possible — the removed code was
  Swift view-layer instrumentation with no core surface — so the gate is the
  iOS suite, which still passes 92 tests, plus `xcodebuild
  -resolvePackageDependencies` now resolving `KeychainAccess` alone.

### The send Live Activity actually runs (2026-09-14)

- **Before:** `SpectraLiveActivityExtension` compiled, embedded as an `.appex`
  and the app declared `NSSupportsLiveActivities`, but nothing ever asked the
  system to run one: `ActivityKit` appeared in the extension's own two files
  and nowhere in the app. A lock-screen widget that could never be shown.
  **After:** `swift/AppState+SendLiveActivity.swift` starts an activity when a
  broadcast is accepted, ends it carrying the outcome when core reports a
  terminal status for that transaction, and reconciles on foreground so a send
  that resolved while the app was not running does not leave a spinner on the
  lock screen. **Why:** the feature was three call sites short of existing, and
  a send is exactly the wait a Live Activity is for — the alternative was
  deleting a finished extension. The activity is identified by
  `TransactionRecord.id` carried in its attributes, so `Activity.activities` is
  enough to find it again, including after a relaunch when no Swift-side handle
  survived; holding handles in `AppState` would be a second copy of a list
  ActivityKit already keeps, and the copy is what goes stale. No CLI check is
  possible — ActivityKit is presentation with no core surface — so the gates are
  `swift/tests/SendLiveActivityContentTests.swift`, which pins the phase text,
  the symbol/amount split and the truncations, plus the iOS suite.

### `app_core_resolve_derivation_path` returns the path (2026-09-14)

- **Before:** it returned `AppCoreDerivationPathResolution`, a four-field
  UniFFI record: `chain` (the argument handed straight back), `normalized_path`,
  `account_index` and `flavor`. **After:** it returns the normalized path.
  `resolved_account_index`, `resolved_flavor`, `derivation_path_segment_value`,
  `chains::derivation_paths_for_chain` and, on the Swift side,
  `SeedDerivationResolution` and the six-case `SeedDerivationFlavor` are gone.
  **Why:** every consumer read `normalized_path` and dropped the rest — the CLI,
  both in-crate callers and the app. Swift decoded `flavor` into an enum whose
  cases nothing ever compared against, so the work, the record and the bytes
  crossing the binding were for nobody. Check: `spectra wallet new --chain
  Bitcoin` still stores `m/84'/0'/0'/0/0` — the existing "stores the catalog
  derivation path" assertion in `scripts/cli-acceptance.sh`.

### UniFFI floor moves to 0.31.2 (2026-09-14)

- **Before:** `uniffi = "0.31"` resolved to 0.31.1, whose Swift template emits a
  bare `static let vtablePtr`, which Swift 6 rejects. `scripts/bindgen-ios.sh`
  rewrote the two declarations with a `sed` pass through a temp file held
  outside `OUT_DIR`, so Xcode's synchronized group could not catch the ghost.
  **After:** both manifests require 0.31.2, whose `CallbackInterfaceImpl.swift`
  emits `nonisolated(unsafe)` itself. The patch, the temp-file dance and their
  rationale are deleted; the script generates and copies the modulemap, nothing
  more. **Why:** a downstream rewrite of generated sources is a second, silent
  owner of them, and the failure that already cost this project a day was
  exactly that — this script and Xcode's build phase patching differently, so
  the bindings depended on which ran last. Check: `./scripts/bindgen-ios.sh`,
  then `grep -c '^    static let vtablePtr:' swift/generated/spectra_core.swift`
  is 0 while both declarations read `nonisolated(unsafe)`; the Xcode build
  compiles that unpatched output under Swift 6.
- **Before:** generated Swift lifted every Rust string with
  `String(bytes:encoding:.utf8)!` — Foundation's NSString decoder, which
  silently drops a leading U+FEFF, behind a force-unwrap.
  **After:** 0.31.2 lifts with `String(decoding:as: UTF8.self)`. A string core
  returns reaches Swift byte for byte, and malformed UTF-8 would substitute
  U+FFFD rather than trap. **Why:** an address, memo or remote-sourced symbol
  that displays differently from what core holds is the wrong kind of surprise
  in a wallet. This is Rust to Swift only; strings Swift passes into core never
  went through that path. No CLI check is possible — the converter exists only
  in the generated Swift — so the gate is the iOS suite.

### Test gate cleanup (2026-09-13)

- **Before:** CLI `contains` and `lacks` ignored command exit status. A failed
  command could satisfy the expected text (or omit forbidden text) and pass.
  **After:** shared assertions require exit zero as well as the output predicate;
  rejection-message checks explicitly require their documented exit code. Five
  duplicate exit-only invocations are folded into the message checks. The
  rebroadcast fixture requires its exact failure code and missing-record message.
  **Why:** crashes and usage errors must not masquerade as successful operations
  or domain refusals. Delete the follow-up's false-positive invalid-amount check:
  its wallet had no holding, so it failed before examining the amount. The
  existing seeded-asset fixture in `cli-owned-send.py` now requires exit 3 and
  an amount-specific error for each invalid amount instead of any failure.
  Negative amounts use `--amount=-1` so argument parsing actually delivers the
  value to core rather than rejecting it as an unknown option.
  CLI check: `bash scripts/test-cli-assertions.sh` injects
  success, wrong output and wrong exit codes; it is included in acceptance.
- **Before:** the CLI gate launched 27 filtered `cargo test` commands, repeating
  workspace coverage and allowing stale filters to succeed with zero tests.
  **After:** those invocations are removed. All Rust tests remain in
  `cargo test --workspace`; actual CLI processes and loopback provider fixtures
  remain in `./scripts/cli-acceptance.sh`. The follow-up fixture now asserts the
  returned maintenance policy and parses the wallet name structurally, instead
  of merely writing JSON or matching text. Both gates are still required.
- **Swift coverage:** remove `DiagnosticsChainTableTests`, which re-evaluated
  the production registry expression. `DiagnosticsBundleCoverageTests` retains
  unique names and per-chain document checks; `DiagnosticsBundleTests` retains
  the export/import and canonical-key checks. Merge setup-flow copy coverage
  into the complete page/mode matrix, including an explicit flow-page coverage
  check; fold private-key precedence into the secret-copy cases and assert both
  title and subtitle. Remove the second asset-wiki image-loading loop, retaining
  the identity-to-image check and the separate network-wiki loop. Swift has 86
  test methods instead of 89; no key, amount, persistence or FFI coverage is
  removed. Verify with the required iPhone 17 Pro `xcodebuild test` command.
- **Setup copy inconsistency caught by the stronger test:** when create mode
  and private-key import were both set, the title said “Record Seed Phrase”
  while the subtitle requested a private key. Both now describe private-key
  import, matching the existing precedence of the input/subtitle. This is
  presentation-only; `./scripts/cli-acceptance.sh` retains private-key import
  and signing-identity coverage, while `testTheSecretPageNamesWhichSecretItIsAskingFor`
  verifies the title/subtitle matrix in the iOS suite.

Verification completed: `cargo test --workspace` (835 Rust tests passed),
`./scripts/cli-acceptance.sh` (351 checks plus the CLI provider fixtures passed),
and the required iPhone 17 Pro `xcodebuild test` suite (86 tests, zero failures,
including `testEthereumTestNetworksExposeExpectedContextsAndEndpoints`).
`git diff --check` passes. Changes remain uncommitted.


### Deep rescan and import boundary cleanup (2026-09-13)

Verification: `cargo test --workspace` passed 835 tests; CLI acceptance passed
351 checks plus its follow-up fixtures; iOS passed 86 tests with zero failures,
including the Ethereum test-network and async refresh-binding checks. The iOS
run used `-derivedDataPath /tmp/spectra-rescan-derived` after a concurrent build
locked the shared build database. `scripts/unreachable-exports.sh` reports zero
and `git diff --check` passes. Logs: `/tmp/spectra-rescan-rust.log`,
`/tmp/spectra-rescan-cli.log`, `/tmp/spectra-rescan-ios.log`.


- **Deep rescan:** Swift previously sequenced discovery, receive reservation
  advancement, balances, history and pending polling, swallowed failures and
  unconditionally logged completion. `AppRefreshIntent::DeepRescan` now owns
  this sequence under core's refresh lock. Unsupported chains refuse early;
  offline scans return a failure; discovery, reservation, balance, history and
  pending failures remain visible. Swift records successful completion only
  when the operation and projection adoption succeed. Balance scope is the
  requested network (or its family for a mainnet request), rather than every
  wallet. Price refresh is not part of a deep rescan.
  CLI check: `spectra --json diagnostics refresh --intent
  '{"deepRescan":{"chain_id":"bitcoin"}}' --conditions
  '{"appIsActive":true,"isNetworkReachable":false,"isConstrainedNetwork":false,"isExpensiveNetwork":false,"isLowPowerMode":false,"batteryLevel":1,"wantsPriceRefresh":false}'`
  reports failures; the same request for Ethereum refuses. Covered in
  `scripts/cli-shell-boundary.py`; core also checks an empty scope and provider
  failure without configured providers.
- **Raw import inputs:** Swift previously lowercased/split mnemonics, normalized
  private keys, gated Bitcoin xpubs and repeated Monero watch-only refusal.
  It now forwards the fields; core canonicalizes mnemonic whitespace/case and
  private-key hex before derivation and storage, while existing registry-backed
  import validation handles watch-only support and xpub applicability.
  Uppercase mnemonics with tabs/newlines now derive and store the same identity
  through either caller. CLI check: `scripts/cli-shell-boundary.py` imports the
  raw and canonical forms and compares addresses plus `send identity` results;
  the acceptance suite also exercises Monero watch-only refusal and xpub import.
- **Dead persistence:** removed unused `PersistedCoin` conversions, Swift's
  unused derivation-path Codable compatibility fallback, and the write-only
  discovered-address cache. Core's current storage format is unchanged; no
  migration shim is added.


### Remaining Swift boundary rewrite (2026-09-13)

- **Derivation inputs:** Swift previously trimmed passphrases/HMAC text and
  converted invalid iteration text to a missing override. Eight other override
  fields were not implemented consistently across import, discovery and signing.
  The schema and UI now contain only exact passphrase and HMAC secrets; core
  maps empty text to absence and the registry validates chain support. Unsupported
  fields are removed directly, with unknown JSON fields refused. Derivation paths
  and chain defaults remain supported. No migration or compatibility aliases.
  CLI: `wallet import --derivation-input-file <file>` drives the same parser;
  `scripts/cli-shell-boundary.py` checks whitespace-distinct wallets, signing
  identity, unsupported parameters, and no writes on refusal.
- **UTXO controls:** maximum-input, RBF/CPFP-intent and Litecoin change-strategy
  controls were displayed but never reached owned preview/execution. They and
  their Swift state are deleted. The actual core send policy and fee-priority
  setting remain authoritative. No new RBF/CPFP or change-strategy support is
  claimed. CLI: owned quotes and fee/refusal fixtures in `cli-owned-send.py`.
- **Send confirmation:** a Swift bypass boolean and caller-held self-send
  acknowledgment previously authorized subsequent composer inputs. Core now
  issues a bounded, two-minute, single-use review of exact input strings,
  resolved recipient, stored network/sender/token identity and quoted fees.
  Swift displays the reviewed amount, network, address, fee and warnings before
  confirmation. Core consumes the review before execution, rejects edited,
  expired, missing or reused reviews, rechecks ENS and sender identity, and pins
  EVM nonce/gas/fees. Automatic nonce freshness is checked under the sender lock;
  a changed nonce requires a new review. A restart intentionally invalidates
  unsubmitted reviews. CLI `send quote` includes review warnings, and
  `send owned-broadcast --yes` reviews and explicitly confirms the same inputs.
  Rust review tests cover sub-float amount edits, expiry and consumption; the
  CLI loopback fixture checks the owned review and watch-wallet send refusal.
  This binds the owned send's inputs and quote; the separately planned durable
  Build / Sign / Broadcast artifacts and per-chain transaction-byte lifecycle
  are still future work, not implemented by this review token.
- **Refresh:** Swift selected foreground/background work, successful clocks,
  post-send pending-versus-history routing, and balance-engine cadence. Core
  now accepts scheduled, user, foreground, balance-update and selected-chain
  intents plus device conditions. It selects work and history cooldowns, returns
  committed state/failures, and avoids network work offline. Foreground full
  refresh has a core-owned 120-second successful-refresh clock. A post-send
  refresh also reads selected-chain history, including receipt-polling chains,
  so incoming activity/details are not omitted. Balance-loop cadence and empty
  wallet decisions read core state, with cadence reconfiguration after settings
  commit. The background timing label now says adaptive rather than displaying
  a separate Swift formula. CLI: `diagnostics refresh --intent <json>
  --conditions <json>`; offline/invalid-network checks live in
  `scripts/cli-shell-boundary.py`.


Verification for this rewrite: `cargo test --workspace` **833 passed**;
`./scripts/cli-acceptance.sh` **355 passed** plus fixture suites;
iPhone 17 Pro `xcodebuild test` **89 passed**, including
`testEthereumTestNetworksExposeExpectedContextsAndEndpoints`. The full local
review/sign/broadcast regression passes, including checksum-case sender identity,
single-use confirmation and stale-nonce refusal before a second broadcast.
Bindings regenerated with UniFFI 0.31; **145** callable exports and **zero**
unreachable candidates. Design-token and diff checks pass.

Logs: `/tmp/spectra-boundary-rust.log`, `/tmp/spectra-boundary-cli.log`,
`/tmp/spectra-boundary-ios.log`. Manual simulator UI inspection was attempted
but the computer-use tool repeatedly timed out; no visual smoke result is
claimed. No Git staging, commits or other Git state changes were performed.

### Domain and boundary names describe the values they carry (2026-09-13)

- `WalletSummary` is now `WalletState`, the authoritative wallet record;
  `CoreImportedWallet` is `WalletView`, its platform projection. Conversion
  methods are `to_wallet_state` / `to_wallet_view`, and the FFI conversion is
  `core_wallet_state`. Swift uses `WalletView` directly without an
  `ImportedWallet` alias.
- Resource names are `artwork_name` / `artworkName`; asset display labels are
  `asset_display_name` / `assetDisplayName` (including native asset labels).
  Catalog TOML, persisted transaction/alert JSON, CLI outputs, fixtures and
  Swift records use the new keys directly. There are no old-key aliases or
  migrations. The artwork lookup functions are `core_*_artwork_name`.
- Database handle locals are `database`, while actual path inputs are
  `database_path`. `WalletDatabase::new` creates a lazy connection handle;
  `WalletService::new` replaces the obsolete `new_typed` constructor.
- `evm_network_for_id(network_id)` resolves an EVM network by registry string
  ID; it does not accept an EVM numeric chain ID. `execute_protocol_send`
  describes dispatch that may either sign only or also submit.
  `EvmSendDetails` replaces the obsolete `EvmSendResultDecoded` name, and
  `SendExecutionResult.protocol_result_json` explicitly names the serialized
  protocol result rather than implying it is the raw broadcast payload.
- As requested, `TokenEntry`, its `id`, `token_id` and `chain` remain named
  as before. This pass changes names and serialized field keys, not wallet,
  signing, broadcast or artwork decisions.
- CLI checks: `spectra --json wallet derived`, `spectra --json token artwork
  --token-id ethereum` (returns `artworkName`), and the transaction, alert and
  send fixtures in `./scripts/cli-acceptance.sh`. Existing persistence and
  Swift bridge tests exercise the coordinated schema and API rename.

Verified 2026-09-13: `cargo test --workspace` **828 passed**;
`./scripts/cli-acceptance.sh` **354 passed** plus its fixture suites;
iPhone 17 Pro `xcodebuild test` **87 passed**, including
`testEthereumTestNetworksExposeExpectedContextsAndEndpoints` and the
all-deployment artwork bridge test. Bindings regenerated with UniFFI 0.31;
export audit reports 145 callables and zero unreachable candidates.
`git diff --check` passes. No Git state changes.


### Core dead-code closure (2026-09-13)

- **Before:** Rust retained a staking action dispatcher, 23 per-chain preview
  builders, two uncalled staking queries, and action request/preview types after
  the Swift execution controls were removed. **After:** staking exposes only
  the validator and position query paths; the isolated action implementations,
  seven unused error variants, the unused staking error FFI export, and four
  self-contained action tests are deleted. Rationale: these were never an executable signing/broadcasting flow;
  retaining them falsely suggested supported transactions. CLI check:
  `./scripts/cli-acceptance.sh` includes the configured Solana validator fixture
  and unsupported-chain/testnet refusals in `scripts/cli-shell-ownership.py`.
- **Before:** Stellar kept an uncalled issued-asset signing entry point and its
  credit-asset encoder. **After:** the live native-XLM sender directly builds a
  native payment; its unused wrapper, asset enum and credit encoding are gone.
  Rationale: no app or CLI route supplies an asset code/issuer. CLI check:
  `spectra send --help` retains the owned send interface; no issued-asset route
  is added. The native envelope's amount, asset tag and Ed25519 signature are
  checked by `native_payment_envelope_encodes_amount_and_verifiable_signature`.
- Removed the obsolete Ethereum error classifier/FFI enum and portfolio
  composition signature together with their two self-only tests. The live
  persisted movement policy and send error paths remain covered by acceptance.
- Removed six uncalled helpers: `Chain::as_evm`, `Chain::has_network_choice`,
  `probe_endpoints`, `SolanaClient::account_exists`,
  `CoreImportedWallet::total_balance`, and `plan_append_chain_operational_event`.
  These removals change no reachable app/CLI behavior. UniFFI bindings are
  regenerated from Rust; no compatibility shim is retained.
- Final verification: `cargo test --workspace` **828 passed**;
  `./scripts/cli-acceptance.sh` **354 passed**; iPhone 17 Pro `xcodebuild test`
  **87 passed**, zero failures. Native XLM signature regression passed.
  `git diff --check` passed; export scan reports **0 unreachable candidates**
  and the callable FFI surface remains **145**. Generated bindings contain none
  of the removed action/classification/error types. Logs:
  `/tmp/spectra-dead-{rust,cli,ios}-final.log`. The first iOS attempt hit a
  simulator service startup failure; restarting the test simulator resolved it.

### Shell ownership: receive, naming, movement and staking

- **Receive:** before, a formatter returned either addresses or instructions in
  one string, and Swift could encode the latter as a QR. After, Swift consumes
  only core's optional validated/reserved address, clears it during refresh,
  rejects stale responses and displays errors separately. Removed the obsolete
  message/source planners and their tests; owned receive validation, reservation
  and wrong-network tests are the replacement coverage. CLI check:
  `spectra pool receive <wallet>` and `scripts/cli-shell-ownership.py`; the async
  Swift binding test also proves a missing wallet refuses before reservation.
- **Names:** before, Swift computed a start index from its wallet projection,
  while CLI supplied chain-specific default names. After, unnamed imports use
  the first available positive `Wallet N`, allocated inside core's import writer
  against the latest stored names. Explicit names remain explicit; no migration
  or compatibility parameter remains. CLI check: repeated `wallet watch` without
  `--name` in the new fixture, each in a fresh process. Rust test
  `default_wallet_names_are_allocated_under_the_import_writer` covers concurrent
  imports, existing names and reopening.
- **Movement:** before, Swift kept session-only totals/composition and submitted
  totals and thresholds to a pure evaluator. After, core reads stored holdings,
  quotes and settings, persists the comparison baseline and returns each observed
  qualifying change once. Changed wallet/deployment composition, disabled alerts
  or incomplete/invalid quotes reset the baseline; testnets are excluded.
  Foreground observations advance it without alerting, and reopening retains it.
  Notification authorization/delivery remain device operations; failures are
  logged. CLI check: `spectra alert movement [--active]`; the new fixture covers
  thresholds, composition changes, missing quotes, restart and deduplication.
- **Staking:** before, Swift and CLI assembled different endpoint-role lists;
  the app cached a second service's endpoints. After, queries use core's current
  effective transport; explicit Rust overrides stay explicit and unsupported or
  testnet requests refuse before network access. CLI check: `spectra staking
  endpoints --chain Solana` and `staking validators --chain Solana` against the
  loopback fixture. iOS Actions previously only reloaded validators. Those
  buttons, unreachable position/preview UI, unused pool reads and wrappers are
  deleted; the page now explicitly says it does not sign/submit transactions.
  Core builders remain internal Rust APIs, not proof of an end-user send flow.
- **History labels:** before, current wallet/global selection could relabel an old
  transaction. After, its title uses its recorded concrete chain. Unused network
  suffix helpers are removed. CLI `txs` already reports the recorded chain; the
  Swift regression switches global network and checks the historical title.
- **Reachability:** before, declarations/comments could make an unused export
  appear reachable and the count omitted exported `pub(crate)` methods. Now both
  scripts count those methods, and the candidate search ignores declarations and
  comments and requires call-shaped references. Internal secret/history/recipient
  methods leave the FFI. This is a candidate search, not whole-program proof;
  constructors and foreign callbacks are checked separately. CLI checks:
  `scripts/count-exports.sh`, `scripts/unreachable-exports.sh` plus regenerated
  binding inspection. System callbacks remain; dead helpers are deleted, not
  ported, and a record-copy helper used only by a test lives with that test.


### Core simplification: commit order, snapshots, results, database ownership and artwork

- **Opening is a commit, not an early binding.** Before: `open_state` replaced
  keypool and resident state and marked the database bound before saving the
  catalog seed; a failed save could make a retry return the uncommitted state.
  After: loading, seeding and saving finish before publishing any of those
  values. A failed first open remains unbound, and a failed rebind retains the
  previous state/connection. Check `cargo test -p spectra_core
  failed_open_does_not_publish_and_can_retry_seeding`.
- **One snapshot per derived response.** Before: wallets, token preferences
  and resolved addresses were read separately, and domain calculations went
  through `CoreImportedWallet`. After: a pure transformation reads one
  `CoreAppState` snapshot directly; refreshable network names reflect the
  wallet's concrete network. Secret availability remains a separately queried
  external capability. Check `cargo test -p spectra_core
  derived_wallet_maps_share_one_snapshot_during_mutation` and
  `spectra --json wallet derived`.
- **Typed send results until the output boundary.** Before: every protocol
  result became JSON, then its transaction identifier and EVM fields were
  scraped back out; missing fields could silently become empty strings or
  fallback numbers. After: a protocol result enum retains the concrete
  structs, uses `SignedSubmission` for common fields and converts EVM integers
  with range checks. JSON is emitted only for the persisted/FFI result.
  An empty submission identifier is refused; a sign-only result must carry
  signed bytes. Protocol signing and submission behavior remains explicit;
  this does not implement the separate planned build/sign/broadcast UI.
  Check `cargo test -p spectra_core service::send_result`, plus the existing
  offline `scripts/cli-owned-send.py` and send/rebroadcast fixtures in
  `./scripts/cli-acceptance.sh`.
- **The service passes its database handle directly.** Before: a held `Arc`
  existed only to keep a weak global path index alive, and key/value storage
  used a second process-global connection pool. After: all storage operations
  take the owned `WalletDatabase`; clones share its connection, independent
  opens have their own handles, and SQLite coordinates separate connections.
  Key/value and domain tables share the same service connection. No database
  lookup by path happens during an operation. Check `cargo test -p spectra_core
  store::wallet_db::connection::tests` and the CLI persistence/reopen checks.
- **Artwork follows identity.** Before: artwork identifiers packed network,
  ticker and contract into strings, then lookup discarded everything except
  ticker. After: token IDs, deployment IDs and network IDs directly resolve
  catalog artwork; holdings derive their deployment identity in core. Unknown
  contracts cannot borrow a known token's mark by copying its ticker. History
  without a deployment identity draws a letter; identified local sends retain
  their token artwork. Removed the unused canonical-chain symbol parameter and
  the old identifier-building/parsing exports. CLI checks:
  `spectra --json token artwork --token-id ethereum`,
  `spectra --json token artwork --network-id base`,
  `spectra --json token artwork --deployment-id base:native`; unknown identities
  return an empty artwork name. The acceptance gate covers each case.
- **Case-sensitive identifiers survive either network spelling.** The full
  artwork deployment test exposed that `normalize_token_identifier` recognized
  display names but not registry IDs and lowercased SPL/TON/TRC-20 identifiers
  when given IDs. It now resolves both spellings before applying the protocol's
  normalization. Check `cargo test -p spectra_core
  artwork_uses_identity_and_unknown_contracts_cannot_borrow_a_symbol`, which
  resolves every catalog deployment's holding template.

Verified 2026-09-12: `cargo test --workspace` **846 passed**;
`./scripts/cli-acceptance.sh` **353 passed** plus its offline fixture suites;
iPhone 17 Pro `xcodebuild test` **85 passed**, including full-deployment artwork
and `testEthereumTestNetworksExposeExpectedContextsAndEndpoints`.
UniFFI bindings regenerated with `scripts/bindgen-ios.sh`; export audit reports
158 callables and zero unreachable candidates. `git diff --check` passes.
Changes remain uncommitted.


### Swift awaits storage readiness before asynchronous core operations

- Before: Swift async bridge calls could reach an unbound core service while
  launch was still opening SQLite (or after a silently ignored open failure).
  Import then reported `transaction store not opened: call open_state first`.
- After: bridge operations await a successful core `open_state` before running;
  concurrent opens use core's existing serialized, idempotent binding. Failed
  opens propagate their actual error and can be retried; commands never fall
  back to unpersisted state. The service has its Keychain adapter at creation,
  before any operation can need signing material. Launch open failures are logged.
- CLI check: `./scripts/cli-acceptance.sh` covers import and reopening persisted
  wallets. Swift tests now import a known test mnemonic through a fresh bridge
  without calling `openState` first, reopen it, and test a failed database open
  followed by recovery without a phantom in-memory setting write.
- Verification: Rust workspace 855 tests, CLI acceptance 347 checks plus offline
  fixtures, and iPhone 17 Pro 85 tests passed, including cold bridge import,
  failed-open recovery and network-selection projections. Launch snapshots
  continue to use core's serialized `open_state` so they cannot overtake writes.

### Wallet editing opens the name field

- Before: Swift opened wallet editing on the chain-selection page, which hides
  its content while editing, leaving no name field. After: editing opens the
  wallet-name page with the edit heading and existing name; Save uses the
  existing core rename intent. New-wallet flows still start at chain selection.
- Rationale: the editor must start on its only page (`SetupFlow.editWallet`).
- CLI check: `./scripts/cli-acceptance.sh` includes the offline follow-up check
  `wallet rename Followup Renamed`, proving persisted rename and portfolio
  inclusion remain independent. Swift's existing rename test checks holdings
  and editor dismissal; the page-copy test checks the name page's edit heading.
- Verification: workspace Rust tests (855), CLI acceptance (347 plus offline
  fixtures), and iPhone 17 Pro tests (84) all passed. The simulator required a
  restart after its test-launch service disconnected; the complete rerun passed.

### Provider history uses transaction identity and checked ownership

- **One identity for sends and refreshes:** provider history previously omitted
  deployment IDs while local sends supplied them, so identical transaction
  hashes failed to merge. EVM native/token history now derives deployment IDs
  from the actual network and contract, using the same helper as sends. Native
  Bitcoin/UTXO and normalized native history carry their network's deployment;
  Solana mints and Tron contracts survive normalization. A ticker alone never
  invents a token contract. Check `cargo test -p spectra_core history_identity`
  and `cargo test -p spectra_core history_tokens_with_the_same_symbol`.
- **Store the actual network:** refreshed testnet records previously used the
  mainnet family's display name, unlike local sends and status polling. Every
  owned refresh now writes the actual network and merges per network; refresh
  scope and pagination remain wallet-scoped and core-owned. Check `cargo test -p
  spectra_core a_testnet_wallet_fetches_and_persists_its_exact_network` and
  `spectra history WALLET --save` via the offline CLI fixtures.
- **Execution outcome:** EVM explorer history discarded execution failure flags
  and marked every transfer confirmed. Native history now retains confirmed or
  failed outcomes; missing execution evidence fails the fetch rather than
  claiming success. Check `cargo test -p spectra_core execution_history_regressions`.
- **Owned refresh writes:** a response arriving after wallet deletion used to
  recreate its history. The merge now checks the wallet's existence and exact
  network inside the same SQLite write transaction, and reads its current name.
  Deleted or moved wallets cannot receive stale rows. Check the history-identity
  regression above, including reopening the database after deletion.
- **Fresh status corrections:** the previous blanket confirmed-state guard also
  discarded real failures and reorgs. Automatic polling now carries the record
  snapshot that initiated its read; changed identity/status/receipt fields
  reject the stale response. A response to the current snapshot may report a
  failure or return to pending, which clears obsolete receipt fields and resumes
  tracking. Check `cargo test -p spectra_core status_commit_regressions`.


Verified on the final code: `cargo test --workspace` (855 core tests),
`./scripts/cli-acceptance.sh` (347 checks plus the offline fixture/regression
batches), and the required iPhone 17 Pro simulator suite (84 tests, zero
failures). UniFFI bindings were regenerated by the Xcode build phase; the
Ethereum testnet endpoint test also passed.


### Finish Swift-owned send and alert decisions (2026-09-12)

- **Before:** Swift rounded alert targets to cents, converted fiat using its
  cached rates, deduplicated and replaced the whole alert list, including trigger
  flags. **After:** `AddPriceAlert`, `TogglePriceAlert` and `RemovePriceAlert`
  apply to current core state. Core validates and converts using its stored rate,
  preserves subcent targets and rejects exact duplicates. Editing one rule cannot
  overwrite another rule's latest trigger flag. **Why:** one durable writer and
  correct small-token prices. **CLI:** `alert add --chain ethereum --target
  0.000001`, `alert toggle <id>`, `alert remove <id>`; `--holding` identifies tokens
  and `--currency` chooses the input currency; a missing stored rate is rejected.
- **Before:** Swift selected protocol previews, source addresses, native/token
  routes, Polkadot seed availability, fee fallback and gas affordability, then
  assembled a send request and multiplied fees by `1e8`. **After:**
  `preview_owned_send`, `quote_owned_send` and `execute_owned_send` accept only
  wallet/holding identity and user input. Core resolves its stored network,
  addresses, token metadata, balances, fees and checked atomic fee units. A failed
  provider read is an error rather than permission to use a guessed fee. A watch
  address can be previewed without reading its signing secret. **Why:** protocol
  and funds decisions must be identical for every front end. **CLI:** `send
  preview`, `send quote` and explicit `send owned-broadcast --yes`; the acceptance
  fixture never signs or broadcasts.
- **Before:** Swift chose the self-send address set and had a separate Dogecoin
  branch. **After:** core checks all managed wallets on the asset's network,
  including owned UTXO discovery, and owns the acknowledgment expiry. Swift keeps
  the transient acknowledgment and confirmation text. **Why:** sending to another
  owned wallet is also a self-send. **CLI:** `send self-check --wallet <wallet>
  --holding <deployment> --amount 1 --destination <address>`.
- **Before:** Swift returned another address slot when the selected network's
  address was absent. **After:** core supplies a validated display projection;
  its family label resolves to the selected network, and absent selected-network
  addresses stay absent. Import awaits this projection. Signing and quoting use
  concrete networks and refuse a mismatch. **Why:** an address on another network
  is not a fallback. **CLI:** `wallet derived` and the network-mismatch fixture.
- **Before:** Swift built replacement destinations/amounts, truncated amounts to
  eight decimals and substituted 4/2 gwei when estimation failed. **After:**
  `replacement_draft` reads the pending record and wallet, preserves the recorded
  decimal value, resolves the live nonce and requires a fee estimate. **Why:**
  cancellation and speed-up must describe the actual stored transfer. **CLI:**
  `send replacement <transaction-id> [--cancel]`.
- **Before:** launch loading merged local wallet/history snapshots over stored
  results; a further Swift difference of transaction IDs sent delete commands.
  **After:** loading adopts core projections only if no newer local projection
  landed during the read. The redundant background orphan deletion is removed;
  core wallet deletion retains responsibility for relational cleanup. Test-only
  whole-record seed helpers are removed from the app target. **Why:** stale views
  must neither resurrect nor delete durable records. **CLI:** existing wallet
  removal/reset and history/recheck acceptance fixtures exercise the durable
  writer; normal `txs` reads do not delete orphan rows.

Verification: `python3 scripts/cli-owned-send.py target/debug/spectra` (included
in `cli-stage3-followup.sh` and the main acceptance gate), core alert-intent and
non-EVM owned-preview regressions, plus the required three complete suites.

### History isolation, atomic status commits and indexed lookups

- **Network isolation:** EVM refresh previously grouped equal addresses across
  networks and fetched only the first wallet's network. Groups now include the
  exact network, so mainnet and testnet histories cannot share a response.
  Check `cargo test -p spectra_core evm_groups_share_only_the_same_network_and_address`.
- **Status commits:** polling previously read a snapshot, changed it and upserted
  it later, overwriting concurrent metadata or recreating deleted records. The
  read, decision and write now share an immediate SQLite transaction; absent
  records stay absent, and late pending/stale-failure results cannot undo a
  committed confirmation. Tracker decisions publish only after the commit.
  Check `cargo test -p spectra_core status_commit_regressions`.
- **Indexed time and IDs:** status updates previously copied Swift reference
  seconds directly into the Unix index and bypassed ID normalization. Every
  status write now derives its indexed row through `history_record_from_payload`.
  Sorting stays stable after confirmation and case-insensitive wallet reads
  still find the row. Check the status-commit regressions above.
- **Wallet deletion:** querying a wallet's history normalized its ID while deleting
  it did not. Deletion now uses the same lowercase identity and the returned
  removed IDs correspond to deleted rows. Check `cargo test -p spectra_core
  store::tests::transaction_store`.
- **Lookup cost:** checking even one history ID scanned the entire ID table.
  A reused primary-key query now reads only the distinct requested IDs, without
  a SQL parameter-count limit. Check `cargo test -p spectra_core
  history_id_lookup_uses_the_primary_key_and_normalizes_duplicates`.
- **Cooldown scope:** refreshing wallet A previously suppressed fresh requests
  for wallet B on the same chain. Success clocks now use `(wallet ID, network ID)`;
  a partial/failed batch stays retryable and an explicit zero interval still
  forces a refresh. Check `cargo test -p spectra_core cooldown` and
  `spectra history WALLET --save`; `scripts/cli-acceptance.sh` includes the
  offline regression checks and the loopback CLI history fixture.


Verified: `cargo test --workspace` (849 core tests),
`./scripts/cli-acceptance.sh` (347 checks plus the offline fixture/regression
batches), and the required iPhone 17 Pro simulator suite (84 tests, zero
failures), including `testEthereumTestNetworksExposeExpectedContextsAndEndpoints`.


### Explicit networks and token deployments

- **Catalog:** mainnets previously lived in `[[chains]]` while only testnets
  appeared in `[[networks]]`, inheriting a mainnet row and then clearing fields.
  Every concrete network now has an explicit `[[networks]]` record, environment,
  family and native deployment reference. Existing stable network ids remain.
  Native symbols, precision and market ids are derived projections of
  `tokens.toml`, not network-owned metadata. The ambiguous chain ticker is
  removed, including chain wiki UI. Check `spectra --json chains --testnets`
  and `spectra --json network list`.
- **Tokens:** `tokens.toml` retains its name and now contains native BTC/ETH/MNT
  alongside protocol tokens. Independent token ids join explicit deployments;
  `Native` and `Protocol` are distinct types. Catalog loading rejects duplicate
  ids, invalid protocol identifiers, wrong-network references and priced
  testnets. Precision remains per deployment. `spectra token catalog --chain
  mantle` and the Ethereum catalog show distinct native/ERC-20 MNT deployments
  associated with the same explicitly registered token identity.
- **Stored identity:** wallets now persist a required `networkId`, including
  mainnet, rather than an optional network mode that silently follows settings.
  Explicit network-selection intents update that selection for wallets in the
  family. Existing balances retain their own concrete network identity rather
  than being relabelled on a switch. Holdings are identified by the validated
  (network, protocol, identifier) tuple; the deterministic deployment key is
  derived from that tuple, never from a ticker. Core refuses missing/invalid
  token identifiers before storing and derives known token metadata from the
  catalog. No old storage-format adapter is provided.
- **Send:** holding selection, routes, fee arithmetic, previews and token
  metadata lookup no longer treat a ticker equal to the gas symbol as native.
  Missing contracts are refused instead of filled in by symbol. An ERC-20
  called ETH builds ERC-20 calldata; ARB uses the ETH gas balance. A send with
  a network different from the wallet's selected network in that family is
  refused rather than retargeted. CLI holding arguments are deployment ids;
  `send affordability --deployment ID` explicitly selects a protocol token.
  Token tracking accepts deployment ids and refuses ambiguous symbols.
- **Portfolio and prices:** grouping uses explicit catalog token identity,
  with separate deployment breakdowns. Equal tickers or market-provider ids
  do not merge unrelated contracts. Quote-cache keys are deployment ids;
  testnet assets are distinct and unpriced. Changing the selected network does
  not change the identity or valuation of an existing mainnet holding/alert.
  Dashboard pins now store token IDs and core owns the candidate list, so
  pinning ETH cannot pin a different contract with the same symbol. Check
  `spectra --json portfolio --stored --pin-token ethereum` and
  `spectra --json portfolio --pin-options`.
  Token settings and asset Wiki lookup also use token IDs; displayed decimals
  and Swift quote lookup use deployment IDs. The UI unpriced set names concrete
  testnets, never mainnets whose wallet selection happens to be a testnet.
  Network artwork uses `network:ID`; token artwork and colors remain separate.
- **Transactions:** local sends persist their deployment id. Provider records
  without enough identity information remain explicitly unknown; they are not
  merged into a known deployment merely by ticker. Native speed-up eligibility
  requires the recorded native deployment, not a matching symbol. This may
  leave an unidentified provider row separate from a locally recorded send;
  preserving identity is preferable to merging unrelated asset movements.
- **CLI proof:** `scripts/cli-network-token-identity.py`, included in acceptance,
  exercises mainnet/testnet parity, native BTC/ETH, both MNT forms, ETH fees for
  ARB, an ERC-20 called ETH, cross-network grouping, unpriced testnets, invalid
  identifiers and reopening explicit wallet network identity. Rust regression
  coverage lives in `store::tests::network_token_identity`.


### Close history, reset, receive, quote and transport ownership

- **History:** Swift previously selected protocol drivers and wrote page/clock
  results. Core now selects stored wallet scope, dispatches through registry facts
  and advances cursors only after successful merges. Empty explicit scope means
  no work; a failed provider does not consume a successful-refresh cooldown.
  Diagnostics now refresh and save the same history, rather than running a second
  fetch only for diagnostics. Check `spectra history WALLET --save` and the
  `cli-bitcoin-history.py` fixture through CLI acceptance.
- **Reset:** Swift previously deleted secrets, cleared collections and scheduled
  detached persistence separately. `spectra settings reset --scope SCOPE --yes`
  now awaits core cleanup; invalid scopes fail before mutation, and a secret-store
  error retains the wallet for retry. History/cache reset includes quotes and
  refresh clocks. It is retryable, not atomic across Keychain and all stores.
  Absence of an iOS install marker no longer silently deletes wallet secrets;
  explicit reset owns that action. Check reset/refusal/reopen in CLI acceptance
  and `owned_reset_refuses_invalid_scope_and_waits_for_secret_cleanup`.
- **Receive:** the app previously read a mnemonic and assumed an empty passphrase
  and `m/84'/0'/0'`, or live-scanned Bitcoin balances for an unused address. Core
  now uses the stored network/path/passphrase and a durable reserved index.
  Opening receive is an offline reservation; advancing used reservations belongs
  to explicit rescan, whose activity check includes spent/pending history.
  Xpub-only wallets still derive receiving addresses. Unsupported custom range
  algorithms use the valid stored address rather than deriving under different
  rules. Display reads do not register addresses; startup no longer invents
  keypool indices for account chains. Check `spectra pool receive WALLET`,
  `spectra pool discover-chain CHAIN`, and the owned receive/passphrase tests.
- **Quotes:** Swift previously passed core's prices back for alert evaluation and
  dashboard grouping. Both now read stored quotes directly; concurrent evaluation
  emits a trigger once and persists it. Enabled native alerts also request quotes
  without requiring a portfolio holding. Check `spectra alert check --stored`,
  `spectra portfolio --stored`, and the concurrent alert/reopen regression.
- **Transport:** Swift previously rebuilt catalog slots and asynchronously pushed
  its settings copy into the service. Core now builds those slots and reads its
  committed settings on requests. CLI and iOS share this configuration; explicit
  CLI/test overrides continue to use only the supplied endpoints. Setting edits
  take effect after their core write commits. Check `spectra settings set
  rpc-endpoint.Ethereum URL` and the owned transport reset/reopen/override test.


### A shared HTTP client does not pool connections under test

- The whole suite was intermittently failing one test with `dispatch task is
  gone | runtime dropped the dispatch task`. The cause is not in that test:
  `HttpClient` is a process-wide singleton, and hyper binds each pooled
  connection's dispatch task to whichever tokio runtime first drove it. The app
  has one runtime for its lifetime, so pooling is free there; a test binary has
  one runtime *per test*, each dropped at the end of its test, so a later test
  reusing a pooled connection fails for a reason unrelated to what it tested.
- `pool_max_idle_per_host(0)` under `#[cfg(test)]` only. Production keeps its
  pool. It flaked once in four full release runs before, and has not in three
  since. CLI check: `cargo test --release -p spectra_core`, repeated.

### Six functions stop pretending to be service methods

- Six methods on `WalletService` took `&self` and never read it. A method that
  ignores its receiver still says "this depends on wallet state" to everyone
  who calls it, and `WalletService` is the type where that claim costs most.
- `derive_bitcoin_hd_address_strings` is **deleted**: `pub(crate)`, zero
  callers anywhere. `scripts/unreachable-exports.sh` did not catch it because
  it was never exported — the script checks the FFI surface, and this was below
  it.
- `sign_and_broadcast_shared_utxo`, `sign_and_broadcast_token`,
  `fetch_prices_typed` and `fetch_fiat_rates_typed` become free functions with
  no FFI change: the first two are internal, and the price reads sat in an
  `impl` block with no `#[uniffi::export]` at all, so the CLI was their only
  caller. Both CLI commands built a whole `WalletService` to reach them and now
  build nothing.
- `derive_bitcoin_account_xpub_typed` becomes a free `#[uniffi::export]`
  function — the one FFI-visible change. Deriving an xpub from a phrase and a
  path reads no wallet, no database and no endpoint. Swift's one call site and
  the bindings are updated.
- `WalletService` is down to eleven fields from fifteen, and no method on it
  ignores its receiver.

### What #11 did not do, and why

- The remaining shape of the god object is ~110 exported methods on one UniFFI
  object. Splitting it was measured and rejected: the transaction cluster
  (`transactions`, `pending_status`, `transaction_recheck`) *reads* only
  `status_trackers`, which looks like a clean seam, but it *calls* `app_state`,
  `evm_transaction_status`, `upsert_history_records` and `bound_state_db_path`.
  A separate object would have to hold the wallet state, the endpoints and the
  database binding as well — the god object would become a shared `Core` and
  every field would still be reachable from every topic.
- `scripts/unreachable-exports.sh` reports zero, so there is no dead surface to
  delete either. What remained worth doing was the state that no type could
  check (the keypool pair, the database binding) and the methods that never
  needed the type at all. Those are done; a split is a decision to take on its
  own evidence, not as the tail of this one.


### The keypool is a type, not two fields on the service

- `WalletService` carried `keypool` and `owned_addresses` as two `pub(crate)`
  maps behind two locks. They are one fact: `owned_addresses` is where a
  chain's baseline comes from — the highest index already issued — and
  `keypool` is where the next one is taken from, so a reader of one that cannot
  see the other can hand out an address somebody already holds.
- Three files took them apart. `open_state` replaced both, wallet deletion
  `retain`ed one and then acquired the other, and `register_owned_address`
  cloned the whole table, mutated the copy and wrote it back — a shape that
  drops any write landing in between. Every one of those is safe, and none of
  them is safe *by itself*: what serializes them is the `state_writer` mutex
  two layers up, which nothing at those call sites can see.
- They are now one `Keypool` with one lock over both tables, and the operations
  that span them are methods on it — `load`, `forget`, `remember_owned` —
  rather than sequences a caller has to get right. No behaviour changes: the
  same mutex still serializes the same writes. What changes is that the
  invariant is now checkable where it lives. CLI check:
  `cargo test -p spectra_core the_keypool_forgets`.
### The database binding is one fact, not two fields

- `state_db_path` and `state_database` were two `Option`s behind two locks
  holding one fact: whether this service is bound to a database, and which.
  `open_state` wrote both, one lock at a time, so a reader between the two
  writes saw a path with no connection behind it.
- The asymmetry is what hid it. Eleven call sites read `state_db_path` and
  re-open by path; *nothing* read `state_database` — it holds the strong `Arc`
  that keeps the connection alive, because the process-wide handle index behind
  `WalletDatabase` keeps only `Weak`s. A field with no readers looks like a
  field with no invariant.
- They are one `StateBinding` now, with one `Option` and one `bind`. The
  callers that only wanted the path ask for the path; the one that must have it
  asks `required_path`, which carries the "call open_state first" error that was
  spelled out at its single call site. CLI check: the existing
  `service_holds_connection_until_rebind_or_drop` still proves the handle is
  dropped on rebind, now through the type rather than around it.

### Where this leaves the god object

- `WalletService` is down from fifteen fields to twelve. Two of the three
  removed were a pair whose invariant nothing could check.
- `status_trackers` is *not* the next one: its uses are a plain map read and
  written consistently, with the decisions already in `store::plan_transaction_status_*`.
  Wrapping it would move code without moving a rule. The remaining pair-shaped
  candidate is `endpoints` + `etherscan_api_key`.


### Addresses compare in their own chain's normal form

- `normalize_address` already applies `AddressNormalization`, which is the
  registry's per-chain answer to "does case matter here". Both the high-risk
  send check and the self-send confirmation then applied a blanket
  `to_lowercase()` on top of it. That is a no-op on the chains whose rule is
  already `Lowercase`, and wrong on every chain whose rule is `None` — "case
  and shape are significant", as the enum puts it.
- On Bitcoin and Solana two base58 strings differing only in case are two
  different addresses, so the fold let a lookalike of an address in the book
  pass as one already seen and the `new_address` warning — the one that catches
  a swapped destination — did not fire. It compares normalized forms now and
  nothing more. CLI check:
  `cargo test -p spectra_core validating_and_normalising_cannot_disagree`.
- **The self-send confirmation keeps folding case**, and the comment there now
  says why rather than leaving it looking like the same oversight. A wrong
  answer points the other way: a false match adds a prompt the user dismisses,
  a miss removes one they should have seen. Bech32 is case-insensitive by
  definition, so an own address typed in caps is the same address, and
  `Bitcoin`'s `AddressNormalization::None` — correct for its base58 forms —
  cannot express that. Two comparisons of one thing, kept apart on purpose.

### A keypool index names a derivable child key

- Keypool indices are `i64` in SQLite and `i32` across the FFI, and the two
  were bridged with `as` — a narrowing that turns anything past `i32::MAX`
  into a *negative* index, which derivation would then take as a path nobody
  asked for.
- The ceiling is BIP-32's own: the last non-hardened child, which is exactly
  `i32::MAX`. It is enforced where an index is produced — reserving a receive
  or change index, and advancing past a used one, all refuse at the end of the
  keypool rather than rolling over — which is what makes the narrowing
  lossless. The narrowing itself clamps rather than wraps, so a future
  regression fails at an exhausted keypool instead of a negative path.
  CLI check: `cargo test -p spectra_core the_keypool_stays_inside`.

### An unreadable balance is not an empty wallet

- `summary_display_balance` answered `0.0` for an unknown chain and for an
  amount it could not parse, and the simple-chain send preview parsed its fee
  the same way. Its caller subtracts the fee from the balance and offers the
  difference as the send sheet's maximum, so an unread balance showed the
  holder an empty wallet and an unread fee made the whole balance look
  sendable. Both return errors now. A real zero still reads as zero. This is
  the same rule already applied to the EVM and Tron previews and to the
  balance-refresh engine; these were the instances left. CLI check:
  `cargo test -p spectra_core an_unreadable_balance_is_an_error`.

### Two smaller collapses

- **One validator result helper:** `make_result` and `make_string_result` had
  identical bodies and 33 call sites between them. One remains.
- **Aptos identifiers are copied, not re-encoded:** the normalizer walked the
  string by byte and pushed each one `as char`, which is a Latin-1 conversion
  that silently rewrote any multi-byte sequence reaching it. An ASCII guard now
  covers the whole function — an identifier is ASCII, and that is also what
  makes the byte indexing below it sound — and the copy takes a `&str` slice
  rather than casting, so the same mistake is a compile error next time.


### Manual transaction status recheck is a core operation

- Before: Swift inspected its transaction projection, reset a tracker using a
  caller-selected finality flag, then ran automatic polling for the whole chain.
  Failed transactions and confirmed non-finality-tracking transactions were
  excluded by that sweep, so their recheck could make no network request. Read
  errors were swallowed and the UI reported the old status as a successful check.
- After: `recheck_transaction_status` takes only the stored transaction ID,
  validates the registry's UTXO/kind capability and a 32-byte hexadecimal hash,
  then reads that transaction on its recorded network regardless of prior status
  or automatic backoff. CLI `txs --recheck <id>` and Swift use this operation.
  The low-level `reset_status_tracker` export and Swift orchestration are removed.
- Only the requested row changes. The write re-reads inside a SQLite transaction:
  deletion or a changed hash/wallet/kind refuses the stale result; concurrent
  metadata edits and indexed timestamps survive. Provider/decode failures and
  mismatched response hashes report errors and preserve saved status and trackers.
- A successful unconfirmed read clears old block/finality/confirmed-fee data and
  resumes pending tracking. Failed-to-confirmed reads clear the old failure reason.
  Confirmation counts and block heights are checked before narrowing integer types;
  absent depth on a depth-tracking chain does not imply finality.
- Rationale: explicit recheck must actually query the requested transaction,
  never turn an unavailable provider into a status claim or resurrect a deleted row.
  The old tracker-reset unit test is replaced by end-to-end recheck/reorg coverage.
- CLI checks: `spectra --json txs --recheck <id> [--endpoint <read-url>]`;
  `python3 scripts/cli-transaction-recheck.py target/debug/spectra` exercises
  failed/confirmed/pending transitions, exact request scope and failed reads across
  CLI processes with a local fixture. `cargo test -p spectra_core --lib explicit_recheck`
  also covers invalid scope, finality reset and concurrent deletion/identity/metadata
  changes. Both are included in offline acceptance.

### Core runs the whole pending-transaction maintenance sweep

- Before: Swift requested the stored network list, filtered it through its UI
  refresh descriptors, pruned trackers, then launched one poll per descriptor.
  A stored network absent from that UI directory was skipped; an empty scope
  returned before pruning trackers for deleted or finalized transactions.
- After: `WalletService::refresh_pending_transactions` derives the scope from
  stored records, prunes even an empty sweep, polls the exact recorded networks
  concurrently and returns the scope, committed changes and per-chain service
  failures. Existing provider-failure backoff remains core-owned. A storage
  failure is reported rather than treated as an empty wallet.
- Swift adopts the result and handles notifications, history-view refreshes and
  localized errors. Its in-flight guard is set before the first suspension.
  The scope query remains a plain Rust method for CLI inspection; tracker
  pruning is internal. The descriptor's pending-only callback is removed.
  Reset no longer races an explicit prune against its detached wallet deletes;
  orphan in-memory trackers are pruned from committed history on the next sweep.
- Rationale: the UI's chain directory must not decide which stored transactions
  receive maintenance, and one failed chain must not discard other committed
  results. No signing or broadcasting occurs in this operation.
- CLI checks: `spectra --json txs --refresh-pending`; `spectra txs --maintenance`
  remains the read-only scope view. `cargo test -p spectra_core --lib
  owned_pending_maintenance` covers mixed mainnet/testnet scope, independent
  backoff, persisted confirmation, empty-scope cleanup and storage refusal;
  these regressions and CLI argument checks run in offline acceptance.

### Remaining known-open verification

- Before: Bitcoin single-address preview and status dispatched only mainnet.
  After: the network supplies endpoints and its mainnet family selects the
  protocol implementation. Testnet4 is tested through both service operations.
- Before: derivation editing accepted indices at/above `2^31`, colliding with
  the separate hardened flag. After: parsing rejects those indices early.
- Before: negative/non-finite portfolio observations or thresholds could produce
  misleading movement alerts. After: invalid inputs suppress the alert.
- Before: portfolio composition joined keys with a delimiter that could also
  appear inside a key. After: sorted keys use JSON string boundaries, retaining
  order independence without collisions between differently grouped keys.
- Before: Base history required an Etherscan key after a client-specific 403.
  After: it uses the documented public Blockscout API, verified with three
  standard HTTP reads and Spectra's own successful `history` query.
- CLI checks: `cargo test -p spectra_core --lib app_boundary_tests` (included in
  acceptance), the existing typed Tron fixtures, and `spectra history Base-probe`
  using a temporary public zero-address watch wallet. Funded Sepolia proof is
  recorded separately and is never repeated by the offline gate.


### Send reliability audit: nonce, journal, network, history and SPL

- Before: EVM read `latest` nonce and concurrent submissions could reuse it.
  After: read `pending`, serialize each database/network/sender submission, and
  reserve nonces from durable pending sends, including after a restart. An
  atomic SQLite check also refuses a stale nonce selected by another process
  before broadcasting. Explicit nonce overrides remain the intentional
  replacement path.
- Before: a pending placeholder preceded signing, but recoverable bytes were
  saved only after the node replied. After: each protocol awaits the send's
  durable journal after signing and before submission; signing failures leave
  no row. Response loss retains the exact submission input, locally available
  identifier and EVM nonce. Rebroadcast submits that input on the recorded
  network. Missing receipts cannot expire a journaled send into a guessed failure.
  Monero now prepares with `do_not_relay`/`get_tx_metadata`, then journals and
  calls `relay_tx`. ICP, Kaspa and the remaining protocol inputs are retained
  in the same submission envelope, without wallet signing keys or re-signing.
- Before: pending polls remapped the recorded network through current settings.
  After: core and CLI poll that exact network, including Sepolia when the
  current selection is mainnet.
- Before: history merged a snapshot outside its write transaction. After:
  SQLite `BEGIN IMMEDIATE` covers read/merge/write, so concurrent refreshes
  share one identity per wallet without collapsing separate wallet records.
- Before: SPL sends trusted caller precision and always used the legacy Token
  Program. After: validate the mint owner and precision, derive ATAs and build
  instructions for that program. Basic Token-2022 transfers work; mints with
  extensions are explicitly refused before signing until their transfer
  semantics and extra accounts are supported.
- CLI checks: `spectra send broadcast --from <wallet> --to <address> --amount
  <amount> --yes`; `spectra send rebroadcast <transaction-id> --yes`;
  `spectra txs --maintenance` and `spectra txs --poll-chain ethereum-sepolia`.
  Deterministic loopback/SQLite regressions: `cargo test -p spectra_core --lib
  audit_fix5` and `cargo test -p spectra_core --lib
  audit_stored_wallets_reach_solana_sui_aptos_and_tron_submission`, run by
  `./scripts/cli-acceptance.sh`. No files or directories were moved.

### Known-open endpoint correctness

- Before: Internet Computer's Rosetta `/network/list` received GET and appeared
  dead. After: core selects POST with metadata and requires a nonempty network
  list. Rationale: the health check must use the provider's protocol.
- Before: an explicit probe equal to its endpoint (Bitcoin SV chain info) was
  skipped. After: explicit probes are honored even when URLs are equal.
- Before: failed REST checks hid HTTP status behind a generic failure. After:
  diagnostics report the method and status, distinguishing a 403 denial from
  an unchecked endpoint. A denied provider remains unavailable.
- CLI check: `spectra endpoints --chain icp`, `spectra endpoints --chain
  'Bitcoin SV'`, `spectra endpoints --chain zcash` (live, read-only); offline
  regression: `cargo test -p spectra_core --lib http_probe_regressions`, also
  included in CLI acceptance. Independent derivation vectors use pinned
  reference-wallet tests, never this implementation's output as the oracle.


### Six follow-up ownership slices

- **Import:** before, Swift committed wallets then wrote secrets; CLI sealed
  separately. After, core requires signing material, derives addresses, mints
  IDs, writes secrets and commits the wallet batch together. Failure cleans
  partial secrets and reports cleanup errors. Rationale: do not store claimed
  signing addresses or partially imported wallets. A crash before the SQLite
  commit can leave an orphan secret because SQLite and Keychain cannot share
  a transaction. CLI: `wallet import`, `wallet create`, and failure/retry
  fixtures in `scripts/cli-stage3-followup.sh`.
- **Wallet edits:** before, rename/inclusion wrote a whole stale projection.
  After, field intents change only the named field on an existing ID.
  Rationale: preserve refreshed holdings and concurrent edits; never resurrect
  deleted wallets. CLI: `wallet rename`, `wallet inclusion`.
- **Quotes:** before, Swift selected requests, merged prices, saved a JSON cache
  and timed fiat retries. After, core owns requests, valid-value merging, cache,
  attempt/success timestamps and errors. Missing/invalid quotes retain previous
  values; cooldown survives reopening; settings reset retains quotes.
  Rationale: one durable authority. CLI: `price --stored`, `price --refresh`,
  `currency --refresh-rates`.
- **Maintenance:** before, Swift hardcoded Dogecoin and the scheduler retained
  every confirmed send. After, both use stored history and the registry's
  polling/finality rule. Rationale: confirmed EVM sends need no more polling;
  depth-tracking chains still do. CLI: `txs --maintenance`,
  `diagnostics maintenance --conditions <json>`.
- **Preview:** before, Swift assembled EVM calldata and requested mainnet fees;
  UTXO/Dogecoin quoting required revealing the seed. After, core resolves the
  stored wallet/asset, network, destination and exact decimal amount; quoting
  needs no seed. Rationale: preview and execution must agree without exposing
  signing material. CLI: `send preview`, `send review`; the local Sepolia
  fixture checks exact 1.1 ETH and invalid-input refusal before network access.
- **Boundary:** before, low-level quote/secret/assembly hops remained exported.
  After, seven become Rust-only and four owned operations replace them. Dead
  Swift wrappers are removed; protocol-specific types and callbacks remain.
  Rationale: remove orchestration rather than combine unrelated operations.
  CLI: export scripts and follow-up acceptance; coverage map linked above.


### Complete the ten Stage 3 / C2 ownership slices

- **Balance refresh:** the engine formerly returned a one-holding summary and
  Swift decided how to merge it, queried tracked tokens separately, and wrote
  whole wallets back. Core now resolves the stored wallet/network and tracked
  contracts, fetches both kinds of balance, merges with chain-specific contract
  identity, and persists before notifying the observer. Invalid amounts refuse;
  unreadable tokens keep their old balance, while a legitimate zero replaces it.
  A result for a deleted wallet or changed address/network cannot overwrite the
  new state. Solana/Tron base58, TON base64 and Move module/type names keep
  their case; only hex address components are case-normalized, on testnets too.
  Swift only coalesces projection reads. CLI: `spectra refresh
  --wallet <id>`; `--endpoint` provides an explicit provider for a selected
  wallet. `scripts/cli-balance-refresh.py target/debug/spectra` proves the real
  CLI persists ETH/USDC and keeps the token balance after a malformed read.
- **Deletion and network changes:** wallet removal now deletes its history,
  addresses and keypool in the same SQLite transaction as the wallet. Registered
  secrets are deleted idempotently first; a secret-backend failure leaves the
  wallet present and reports an error. A database failure after secret deletion
  may leave a wallet without signing material, never a false deletion success;
  retrying finishes cleanup. A signing wallet in a bound store requires its
  SecretStore. The platform cannot transactionally roll back Keychain together
  with SQLite. Network selection now clears every member of the changed
  family's derivation tables inside the settings transaction, including the
  in-memory projections after commit. Re-selecting the same network is a no-op
  and no longer clears reservations. CLI: `wallet delete ... --yes`, `network
  set ...`; the `service::balance_refresh::lifecycle_tests` fixtures inject a
  cleanup failure and prove settings roll back, then retry and reopen.
- **Outgoing records:** only Swift used to create a pending transaction after
  `execute_send` returned. Core now writes an uncertain-submission record before
  submitting, and saves the resulting hash, actual network, nonce and opaque
  signed payload before returning. The work survives caller cancellation.
  Sign-only requests do not create pending transactions. An ambiguous failure
  remains explicitly uncertain, so a timeout is not mistaken for proof that
  nothing was submitted. Swift reads the committed record for its last-send
  UI and notification permission. Preview-only fee/change guesses are no longer
  copied into stored transaction facts. CLI: `send broadcast`; the
  `audit_stored_wallets_reach_solana_sui_aptos_and_tron_submission` fixture
  verifies records survive reopening after mock submissions, without funds.
- **Rebroadcast:** Swift used to inspect payload formats, select a low-level
  broadcaster, and update a whole transaction. Some unsupported formats even
  returned deferred success without sending. `rebroadcast_transaction(id)` now
  reads the stored chain and payload, refuses missing/competing formats and
  confirmed records, requires a nonempty node transaction ID, and updates the
  same record. Submission updates merge into the stored row and cannot revert
  a receipt that confirmed it while the request was in flight. Errors cannot
  become fabricated success. CLI: `send rebroadcast
  <transaction-id> --yes`; `service::send_records::tests` exercises a mock
  Sepolia provider, missing ID and confirmed-record refusal.
- **Funds Finder:** separate Swift and CLI loops chose concurrency and classified
  balances; Swift silently dropped failures and CLI used floating point to find
  nonzero amounts. A core scan session owns candidates, batches of four and
  progress. Its results distinguish funded, zero and failed reads using exact
  smallest-unit digits. Cancelling a batch does not advance its cursor. The UI
  only adopts progress/hits/errors. CLI: `rescan` and `rescan --dry-run`;
  `service::funds_scan` checks all three outcomes against a local provider.
- **Durable diagnostics:** Swift's log and sync-state JSON blobs and core's
  separate per-chain event store are replaced directly by one core-owned typed
  diagnostic state. Intents append, mark a chain healthy/degraded/synced, or clear
  logs. Core stamps, trims and caps the global log at 800; the per-chain view
  exposes its most recent 200. Recovery and last-success decisions live there.
  The app no longer writes both copies of a chain event. Swift localizes the
  projection and queues commands; persistence failures remain available as
  `persistenceError`. CLI: `diagnostics state [--command <typed-JSON-intent>]`;
  the offline gate changes state in separate processes and proves it survives.
  Generic JSON storage remains only for the existing platform preferences and
  live-price cache, outside this diagnostics slice. The six-item follow-up
  subsequently moved that cache into typed core state; only platform
  preferences retain generic blob storage.
- **Derived wallet state and startup:** `wallet_derived_state()` reads signing
  availability through its own SecretStore, removing the per-wallet Swift
  queries and two caller-built ID lists. CLI: `wallet derived`. The duplicate
  Swift token catalog builder is removed; Known Tokens shows a loading indicator
  until core's seeded catalog arrives.
- **Endpoint diagnostics and FFI:** iOS now calls `probe_chain_endpoints` like
  `spectra endpoints`, with configured endpoints using the catalog's protocol
  probe. Unprobeable rows explicitly say they were not checked. Swift no longer
  chooses HTTP versus JSON-RPC or orchestrates endpoint loops. Low-level HTTP,
  token fetch and payload-preparation helpers remain Rust internals where used;
  obsolete per-wallet/per-chain cleanup methods are deleted. Protocol-specific
  send input types are retained. Reproduce the surface with
  `scripts/count-exports.sh`; no arbitrary export-count target is an acceptance
  criterion. `scripts/cli-stage3.sh` is included in CLI acceptance.


### Wiki headers use the original coin artwork once

- Before: coin and chain detail pages placed a continuously rotating imitation
  coin above a second small badge. Metallic rings, ridges, glare and orbit lines
  surrounded artwork that already included its own background.
- After: one 52pt original coin badge sits beside the name and symbol. The
  rotating decoration, drag interaction and its rendering helpers are removed;
  list-row artwork and core's artwork selection remain unchanged. This makes
  the identity readable without duplicated symbols or decorative animation.
- Presentation-only change, with no domain logic moved. CLI check:
  `! rg 'WikiRotatingCoin|WikiStampedCoinLogo' swift/views`;
  `scripts/check-design-tokens.sh` checks the surrounding UI tokens.

### Protocol encoders and Bitcoin history pagination (second core audit)

- **Kaspa signing:** the encoder used Bitcoin CompactSize lengths, reversed
  transaction IDs and a hash of empty payload bytes. It now uses Kaspa's u64-LE
  lengths, original transaction-ID bytes and zero native-payload hash. Sender
  keys must match the requested address, zero amounts and arithmetic overflow
  refuse before signing. The native-all-0 digest is checked against rusty-kaspa's
  independent consensus vector. CLI check: `cargo test -p spectra_core kaspa_official_native_sighash_vector`.
- **Solana account aliases:** SOL/SPL self-transfers emitted duplicate account
  keys and were rejected by account locking. A shared local message compiler
  merges identities and writable privileges, then remaps instruction indices.
  CLI check: `cargo test -p spectra_core audit_solana_self_transfers` (the usual
  SDK vectors still exercise distinct-recipient transfers).
- **Bitcoin pagination:** single-address continuation ignored its saved cursor;
  HD aggregation truncated results and marked them exhausted. Both now use one
  buffered `HistoryPage<T>` pager. Each source retains its Esplora continuation
  and unread rows; complete block cohorts merge across addresses before UI
  pagination, with integer satoshi accounting. A failed read or persistence
  write cannot advance the cursor. Exactly full provider pages are probed for
  continuation; repeated cursors refuse instead of looping. CLI check:
  `python3 scripts/cli-bitcoin-history.py target/debug/spectra` uses only a local
  fixture and a temporary directory, and proves 61 persisted transactions over
  seven UI pages and three provider pages. It is part of CLI acceptance.
- **Bitcoin HD network and script:** history used mainnet endpoints, mainnet
  addresses and the canonical xpub's BIP44 interpretation even for other
  derivation purposes. History now carries the selected network through address
  derivation and HTTP reads; mnemonic wallets use their recorded address path's
  BIP44/49/84/86 script, while imported extended keys retain their prefix's script
  type. The scan window remains 20 receive and 10 change addresses. CLI check:
  `cargo test -p spectra_core stored_testnet_hd_history` and
  `cargo test -p spectra_core hd_history_addresses_match_individual_derivation`.
- **CLI history:** `history --save` for Bitcoin now uses the same HD-aware core
  operation as iOS; `--pages N` drives continuation within the process and
  `--endpoint URL` permits an explicit history provider or loopback fixture.
  Reads resolve the wallet's active network instead of assuming mainnet.

- **Contact projection ordering (found by iOS verification):** fire-and-forget
  address-book writes could race each other or a launch read, leaving deleted
  contacts visible after core had removed them. Swift now forwards these intents
  in order and invalidates older reads at commit completion. Core remains the
  only domain store. The regression queues three additions/deletions, then
  attempts to adopt the stale pre-delete snapshot; iOS check:
  `testQueuedContactWritesCannotBeUndoneByAnOlderRead`. Existing CLI address-book
  persistence checks continue to cover the core commands.

The refactor scopes Bitcoin wire helpers to `bitcoin_wire`, separates network
balances/tokens/history/HD/prices, isolates Bitcoin history orchestration, and
splits SQLite CRUD and store tests by domain. Connection ownership and atomic
cross-table writes remain shared. No storage migration or compatibility shim
was added.



### Audited signing boundaries, fresh recipients and service ownership

- **Aptos receiving:** mnemonic addresses used Keccak-256(public key || 0),
  which does not identify the Ed25519 account. Derivation now uses SHA3-256,
  matching the official Aptos SDK. No compatibility path preserves the wrong
  address. Existing incorrectly derived sender records are refused by the
  identity check; reimport the mnemonic to derive its correct address.
- **Tron signing:** CreateTransaction/TriggerSmartContract responses supplied
  both the transaction and the hash that core blindly signed. Core now reads
  only a block reference and locally encodes the one requested TRX transfer or
  TRC-20 transfer call, hashes it, checks the signing key's owner and signs.
  Node-supplied contracts, destinations, amounts and txIDs never enter signing.
  Positive int64 amounts/fees, coherent block references and fixed one-minute
  expiry are enforced. The old trigger simulation is removed; token execution
  can still fail on chain and a successful broadcast is not confirmation.
- **Solana/Sui/Aptos seeds:** mnemonic derivation returns a 32-byte seed, while
  these sending branches demanded a 64-byte keypair and failed before signing.
  They now accept only the explicit Ed25519Seed type, derive the verifying key
  and refuse a competing sender. Secret-bearing internal params no longer
  deserialize JSON or derive Clone; their hex strings zeroize and Debug is
  redacted. Decoded signing buffers are zeroizing too. The unused Stellar token
  parameter placeholder and legacy number/string deserializers are deleted.
- **Sui receiving:** the independent SDK fixture also exposed Keccak-256
  address derivation. This is now Blake2b-256(flag || public key), matching the
  signer and official SDK. Reimporting derives the correct address; stale sender
  records fail closed. The acceptance gate checks this receiving address too.
- **Sui transaction correctness:** SHA-256 intent signatures are replaced with
  Blake2b-256. The malformed unsafe_transferSui request and blind signing of
  returned bytes are replaced by a local BCS programmable transfer: split the
  exact MIST amount from gas and transfer that result to the requested owner.
  Paginated coin references fund amount plus gas budget, up to 256 gas objects;
  duplicates, overflow and insufficient funding refuse. Failed execution effects
  and absent transaction digests no longer look like success.
- **Aptos transaction construction:** /transactions/encode_submission is no
  longer trusted with signing bytes. A local BCS native APT entry-function
  transfer includes sequence, gas, expiry and chain id. The routed mainnet
  sender refuses a node reporting another network. Broadcast-only functions
  consume signed payloads and require transaction ids.
- **ENS destinations:** the service-lifetime cache is removed outright. Each
  resolution asks the provider again and a failed lookup cannot reuse an older
  address. The review screen captures the resolved address as view state;
  submit calls core's verify_send_destination, which resolves afresh and refuses
  a different address. The user must return to Review before retrying. CLI:
  `spectra send destination --chain Ethereum --to <input> --expected <address>`.
- **Structure:** state persistence retains its single serialized writer;
  keypool, address discovery, transaction tracking, wallet import and operational
  events have sibling service modules. Send previews, recipient resolution,
  signing dispatch and rebroadcast are separate modules, with internal typed
  params and offline protocol builders. The unused Solana ATA existence read
  is deleted: the transaction already creates it idempotently.
- **Independent checks:** `scripts/generate-send-audit-vectors.cjs` pins the
  official Aptos, Sui, Solana and Tron SDKs and generates offline fixtures under
  `core/testdata/`. `cargo test -p spectra_core audit_` compares real mnemonic
  derivation, exact protocol bytes/signatures or decoded SPL instructions, and
  drives stored-wallet execute_send through mock submission. The CLI acceptance
  gate checks the Aptos address and reviewed-destination mismatch without a
  network; Swift tests cover the new async verification binding. These prove
  local encoding and signing, not funded live broadcast or mining.

### Remove the redundant Asset Hub card

- Before: My Assets detail inserted an Asset Hub card with a repeated symbol,
  a holding count labelled Networks, catalog contract count and pinned badge.
  After: the hero leads directly into totals and chain holdings; contract
  information remains available through Details. The card offered no action
  and mixed catalog metadata with the user's holdings, so it and its private
  rendering helpers are deleted rather than redesigned.
- Presentation-only removal; no domain rule moved. CLI check:
  `! rg -n 'AssetDetailHubCard|Asset Hub' swift/views/DashboardViews.swift`.
  The existing core/CLI suites continue to cover asset data and pinning.

### A stored password verifier is input, not a promise

- **Work factor:** `verify` read `rounds` out of the envelope and handed it
  straight to PBKDF2, which does exactly as many iterations as it is told. An
  envelope edited to `1` turned the verifier into a single precomputable HMAC;
  one edited to `u32::MAX` made unlocking run for hours on the calling thread,
  so the app never came back. `rounds` is now bounded by `MINIMUM_ROUNDS` and
  `MAXIMUM_ROUNDS` (twenty times the default), and salt and digest lengths are
  checked the same way — an empty salt is the same weakening by another field.
  The version was already checked; these were not.
- **Two constants, on purpose:** `DEFAULT_ROUNDS` is what `create_verifier`
  writes today, `MINIMUM_ROUNDS` is the floor `verify` will honour. Keeping
  them apart is what lets the cost rise later without locking anyone out of a
  verifier sealed at the old one. CLI check:
  `cargo test -p spectra_core a_stored_envelope_is_not_trusted`.

### A BSV address belongs to one network

- **Validation:** `validate_bsv_address` took no network and accepted all four
  version bytes (`0x00`/`0x05` mainnet, `0x6f`/`0xc4` testnet), while both
  `"bitcoinSV"` and `"bitcoinSVTestnet"` dispatched to it — so the testnet kind
  decided nothing, a mainnet send accepted an `m…`/`n…`/`2…` destination and a
  testnet send accepted a `1…`/`3…` one. It takes the network now, the way
  Litecoin, Dash, Decred and Zcash already did in the same file.
- **One table:** a `BsvNetwork` enum owns the version bytes, and
  `BSV_MAINNET_VERSION` / `BSV_TESTNET_VERSION` derive from it, so the
  validator, the decoder and the deriver cannot disagree about which byte
  belongs where.
- **Signing:** `decode_bsv_address` returned only the hash, discarding the
  version byte it had just checked — which is why the bug was possible. It
  returns the network alongside, and the signer refuses a transaction whose
  destination and change name different networks. CLI check:
  `cargo test -p spectra_core a_bsv_address_belongs_to_one_network`.

### The high-risk send check asks the address question the same way

- `is_valid_send_address` validates the *normalized* address, with a comment
  explaining why: on Sui a 64-hex address typed without its `0x` is invalid raw
  and valid once `LowercaseHexPrefixed` has added the prefix. The high-risk
  evaluator kept its own copy of that question and validated the raw string, so
  the composer accepted such an address, the store accepted it, and the warning
  sheet called it `invalid_format` at the same time. It calls
  `is_valid_send_address` now — one question, one form, one answer, and the
  local `hrsr_validate` duplicate is gone. CLI check:
  `cargo test -p spectra_core validating_and_normalising_cannot_disagree`.

### Integer overflow panics in release instead of wrapping

- Rust checks overflow in debug and wraps in release, so every arithmetic bug
  in this workspace was invisible to the suites — which run in debug — and
  silent in the build that ships. For satoshis, wei and planck a wrapped total
  is a wallet that believes it can afford a spend; the crash is a bug report
  and the wrap is a wrong transaction.
- Set per package (`spectra_core`, `spectra_core_ffi`, `spectra-cli`) rather
  than on `[profile.release]`, which would apply it to the whole dependency
  graph. The crypto crates below us do deliberate wrapping arithmetic, and
  whether every one of them spells it `wrapping_add` is not a property this
  workspace should bet a panic on. CLI check:
  `cargo test --release -p spectra_core`.

### Send composer and everyday navigation

- **Amount shortcuts:** Swift's `balance * fraction` is replaced by core's
  fee-adjusted preview maximum, quantized down with integer arithmetic. An
  unready or unsupported asset quote offers no shortcut. The UI calls it an
  estimated maximum: existing preview balances are floating-point estimates,
  so core reserves one representable step before flooring to asset precision;
  fees may change before submission. Native gas quotes cannot populate token
  amount fields (currently only EVM/TRC-20 previews quote tokens). Custom EVM
  fees now recompute the native maximum from the original balance, leaving
  token balances in token units; lowering a fee never reconstructs a balance
  from a previously zero-clamped spendable quote.
  Before typing, a provisional 10% amount loads a fee quote without changing
  the field. Checks: `spectra send shortcut --maximum 0.99999 --decimals 8`,
  `cargo test -p spectra_core shortcut_tests`; the CLI acceptance gate exercises
  MAX, percentage rounding and refusal. `parse_amount_input` now uses the same
  bounded exact parser as `spectra send amount`, refusing integer overflow.
- **Send UX:** five composer pages become four; Review shows the fee and offers
  fee/advanced settings in a disclosure. Recipient resolution runs before Next
  using core's destination service (including ENS), and invalid amounts receive
  inline feedback. Input changes invalidate shortcut readiness until the new
  preview finishes. CLI: `spectra send destination --help` drives the same
  destination resolver; offline address refusal remains in acceptance.
- **Layout and navigation:** send/receive bottom controls use safe-area insets,
  amount entry uses system Dynamic Type and a keyboard Done action, and progress
  is a readable step count. Home has a visible Assets/Wallets picker and an
  Add Wallet empty-state action. Receive emphasizes the network and wraps the
  complete address; send review also displays its complete recipient.
- **History:** replace ten-row page switching with cumulative batches of twenty
  and Load more, retain earlier rows, offer history-read Retry and visible
  per-transaction Recheck. Fetch eligibility follows selected wallets even when
  filters have no matches. These are presentation changes; core history and
  pagination persistence remain exercised by `spectra txs` and the existing
  offline acceptance checks. Verify layouts, keyboard, larger text and history
  interactions in the iOS simulator in addition to the three required suites.

### TON/NEAR wire correctness and pending-poll integrity

- **TON sends:** flat byte concatenation with an invalid BoC header is replaced
  by ordinary TON cells, representation-hash signatures, a complete external
  message addressed to the derived V4R2 wallet, and StateInit for seqno zero.
  Remarks are encoded as text-comment cells (including snake continuations),
  rather than discarded. A single cell implementation now also computes the
  derivation StateInit, keeping the receiving and signing identities identical.
  Explicitly supported: workchain-zero V4R2 sender, fixed-value mode 3, comments
  up to 4096 UTF-8 bytes; other modes and longer comments are refused. Raw
  destinations default to non-bounceable, friendly destinations retain bounce
  flags. Sender key mismatch and mainnet test-only recipients are refused.
- **TON submission prerequisites:** the derived 32-byte signing seed is consumed
  as 32 bytes, not rejected by a 64-byte decoder. A successful uninitialized
  account read alone selects seqno zero; active accounts read the numeric stack
  from runGetMethod, and failed reads no longer become zero. Submission uses
  sendBocReturnHash and requires a successful envelope and 32-byte message hash,
  rather than treating an error or absent hash as success.
- **NEAR wire format:** public-key and signature tags are one-byte Borsh enum
  discriminants instead of four-byte integers. Native and FunctionCall builders
  consume the actual 32-byte derived seed and verify its public key before
  signing. CLI checks: `cargo test -p spectra_core protocol_tests`. Fixtures
  for both chains come from the pinned official SDKs, independently of Rust;
  `scripts/generate-protocol-vectors.cjs` documents regeneration. These checks
  prove encoding and signing, not live acceptance or mining.
- **TON addresses:** the shared parser now verifies checksum, flags and
  workchain, supports standard and URL-safe base64, and rejects test-only
  recipients on mainnet. Previously any 48-character alphanumeric string was
  accepted and the send decoder ignored its checksum. Validation, persistence
  and sends now use the checked parser. CLI: `spectra address validate --chain
  TON <address>`; the offline acceptance gate covers typo and network refusals.
- **NEAR token amounts:** contract ft_metadata decimals replace caller-supplied
  precision, using the reader already present in core. A missing, malformed or
  failed metadata read refuses the send even if caller decimals exist. This
  prevents a typed amount from being scaled into a different quantity. CLI:
  `cargo test -p spectra_core build_send_params_tests` includes conflicting
  precision and provider-failure checks.
- **Pending polling:** pruning now shares the same tracked-record predicate as
  polling, retaining EVM/history trackers and their failure backoff as well as
  UTXO trackers. History polls only fetch addresses with due records. Previously
  every prune erased non-UTXO schedules. Both storage reads now propagate errors
  instead of returning a successful empty result; a corrupt read leaves trackers
  and records intact. CLI: `cargo test -p spectra_core pending_status::tests`;
  `spectra txs --poll-chain Ethereum` drives the operation, and the offline
  corruption acceptance check verifies refusal before any network request.
- Swift bridge regressions cover TON destination refusal and polling an unopened
  store. No FFI record or exported method signature changes.


### One typed amount, one conversion into integer units

- **What was wrong:** core had two decimal→units conversions. `execute_send`
  takes the typed string and shifts it exactly with `parse_raw_amount`, so the
  amount a transaction is *signed* for has always been correct. The EVM
  assembler took an `f64` and shifted that, with a doc comment claiming to
  "avoid float rounding by doing string arithmetic" — but the rounding had
  already happened in the caller's `f64`, and `format!("{:.18}", …)` then wrote
  it out in full. `1.1` assembled 1100000000000000089 wei, `0.1`
  100000000000000006. An `f64` also cannot represent an 18-decimal amount at
  all: it runs out of significant digits around the sixteenth.
- **What that reached:** the assembler feeds the send *preview* — the gas
  estimate, and an ERC-20 transfer's calldata — and `spectra send assemble`,
  the command whose stated purpose is printing the transaction a send would
  sign. So the preview priced a transaction differing from the one made, and
  the CLI printed a wei figure `spectra send broadcast` would not use. It never
  reached a signature.
- **What changed:** `EvmSendAssemblyInput.amount` is a `String`, the assembler
  calls `parse_raw_amount`, and `amount_to_smallest_unit` / `u128_str_to_hex`
  are gone. Swift's send sheet passes the typed `sendAmount` through; its
  `Double(sendAmount)` now only decides *whether* to preview (the zero-amount
  rule), never what is assembled. The CLI passes `--amount` through untouched.
- **Stricter, on purpose:** the assembler now refuses whatever the signing path
  refuses. `1e3` previously assembled as a thousand and over-precision was
  truncated by `format!`, so a preview could succeed for an amount
  `execute_send` would then reject; both are refused at the preview now.
- **Not changed:** the `f64` money fields on the *display* side — balances,
  fees, spendable, max-sendable — stay as they are. Those are numbers to render,
  not to commit to a transaction, and moving all of them is a separate change.
  CLI checks: `cargo test -p spectra_core one_amount_one_conversion`, and
  `spectra send assemble --chain ethereum --amount 1.1 …` prints
  1100000000000000000.

### A Bitcoin transaction pays out no more than it spends

- **Batch sends:** `build_unsigned_spend` sized the fee for `extra_outputs`
  but funded none of them. Coin selection targeted the recipient amount alone,
  the extras were appended after change was computed, and `saturating_sub`
  turned the resulting shortfall into a change of zero rather than a refusal.
  A signed batch therefore paid out more than it spent — 200_000 sats in and
  204_980 out in the case the tests pinned — and the node rejected it with
  `bad-txns-in-belowout`. Extras are now built before selection, their values
  are part of the target on both the selected and the pinned branch, and the
  change subtraction is `checked_sub`. Nothing in the app reaches this yet:
  every production caller passes `extra_outputs: vec![]`, so this is a latent
  path made correct rather than a live bug fixed.
- **The invariant, stated:** the builder now refuses if the outputs it laid out
  exceed the inputs it selected, on every path — pinned or selected, change
  kept or absorbed. It cannot fire given the arithmetic above, which is the
  point: it is what the next change to that arithmetic has to keep true.
  UTXO value sums are `checked_add` for the same reason, since release builds
  wrap and the values come from an endpoint.
- **The test that pinned the bug:** `extra_outputs_are_appended_after_change`
  asserted the unfunded change figure and passed, because it read output
  values and never asked whether the inputs covered them. It is now
  `extra_outputs_are_funded_and_not_conjured` and asserts outputs plus fee
  equal the inputs. CLI checks: `cargo test -p spectra_core
  a_batch_the_inputs_cannot_cover_is_refused` and
  `cargo test -p spectra_core no_layout_pays_out_more_than_it_spends`.

### A provider's timestamp is parsed as the untrusted string it is

- **Char boundaries:** `parse_iso8601_timestamp` indexed a `&str` by byte
  offset at 4, 7, 10, 13 and 16 after checking only that the string was
  nineteen *bytes* long. Any non-ASCII character in a history response —
  full-width digits, a stray ellipsis, an Arabic-Indic numeral — split a
  multi-byte character and panicked inside core. It now refuses a non-ASCII
  string up front, which is also what makes every index below it sound.
- **Zone offsets:** it read the wall clock and dropped the offset its own
  comment claimed to handle, so a `+08:00` stamp was filed eight hours early.
  `Z`, an absent suffix, `±HH:MM`, `±HHMM` and `±HH` are now all read, and
  fractional seconds are skipped rather than confusing the zone.
- **Refusal instead of 1970:** it answered `0.0` for everything it could not
  read, which is a real date that sorts and renders as one. It returns
  `Option` now. The history caller still files an unreadable stamp as `0.0`
  — a transaction shown with a wrong date beats one the history omits — but
  that is now the caller's stated choice rather than the parser's silence.
  Field ranges (month, day, hour, minute, second) are checked, so a malformed
  field can no longer roll the date somewhere plausible.
- **The divisor:** a string timestamp parses straight to seconds, so the
  shape's `time_divisor` never applied to it; the call site applied it anyway.
  No shipped chain hit this — the three scaled shapes all read numeric
  `timestamp_ms` / `_us` / `_ns` fields — but a nanosecond chain that answered
  with a string would have dated every row to 1970. The divisor is now on the
  numeric branch only. CLI check: `cargo test -p spectra_core iso8601_tests`.

### A send preview quotes the asset the amount field moves

- **EVM token previews:** `fetch_evm_send_preview` answered `spendable` and
  `max_sendable` with the *native* balance whatever was being sent, so an
  ERC-20 send offered the sender's gas-coin balance as its maximum and the
  send sheet rendered it through the token's own formatter — 1 ETH shown as
  "1 USDC", with "Max" filling in a number the transfer could not move. The
  preview now reads the token's `balanceOf` and `decimals` and quotes those.
  Which case it is comes off the calldata, not off a caller-supplied
  descriptor: an ERC-20 transfer *is* `transfer(address,uint256)` addressed to
  the token contract, so the selector names the case (`is_erc20_transfer`,
  shared with the assembler that writes it) and nothing on the funds path is
  trusted from a typed value. A native send is unchanged: it pays the fee out
  of the balance it is moving, so its spendable is still `balance - fee`. The
  wire field is `spendable_balance` rather than `spendable_eth`, and the dead
  `balance_eth` it also emitted is gone. No FFI shape changed —
  `EvmSendPreview.spendableBalance` already had that name. CLI check:
  `cargo test -p spectra_core a_preview_quotes_the_asset_it_moves` (mock
  JSON-RPC node, no live chain).
- **TRC-20 previews:** the Tron preview divided every token balance by a fixed
  `1e6`, which is TRX's scale, not the contract's — an 18-decimal TRC-20 was
  quoted at 10^12 times the holding it is, and "Max" offered it. It now reads
  the contract's `decimals()` alongside the balance. It also no longer fetches
  the TRX balance on the token path, where nothing looked at the result.
- **Unread balances on both:** a failed token read became `0.0`, so a network
  failure looked like an empty wallet and quoted a maximum of zero. Both
  previews now fail instead, matching the rule the rest of the send path
  already follows — everything a send decides is computed from this number, so
  it must not be one nobody read. CLI check:
  `cargo test -p spectra_core an_unreadable_trc20_balance_refuses_rather_than_quoting_zero`.
- **Still not checked:** neither preview verifies that the gas coin covers the
  fee for a token send. That needs a second field on `EvmSendPreview` and is
  not in this change.

### Refuse malformed signing data and failed history reads

- **ICP signing and results:** invalid/missing hex became an empty preimage,
  missing payload arrays became empty signature lists, and the returned hash
  was parsed as a numeric block index with zero fallback. Payload arrays must
  now be nonempty, each payload must be valid hex with the IC request domain
  prefix and 32-byte request id, and the signature type must be ECDSA. The
  public key is derived and checked against the supplied key. Submission keeps
  the actual 32-byte transaction hash in `txid`, including across result
  classification; missing/malformed hashes fail. This follows the
  [ICP request signing specification](https://docs.internetcomputer.org/references/ic-interface-spec/https-interface/)
  and [Rosetta construction flow](https://docs.internetcomputer.org/guides/digital-assets/rosetta/).
  CLI check: `cargo test -p spectra_core strict_payload_tests` (local mock
  construction API, signature verification, no live transfer).
- **Input txids:** the shared Bitcoin-family decoder accepted arbitrary hex
  lengths; BCH/BSV/BTG signature preimages and Cardano input encoding also
  swallowed invalid hex. All these paths now reject malformed/non-32-byte
  hashes instead of constructing invalid transactions. CLI checks:
  `cargo test -p spectra_core a_bad_txid_is_refused` and
  `cargo test -p spectra_core cardano_refuses_malformed_input_hashes`.
- **Blockbook amounts:** malformed, negative and overflowing balance/UTXO
  strings previously became zero. A balance read or complete UTXO list now
  fails if any amount is invalid; actual zero and u64 maximum still parse.
  CLI check: `cargo test -p spectra_core strict_amount_tests` uses mock HTTP.
- **Derived history:** normalized history, earliest dates, active-wallet IDs
  and replaceable sends previously hid storage errors as empty lists. All four
  now return Result/throw across UniFFI, including an unopened store. An open,
  empty store remains a successful empty result. Swift keeps its existing
  derived views and shows an error; pruning aborts on failure so a read error
  cannot delete transaction records. CLI `spectra txs --replaceable` fails
  rather than claiming nothing can be replaced. `./scripts/cli-acceptance.sh`
  injects a corrupt row into an isolated database and verifies refusal plus
  unchanged bytes. `cargo test -p spectra_core read_failure_tests` covers all
  four reads; Swift `testUnopenedHistoryReadsThrowAcrossBinding` tests the FFI.
- **Destination activity:** the EVM fallback probe re-fetched balance and
  collapsed history/nonce failures into "unused". Balance and activity now run
  concurrently with one balance read; a positive EVM nonce proves activity
  without an explorer call, otherwise history must answer. A failed read is an
  error/unknown result, not a successful no-history verdict. CLI `spectra send
  probe` propagates the error; Swift displays that activity could not be
  verified. CLI check: `cargo test -p spectra_core destination_probe_tests`
  verifies request counts, provider failures and successful empty history.


Keep entries to the previous behaviour, the new behaviour, the reason and a
CLI check. If a check needs network, a simulator or new coverage, say so.
Completed refactor diaries and old test/line counts do not belong here.
The entries below summarize the retained decisions, not a fresh test run.

### Existing core-state migration: Swift integration

The full iOS gate exposed a stale call to the already-removed
`priceMergeFiatRateUpdates` export. Swift now calls core's existing
`refresh_fiat_rates` operation and mirrors `CoreAppState.fiat_rates_from_usd` on
load. Its duplicate SQLite/UserDefaults rate reads and writes are removed, so
an older platform cache cannot overwrite core's stored rates. CLI check:
`cargo test -p spectra_core stored_fiat_rates_survive_reopening`; the complete
iOS suite verifies the generated `refreshFiatRates` binding compiles. The same
build exposed stale references to the four removed Tor UserDefaults keys; those
reads/deletes are removed too, since `adoptAppSettings` already mirrors all four
values from core. Check `spectra settings list` and `./scripts/cli-acceptance.sh`
for the core-owned settings and reset paths.

### Checked signing, fees and history pages

- **EVM signing gas:** native signing used 21000 without estimating, and ERC-20
  signing swallowed estimate failures into 65000. ERC-20 estimates also preceded
  calldata overrides. Both paths now estimate the final calldata, value, nonce,
  fees and access list; failures and zero/overflowing estimates refuse signing.
  An explicit positive gas limit still skips estimation. The 20% default buffer
  uses checked-width integer arithmetic and rounds upward. CLI check:
  `cargo test -p spectra_core gas_tests` uses local RPC mocks and sign-only mode;
  no transaction is broadcast to a live chain.
- **Fixed-fee UTXO accounting:** BCH, BSV, BTG, DASH, DOGE, LTC, ZEC, DCR and ADA
  summed inputs unchecked and saturated insufficient funds into zero change.
  They now share checked totals, amount-plus-fee addition and subtraction, and
  refuse zero sends. Cardano previously omitted small change without adjusting
  its explicit fee, creating an unbalanced body. It now refuses sub-minimum
  change; exact minimum change is included. Bitcoin-family dust continues to
  become implicit fee. CLI check: `cargo test -p spectra_core accounting`.
- **Fee conversion:** DOGE fee rates, ADA fees and Sui budgets used a rounding,
  saturating float-to-u64 cast. Native fees now convert the shortest decimal
  representation exactly, rejecting nonpositive/nonfinite, excess precision and
  out-of-range input before signing identity reads. DOGE's existing 350-byte
  estimate uses integer arithmetic and rounds up to a whole unit. CLI checks:
  `spectra send fee-units --chain Cardano --amount 0.17` returns 170000;
  negative, NaN, excess-precision and overflowing inputs are covered by
  `./scripts/cli-acceptance.sh`; `cargo test -p spectra_core invalid_fees` checks
  the actual execute-send entry point before keys/network.
- **EVM history:** native history always fetched page 1 / 50 while token history
  used the requested page. Both now use the requested page and size (minimum
  page 1, size 1–500), and either provider failure fails the operation rather than
  returning an empty/partial success. An empty tracked-token list skips the token
  request as documented. CLI check: `cargo test -p spectra_core history_page`
  exercises distinct pages and provider errors with an offline HTTP server.
- **Zcash expiry:** a failed tip read became height 0 and a transaction expiring
  at 40. The read now propagates errors and tip + 40 must fit u32 before any
  signing/broadcast. CLI check: `cargo test -p spectra_core expiry_tests` covers
  missing height and both addition/conversion overflow boundaries.

### Failed reads refuse rather than fabricate state

- **Token sends:** EVM/Tron metadata failures previously fell back to caller
  decimals. These failures now abort parameter construction; caller precision is
  used only for families without a metadata reader. A successful on-chain read
  still overrides the caller. Check `cargo test -p spectra_core failed_metadata`
  and `exact_amounts_reach_native_and_token_signing_params` (local mock RPC).
  Live `spectra send broadcast` needs network and a funded test wallet; no live
  transaction is part of the offline gate.
- **Token integers:** TRC-20 returned only the low u128 bits of uint256 and cast
  decimals to u8. ABI integers now require one complete hex word, reject nonzero
  high bits and malformed data, and reject unsupported precision instead of
  wrapping. EVM/Tron and typed token-balance paths enforce the core's 38-decimal
  scaling limit before formatting; sends already enforce this limit. Check
  `cargo test -p spectra_core integer_tests` (mock RPC and boundary vectors).
- **Keypool failures:** failed history queries were treated as absent history.
  Reads, reservations and conditional advancement now return the error, without
  lowering the floor or publishing a mutation. Out-of-range indices also refuse.
  The UniFFI read now throws; CLI `spectra pool show/next <wallet>` reports failure,
  diagnostics show the error, and receive flows do not substitute old addresses
  after a failed read. Check `cargo test -p spectra_core unreadable_history` and
  the Swift `testInvalidKeypoolBaselineThrowsAcrossAsyncBinding` (offline).
- **Balance/preview failures:** token reads across supported families propagate
  provider errors instead of returning zero/fallback precision. A successfully
  queried empty account set remains zero. SPL reads validate and sum all
  returned accounts for a mint instead of taking only the first. EVM preview
  requires valid amount input plus successful nonce, gas, balance and fee reads;
  no invented 21000 gas or default fee quote.

  What a propagated error is allowed to take with it depends on who asked.
  `fetch_token_balances` first collected its per-token futures into one
  `Result`, so a single self-destructed contract failed every other token in
  the request, and the only caller does `try?` — a wallet's whole token list
  silently stopped refreshing until that contract was removed. A token that
  cannot be read is now left out of the result instead: the caller does not
  update it, so its last known balance stays, which claims nothing. A call
  covering the whole chain rather than one token — TON reads every jetton at
  once — is still an error, and so is a row whose contract is empty, except
  that the row is skipped rather than failing its neighbours. The send path is
  the one that must not read an absence as zero, so `send_destination_risk`
  refuses a token it got no row for rather than treating it as an empty wallet.
  Check `cargo test -p spectra_core failed_reads`,
  `an_unreadable_token_does_not_take_the_readable_ones_with_it`,
  `an_unreadable_token_balance_is_not_an_empty_wallet` and `balance_read_tests`
  (offline/mock RPC). `spectra send probe` needs network; Swift tests check that
  an unavailable EVM preview throws across the binding.
- **Fee history with no sample:** an `eth_feeHistory` whose most recent block
  carries an empty reward list is not a broken response — it is a block nobody
  transacted in, which is the normal state of a quiet L2 or testnet — and
  refusing it left the send screen unable to quote a fee at all there. That
  case reads as a zero priority fee: `max_fee` still covers the base fee, so
  the transaction lands, where a gwei picked to stand in would only overpay. A
  missing `reward` array remains an error, since the request always names
  percentiles and a node that omits it did not answer what was asked. Check
  `cargo test -p spectra_core fee_history_tests`.
- **User metadata:** an unreadable `token_preferences`, `price_alerts` or
  `fiat_rates_from_usd` row is named in the log, dropped, and rebuilt — the
  first two from the catalog on the next evaluation, the third on the next
  refresh. The stored bytes are left exactly as they were, so a build that can
  read the row still will, and no reset or recovery shim is added. Failing the
  load over one of them was tried and reverted: it takes the wallet list down
  with it, and the wallet list cannot be rebuilt from anything, which is the
  whole reason `settings` and the wallet rows *do* stay fatal. Check
  `cargo test -p spectra_core unreadable_rebuildable_metadata_costs_only_that_row`,
  `unreadable_settings_still_fail_the_load` and
  `unreadable_preferences_refuse_loading_without_deleting_wallets`. CLI
  acceptance covers normal persistence and the no-op paths.
- **Tor's proxy goes on before the status says Ready:** `bootstrap_tor`
  published `Running` and applied the SOCKS proxy afterwards. The kill switch
  reads that status, so between the two a request found the switch open and the
  shared client still unproxied, and went to the provider over a direct
  connection — the one thing the switch exists to prevent. Reversed, the window
  fails the other way: a request landing in it is blocked a moment longer than
  it needs to be. No CLI check; the window is internal to bootstrap.

### Blockbook clients, history shapes and Bitcoin signers

- **Blockbook family:** Litecoin, Bitcoin Cash, Bitcoin Gold, Zcash and Dash
  each carried a private copy of the same Trezor Blockbook client — five files
  matching 76–91% pairwise, redeclaring the same six JSON shapes. They are now
  one `BlockbookClient<N>` in `core/src/fetch/chains/blockbook.rs`, with a
  per-chain marker carrying the only real difference (Bitcoin Cash normalizes
  CashAddr before asking). The copies had drifted rather than diverged:
  `has_activity` existed on two of the five and `fetch_chain_tip_height` on
  one, so Dash, Bitcoin Gold and Zcash now have both. Nothing calls them there
  yet — `Chain::supports_deep_utxo_discovery` still gates discovery to the
  five chains it always did. Check `spectra balance <wallet> --chain zcash` and
  `cargo test -p spectra_core blockbook` (offline for the unit tests; the
  balance check needs network).
- **UTXO testnet history:** `normalize_chain_history` matched on the exact
  chain, so every UTXO testnet fell through to an empty result however well its
  fetch had gone — the same shape as the five-name lists this repo has removed
  elsewhere. It now reads its mainnet's shape through `mainnet_counterpart()`.
  Check `cargo test -p spectra_core testnets_normalize_like_their_mainnets`.
- **History amounts are magnitudes:** the fifteen per-chain arms disagreed on
  whether `amount` could be negative — five took `unsigned_abs()`, the rest
  passed the raw value through. It is now always a magnitude, with direction in
  `kind`, which is what every reader already assumed. Only rows whose chain
  reports a signed amount *and* an explicit `is_incoming` are affected, and no
  client emits that combination today. Check
  `cargo test -p spectra_core amounts_are_magnitudes`.
- **Legacy P2PKH fee sizing:** `select_coins` hardcoded 31 vbytes per output
  and ignored the size it was passed, so a Bitcoin legacy send that selected
  its own coins paid for 31-byte outputs while the same send with pinned UTXOs
  paid for 34. Both now size from `SpendSizing`, so a P2PKH send costs 3
  vbytes per output more than before — the correct amount. Check
  `cargo test -p spectra_core pinned_and_selected_coins_charge_the_same_fee`.
- **The signed fee is the quoted fee:** the fallback used when a caller
  supplies no fee lived in three tables — a literal per signing arm, a second
  per arm of `build_send_params`, and `Chain::static_fee_units`, which is where
  the fee shown on the send screen comes from. Litecoin signed 10 000 against a
  1 000 quote and Bitcoin Cash 1 000 against 2 000. All three now read the
  registry. Check
  `cargo test -p spectra_core an_unquoted_fee_falls_back_to_what_the_estimate_reports`.
- **A bad txid is refused:** two copies of the legacy input builder decoded the
  txid with `unwrap_or_default()`, so a malformed one produced an empty
  outpoint and a structurally invalid transaction reported as success. The
  shared `send::chains::wire` builder returns an error, and Decred decodes once
  when it builds its inputs rather than at each serialization. Check
  `cargo test -p spectra_core a_bad_txid_is_refused`.
- **Bitcoin Cash `has_activity` normalizes first:** it asked its Blockbook
  instance about the address exactly as given, while `fetch_balance` — the same
  endpoint, one line away — normalized CashAddr to legacy first. The shared
  client normalizes once for every read. Check
  `cargo test -p spectra_core blockbook`.
- **The Dogecoin fee fallback is the registry's:** the signing arm carried
  `unwrap_or(200_000)` where `Chain::static_fee_units` says 1 000 000. Nothing
  reached it — `build_send_params` computes Dogecoin's fee from its kB rate and
  always supplies one — so this removes a dead third opinion rather than
  changing a fee. Check `cargo test -p spectra_core dogecoin_uses_its_own_scale`.
- **A history row with no amount is kept, not dropped:** Bitcoin and the
  Blockbook family required their amount field and dropped the entry without
  it; the other thirteen chains defaulted it to zero. That split followed which
  arm was written first, not a decision. All of them now default, which is the
  majority behaviour — and the rows reach this function serialized from typed
  structs where the field is not optional, so neither branch fires in practice.
  Check `cargo test -p spectra_core every_chain_shape_normalizes`.
- **Error precedence in `sign_p2wpkh`:** it parsed the from-address after coin
  selection while the other three signers parsed it before, so a send that was
  both underfunded and malformed reported different errors depending on script
  type. All four now parse first. No CLI check: only the message differs.

### Keypool history projections and TRC-20 read metadata

- **Keypool reads:** every read/reservation loaded and decoded a wallet's full
  transaction history, then filtered by chain and parsed repeated paths. Core
  now reads distinct source/change paths scoped to the wallet and chain through
  SQLite expression indexes. Only those paths are decoded; transaction bodies
  are not loaded. This is an indexed projection, not a constant-time cached
  maximum. Indexes follow transaction edits/deletions automatically. Owned-address
  maxima are folded without temporary vectors. Check `spectra pool show <wallet>`
  and `cargo test -p spectra_core keypool_history_projection` (offline; checks
  chain/wallet isolation, query plans and mutations).
- **Repeated reservations:** an existing reservation used to write the same row
  again. Core still merges the latest discovered floor, then skips persistence
  when the complete merged record is unchanged. New discoveries still raise the
  persisted floor without replacing the held reservation. Check consecutive
  `spectra pool next <wallet>` calls and
  `cargo test -p spectra_core unchanged_receive_reservation` for zero redundant
  SQL updates and persistence of newly owned indices.
- **TRC-20 metadata:** each balance/enumeration read queried symbol and decimals
  again, including concurrent reads of the same contract from different wallets.
  A service now shares up to 256 cached metadata entries for five minutes, keyed
  by chain, endpoint list and contract. In-flight cached requests share one result;
  failures are discarded and cancelled initialization can be retried. Changing
  endpoints or chain uses a separate entry. Read metadata can now remain unchanged
  for up to five minutes after a contract update; balances remain live. Send
  preparation uses the uncached metadata API and still fetches current decimals.
  Check `cargo test -p spectra_core metadata_cache` for mock-RPC call counts,
  source isolation, send bypass, expiry, capacity and cancellation (no live chain).
  Live `spectra token discover --wallet <wallet>` needs network;
  cross-wallet reuse occurs within the long-lived core service, not across CLI
  processes. CLI acceptance continues to check the offline token/state paths.

### Bounded discovery and incremental state writes

- **Address derivation:** every external index repeated BIP-39 and the complete
  BIP-32 path. A scan now derives its external xpub once on a blocking worker,
  then derives public children; no mnemonic is retained during network probes.
  Check `cargo test -p spectra_core public_children_match_full_derivation`
  for every discovery network, Bitcoin script types and the BIP-84 vector.
  `spectra pool next <unsealed-Bitcoin-wallet>` now includes the derived receive
  address; CLI acceptance checks this offline and checks a stable reservation
  after reopening.
- **Activity:** scans waited for each address and sometimes fetched full histories
  (BSV also enriched every transaction). Four probes can now be in flight, with
  index-ordered results. Esplora, Blockbook and BlockCypher use confirmed/pending
  counters; BSV uses balance plus its history index without transaction details.
  Missing counters, malformed responses and provider failures now return errors
  instead of claiming an address is unused. A failed scan may have recorded earlier
  successful probes; it never advances a reservation from a failed probe.
  Check `cargo test -p spectra_core service::state::performance_tests` using local
  mock HTTP only. Counter schemas: [Blockbook](https://github.com/trezor/blockbook/blob/master/openapi.yaml),
  [BlockCypher](https://www.blockcypher.com/dev/bitcoin/#address-balance-endpoint).
- **CLI discovery:** `pool discover` used a fresh endpoint-only service, so it had
  no wallets or secrets to scan. It now opens the selected database and secret
  store. Check `spectra pool discover <wallet>`: sealed wallets list their known
  addresses offline (CLI acceptance); unsealed discovery needs network or a mock
  provider. Live provider interoperability is outside the offline gates.
- **State commits:** small commands cloned intermediate snapshots and replaced
  every wallet/address-book row. Commands now build an incremental write set,
  serialize changed records only, and save it in one transaction before publishing.
  Settings changes write only their metadata; no-op commands do not write.
  Snapshot replacement remains available for standalone store callers. Check
  `spectra currency EUR` then `spectra currency` in separate processes, CLI
  acceptance, and `cargo test -p spectra_core a_setting_update_only_writes` plus
  `cargo test -p spectra_core incremental_state_reorders` for SQL write counts,
  reordering, removals and transaction rollback. Existing cancellation/concurrent
  writer tests still apply.

### Stored send identity and database connection ownership

- **Send identity:** the execution request separately accepted chain id/name,
  sender, derivation data and two optional secrets, preferring the seed when both
  were supplied. It now accepts a wallet id and optional unlock password; core
  resolves its stored path/overrides and exactly one signing source. Missing or
  watch-only wallets, unrelated chains, ambiguous secret blobs and addresses
  disagreeing with the derived key are refused before provider reads. Private-key
  wallets no longer require a caller-supplied seed/path. Check `spectra send
  identity --from <wallet>` (offline, using the usual password file/environment
  options), including `--chain Arbitrum` for an Ethereum wallet and refusal of
  `--chain Solana`. CLI acceptance and `service::send_identity::tests` cover seed,
  private-key, password, mismatch and ambiguity cases; Swift tests cover the
  new request and missing-wallet refusal across the async binding. Swift passes
  no password today, so sealed-wallet sends still refuse until a password is
  provided; the CLI supports the password path.
- **Account-based signers:** NEAR named-account key authorization remains the
  protocol client's access-key lookup before signing; offline identity resolution
  only resolves that name, while implicit account ids must match the derived key.
  Monero previously sent through any configured wallet-rpc without checking which
  wallet it held. It now checks account 0 against the resolved sender and submits
  through that same endpoint; a mismatch refuses and a failed transfer is not
  retried through a different endpoint. The local mock-RPC test
  `monero_rpc_is_bound_to_the_checked_sender_and_endpoint` checks refusal and
  matched submission. Actual broadcasts still require controlled network testing.
- **Database connections:** a process-global connection map held one mutex through
  every SQL operation and retained connections forever. It now indexes weak
  handles, with each database independently locking initialization and SQL; the
  service retains its active connection and releases it on rebind/drop. Standalone
  path-based operations close their connection when their last owner finishes.
  SQLite waits up to five seconds for file-lock contention. Check ordinary
  `spectra currency` / `spectra pool` persistence in CLI acceptance, and
  `cargo test -p spectra_core connection_lifecycle_tests` for independent-database
  progress, shared handles and connection release (all offline).

### Atomic writes, exact send amounts and receive advancement

- **Persistent mutations:** app-state and event snapshots could be saved out of
  order; keypool/address writes and deletes changed memory before SQLite could
  fail. Persistent mutations now share a service writer, save candidates before
  publishing, and finish admitted writes even when the caller cancels its wait.
  Opening a database uses the same writer and loads all tables before publishing;
  malformed event data is refused instead of silently cleared. Combined wallet
  deletion uses one SQLite transaction. Check `spectra currency EUR` followed by
  `spectra currency` in a separate process and the CLI keypool checks. Failure,
  cancellation and concurrent-write coverage: `cargo test -p spectra_core
  service::state::tests` (offline).
- **Send amount:** `SendExecutionRequest` had both a float and optional decimal
  string; different chains rounded, truncated or ignored the exact input.
  It now requires one decimal string, uses checked integer arithmetic for every
  native/token builder, and refuses invalid syntax, excess precision and integer
  overflow. Supported precision is bounded to 38 digits for the u128 conversion;
  protocol u64/i64 limits are checked before signing. Zero remains allowed for
  native EVM transactions; other transfers require positive amounts. Check
  `spectra send amount --chain Solana --amount 9007199.254740993` (raw units
  `9007199254740993`); Bitcoin `0.000000001`, negative/nonfinite values and u128
  overflow exit 3. CLI acceptance, Rust builder tests and `SendAmountBridgeTests`
  exercise the change offline; live broadcast remains outside these checks.
- **Receive advancement:** probing an address then clearing the current reservation
  could discard a newer reservation created while the probe was in flight.
  Advancement now atomically compares the checked index and reserves its successor;
  stale results do nothing and the successor respects addresses discovered during
  the probe. Check `spectra pool next <wallet>` for stable receive
  reservations and `cargo test -p spectra_core
  concurrent_probes_advance_only_the_reservation_they_checked` for concurrent/stale
  probe results and persisted successors. Live activity probes still need network.

### State and persistence

- **Collections and settings:** Swift arrays, snapshots and separate settings
  blobs became core-owned state and commands. Core validates address-book
  entries, rejects duplicates and clamps setting bounds so front ends cannot
  persist different answers. Check `spectra address book`, `currency` and
  `settings` in separate processes; CLI acceptance covers persistence/refusals.
- **Wallet lifecycle:** repeated `open_state` no longer replaces newer state;
  refresh updates only wallets still present, and launch loads cannot erase a
  newer UI projection. This prevents deleted wallets returning or new imports
  disappearing. Check wallet lifecycle in CLI acceptance; launch races require
  the Swift tests.
- **History:** Swift decided what to merge and save; core now merges incoming
  records against its own store and writes only changes. History methods use
  the database bound by `open_state`, preventing mismatched read/write paths.
  Check `spectra txs` and the Rust history-store tests, including
  `records_land_in_the_database_the_service_was_opened_on`.
- **Tokens and alerts:** tracked tokens previously vanished on restart; alert
  lists had separate platform storage. Both now persist in resident state.
  Display places are capped at token decimals, alert targets must be positive,
  and alert evaluation saves trigger state in core. Built-in token ids are
  `builtin:<chain>:<contract>` instead of regenerated UUIDs. Check `spectra token`
  and `spectra alert` across separate invocations; CLI acceptance covers them.
- **Operational events:** caller-minted ids/timestamps and caller-applied limits
  became an atomic core append with newest-first ordering and a per-chain cap
  of 200. Check `cargo test -p spectra_core the_log_is_newest_first_and_bounded`;
  that bound has a Rust test rather than a dedicated CLI assertion.

### Keys, import and receive addresses

- **Core derives a private-key import's address too.** The seed path stopped
  having its front ends derive first and hand the result over; the private-key
  path had not, so both front ends still derived it, each with its own refusal
  when the chain could not — two copies of a rule core already owned, and a
  wallet stored with whatever address the caller passed. `WalletImportCommit`
  carries the key the way it carries the seed, and
  `derive_private_key_import_address` is the one place that refuses. The CLI
  still asks it before sealing, because a refusal after sealing leaves a key
  stored under an id no wallet references — a check of order, not of the rule.
  Swift keeps only the field check on the key's shape, and
  `WalletRustDerivationBridge.deriveFromPrivateKey` is gone. Check CLI
  acceptance's "refuses a chain that cannot derive from a key" and the sealing
  check beside it.

- **A seed phrase gets one verdict.** Whether an entry was finished, which of
  its words were off the list, whether the count was right, whether the
  checksum held and what to say about it were five separate reads assembled in
  Swift from three exports and a wordlist Swift cached itself — so the order
  they had to be asked in was a Swift rule, and `BIP39WordList` was a second
  copy of core's word list. `core_check_seed_phrase` answers all of it in one
  pass, in that order: an unfinished entry says nothing, words off the list are
  their own message, and only a phrase of real words is worth checksumming.
  `validate_mnemonic`, `bip39_wordlist` and
  `core_validate_seed_phrase_word_count` are gone with it, and so is
  `WalletImportDraftValidation`, which nothing called.

  Two things the one verdict changed. **A mnemonic is read in the language its
  words are in.** `Mnemonic::from_str` refuses a phrase it cannot pin to a
  single language, and the Simplified and Traditional Chinese lists overlap
  almost entirely — so Chinese mnemonics were exactly the phrases it refused,
  and derivation, which assumed English whenever no wordlist was named,
  refused every non-English phrase outright. A caller that names a language
  now gets that language and only that language, which is the stricter side
  where a picker exists (English and French share about a hundred words); a
  caller with no picker — the CLI, reading a phrase from a file — gets any
  BIP-39 language. An explicitly named wordlist that is not a language is
  still an error, because it comes from the Advanced-mode override field and a
  typo there would otherwise derive a different wallet under English.

  **And an import that derives no address refuses.** A phrase the deriver
  could not read produced a stored wallet with an empty address that read to
  the user as "imported" — the same mistake watch-only imports were already
  fixed not to make. Check CLI acceptance's "imports a Chinese mnemonic" and
  the two refusal messages beside it, plus `cargo test -p spectra_core
  seed_phrase_tests mnemonic_language_tests`.

- **Core mints the ids for the wallets it creates.** A caller supplied them,
  which meant predicting how many wallets an import would make — and for a
  watch-only import that meant parsing the address entries the same way the
  planner does, under a second copy of the "which chain's input holds them"
  rule, with a refusal when the two counts disagreed. An empty id plan is
  minted here now; a supplied one must still match, because silently ignoring
  a mismatch would file a wallet under an id nothing else knows. The private-key
  import also stopped filling both EVM slots by hand — core fills the sibling
  from the wallet's own address, in both directions. `spectra wallet watch`
  takes `--address` more than once for the same reason: one wallet per entry is
  what the planner does, and the CLI could only ever drive one. Check
  `cargo test -p spectra_core minted_wallet_id` and CLI acceptance's
  multi-address watch import.

- **A wallet's address is stored, not derived on read:** resolving "this
  wallet's address on this chain" was a Swift function that read the seed out
  of the Keychain, resolved a derivation path, called the deriver and validated
  the result — per call, on the render path, for a value core had already
  computed at import. A password-sealed wallet has no seed to read, so it fell
  through to the stored address whatever network it was on, and said nothing.
  Core now derives an address for **every network of a selected chain's family**
  at import, when the seed is in hand, and stores each under that network's own
  slot; `WalletSummary` carries them all instead of only the selected chain's,
  so they survive into the database and out to a front end. Switching to
  Testnet4 is a lookup. Three consequences, all deliberate: a chain the wallet
  was never imported for now has no address rather than one invented from its
  seed; the Ethereum/Ethereum Classic slot pair is filled in both directions
  from the wallet's *own* address, where before only an ETC wallet filled both,
  so an Ethereum wallet answers on all 23 EVM mainnets; and a wallet whose
  chain the registry does not know converts with the slots it holds rather than
  with none. Swift derives no addresses at all now, and
  `core_resolve_derived_or_stored_address` — the export it handed its two
  candidates to — is gone with `DerivedAddressPostProcess`. Wallets imported
  before this change hold only their mainnet slot; Spectra is prelaunch, so
  they are re-imported rather than migrated. Check `spectra wallet show --json`
  for the `addresses` map and CLI acceptance's "addresses per network" section,
  `cargo test -p spectra_core a_seed_import_stores_one_address_per_network_of_its_family`,
  and iOS `testImportingBitcoinWalletOnTestnet4StoresTheMainnetDerivedAddress`
  and `testAWalletAnswersForItsOwnChainsAndNoOthers`.


- **Address resolution columns:** resolving a wallet's address for a chain went
  through a nineteen-row table in Swift carrying three per-chain columns —
  whether to read the configured derivation path raw or resolved, how to
  post-process the derived address (`trim`, `lowercase`, nothing), and whether
  to normalize the stored one. None was a fact about a chain: ten rows read the
  path raw and nine resolved it, two asked for lowercase and three for a trim,
  six normalized the stored address and thirteen did not, and no row had a
  reason. The table is gone. The path is the resolved one everywhere, so a
  chain the wallet predates derives from the catalog default instead of from
  the empty string; the derived address is used as derived, which
  `a_derived_address_needs_no_post_processing` asserts is already trimmed and
  already lowercase on the two chains that asked; and a stored address comes
  back in its canonical spelling on every chain rather than on six — a stored
  Bitcoin Cash or Sui address may now display in its prefixed form where it
  previously displayed as typed. `core_resolve_derived_or_stored_address` lost
  both parameters and `DerivedAddressPostProcess` with them. A chain outside
  the old table no longer resolves to `nil` on that ground alone, which is what
  a testnet chain name asked by name used to get. Check `spectra wallet show`
  and CLI acceptance's derived-address assertions; iOS
  `testEveryMainnetThatDerivesResolvesAWalletAddress` walks the whole catalog
  and names Monero as the one chain with no derived address, and
  `a_derived_address_passes_its_own_validator` covers the derivation side.


- **Secret layout:** iOS plaintext-plus-password-verifier and CLI encrypted
  blobs became one core-managed layout. Passwords now encrypt seed/private-key
  material rather than only gating reveal. No-password wallets are explicitly
  unsealed; supplying a password for one returns `PasswordNotRequired`.
  Check `spectra wallet import --no-password` and `wallet export`; CLI acceptance
  verifies sealed/unsealed paths and wrong-password refusal.
- **Private-key imports:** disagreeing chain lists became the registry's
  capability flag and one derivation call. Unsupported chains are refused
  before saving a key; export handles private keys as well as phrases.
  Check `spectra chains --json` and `wallet import --private-key-file` in CLI
  acceptance, including supported Decred and refused Cardano imports.
- **Watch imports:** incomplete input lists and address-slot mappings became a
  registry-driven picker with core address validation. Ethereum Classic uses
  its own slot, and Monero watch-only import is refused. Draft reset clears all
  address inputs. Check `spectra wallet watch` and `chains --json`; the input
  layout/reset is checked on iOS.
- **Address validation:** the send UI refused prefixless Sui addresses that the
  store normalized and accepted. Validation now accepts the normalized Sui
  form consistently. This does not waive EVM mixed-case checksum validation.
  Check `spectra address validate`; Rust's
  `a_sui_address_without_its_prefix_is_accepted_either_way` covers that case,
  and CLI acceptance covers EIP-55 refusal.
- **Keypool ownership:** caller baselines and detached writes became core-owned
  reservations and owned-address records; startup loads before reserving and
  deletion clears memory and disk. Display reads do not reserve. Receive
  reservations remain stable until released; change reservations consume an
  index. Check `spectra pool next`, `next-change`, and keypool concurrency tests.
- **UTXO discovery:** inconsistent balance/UTXO/history probes and a mainnet-only
  Swift list became one activity rule for every registry-supported network.
  Password-sealed wallets return known addresses without deriving new ones
  when no password is available. Check `spectra pool discover <wallet>`
  (network needed for the walk); CLI acceptance covers the offline cases.
- **Receive reservation:** five Swift steps became `utxo_receive_address` and
  `advance_used_utxo_reservations` in core to remove caller interleaving between
  release and reservation. Deep-UTXO receive indices start at 1. Check
  `spectra pool next <wallet>`; CLI acceptance checks the floor.

### Sending and refresh

- **An extra output is priced where the fact lives.** A destination that costs
  more than a plain output — Litecoin's MWEB peg-in is the one the registry
  names — pays for those bytes at the previewed rate, and both the estimate and
  what is left sendable move with it. The front end fetched
  `extra_output_overhead_bytes` and did that arithmetic on its side, untested;
  `fetch_utxo_fee_preview_typed` takes the destination and returns a preview
  that already includes it. The export lost its caller and is internal. Check
  `cargo test -p spectra_core extra_output_overhead` — the bytes at the rate,
  no overhead leaving the preview untouched, and what is left sendable stopping
  at zero.
- **Which Tron assets have a preview** was `TRX || USDT` written out in the
  refresher — a third copy of `route_send_asset`'s answer, beside the submit
  path's and the dispatcher's, and the one that would have kept refusing if
  core's router were widened. It asks for the route now, as the Solana
  refresher already did.

- **Which token a send moves:** every submit branch resolved this on the
  caller's side, from its own mirror of the tracked-token list, with its own
  refusal message — and each got it slightly differently. Tron hard-coded six
  decimals for every token on it, right for USDT and wrong for the four
  eighteen-decimal ones in the catalog, so a raw amount would have been 10^12
  too small; NEAR fell back to six for a token it could not find, which is a
  scale guessed on the funds path; Solana read a token map built with
  `includeDisabled: true`, so a token the user had switched off could still be
  sent. The preflight carries `token_contract_address` and `token_decimals` as
  core resolved them: matched by contract where the holding names one and by
  symbol on the chain where it does not, and absent for a native asset or a
  token nothing tracks — which is what makes the send refuse rather than guess.
  Check `cargo test -p spectra_core send_token_identity` for the two scales,
  both match paths, the native asset and the two refusals.
- **A chain that computes its own fee can send without an estimate.** The
  generic submit refused unless a preview had produced a fee or the registry
  gave a fallback — but a chain whose `SendFeeField` is `None` computes its fee
  when it signs, so the estimate is a display and an affordability input, not
  something the send needs. Stellar, XRP and Internet Computer were blocked
  whenever no preview had loaded, which is why Internet Computer had a submit
  arm of its own that skipped the generic path entirely. That arm is gone: with
  the requirement scoped to the chains that actually take a fee field, it is
  exactly what the generic path does.

- **Pending-status polling:** how a chain reaches finality is a registry fact,
  and the three shapes it takes — a UTXO status endpoint, an address history
  naming confirmed txids, an EVM receipt — were three loops on the front end's
  side of the boundary. Each selected the records to poll from its own
  projection of core's store, asked core whether each was due, fetched, told
  core the outcome, collected the resolutions and handed them back to be
  applied: five crossings per transaction, for a store, a schedule and a fetch
  core already owned. `poll_pending_transactions` is the loop, and it answers
  with what changed — which is what a front end needs to write an operational
  event and send a notification, the two things that are genuinely a
  platform's. The poll now follows the network the wallet is on, like the rest
  of the fetches. Five exports lost their last caller and became internal:
  `transactions_due_for_status_poll`, `record_status_poll`,
  `apply_resolved_pending_statuses`, `fetch_utxo_tx_status_typed` and
  `evm_transaction_status`; the export surface is 189. Check `cargo test -p
  spectra_core pending_status` for which records each shape tracks — a receive
  where the chain tracks only sends, a confirmed record where it counts
  confirmations, a missing or blank hash, another chain's record — and for the
  unpolled and unknown chains. The polling itself needs network.

- **Fetched from the network, filed under the family.** A wallet on a testnet
  fetched its testnet address from the family's *mainnet* endpoints and read
  zero — balances and history both — because one chain id answered two
  questions: where to fetch from, and what to file the result under. They are
  different answers. The fetch follows the network the wallet is on
  (`WalletSummary::network_chain`, the same rule the send path now uses); the
  record is filed under the family, because that is what the store groups by,
  what the holding is named after, and what pricing keys on — and pricing
  already marks a family unpriced while it is on a testnet, so a testnet
  balance shows no fiat value without any renaming. `RefreshEntry` carries both
  ids for that reason rather than one. Two things fell out of it: the Bitcoin
  xpub arm compared the chain id to the literal `"bitcoin"`, so a Testnet4
  wallet's xpub was walked as a plain address, and the xpub balance always read
  Bitcoin's endpoints. Both take the network now. Check `cargo test -p
  spectra_core history_refresh` —
  `a_testnet_wallet_fetches_its_network_and_files_under_its_family` asserts the
  two answers separately — and `refresh_entry_tests`. **Still mainnet-only:**
  the Bitcoin HD walk (`fetch_bitcoin_hd_history_page`) reads Bitcoin's
  endpoints whatever network the wallet is on; it needs a chain argument.


- **A send follows the network the wallet is on.** It was signed for the
  family's mainnet whatever network was selected: with the app switched to
  Sepolia, a send still signed chain id 1 and read mainnet endpoints, so a
  transaction the user believed was a testnet one was a valid mainnet one, and
  broadcasting it would have moved real funds. `execute_send` resolves the
  chain through `WalletSummary::network_chain` — the same rule the balance and
  history refreshes use — and the CLI resolves its endpoints the same way. This
  was found by reading what `--sign-only` signed: the chain id in the payload
  was `01`. Check `cargo test -p spectra_core send_chain_tests` for the
  selection, the wallet's own record winning over it, another family's
  selection not moving this one, and an unknown wallet keeping the requested
  chain.
- **Sign without broadcasting:** `sign_only` was a field on the EVM overrides
  and a flag the Bitcoin builder had while the execution path hard-coded it to
  `false`, so "sign but do not broadcast" was reachable on one family by one
  route. It is one field on `SendExecutionRequest` now, honoured by both
  builders, and the signed payload comes back as a typed
  `SendExecutionResult::signed_payload` rather than something to dig out of the
  opaque rebroadcast blob. Both routes ask through one function,
  `SendExecutionRequest::wants_sign_only` — written out at each of the four
  places that asked, they disagreed: the refusal read both routes while the
  result field and the Bitcoin builder read only the new one, so a caller
  asking through the EVM overrides got a signed transaction and a
  `signed_payload` of `None`. A run that stops without a payload to show is an
  error rather than an empty string that reads like one. A chain whose builder
  cannot stop before
  broadcasting refuses the request before any key is read, rather than
  broadcasting a caller's dry run — `Chain::supports_sign_only` says which. The
  CLI's `send broadcast` gained `--sign-only` (no `--yes`, since nothing
  irreversible happens) plus `--gas-limit` and `--nonce`, which is what lets an
  unfunded address sign: a node refuses to estimate gas for a transfer it
  cannot pay for. Check CLI acceptance's "sign without broadcasting" section
  and `spectra send broadcast --from <wallet> --to <address> --amount 0.001
  --sign-only --gas-limit 21000` against a testnet (network).


- **Bitcoin history:** Bitcoin is the one chain with an account xpub, so its
  history is the HD range's rather than one address's, and three arms decided
  which: derive the account xpub from the seed and walk the range, else fetch
  the stored address, else walk a stored xpub. The front end held all three —
  it read the seed out of the Keychain, cut the account path out of the
  wallet's derivation path by string surgery, derived the xpub, chose between
  the results and built the records. `refresh_bitcoin_history` does it over the
  seed, paths and cursors core holds; a sealed wallet derives no xpub and falls
  through to its stored address, as it did before. Three things change with it.
  An entry with no timestamp keeps the sentinel the merge recognises instead of
  being stamped with the time of the refresh, which sorted an undated
  transaction to the top of the list and moved it there again on every refresh.
  The Bitcoin *diagnostics* run was a second copy of the same source selection
  that fetched a page, read a row off it and threw the page away; it runs the
  refresh now, so what it fetched is merged — which is what the button says it
  is for. And the refresh reports one diagnostics row per wallet in the shape
  every history path now uses, `HistoryWalletDiagnostics`. The account xpub is
  derived with the wallet's own BIP39 passphrase, not the empty string the
  front end passed: derived without it the xpub belongs to a different wallet,
  so the range walked came back empty and the refresh fell through to the
  single stored address — a passphrase wallet never had HD history at all,
  though the send identity has always derived with it. The phrase itself never
  leaves a `Zeroizing`: `wallet_seed_phrase` hands back a plain `String` copy
  that drops unwiped, and this runs on every refresh rather than only at send
  time, so it reads through `load_signing_material` like the send identity
  does. The derivation secrets a cloned `WalletSummary` carries are wiped at
  the end of each wallet's turn through `SensitiveOverrides`, which was
  `send_identity`'s private guard and is now shared with this, its second
  caller. A wallet whose fetch
  failed keeps the cursor it had: writing `None` there is how a caller says
  "the chain confirms there is no more", which a fetch that failed did not say,
  and it marked the wallet exhausted — so the outcome reported more to load
  while the wallet's own cursor refused to load it, and "load more" did nothing
  until a pull-to-refresh reset it. Two exports lost
  their last caller: `fetch_bitcoin_hd_history_page` is internal, and the Swift
  `BitcoinHistoryPage` type is gone. Check `cargo test -p spectra_core
  history_refresh` for the wallet with nothing to fetch for, the cursor a
  failure leaves alone, and the empty case; the three sources need network. **Not fixed:** the stored cursor still does
  not page. `next_cursor` is written and read back, but the address arm refetches
  the same history and re-caps it, so "load more" on a Bitcoin wallet with more
  than a page of history returns what it already had. That was true before this
  moved and is now visible in one place.

- **Multi-address UTXO history:** a UTXO wallet spends from many addresses, so
  one transaction arrives once per address it touched and the records are
  netted per transaction before they are stored. The front end asked core for
  each wallet's known addresses, handed them straight back inside a planning
  request, fetched per address, called core's aggregator, built the records and
  sent them to be merged — six crossings for data core already had, since the
  addresses are its keypool. `refresh_utxo_chain_history` does it where they
  are. An aggregate with no known timestamp keeps the sentinel the merge
  recognises rather than being stamped with the time of the refresh. A wallet
  one of whose addresses did not answer now merges nothing rather than what it
  managed to fetch: netting is over the whole address set, so a missing address
  is a wrong amount rather than a missing row — a transaction whose change went
  there nets to the legs that did answer, and the figure stored was one no
  address agreed with. The wallet counts as failed and its cursor is left
  loadable, so a later refresh nets the whole set again. Two more
  exports lost their only caller and became ordinary functions:
  `core_refresh_targets` and `history_aggregate_by_transaction`. Check `cargo
  test -p spectra_core history_refresh` for the skip and refusal cases and for
  the wallet with one address answering and one refusing, which runs against a
  mock backend because the offline gate cannot produce a half-answered fetch.

- **A send preview is priced on the network the wallet is on.** The chain id
  reached `refreshUTXOChainPreview` from its callers as the family's mainnet.
  While a send signed for mainnet too, that cost only a wrong fee estimate;
  once `execute_send` began following `WalletSummary::network_chain`, the two
  disagreed — a testnet send was priced, and its spendable balance read,
  against mainnet. The preview resolves the network from the wallet it is
  previewing, so the parameter is gone and both callers lost an argument, and
  `fetch_bitcoin_hd_send_preview_typed` takes the network id the way the
  balance refresh does (its own comment had already said it should). Its fee
  read, `bitcoin_fee_rate`, takes the chain rather than assuming mainnet, and
  a chain id outside the Bitcoin family is refused rather than quietly priced.
  Check `cargo test --workspace`; the preview itself needs network.

- **Bitcoin's diagnostics run is bounded.** It runs the real refresh now, and
  ran it unbounded: a refresh that never answered left the screen's `isRunning`
  flag set and the button dead for the rest of the session. It is wrapped in
  the same `withTimeout(seconds: 20)` every other probe on that screen uses.
  The rows the refresh already wrote stand; the timeout only releases the
  button. This is an iOS screen and needs network.

- **EVM history refresh:** eight steps stood on the front end's side of the
  boundary — plan the wallets, group them by normalized address, reset or
  advance each group's page, build the token descriptor list from its mirror of
  the token preferences, fetch the page, plan the records, convert them, merge.
  `refresh_evm_chain_history` does all eight over the wallets, preferences and
  history cursors core already owns, and answers with what changed, whether
  every group reported a short page, and one diagnostics row per wallet — which
  source answered, how many transfers, what failed. The front end keeps the
  rows and the banner, which are its screen. Three exports lost their only
  caller and became ordinary functions core calls itself:
  `core_evm_refresh_targets`, `history_evm_native_asset` and
  `plan_evm_transaction_records`. Check `cargo test -p spectra_core
  history_refresh` — token descriptors, the per-wallet failure accounting for a
  chain no explorer serves (offline: the URL builder refuses before any
  request), the non-EVM refusal and the empty case. The fetch and pagination
  themselves need network: an `Open` history source's base URL comes from the
  registry rather than from the endpoint table, so there is no way to point one
  at a local mock. The Swift implementation this replaces had no test at all.

- **Normalized history refresh:** fetching one chain's history was four steps
  on the front end's side of the boundary. It mapped its wallet projection into
  a planning request and handed that back for core to filter; fetched each
  target; built a transaction record per entry — minting the id, naming the
  wallet, stamping the source; and sent the result back to be merged.
  `refresh_chain_history` does all four where the wallets are, and answers with
  what changed: wallets refreshed, wallets whose provider failed, records added
  and updated. A provider failure for one wallet is counted rather than raised,
  so a partial refresh still merges what answered — which is what the front
  end's "loaded with partial provider failures" banner already said. Core mints
  the record id as a dashed v4 UUID, because a front end reads ids back with
  `UUID(uuidString:)` and silently drops a row whose id does not parse. Which
  network a wallet is on and which address it fetches with is now one rule,
  `WalletSummary::active_address`, shared with the balance refresh; the two used
  to work it out separately. Check `spectra history <wallet> --save` (network:
  it fetches, merges and prints the counts, and `spectra txs` then shows the
  rows) and `cargo test -p spectra_core history_refresh` for targets, network
  selection, record shape and the empty and unknown-chain cases offline. The
  multi-address UTXO and EVM paths still plan in Swift.

- **What the balance refresh refreshes:** the engine's entry list — one
  `(chain, wallet, address)` triple per wallet — was built by whichever front
  end was driving. iOS mapped its wallet projection and resolved each address
  by deriving it from the seed, dropping a wallet it could not resolve with a
  `print` and no other trace; the CLI built its own list beside it and
  disagreed twice, taking any wallet's `xpub` as the fetch key rather than only
  Bitcoin's, and the wallet's first stored address rather than the one for the
  network it is on. `BalanceRefreshEngine::sync_entries` builds the list from
  the wallets core holds, and both front ends ask for it. Check `spectra
  refresh` and `spectra refresh --wallet <wallet>` (network), and `cargo test -p
  spectra_core refresh_entry_tests` for the network, xpub, missing-address and
  EVM-family cases offline.


- **Replaceable pending sends:** two Swift filters over the app's own
  transaction projection decided which pending send could be sped up or
  cancelled, and both spelled the chain as the string `"Ethereum"`. Core now
  answers with `replaceable_sends` — an EVM chain by the registry's family, a
  send, still pending, with a hash to read its nonce by — so the actions appear
  on every EVM chain, and the replacement is signed for the pending send's own
  chain rather than for Ethereum mainnet. Three further changes come with it.
  Speed Up is offered only when the pending send moved the chain's gas asset:
  a token transfer cannot be rebuilt from a record, and composing one used to
  offer a *native* transfer of the token's amount to the token's recipient —
  speeding up a 100 USDC send offered to send 100 ETH. Cancel is unchanged and
  still offered for both, since it is a zero-value self-transfer at the same
  nonce. The composer's Speed Up/Cancel now act on a pending send on the chain
  the composer is showing, not on whichever pending send the wallet had. And the
  replacement fee starts from the chain's live estimate bumped 20% instead of a
  fixed 4/2 gwei pair, which was under the base fee on some chains and far over
  it on others; the constants remain only for when no estimate loads. Check
  `spectra txs --replaceable` (offline, and with `--wallet`), `cargo test -p
  spectra_core replaceable` for the rule, chain family, token/native split and
  store reads, and `ReplaceableSendBridgeTests` across the async binding.
  Recording a pending send needs a broadcast, so CLI acceptance covers the empty
  answer and the wallet filter, not a live replacement. One export was added;
  it removed the Swift rule and the two filters.

- **EVM composer controls:** custom EIP-1559 fees and the manual nonce were
  offered on every EVM chain but resolved only when the selected chain was named
  "Ethereum" — `customEvmFeeConfiguration` returned nothing elsewhere, so fees
  typed on Arbitrum or Base were parsed, reported as applied, and dropped, and
  selecting another asset cleared both toggles on the 22 EVM chains that accept
  them. What the composer shows is now what is applied: the fields are cleared
  when the selection leaves the EVM family, which is `Chain.isEVM` and the same
  family `EvmSendOverridesInput::resolve` validates against, and no second chain
  test stands between the entered value and the send. Check `spectra send fees
  --max-fee 30 --priority-fee 1` and `send overrides --nonce 0` for the shared
  parsers; the composer gate is iOS-side, covered by `EvmCustomFeesBridgeTests`
  and `EvmNonceBridgeTests` for the parsers behind it.

- **Manual EVM nonce:** a Swift reparse and Int32-only validation became one
  core decimal parser returning the nonce. The range now matches the signed
  Int64 preview/FFI fields; signed, fractional, hexadecimal and overflowing
  text is refused. Invalid manual input throws instead of becoming an automatic
  nonce, and previews reject negative caller/RPC nonces. Check `spectra send
  overrides --nonce 2147483648` and refusal of `--nonce +1`; CLI acceptance and
  Swift binding tests cover the parser and its errors.

- **EVM override validation:** access lists were discarded, malformed calldata
  became absent, and negative nonce/gas values wrapped to unsigned integers.
  `EvmSendOverridesInput::resolve` now validates the chain, numeric signs, hex
  bytes and access-list schema/address/storage-key sizes, and carries accepted
  bytes into both native and token sends. Explicit empty calldata stays distinct
  from absent calldata. Custom calldata or a non-empty access list requires an
  explicit gas limit, avoiding the default native limit or an estimate for
  different token calldata. Unknown chains and malformed overrides fail before
  key derivation or network reads. Check `spectra send overrides --nonce 0
  --gas-limit 50000 --calldata 0x0102`; invalid inputs exit 3. Core and CLI tests
  cover validation and builder propagation; gas sufficiency and live broadcast
  remain outside these offline checks. No new FFI export was added.
- **Custom EVM fees:** core previously returned validation advice, accepted
  infinity, and left Swift to parse again. `parse_evm_custom_fees` now returns
  the configuration or a typed error. Both fees must be finite, at least one
  wei, within the current u64-wei conversion range, and max must cover priority.
  Preview and send construction enforce the same checks, preventing zero or
  saturated fees when a caller bypasses the parser. Whole-wei rounding remains.
  Check `spectra send fees --max-fee 30 --priority-fee 1`; `inf`, sub-wei and
  overflow values exit 3. CLI acceptance, direct send-builder regression tests
  and `EvmCustomFeesBridgeTests` cover the paths without a broadcast.

- **Routing and gates:** preview/submit paths used different lists; some submit
  branches bypassed shared biometric/risk checks or sent unroutable tokens.
  Core now derives routing from its wallets, registry and tracked contracts;
  Swift follows the preflight route and shared gates. Check
  `spectra send assemble` and routing tests; biometric/broadcast paths still
  need app/testnet verification.
- **Affordability:** callers checked amounts and fees using separate rules,
  sometimes confusing the catalog ticker with the gas asset. Core now checks
  amount plus fee for native sends and the separate gas balance for tokens.
  Check `spectra send affordability --chain Arbitrum --symbol ARB --amount 1
  --fee 0.01 --balance 2 --gas-balance 0`; the fee is in ETH, not ARB.
- **Recipient risk:** per-chain probes sometimes checked the wrong asset or
  treated spent Bitcoin addresses as having no history. Core checks the chosen
  token/native asset and actual history; unknown chains error and unknown
  assets are not probed as a different asset. Swift uses one localized message
  pair. Check `spectra send probe --chain <chain> --address <address>` with token
  options when needed; verdicts require network, refusals have offline checks.
- **Bittensor and Monero:** Bittensor was unroutable despite having fee and
  signing support; it now uses the shared path with its own preview tag.
  Monero uses that path with core's default priority and its stored signing
  material, without requiring a seed derivation path. Check routing and
  `monero_takes_the_shared_submit_path` tests; live broadcast remains untested.
- **Concurrent sends:** a guard separated from insertion by awaits allowed two
  sends through. The shared runner now checks and claims the in-flight flag
  without suspension. This is an audited Swift fix, not covered by the offline
  CLI gate; verify with controlled app/testnet sends.
- **Post-send refresh:** Dogecoin omitted balance refresh and verification
  updates; it now runs the shared actions. UTXO testnets use their registry
  pending-poll policy instead of falling through to history refresh. Check
  `every_utxo_testnet_polls_the_way_its_mainnet_does`; observing the Dogecoin
  result requires a broadcast.
- **EVM pending status:** a recursive refresh arm became a real receipt poll.
  Reverted receipts resolve to failed instead of being mistaken for successful
  history entries. Check Swift's `testAnEVMPendingRefreshTerminates`; live
  receipt handling is outside offline CLI acceptance.
- **Refresh and CLI output:** fire-and-forget refresh gained awaited
  `refresh_now`, so `spectra refresh` completes before the process exits.
  Logging moved off stdout so `--json` stays parseable; self-test failures emit
  one document. Check CLI acceptance and `spectra diagnostics self-test`.

### Tor routing and the display currency

- **Tor settings:** the four values that decide whether traffic goes through
  Tor — on/off, embedded client vs the user's own SOCKS5 proxy, that proxy's
  address, and the kill switch — lived in one front end's `UserDefaults`. No
  other front end, no test and no script could read or set them, which is the
  one thing "no platform persists a second authoritative copy" rules out. They
  are `AppSettings` fields now, set through `SetAppSetting` like every other
  setting. The address is validated where it is stored: a value that is not a
  `socks5://` or `socks5h://` URL with a host and a port is refused and nothing
  changes, because the HTTP layer fails closed on a proxy it cannot build — so
  storing a typo used to mean every request quietly failing with no reason
  given. An empty value restores the default port. Check `spectra settings set
  tor-enabled true`, the refusals in CLI acceptance's "tor routing" section, and
  `cargo test -p spectra_core tor_settings_persist_and_refuse_an_address_that_is_not_socks5`.
- **The kill switch had no reader.** Its own settings screen promised that
  "network requests are paused if the Tor circuit drops instead of falling back
  to a direct connection"; nothing anywhere read the value. Core enforces it
  now: while Tor is wanted, the switch is on and Tor is not carrying traffic,
  `fetch::http` refuses the request with that reason, and the one function every
  caller takes its client from hands back the blocked client so a path that
  skips the check still cannot send in the clear. The rule is asserted over
  values rather than the process-wide flag — `the_kill_switch_engages_only_while_tor_is_wanted_and_not_ready`
  — because a test that engaged it would refuse every other test's HTTP call.
  Live behaviour with a real circuit still needs the app.
- **Display currency:** `SetFiatCurrency` stored whatever string it was handed,
  so `spectra currency ZZZ` set the display currency to `ZZZ` and every amount
  then rendered unconverted beside that code. The twelve codes the app quotes in
  are `FIAT_CURRENCY_CODES` in core; anything else is refused with an event, and
  the CLI turns that into exit 3. A settings update core cannot store now
  reports the same way — an unknown chain or a bad proxy address used to change
  nothing and say nothing, leaving the caller to notice that the read-back was
  the old value. Check `spectra currency ZZZ` (exit 3) and CLI acceptance.
- **Fiat cross-rates:** iOS fetched them, merged them through an export that
  took its own copy of the stored rates back, and wrote the result to a SQLite
  blob of its own, seeded at launch from an older `UserDefaults` key that still
  won the race. They are `CoreAppState.fiat_rates_from_usd` now, with their own
  persistence row, and `refresh_fiat_rates` is one operation: fetch, merge
  against what core holds, store, publish. A provider failure leaves the stored
  rates alone. `price_merge_fiat_rate_updates` had no caller left and is gone.
  Check `spectra currency --rates` (offline, reads the store) and
  `--refresh-rates` (network), plus `cargo test -p spectra_core
  stored_fiat_rates_survive_reopening`.

### Diagnostics

- **Per-chain diagnostics wiring:** an eight-row table in Swift named each
  chain's history driver and its own address resolver, and the diagnostics hub
  dispatched through it. Six rows had become the generic run written out —
  `resolvedAddress(for:chainName:)` resolves every chain, the EVM family
  included, and which fetch to make is `chain.isEVM` — so the table is gone and
  Bitcoin (an xpub history page) and Monero (its configured backend) are the two
  cases left, each a line in a switch. Dogecoin's row carried no per-wallet
  entry, so a per-wallet Dogecoin run did nothing; that path had no screen at
  all and is deleted rather than fixed, along with the four wrappers that passed
  a chain's own name back to the catalog probe. Check `spectra endpoints --chain
  Dogecoin` for the catalog side; the runs themselves are iOS screens and need
  network, and `DiagnosticsChainTableTests` still pins the hub, bundle and
  screens to one chain list.
- **Run All Endpoint Checks:** the button called twenty-two hand-written chains,
  six of them through a per-chain wrapper. The list predated Base, Polygon,
  Zcash, Kaspa, Dash, Decred, Bitcoin SV, Bitcoin Gold, Dogecoin, Sei and the
  newer rollups, so "all" skipped them. It is now `Chain.mainnets`, the same
  list the hub and the export bundle read. A chain with no catalog endpoints
  publishes no rows, as before.
- **Chain self-tests:** the self-test button lived inside the UTXO block, so the
  suite core keeps for *every* chain in the catalog was reachable on five chains
  — plus Ethereum, which had a second button running a near-copy of the same
  bookkeeping with three extra probes. Every chain's screen now offers the
  suite. Of the three extras, the EVM RPC probe stays and applies to the whole
  family: `self_tests_run_ethereum_rpc` compared the node's reported chain id
  with a literal `1` and labelled every row "Ethereum"; `self_tests_run_evm_rpc`
  takes the chain and compares against `Chain::evm_chain_id`, refusing a
  non-EVM chain instead of probing it. The endpoint comes from the chain's
  configured RPC or its catalog list, not from a hard-coded mainnet URL. The
  other two extras are deleted on purpose: the JSON-shape check tested core's
  own document builder, which `a_document_carries_history_and_endpoints` now
  asserts where the document is built (`core_diagnostics_json_shape_ok` is gone
  with it), and the portfolio probe was the balance refresh with a different
  error message. Check `spectra diagnostics self-test` for the offline suite and
  `cargo test -p spectra_core only_an_evm_chain_has_a_json_rpc_id_to_check` for
  the refusal; the RPC probe itself needs network.

### Networks, assets and UI

- **Presentation catalog:** token colors previously required a Swift hosting-chain
  mapping and a full unused display record; they now come directly from core's
  token catalog into a color cache. Native-chain colors still take precedence.
  This removes a platform-specific filter from asset styling. Six unused
  copy fields and their four locale variants were surplus and have been removed,
  including the outdated private-key support claim. Check catalog metadata with
  `spectra token catalog --chain Ethereum`; color lookup and resource decoding
  require the Swift tests, not the CLI.

- **Network selection:** duplicated network-mode models became registry chain
  ids in `network_chain_by_family`. Endpoint indexing separates per-network
  RPC selection from per-chain settings groups, preventing testnet/mainnet
  mixing. Check `spectra network` and iOS test
  `testEthereumTestNetworksExposeExpectedContextsAndEndpoints`.
- **Staking:** separate support lists became `Chain::supports_staking` and a
  shared service gate. Unsupported chains get a staking refusal; testnets do
  not route to mainnet validators. Async exports declare the Tokio runtime.
  Check `spectra staking validators --chain Bitcoin`, registry routing tests,
  and the staking tab through Swift bindings.
- **Endpoint catalog:** mixed `roles` became `kind` plus `capabilities` in
  `core/data/endpoints.toml`. EVM RPC nodes do not claim address-history access;
  web links are not probed. Supplemental RPC-list registration is explicit.
  Settings show available tags and include chains with actual catalog entries,
  including previously hidden Bitcoin SV. Check `spectra endpoints --chain
  Ethereum` (network); CLI acceptance checks catalog shape.
- **Endpoint health:** unchecked stale URLs became a CLI sweep, with JSON-RPC
  error-body inspection and a retry before reporting failure. Dead entries
  were removed or replaced; row-specific reasons stay in `endpoints.toml`.
  Check `spectra endpoints --json`; availability must be remeasured live.
- **EVM history:** a blanket Etherscan-key requirement and swallowed refusals
  became registry-selected open/keyed/unavailable sources. Errors are distinct
  from empty arrays, and EVM records now reach normalized history. Check
  `spectra chains --json` for source metadata and `spectra history` with a
  watch wallet on a configured open source (network).
- **Asset identity:** dashboard rows previously grouped by chain; they now
  group by asset with a per-chain breakdown. The wiki has asset pages linked to
  chain/contract information even for assets not held. Token decimals come from
  contract/account data rather than caller assumptions. Inspect
  `spectra portfolio` and token balances; wiki layout needs iOS verification.
- **Artwork:** chain-qualified identifiers hid token logos and matched token
  symbols by substring. `core_icon_asset_name` now resolves exact symbols from
  chain, token and gas-asset catalog metadata; chain badges still draw the chain.
  Both wikis use their row's asset name, and USDB artwork now ships. Check
  `cargo test -p spectra_core artwork_follows_the_coin_not_the_chain` and Swift's
  `CoinBadgeArtworkTests`, which loads the actual bundled assets. The Rust tests
  specify the resolver; the Swift test covers the original missing-image bug.
- **Purchase directory:** provider names, descriptions and links scattered
  through locale files became `resources/BuyProviders.json`, grouped into
  on-ramps and exchanges. Rows show names/domains; localized category notes
  explain custody. Banxa's FAQ link and Ramp's redirect were corrected.
  JSON ids replace per-render UUIDs to keep rows stable. Check the directory
  in iOS: MoonPay and Kraken were previously opened successfully. The reported
  “none of the links work” issue was not reproduced on the old clean simulator;
  unstable row identity remains the proposed explanation, not proven causation.

### Omnichain Tether tokens and the hosting-chain list

- **Token hosting:** `CoreTokenHostingChain` listed eighteen chains while
  `chains.toml` gives twenty-eight a `token_standard`. Berachain, Sei, Celo,
  Cronos, opBNB, zkSync Era, Sonic, Unichain, Ink and X Layer were therefore
  chains the app could select but not hold a token on: a catalog row there was
  dropped by `built_in_token_preferences`, and the "add custom token" picker
  did not offer them. The variants are now exactly that column, which is where
  the fact already lived. Balance reads need nothing new — the EVM arm of
  `fetch_token_balances` is generic over `Chain::is_evm`. Check
  `cargo test -p spectra_core token_hosting_chain_tests` and
  `spectra token catalog --chain Berachain`.
- **Polygon USDT is USDT0:** the row for `0xc2132D05…` read symbol `USDT`,
  name "Tether USD", priced from CoinGecko `tether`. Tether upgraded that
  contract in place — it answers `symbol()` with "USDT0" now, and CoinGecko
  moved it from `tether` to `usdt0` — so the row is `USDT0` and prices from
  `usdt0`. It stays enabled by default, as the same holding. Check
  `spectra token catalog --chain Polygon`.
- **New tokens:** USDT0 on Arbitrum, Optimism, Polygon, Mantle, Hyperliquid,
  Berachain, Sei, Unichain, Ink and X Layer; XAUT0 on Arbitrum, Avalanche, BNB
  Chain, Polygon, Hyperliquid, Solana, TON, Celo and Ink; USAT on Ethereum and
  Celo; BUSD (Bera USD, formerly HONEY) on Berachain. Addresses come from the
  [USDT0 deployments API](https://docs.usdt0.to/api/deployments) — the token
  contract, not the OFT adapter beside it — cross-checked against CoinGecko's
  platform lists, and `symbol()`/`decimals()` were read from the chain for the
  ones that decide a label. USDT0 is enabled by default where it is the chain's
  canonical dollar; XAUT0, USAT and the rest follow the catalog's habit of
  shipping disabled. Check `spectra token catalog --chain <chain>` and
  `cargo test -p spectra_core wiki`.
- **Swift lists that shadowed the hosting enum:** `addCustomTokenPreference`
  validated contract addresses through a switch that hand-listed twelve EVM
  chains, and `TokenRegistrySettingsView` had a parallel `TokenRegistryChainFilter`
  enum with a case per chain. Both would have silently ignored the ten chains
  the hosting list gained — the first by failing to compile, the second by
  offering eighteen networks in the filter. Validation now reads the registry's
  `addressValidationKind` for anything not named (a Sui or Aptos coin type, or a
  chain with its own wording), and the filter is `TokenHostingChain?` over
  `allCases`. The view's unused `entries(for:)` helper went with it. Check the
  Known Tokens filter and the New Token form in iOS.

### One owner for "what address is this send going to"

- **Destination resolution:** the composer's recipient probe, the EVM preview
  and the submit path each turned the destination field into an address
  themselves, and each spelled the ENS rule as `chainName == "Ethereum"`. Two
  of the three normalized a typed address and none normalized a resolved one;
  one kept a resolved-name cache and the other two re-asked the ENS API on
  every debounced keystroke. `WalletService::resolve_send_destination` is the
  one answer now: the rule is `Chain::resolves_ens_names`, the cache is the
  service's, and the address comes back normalized for the chain either way —
  so a name-reached destination compares equal to the same address in the
  address book and is no longer reported as `new_address`. Anything that is
  neither a valid address for the chain nor a name that chain registers is
  `InvalidInput`; nothing guesses a destination. Off-Ethereum `.eth` input is
  still refused rather than resolved through mainnet, which is the stricter
  reading and leaves `ens_off_ethereum` a warning nothing can currently raise.
  `is_ens_name_candidate` and `resolve_ens_name_typed` are no longer exported —
  each answered half the question — and the evaluator's inline copy of the
  `.eth` heuristic now calls the one function. CLI check: `spectra send
  destination --chain <chain> --to <input>`; `./scripts/cli-acceptance.sh`
  covers the normalized form, the empty and wrong-family refusals and the name
  refused on Arbitrum, Base, Polygon and Bitcoin without a request leaving the
  machine. `cargo test -p spectra_core destination_resolution_tests` covers the
  same plus a cache hit reporting `used_ens`. Swift
  `SendDestinationBridgeTests` runs the refusals and the normalized answer
  across the async binding, which is the half no CLI run can vouch for.

### The destination probe names an asset, not a token descriptor

- **`send_destination_risk`:** took `(chain_id, address, token: Option<TokenDescriptor>)`
  and takes `(wallet_id, holding_key, destination_input)`. Which contract a
  symbol means on a chain is a catalog question and the catalog is core's, so
  both callers were reading core's token preferences only to hand the answer
  straight back: the composer decided native vs token, built the descriptor,
  and clamped the catalog's precision into a `u8` with `UInt8(clamping:)` —
  which turns an impossible 300 into a plausible 255 and reads a balance off by
  45 decimal places. Core derives the descriptor from the holding now, and an
  out-of-range precision is a refusal rather than a clamp.
- **An unidentifiable asset is an error, not silence:** a token no preference
  row vouches for made the composer clear the probe and show nothing, which
  reads as "checked, and fine". It is now `InvalidInput` naming the asset, and
  the composer shows "Unable to verify this address's activity" — the same
  thing a failed read shows, because it is the same fact.
- **The destination is resolved rather than trusted:** the probe runs
  `resolve_send_destination` on what it is given, so it asks about the address
  a send would actually reach. For an already-resolved address that is
  re-validation and nothing more.
- CLI check: `spectra send probe --wallet <w> [--asset SYM] [--chain C] --to <input>`;
  the descriptor flags are gone. `./scripts/cli-acceptance.sh` covers the
  missing wallet, the asset the wallet does not hold and the unknown chain —
  the verdict itself needs a balance and a history read, and a holding only
  exists after one. `cargo test -p spectra_core a_destination_probe_refuses_before_it_guesses`
  covers the unfindable holding and the unvouched token; the existing mock-RPC
  probe tests now seed a wallet holding. Swift
  `SendDestinationBridgeTests.testAProbeForAnUnknownHoldingThrowsRatherThanProbingAcrossAsyncBinding`
  checks the refusal across the async binding.

### A fallback price quote names the asset it prices

- **CoinPaprika and CoinLore ids are catalog columns:** `price.rs` carried two
  hand-written tables — 38 gecko-id and 32 symbol entries — mapping a third of
  the token catalog to CoinPaprika ids, plus a `match` for CoinLore's one
  `nameid` exception. They are `coinpaprika_id` and `coinlore_nameid` in
  `tokens.toml` and `chains.toml` now, beside the `coingecko_id` they belong
  with, and `price.rs` knows no asset by name. Three of the retired ids
  resolved to nothing at CoinPaprika (`aave-aave`, `cro-cronos`,
  `leo-unus-sed-leo`), and 23 of the 35 tokens were in neither table —
  including USDT0, USAT and XAUT0.
- **Nothing matches a quote to a holding by ticker symbol:** both fallback
  providers took the first listing sharing the holding's symbol when the id
  lookup missed. CoinPaprika lists several thousand coins, so BUSD — Bera USD
  here — priced as Binance USD, and TON priced as TONToken. A ticker is not an
  identity, and this is the funds path: an asset the catalog cannot name at a
  provider now goes unpriced there rather than being quoted as something else.
  The other two providers still answer for it, and an unpriced asset keeps its
  last known price.
- **Empty is a decision:** `coinpaprika_id = ""` means "not listed there under
  an identity we verified", and one row says it — BUSD, because ours is Bera
  USD and paprika's BUSD is Binance USD.
  `only_deliberately_unlisted_assets_have_no_paprika_id` holds the list to that
  one, so adding a token is a decision about where it is priced rather than a
  blank nobody notices.
- **`PriceRequestCoin` lost `symbol`:** it existed for the symbol fallback and
  nothing else reads it, so the record is `{holding_key, coin_gecko_id}` and
  the front ends stop sending a field that could only be used to guess.
- The ids are off the FFI records for the reason `ChainWikiEntry` exists: no
  front end prices anything, so `chains::native_market_ids()` and
  `tokens::market_ids()` serve core, and `ChainEntry` and `TokenEntry` are
  unchanged.
- CLI check: `cargo test -p spectra_core market_id_tests` — the catalogs agree
  where a gecko id repeats, no two assets claim one listing, ids are plain
  lowercase, and `paprika_id_for` resolves the ones no table would have
  guessed (`aave-new`, `cro-cryptocom-chain`, `bttc-bittorrent-chain`, bare
  `usat`) while answering `None` for unlisted, unknown and empty alike. The
  quotes themselves are a live provider read, so `spectra price <chain>` is the
  online check.

### Chain tokens: ARB, OP and the rest of the L2 slate

- **New tokens:** ARB on Arbitrum and Ethereum, OP on Optimism, LINEA on Linea
  and Ethereum, SCR on Scroll, BLAST on Blast, ZK on ZKsync Era and Ethereum,
  UNI on Unichain, and the Ethereum contracts for MNT and POL — gas tokens the
  wiki already carried as native coins, which now list the L1 contract holding
  effectively all of their supply, the same shape CRO has. Every contract was
  read on-chain for `symbol()` and `decimals()` before being written down; all
  are 18 decimals. They ship disabled, as every non-dollar built-in does.
  Check `spectra token catalog --chain Arbitrum`.
- **Polygon priced MATIC, not POL:** `native_coingecko_id` was `matic-network`,
  which CoinGecko now titles "MATIC (migrated to POL)" and lists on no chain,
  while the `native_coinpaprika_id` beside it already said
  `pol-polygon-ecosystem-token`. The two disagreed about which coin Polygon
  runs on, and the ids quote different prices — $0.126 against $0.092 on the
  same balance. It is `polygon-ecosystem-token` now, which is also what the new
  POL token row must say for
  `a_symbol_has_one_market_data_id_across_both_catalogs` to hold. Check
  `spectra price --chain Polygon` (network).
- **Scroll's ticker:** the chain's `symbol` read `SCRL`, which is Wizarre
  Scroll — an unrelated token. Scroll's own ticker is `SCR`, and that is now
  both the chain's symbol and the token's. Check `spectra chains`.
- **Not added, and why:** Base has no token; Ink's does not appear in either
  price catalog, so a row for it would carry no market id and the wiki refuses
  that; opBNB runs on BNB and X Layer on OKB, which are native coins already —
  and OKB's Ethereum contract holds 429,065 of a ~21M supply after the X Layer
  migration, so it is a remnant rather than the token. BERA, CELO, SEI, S, CRO,
  AVAX and HYPE are gas tokens the registry already carries as native coins.

### Held assets: wrapped, staked and protocol tokens

- **A coin may ship without a mark.** `every_wiki_coin_has_artwork` asserted
  that every catalog row names artwork *and* that the file exists, which is a
  stricter rule than the bug it was written for — that bug was 31 of 66 coins
  resolving to *someone else's* mark, and the equality assertion is what caught
  it. A row may now name nothing: `CoinBadge` already draws the coin's letter
  for an empty name, and `an_unknown_symbol_resolves_to_nothing` is the other
  half of that contract. The test keeps the equality, `every_named_mark_ships_a_file`
  skips unnamed rows, and a new `every_chain_names_a_mark` holds the old rule
  where it still belongs — a chain drawn as a letter in the network picker is a
  hole, not a pending drawing. Swift's `CoinBadgeArtworkTests` gained the same
  branch. Check `cargo test -p spectra_core artwork_follows_the_coin_not_the_chain`.
- **23 new tokens, 66 deployments.** Wrapped and staked forms of coins already
  in the catalog — stETH, wstETH, WETH, cbBTC, LBTC, rETH, sUSDS, sUSDe — and
  the tokens of protocols people hold balances in: GHO, ONDO, MORPHO, AERO,
  CRV, CAKE, PENDLE, ETHFI, LDO, ENS, PYTH, RAY, ZRO, WLD, PEPE. Base carried
  two tokens before this and carries fifteen now; Solana gained six. Every one
  of the 66 contracts was read on-chain for `symbol()` and `decimals()` before
  being written down, which is how the per-chain decimals came out right —
  cbBTC and LBTC are 8, and sUSDe and CAKE are 9 on Solana against 18 on the
  EVM chains. All ship disabled. Check `spectra token catalog --chain Base`.
- **Which chains each token lands on:** its home chain, then the largest venues
  CoinGecko lists it on, capped at four. sUSDe is on seventeen chains Spectra
  supports and PENDLE on eight; carrying every one would add rows for places
  the token barely trades, and a holder there can still add it as a custom
  token. The cap is a judgement call, not a fact — it is written down here so
  it can be revisited rather than rediscovered.
- **Symbols are uppercase**, as all 43 existing rows are: `STETH`, not `stETH`.
  The contracts say `stETH`, `cbBTC`, `sUSDe`; the catalog has never carried a
  mixed-case symbol and `holding_identity` keys on the string, so matching the
  file beats matching the brand.
- **Not added:** permissioned RWA funds — BUIDL ($2.8B), USYC, USTB, JTRSY,
  JAAA, EUTBL, OUSG, YLDS, BCAP, FIGR_HELOC — transfer only between whitelisted
  addresses, so a self-custody wallet cannot hold them. RAIN ranks 13th by
  market cap at $11.15B and trades $34M a day, a ratio roughly fifty times
  worse than anything else considered.

### The catalog names an asset and the registry answers for a chain

- **History rows are named from the token catalog:** a four-entry
  `tron_asset_name` table gave TRX, USDT, USDC and BTT their display names on
  Tron — the same strings `tokens.toml` carries, for four of the tokens it
  carries, and USDC is not deployed on Tron at all. The row's own ticker is
  looked up in the catalog for the chain it arrived on now, so every token on
  every chain has its name, and a ticker the catalog does not carry stays the
  ticker rather than being invented. The three-armed `SymbolOverride` is two:
  Solana and Tron were "a row names its own asset" spelled twice, so an SPL
  USDC transfer now reads "USD Coin" rather than "USDC", as the Tron rows
  already did.
- **A chain has one name for its coin:** `history_chain_meta` overrode four
  chains with "history-specific" names. Two (`Toncoin`, `Internet Computer`)
  were what the catalog already said. The other two were a second source of
  truth for one asset's name, so history rows read "Stellar Lumens" and "NEAR
  Protocol" while every other screen read "Stellar" and "NEAR"; they read the
  catalog's name now. CLI check:
  `cargo test -p spectra_core every_chain_shape_normalizes_to_its_expected_row`,
  which gained a TRC-20 the old table could not name and a ticker nothing can.
- **The receive message asks the registry, not the symbol:** the input carried
  the coin's symbol and an `is_evm_chain` flag Swift computed from its own
  catalog, and the branches matched `symbol == "BTC"` and `("BCH", "Bitcoin
  Cash")` pairs. It takes the chain name, resolves it once, and reads the
  family off `supports_deep_utxo_discovery` and `is_evm` — so the testnets of
  Bitcoin Cash, Bitcoin SV, Litecoin and Dogecoin, which matched none of those
  pairs and were told "Receive is not enabled for this chain", get their
  family's message, and a testnet names itself rather than its mainnet. The
  per-chain hint copy stays a table, keyed by `Chain` so that renaming a chain
  in the catalog cannot silently drop it. An address of blanks is no longer
  returned as the receive address. iOS passes the shown chain's watch address
  rather than Dogecoin's, which is what the flag was always named. CLI check:
  `cargo test -p spectra_core receive::tests`.
- **NEAR's gas floor is core's:** a NEP-141 send refused below `0.001` NEAR,
  written into the iOS submit branch beside the balance it compared. It is
  `Chain::token_send_gas_reserve` and rides on the preflight, so the number is
  stated once, for the chain it is about, and only on the path that needs it —
  a native send pays its fee from the amount it moves. The three gas-balance
  lookups that spelled out `("Tron", "TRX")`, `("Solana", "SOL")` and
  `("NEAR", "NEAR")` read the catalog's pair. CLI check:
  `cargo test -p spectra_core a_near_token_send_carries_the_chains_gas_floor`.
- **The dashboard's default pins are core's:** iOS held `["BTC", "ETH",
  "USDT", "USDC"]`, so a fresh wallet's pin cards showed four assets that
  core's own grouping did not mark pinned, did not order first, and gave no row
  to when the wallet held none of them. `AppSettings::pinned_dashboard_assets`
  answers with `DEFAULT_PINNED_DASHBOARD_ASSETS` while nothing is pinned, and
  both sides read that. Storage is unchanged: empty still means "not chosen",
  and clearing the pins is still a real change with an event. CLI check:
  `cargo test -p spectra_core an_unpinned_dashboard_reads_as_the_default_four`.
- **Symbols that were chain facts:** the Tron send preview's `symbol == "TRX"`,
  and `supports_solana_send` / `supports_near_token_send` testing `"Solana"`,
  `"SOL"`, `"NEAR"`, now read `chain_display_name`, `coin_symbol` and
  `str_id` off the registry entry. No behaviour change — the strings agreed
  with the catalog. They are the ones that would not have, later.

### The receive screen asks where an address comes from, not which chain it is

- **`ReceiveAddressResolverKind` is `ReceiveAddressSource`:** twenty-five
  variants, one per chain, picked from a table of `(symbol, chain display
  name, is_evm_chain)` — and the single caller switched on four of them and
  sent two of those four down the same path. The chains a variant named have
  nothing in common but the rule they land on, so the enum is the three rules:
  the wallet's stored Bitcoin account address, the address stored for the
  chain, or nothing. `core_receive_address_source` takes the chain name alone.
- **A ticker is not a chain:** the table's first arm was `("BTC", _)`, so any
  holding whose symbol read BTC — on any chain — was shown the wallet's
  Bitcoin address as its receive address. Nine other arms required the ticker
  to agree with the chain (`("ZEC", "Zcash")`, `("LTC", "Litecoin")`, …), so a
  holding whose symbol did not spell its chain's resolved to no address at all
  and the screen said receive was not enabled. What a receive address is read
  from is a fact about the chain, and a token is received at the same address
  as its chain's coin.
- **`is_evm_chain` is gone from this boundary too:** Swift passed its own
  answer and core dispatched the whole EVM family off it. The family reads the
  registry's `address_slot`, which already says the EVM chains share
  Ethereum's, so the EVM arm and the general one were the same address by two
  routes. iOS no longer computes `isEvmChain` here at all.
- CLI check: `cargo test -p spectra_core a_receive_address_source_is_the_chains_rule_not_its_ticker`,
  which also holds every chain in the registry to answering a rule — Dogecoin's
  family aside, which is the one deliberate "nothing to read".

### A failed address lookup is not an empty wallet

- **Self-send confirmation:** `knownUTXOAddresses` returned `(try? …) ?? []`,
  so a failed read of a wallet's Dogecoin addresses arrived at the self-send
  guard as "this wallet owns no addresses" and the guard waved the send
  through. The lookup now returns `[String]?` and the guard treats `nil` as
  unknown ownership: it asks for confirmation and says why, rather than
  assuming the destination is not yours. Ownership that cannot be established
  is the stricter side, and the cost of asking is a tap.
- **Discovery caching:** `refreshUTXOAddressDiscovery` wrote whatever the
  discovery returned, so one failed refresh replaced a wallet's previously
  discovered addresses with an empty list until the next successful one. A
  wallet whose discovery fails now keeps what it had.
- Both failures are recorded under the `Owned Addresses` operational-log
  category instead of being dropped, so a repeated failure is visible in a
  diagnostics export rather than only in its consequences.
- No FFI record or exported method signature changes; `knownUTXOAddresses` and
  `discoverUTXOAddresses` are Swift-side wrappers. CLI check: the core reads
  behind them already propagate errors — `cargo test -p spectra_core
  pending_status::tests` covers the same "error is not an empty success" rule
  on the storage reads this pair calls.

### A network switch takes the chain's derivation state with it

- **One operation, one transaction:** `delete_keypool_for_chain` and
  `delete_owned_addresses_for_chain` are replaced by
  `reset_chain_derivation_state`, which takes both tables in a single SQLite
  transaction and drops the in-memory copies with them. iOS used to issue the
  two deletes itself, each swallowing its own error, so a failure on the second
  left a keypool handing out indices for addresses the new network never
  derived while the addresses it did derive were attributed to a network they
  did not come from. Half-applied is the one outcome this cannot have, and no
  sequence on the caller's side can prevent it; the guarantee only exists where
  the transaction does.
- **The CLI drives the switch, not just the setting:** `spectra network set`
  applied `SelectNetworkChain` and stopped, so the reset existed only on iOS
  and this axis could not see it. It now performs the same reset and reports
  `clearedDerivationState`. Both sides of a family are cleared — the network
  being left holds indices that no longer describe anything, and the one being
  entered may hold stale rows from the last time it was selected.
- **Exports removed:** the two superseded methods are deleted rather than kept
  beside the combined one; they had a single caller between them. FFI methods
  go from 101 to 100.
- CLI checks: `cargo test -p spectra_core chain_derivation` covers the
  transaction taking both tables and leaving other chains alone, and a no-op
  delete for a chain nothing derived on. The offline acceptance gate asserts
  `clearedDerivationState` on a real switch.

### The tracked-token list is edited by intent, not replaced wholesale

`SetTokenPreferences { entries }` — replace the list with the one the caller
built — is gone. Five intents took its place: `AddCustomToken`,
`RemoveCustomToken`, `SetCustomTokenDecimals`, `SetTokenPreferencesEnabled`,
`ResetTokenPreferences`, plus `MergeBuiltInTokens` for core's own catalog fold.
Refusals arrive as a `tokenPreferenceRejected` event carrying a
`TokenPreferenceRejection`, the way the address book already worked; each front
end supplies the wording.

- **The rules were the caller's, in two disagreeing copies.** Swift validated
  the symbol, judged the contract with a seven-arm switch over hosting chains,
  checked for a duplicate by normalized contract, clamped the decimals and
  re-sorted; the CLI checked none of that and called a duplicate a matching
  *symbol*, case-insensitively. So a Solana mint could be added to the Base
  list from the command line, and two contracts sharing a symbol on one chain
  were a duplicate there and not in the app. One rule now, in the reducer.
- **A contract is judged by the chain that would host it:**
  `CoreTokenHostingChain::contract_validation_kind`. Swift's switch named six
  chains and let a `default` arm assume EVM, which is how the ten hosting
  chains added with the omnichain work would have been validated by whichever
  arm was written last. Sui gets a real validator: `"suiCoinType"` accepts a
  package address or `address::module::NAME`, where the composer's
  `hasPrefix("0x") && (contains("::") || count > 2)` accepted `0xzz` and
  `0x::::`. Check `cargo test -p spectra_core validates_sui_coin_types`.
- **An impossible precision is refused, not clamped.** `min(max(decimals, 0), 30)`
  in Swift and `.min(MAX_TOKEN_DECIMALS)` in the reducer both stored a number
  the user did not type, and every later balance read at that scale. Over 30
  places is now `tooManyDecimals` and nothing is stored. Check
  `cargo test -p spectra_core an_impossible_precision_is_refused_rather_than_clamped`.
- **Opening seeds the catalog.** `open_state` applies `MergeBuiltInTokens` and
  persists the result, so the stored list is always the catalog plus what the
  user added. The app happened to ask for the merge and the CLI never did, so
  `spectra token track` had no row to turn on and `token track` had grown into
  "append a built-in entry" — a second model of tracking. Tracking is
  `is_enabled` on a row that always exists; untracking keeps the row, which is
  what carries the choice through the next merge.
- **Swift's `tokenPreferences` is a projection**, `private(set)` like
  `coreAddressBook`, and the debounced whole-list commit is gone with
  `commitTokenPreferences`. `AppState.addCustomTokenPreference` is `async` now:
  the answer is core's, so it arrives with the state it changed.
- **Wording:** the two chain-named refusals ("%@ already knows this token.",
  "Enter a valid %@ token contract address.") became chain-agnostic sentences,
  since the reason core reports does not name a chain and the form's own chain
  picker is beside the field. Eight per-chain contract-hint strings went with
  them; eight reason strings replace them in all three locale files.
- CLI check: `spectra token add|remove|decimals|reset` and
  `spectra token track|untrack --chain <c> <SYM>`.
  `./scripts/cli-acceptance.sh` covers the seeded catalog, tracking as a flag
  that survives a new process, the cross-family contract in both directions,
  the case-insensitive duplicate, the pasted symbol, both precision refusals
  and the reset. `cargo test -p spectra_core store::state::tests` covers the
  reducer; Swift `TokenPreferenceBridgeTests` covers the same across the async
  binding.

Follow-up completed in the ten Stage 3 / C2 slices above: the duplicate
`TokenPreferenceEntry.builtIn` builder and startup fallback are removed.
Known Tokens shows loading until core's seeded catalog arrives.
