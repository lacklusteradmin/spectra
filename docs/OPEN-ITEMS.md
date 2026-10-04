# Open items

Engineering work that remains. Rule 0 in [AGENTS.md](../AGENTS.md) applies.
Delete an item once it is done; what changed belongs in
[BEHAVIOUR-CHANGES.md](BEHAVIOUR-CHANGES.md).

## Tasks

- [ ] **Decode complete actual OP Stack receipt fees.** World Chain and the
  other registry-marked OP Stack networks reserve reviewed execution, L1 data
  and applicable operator fees before signing and broadcasting. Their receipts
  currently expose only execution gas to the service, so confirmed receipt cost
  details are intentionally omitted rather than label a subtotal as the total
  network fee. Extend the existing `evm_json_rpc` receipt decoder using verified
  historical L1/operator fields and the network's deployed fee model; require
  all applicable components before publishing a total. Missing components must
  remain unknown, not become zero. Cover success and revert receipts, older
  fee models and absent/malformed data through the service and CLI. The reviewed
  maximum budget is an estimate and cannot stand in for actual confirmed cost.
- [ ] **Certify the 59 token contract identities still missing primary evidence.**
  The [exact deployment list](#token-contract-provenance-59-deployments)
  contains nine stablecoin and 50 other EVM deployments from the token audit.
  Prove each network/address pair from the issuer or official bridge, including
  proxy and issuance identity; retain pinned source evidence and a dated RPC
  cross-check. Code, symbol and decimals alone cannot certify an issuer.
- [ ] **Add Monad address history and automatic token discovery.** Monad
  mainnet, native MON, CAKE and USDC are supported, but its catalog does not
  claim an address indexer. On 2026-10-04, Routescan's chain-143 account
  `txlist` returned `chain not supported`; the public Etherscan v2 account
  API returned `Missing/Invalid API Key`. The
  [official provider directory](https://docs.monad.xyz/tooling-and-infra/indexers/common-data)
  lists SQD's public Portal. Its Monad dataset at
  `https://portal.sqd.dev/datasets/monad-mainnet` answered `/metadata`,
  `/head` and a bounded two-block `/stream` read without a key. See
  [the dataset](https://docs.sqd.dev/en/data/evm/monad-mainnet) and
  [stream schema](https://docs.sqd.dev/en/api/evm/stream). Integrating this
  JSONL block-range API needs its own core API adapter and explicit scan,
  pagination and history-completeness rules; it cannot be configured as an
  Etherscan-style account indexer. Design token discovery from indexed
  transfers plus current on-chain balances, and only declare capabilities
  after adapter and CLI coverage. The present RPCs still read tracked token
  balances and verify submitted transaction receipts.
- [ ] **Canonicalize TON token master account identity.** The same jetton
  master can be supplied as raw, bounceable-friendly or non-bounceable-friendly
  addresses. Token identity currently preserves those spellings, while the
  balance reader resolves them to the same account. Normalize to one account
  identity before catalog/custom-token duplicate checks and holding persistence
  so address aliases cannot create duplicate tracked holdings. Preserve the
  testnet flag check before normalization; update catalog identifiers and
  stored shapes directly, without migration shims.
- [ ] **Detect FFI record fields with no production writer.** A syntactic scan
  cannot reliably distinguish unwritten fields from serde or multi-line writes.
  A useful gate needs type-aware analysis before it can reject unused fields.
- [ ] **Staking transaction execution.** The app currently provides information
  and validator queries. Transaction execution remains future work.
- [ ] **Staking reads go through the API adapters.** AGENTS.md puts an API's
  network I/O in `core/src/api/<api>.rs`; `staking/` predates that rule.
  `SolanaStakingClient`, `SuiStakingClient`, `NearStakingClient` and
  `AptosStakingClient` each build their own JSON-RPC or REST request and race
  their own endpoint list in `fetch_validators` (`getVoteAccounts`,
  `suix_getLatestSuiSystemState`, `validators`, the Aptos validator-set
  resource). Move each request and its response type into the matching adapter
  (`solana_json_rpc`, `sui_json_rpc`, `near_json_rpc`, `aptos_rest`) as a client
  method, and have `staking/` take that client and keep only the staking
  decisions: ranking, commission and APY projection, position shaping.
  ICP's static NNS neuron directory and Polkadot's unwired Sidecar queries
  make no network requests. Record any change in what a validator query returns
  in [BEHAVIOUR-CHANGES.md](BEHAVIOUR-CHANGES.md) and pass `make verify`.
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

## Token contract provenance (59 deployments)

The 2026-10-03 audit flags these exact 59 EVM deployments: nine stablecoin
and 50 other deployments. All 59 still match the current catalog. Each lacks
recorded issuer/bridge identity proof; this is not a finding that all these
addresses are wrong. This list is the retained record of the remaining work.

Each address had deployed code and matching decimals in the recorded RPC read,
with network chain ID, block and code hash. Of the metadata results, 58 are
`metadata_match`; Avalanche DAI is `symbol_difference` (`DAI.e` on-chain).
Those reads do not establish who issued a token or which bridge representation
it is. CoinGecko/CoinPaprika listings and explorer labels alone do not close
this task.

For each row, obtain an issuer or official bridge reference that explicitly
identifies the network and exact address. Record native/bridged issuance and
proxy versus implementation, pin the source revision or capture its contents
and hash, and cross-check code and metadata at a recorded block. Record the
result and source reference here as each row is resolved. Close a row only
with that identity evidence, or an explicit decision to remove the unverified
built-in deployment; another decimals/symbol match is insufficient.

### Stablecoin gaps (9)

| # | Asset | Network | Standard | Contract |
| --- | --- | --- | --- | --- |
| 1 | DAI (`dai`) | `base` | ERC-20 | `0x50c5725949a6f0c72e6c4a641f24049a917db0cb` |
| 2 | DAI (`dai`) | `polygon` | ERC-20 | `0x8f3cf7ad23cd3cadbd9735aff958023239c6a063` |
| 3 | DAI (`dai`) | `avalanche` | ARC-20 | `0xd586e7f844cea2f87f50152665bcbc2c279d8d70` |
| 4 | DAI (`dai`) | `linea` | ERC-20 | `0x4af15ec2a0bd43db75dd04e62faa3b8ef36b00d5` |
| 5 | DAI (`dai`) | `zksync-era` | ERC-20 | `0x4b9eb6c0b6ea15176bbf62841c6b2a8a398cb656` |
| 6 | DAI (`dai`) | `unichain` | ERC-20 | `0x20cab320a855b39f724131c69424240519573f81` |
| 7 | DAI (`dai`) | `celo` | ERC-20 | `0xac177de2439bd0c7659c61f373dbf247d1f41abe` |
| 8 | SUSDS (`susds`) | `avalanche` | ARC-20 | `0xb94d9613c7aab11e548a327154cc80eca911b5c1` |
| 9 | USDS (`usds`) | `avalanche` | ARC-20 | `0x86ff09db814ac346a7c6fe2cd648f27706d1d470` |

### Other token gaps (50)

| # | Asset | Network | Standard | Contract |
| --- | --- | --- | --- | --- |
| 10 | POL (`polygon-ecosystem-token`) | `ethereum` | ERC-20 | `0x455e53cbb86018ac2b8092fdcd39d8444affc3f6` |
| 11 | MNT (`mantle`) | `ethereum` | ERC-20 | `0x3c3a81e81dc49a522a592e7622a7e711c06bf354` |
| 12 | CRO (`crypto-com-chain`) | `ethereum` | ERC-20 | `0xa0b73e1ff0b80914ab6fe0444e65848c4c34450b` |
| 13 | AERO (`aerodrome-finance`) | `base` | ERC-20 | `0x940181a94a35a4569e4529a3cdfb74e38fd98631` |
| 14 | ARB (`arbitrum`) | `ethereum` | ERC-20 | `0xb50721bcf8d664c30412cfbc6cf7a15145234ad1` |
| 15 | ARB (`arbitrum`) | `arbitrum` | ERC-20 | `0x912ce59144191c1204e64559fe8253a0e49e6548` |
| 16 | BGB (`bitget-token`) | `ethereum` | ERC-20 | `0x54d2252757e1672eead234d27b1270728ff90581` |
| 17 | BLAST (`blast`) | `blast` | ERC-20 | `0xb1a5700fa2358173fe465e6ea4ff52e36e88e2ad` |
| 18 | CRV (`curve-dao-token`) | `celo` | ERC-20 | `0x75184c282e55a7393053f0b8f4f3e7beae067fdc` |
| 19 | CRV (`curve-dao-token`) | `mantle` | ERC-20 | `0xe265fc71d45fd791c9ebf3ee0a53fbb220eb8f75` |
| 20 | CRV (`curve-dao-token`) | `x-layer` | ERC-20 | `0x3d5320821bfca19fb0b5428f2c79d63bd5246f89` |
| 21 | ENS (`ethereum-name-service`) | `ethereum` | ERC-20 | `0xc18360217d8f7ab5e7c516566761ea12ce7f9d72` |
| 22 | ETHFI (`ether-fi`) | `ethereum` | ERC-20 | `0xfe0c30065b384f05761f15d0cc899d4f9f9cc0eb` |
| 23 | ETHFI (`ether-fi`) | `arbitrum` | ERC-20 | `0x7189fb5b6504bbff6a852b13b7b82a3c118fdc27` |
| 24 | ETHFI (`ether-fi`) | `base` | ERC-20 | `0x6c240dda6b5c336df09a4d011139beaaa1ea2aa2` |
| 25 | ETHFI (`ether-fi`) | `scroll` | ERC-20 | `0x056a5fa5da84ceb7f93d36e545c5905607d8bd81` |
| 26 | ETHFI (`ether-fi`) | `optimism` | ERC-20 | `0xe0080d2f853ecddbd81a643dc10da075df26fd3f` |
| 27 | KCS (`kucoin-shares`) | `ethereum` | ERC-20 | `0xf34960d9d60be18cc1d5afc1a6f012a723a28811` |
| 28 | LDO (`lido-dao`) | `arbitrum` | ERC-20 | `0x13ad51ed4f1b7e9dc168d8a00cb3f4ddd85efa60` |
| 29 | LDO (`lido-dao`) | `optimism` | ERC-20 | `0xfdb794692724153d1488ccdbe0c56c252596735f` |
| 30 | LDO (`lido-dao`) | `polygon` | ERC-20 | `0xc3c7d422809852031b44ab29eec9f1eff2a58756` |
| 31 | LEO (`leo-token`) | `ethereum` | ERC-20 | `0x2af5d2ad76741191d15dfe7bf6ac92d4bd912ca3` |
| 32 | LINEA (`linea`) | `ethereum` | ERC-20 | `0x1789e0043623282d5dcc7f213d703c6d8bafbb04` |
| 33 | LINEA (`linea`) | `linea` | ERC-20 | `0x1789e0043623282d5dcc7f213d703c6d8bafbb04` |
| 34 | ONDO (`ondo-finance`) | `ethereum` | ERC-20 | `0xfaba6f8e4a5e8ab82f62fe7c39859fa577269be3` |
| 35 | OP (`optimism`) | `optimism` | ERC-20 | `0x4200000000000000000000000000000000000042` |
| 36 | PAXG (`pax-gold`) | `ethereum` | ERC-20 | `0x45804880de22913dafe09f4980848ece6ecbaf78` |
| 37 | PEPE (`pepe`) | `ethereum` | ERC-20 | `0x6982508145454ce325ddbe47a25d4ec3d2311933` |
| 38 | PEPE (`pepe`) | `arbitrum` | ERC-20 | `0x25d887ce7a35172c62febfd67a1856f20faebb00` |
| 39 | PEPE (`pepe`) | `avalanche` | ARC-20 | `0xa659d083b677d6bffe1cb704e1473b896727be6d` |
| 40 | PEPE (`pepe`) | `bnb` | BEP-20 | `0x25d887ce7a35172c62febfd67a1856f20faebb00` |
| 41 | RETH (`rocket-pool-eth`) | `ethereum` | ERC-20 | `0xae78736cd615f374d3085123a210448e74fc6393` |
| 42 | RETH (`rocket-pool-eth`) | `arbitrum` | ERC-20 | `0xec70dcb4a1efa46b8f2d97c310c9c4790ba5ffa8` |
| 43 | RETH (`rocket-pool-eth`) | `optimism` | ERC-20 | `0x9bcef72be871e61ed4fbbc7630889bee758eb81d` |
| 44 | RETH (`rocket-pool-eth`) | `base` | ERC-20 | `0xb6fe221fe9eef5aba221c348ba20a1bf5e73624c` |
| 45 | SCR (`scroll`) | `scroll` | ERC-20 | `0xd29687c813d741e2f938f4ac377128810e217b1b` |
| 46 | SHIB (`shiba-inu`) | `ethereum` | ERC-20 | `0x95ad61b0a150d79219dcf64e1e6cc01f0b64c4ce` |
| 47 | SKY (`sky`) | `ethereum` | ERC-20 | `0x56072c95faa701256059aa122697b133aded9279` |
| 48 | WBTC (`wrapped-bitcoin`) | `ethereum` | ERC-20 | `0x2260fac5e5542a773aa44fbcfedf7c193bc2c599` |
| 49 | WETH (`weth`) | `ethereum` | ERC-20 | `0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2` |
| 50 | WLD (`worldcoin-wld`) | `ethereum` | ERC-20 | `0x163f8c2467924be0ae7b5347228cabf260318753` |
| 51 | WLD (`worldcoin-wld`) | `optimism` | ERC-20 | `0xdc6ff44d5d932cbd77b52e5612ba0529dc6226f1` |
| 52 | WLFI (`world-liberty-financial`) | `ethereum` | ERC-20 | `0xda5e1988097297dcdc1f90d4dfe7909e847cbef6` |
| 53 | WLFI (`world-liberty-financial`) | `bnb` | BEP-20 | `0x47474747477b199288bf72a1d702f7fe0fb1deea` |
| 54 | ZK (`zksync`) | `ethereum` | ERC-20 | `0x66a5cfb2e9c529f14fe6364ad1075df3a649c0a5` |
| 55 | ZK (`zksync`) | `zksync-era` | ERC-20 | `0x5a7d6b2f92c77fad6ccabd7ee0624e64907eaf3e` |
| 56 | ZRO (`layerzero`) | `ethereum` | ERC-20 | `0x6985884c4392d348587b19cb9eaaf157f13271cd` |
| 57 | ZRO (`layerzero`) | `arbitrum` | ERC-20 | `0x6985884c4392d348587b19cb9eaaf157f13271cd` |
| 58 | ZRO (`layerzero`) | `base` | ERC-20 | `0x6985884c4392d348587b19cb9eaaf157f13271cd` |
| 59 | ZRO (`layerzero`) | `bnb` | BEP-20 | `0x6985884c4392d348587b19cb9eaaf157f13271cd` |

## Known limitations

- **TRC-10 execution:** token identity can express and persist Tron TRC-10
  asset IDs alongside TRC-20 addresses. TRC-10 metadata, balance reads and
  transfers are not implemented; core refuses those operations before making
  chain requests. Do not route a TRC-10 asset ID through the TRC-20 adapter.
- **Endpoint availability and redundancy:** the 2026-09-23 live audit removed
  failed built-in providers instead of retaining broken fallbacks. Zcash,
  Bitcoin Gold, Dash, Dogecoin testnet and Monero stagenet now have no built-in
  API; supported custom nodes remain configurable. Tron PublicNode uses its
  verified `/jsonrpc` path. Diagnostics probe actual read methods on the selected
  endpoint, including testnets and catalogued EVM history sources. Passing these
  checks does not prove every advertised capability or transaction broadcast.
- **EVM history availability:** only verified keyless indexers are configured.
  BNB Chain, Sonic, opBNB, Sei, Linea, Hyperliquid, Cronos, X Layer and
  Berachain have no built-in history source;
  Etherscan V2 and its API-key setting were removed by user request.

- **Keyless provider policy:** API-key configuration and authenticated provider
  adapters are removed. Polkadot/Westend and Bittensor read balances from
  `System.Account` storage over their own RPC; their history remains
  unavailable until a keyless indexer exists. Blockchair, SoChain and Trezor's
  Blockbooks refuse keyless clients and are not candidates. Cardano staking
  queries are unavailable; its keyless Koios broadcast is implemented.
