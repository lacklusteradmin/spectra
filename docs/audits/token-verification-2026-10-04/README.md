# Reverification of 59 preserved token deployments

Checked on 2026-10-04. The earlier audit recorded missing retained issuer/bridge evidence; it did not establish that the original contracts were wrong. This follow-up examines every one of those deployments, including official announcements, issuer repositories, canonical bridge token lists and live bridge mappings.

**58 deployments are verified and active again. The unresolved Avalanche PEPE deployment was removed at the user's explicit request.** The reactivated records retain their original addresses, standards and decimals. The 21 previously commented token identities and 21 wiki descriptions are active again, including PEPE on Ethereum, Arbitrum and BNB.

Every original contract passed a read-only network-ID, nonempty-bytecode and ERC-20 decimals check at a fixed block. Metadata is supplemental evidence: a symbol or precision does not authenticate a token. GitHub sources are pinned to reviewed commits where available, and the evidence records distinguish issuer deployments from bridge wrappers.

## Evidence

- [Complete decisions for all 59 deployments](verification.json).
- [Issuer assets](issuer-assets.json), [DeFi and canonical bridges](defi-bridges.json), and [multichain assets](multichain-assets.json): exact original address, primary sources, excerpts, bridge version and precision evidence.
- [All 59 fixed-block metadata checks](rpc-metadata.json).
- [Offline CLI catalog checks](cli-catalog.json): 58 exact active matches and the excluded Avalanche deployment across 15 networks, recorded before its commented source entry was removed.
- [zkSync DAI bidirectional bridge mapping](zksync-dai-bridge.json) and [Ethereum ZK official Vault registration](zksync-zk-bridge.json).
- [Official LayerZero metadata excerpts](official-layerzero-metadata.json) and [OFT registry excerpts](official-oft-registry.json).

## Removed deployment

PEPE on Avalanche at `0xa659d083b677d6bffe1cb704e1473b896727be6d` is an ERC-20 OFT v2 wrapper. [Reciprocal peers and the actual Ethereum adapter](pepe-avalanche-bridge-readings.json) connect it to Ethereum PEPE at `0x6982508145454ce325ddbe47a25d4ec3d2311933`. That proves the underlying asset relationship. It does not establish issuer or official bridge operator identity: the actual adapter is absent from the reviewed official LayerZero metadata and has a different owner from the registered PEPE bridges. [Operator readings](pepe-bridge-operator-readings.json) record this difference. The issuer-linked bridge page was unavailable, and no reviewed issuer, Avalanche bridge or official token-list source authenticated this exact deployment.

The user explicitly requested deletion after reviewing this unresolved provenance question. The commented TOML deployment and its open verification item have been removed. Its evidence remains here as research history; no alternate address is substituted. This is not a finding that the asset is counterfeit.

## Findings worth retaining

- Celo and Mantle CRV originals are listed by their official native bridge directories. Other addresses in Curve SDK belong to other deployments and do not justify replacing these records.
- Optimism ETHFI at `0xe0080d2f853ecddbd81a643dc10da075df26fd3f` is a token on that network. The same address on Ethereum is an adapter; official LayerZero metadata identifies the Optimism token and its issuer Ethereum origin independently.
- Ethereum ZK is registered by the documented official NativeTokenVault against issuer ZK on chain 324. The current factory calculation gives another address; the existing registered mapping establishes this deployment.
- Avalanche DAI is the official Avalanche Bridge DAI.e wrapper. WLD on Optimism remains the documented legacy bridge deployment. PEPE on Arbitrum/BNB is a registered third-party LayerZero OFT v1 bridge asset, without an assertion of issuer endorsement.

## Deployment source index

| Asset | Network | Result | Primary source |
| --- | --- | --- | --- |
| POL | ethereum | Active | [Source](https://polygon.technology/blog/polygon-2-0-milestone-pol-contracts-are-live-on-ethereum-mainnet) |
| MNT | ethereum | Active | [Source](https://raw.githubusercontent.com/mantlenetworkio/mantle/main/packages/sdk/src/utils/contracts.ts) |
| CRO | ethereum | Active | [Source](https://crypto.com/price/cronos) |
| AERO | base | Active | [Source](https://raw.githubusercontent.com/aerodrome-finance/contracts/main/README.md) |
| ARB | ethereum | Active | [Source](https://raw.githubusercontent.com/ArbitrumFoundation/docs/main/docs/deployment-addresses.md) |
| ARB | arbitrum | Active | [Source](https://raw.githubusercontent.com/ArbitrumFoundation/docs/main/docs/deployment-addresses.md) |
| BGB | ethereum | Active | [Source](https://www.bitget.com/asia/activity-hub/BGB/intro) |
| BLAST | blast | Active | [Source](https://assets.blast.io/cn/q2-2024.pdf) |
| CRV | celo | Active | [Source](https://raw.githubusercontent.com/ethereum-optimism/ethereum-optimism.github.io/d2065d3c28ea4a3006d147bd16508a9bb1e016ad/data/CRV/data.json) |
| CRV | mantle | Active | [Source](https://raw.githubusercontent.com/mantlenetworkio/mantle-token-lists/3e0c8212bf12d6848dc6bd01be60b920bb576a5e/data/CRV/data.json) |
| CRV | x-layer | Active | [Source](https://raw.githubusercontent.com/curvefi/curve-js/fdf70ea6af581790699a797e522360d27400c105/src/constants/coins/xlayer.ts) |
| DAI | base | Active | [Source](https://raw.githubusercontent.com/ethereum-optimism/ethereum-optimism.github.io/d2065d3c28ea4a3006d147bd16508a9bb1e016ad/data/DAI/data.json) |
| DAI | polygon | Active | [Source](https://raw.githubusercontent.com/maticnetwork/polygon-token-list/28c3afa156d3ffb95cd553b4c4ef6e48ee7e3756/src/tokens/defaultTokens.json) |
| DAI | avalanche | Active | [Source](https://raw.githubusercontent.com/ava-labs/avalanche-bridge-resources/97e5c9339712704c7bdbc2de6df40315db5c5c7c/avalanche_contract_address.json) |
| DAI | linea | Active | [Source](https://raw.githubusercontent.com/Consensys/linea-token-list/5b7fb76ba5e3db3ffc879f38ca8f0b82fa678ac9/json/linea-mainnet-token-shortlist.json) |
| DAI | zksync-era | Active | [Source](https://raw.githubusercontent.com/matter-labs/era-contracts/be9a1fec4938e81c4f5d21c127fe49e51bac863e/l1-contracts/contracts/bridge/L2SharedBridgeLegacy.sol) |
| DAI | unichain | Active | [Source](https://raw.githubusercontent.com/ethereum-optimism/ethereum-optimism.github.io/d2065d3c28ea4a3006d147bd16508a9bb1e016ad/data/DAI/data.json) |
| DAI | celo | Active | [Source](https://raw.githubusercontent.com/ethereum-optimism/ethereum-optimism.github.io/d2065d3c28ea4a3006d147bd16508a9bb1e016ad/data/DAI/data.json) |
| ENS | ethereum | Active | [Source](https://basics.ensdao.org/ens-token) |
| ETHFI | ethereum | Active | [Source](https://github.com/etherfi-protocol/ethfi-wormhole/blob/4e3ad2912f98f6f967b9043f2774aba19f714f8b/utils/constants.sol) |
| ETHFI | arbitrum | Active | [Source](https://github.com/etherfi-protocol/ethfi-wormhole/blob/4e3ad2912f98f6f967b9043f2774aba19f714f8b/utils/constants.sol) |
| ETHFI | base | Active | [Source](https://github.com/etherfi-protocol/ethfi-wormhole/blob/4e3ad2912f98f6f967b9043f2774aba19f714f8b/utils/constants.sol) |
| ETHFI | scroll | Active | [Source](https://github.com/etherfi-protocol/ethfi-wormhole/blob/4e3ad2912f98f6f967b9043f2774aba19f714f8b/utils/constants.sol) |
| ETHFI | optimism | Active | [Source](https://metadata.layerzero-api.com/v1/metadata) |
| KCS | ethereum | Active | [Source](https://www.kucoin.com/announcement/en-kucoin-token) |
| LDO | arbitrum | Active | [Source](https://tokenlist.arbitrum.io/ArbTokenLists/arbed_uniswap_labs_default.json) |
| LDO | optimism | Active | [Source](https://raw.githubusercontent.com/ethereum-optimism/ethereum-optimism.github.io/d2065d3c28ea4a3006d147bd16508a9bb1e016ad/data/LDO/data.json) |
| LDO | polygon | Active | [Source](https://raw.githubusercontent.com/maticnetwork/polygon-token-list/28c3afa156d3ffb95cd553b4c4ef6e48ee7e3756/src/tokens/defaultTokens.json) |
| LEO | ethereum | Active | [Source](https://support.bitfinex.com/hc/en-us/articles/360023617554-Unus-Sed-LEO-Token-Conversion) |
| LINEA | ethereum | Active | [Source](https://docs.linea.build/network/overview/tokenomics) |
| LINEA | linea | Active | [Source](https://docs.linea.build/network/overview/tokenomics) |
| ONDO | ethereum | Active | [Source](https://docs.ondo.foundation/ondo-token) |
| OP | optimism | Active | [Source](https://raw.githubusercontent.com/ethereum-optimism/ethereum-optimism.github.io/master/data/OP/data.json) |
| PAXG | ethereum | Active | [Source](https://raw.githubusercontent.com/paxosglobal/paxos-gold-contract/master/README.md) |
| PEPE | ethereum | Active | [Source](https://pepe.vip) |
| PEPE | arbitrum | Active | [Source](https://metadata.layerzero-api.com/v1/metadata/experiment/ofts/list?symbols=ZRO,ETHFI,PEPE) |
| PEPE | avalanche | Removed at user request | [Source](https://docs.layerzero.network/v2/concepts/technical-reference/oft-reference) |
| PEPE | bnb | Active | [Source](https://metadata.layerzero-api.com/v1/metadata/experiment/ofts/list?symbols=ZRO,ETHFI,PEPE) |
| RETH | ethereum | Active | [Source](https://raw.githubusercontent.com/rocket-pool/docs.rocketpool.net/f5ebf7e5b387bd3b10d8ecdeeb37dc6c387688b3/docs/en/protocol/contracts-integrations.md) |
| RETH | arbitrum | Active | [Source](https://raw.githubusercontent.com/rocket-pool/docs.rocketpool.net/f5ebf7e5b387bd3b10d8ecdeeb37dc6c387688b3/docs/en/protocol/contracts-integrations.md) |
| RETH | optimism | Active | [Source](https://raw.githubusercontent.com/rocket-pool/docs.rocketpool.net/f5ebf7e5b387bd3b10d8ecdeeb37dc6c387688b3/docs/en/protocol/contracts-integrations.md) |
| RETH | base | Active | [Source](https://raw.githubusercontent.com/rocket-pool/docs.rocketpool.net/f5ebf7e5b387bd3b10d8ecdeeb37dc6c387688b3/docs/en/protocol/contracts-integrations.md) |
| SCR | scroll | Active | [Source](https://gov.scroll.io/proposals/32195966105875256982966851806785706747970993241469507520936106720258197550754) |
| SHIB | ethereum | Active | [Source](https://shib.io/_next/static/chunks/77046-9bdd5a8a3361c361.js) |
| SKY | ethereum | Active | [Source](https://chainlog.sky.money/api/mainnet/active.json) |
| SUSDS | avalanche | Active | [Source](https://forum.skyeco.com/t/skylink-bridge-to-avalanche/27825) |
| USDS | avalanche | Active | [Source](https://forum.skyeco.com/t/skylink-bridge-to-avalanche/27825) |
| WBTC | ethereum | Active | [Source](https://docs.wbtc.network/resources/contract-addresses) |
| WETH | ethereum | Active | [Source](https://raw.githubusercontent.com/gnosis/canonical-weth/master/README.md) |
| WLD | ethereum | Active | [Source](https://whitepaper.world.org/designing-for-scale/2025-04-28) |
| WLD | optimism | Active | [Source](https://whitepaper.world.org/designing-for-scale/2025-04-28) |
| WLFI | ethereum | Active | [Source](https://docs.worldlibertyfinancial.com/wlfi-token/contract-addresses) |
| WLFI | bnb | Active | [Source](https://docs.worldlibertyfinancial.com/wlfi-token/contract-addresses) |
| ZK | ethereum | Active | [Source](https://docs.zksync.io/zksync-protocol/contracts/l1-contracts/zk-chain-addresses) |
| ZK | zksync-era | Active | [Source](https://docs.zknation.io/zk-token/zk-token) |
| ZRO | ethereum | Active | [Source](https://metadata.layerzero-api.com/v1/metadata/experiment/ofts/list?symbols=ZRO,ETHFI,PEPE) |
| ZRO | arbitrum | Active | [Source](https://metadata.layerzero-api.com/v1/metadata/experiment/ofts/list?symbols=ZRO,ETHFI,PEPE) |
| ZRO | base | Active | [Source](https://metadata.layerzero-api.com/v1/metadata/experiment/ofts/list?symbols=ZRO,ETHFI,PEPE) |
| ZRO | bnb | Active | [Source](https://metadata.layerzero-api.com/v1/metadata/experiment/ofts/list?symbols=ZRO,ETHFI,PEPE) |

## Offline verification

Catalog and wiki regression tests exercise identity uniqueness, canonical deployment IDs, protocol validation, exact original-data retention and wiki/catalog coverage. The catalog regression now follows each recorded decision, including explicit removal. The earlier CLI check reads every audited network in a throwaway data directory and matches each verified deployment by contract and precision. The removed Avalanche wrapper remains absent.

```sh
cargo test -p spectra_core tokens:: --lib
cargo test -p spectra_core wiki:: --lib
spectra --json token catalog --chain ethereum
spectra --json token catalog --chain avalanche
```

Actual verification results are recorded in [BEHAVIOUR-CHANGES.md](../../BEHAVIOUR-CHANGES.md). No transaction was signed or broadcast during this research.
