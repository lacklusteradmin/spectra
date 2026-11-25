# Token catalog audit and multi-network additions, 2026-11-24

Every contract deployment in `core/data/tokens.toml` at Beta Commit 258 was
read on its network and compared with CoinGecko's and CoinPaprika's listings.
Deployments that disagreed or had no kept source were traced to their issuer or
bridge. 252 deployments were then added: 122 of 44 new tokens, and 130 that
put catalog tokens on more networks.

**All 278 readable existing deployments are live contracts on the right network
with the catalog's decimals, and every disagreement with CoinGecko resolves in
the catalog's favour. Three CoinPaprika ids were wrong and are fixed. Each added
deployment is named, at its exact identifier, by its issuer or by the official
list of the bridge that mints it, and was read on chain at a fixed block.**

## Evidence

- [Existing deployments read on chain](existing-reads.json): 283 rows, every
  contract deployment except the two Asset Hub assets, which
  [token-additions-2026-11-24](../token-additions-2026-11-24/README.md) read.
- [CoinGecko and CoinPaprika cross-checks](cross-check.json): the 40
  deployments whose contract CoinGecko does not list under the same coin, each
  with its resolution, and the CoinPaprika ids that did not match.
- [Identity sources](sources.json): for each added deployment and each
  existing one whose source this record supplies, the issuer's or bridge
  operator's list, a verbatim excerpt and who mints it. 199 source entries are
  GitHub files pinned to a commit; the rest are live pages read on the date
  above.
- [Added deployments read on chain](added-reads.json): chain ID, bytecode,
  decimals, name, symbol and supply, with a rough value from supply and price.
- [Left out](excluded.json): 155 candidate deployments and why each was not
  added.
- [Icon sources](icons.json): where each of the 37 new icons comes from, its
  terms and the source file's SHA-256. BONK, WIF, PENGU, TRUMP and DOGS publish
  no first-party vector mark (TRUMP only a wordmark), and Aster's and Orca's
  brand terms forbid altering theirs, so these seven name no artwork and the
  apps draw their letter badge.

## Existing deployments

- **278 of 283 read correctly**: chain ID, bytecode (or mint, jetton master,
  coin metadata, account) and decimals all match.
- **The five Blast deployments were not read.** Both built-in Blast endpoints
  refuse this client (`blast-rpc.publicnode.com` 403,
  `rpc.blast.io` closes the TLS connection), as did the public alternatives
  tried. Their contracts are unchanged.
- **Sui was read through `graphql.mainnet.sui.io`.** Every built-in Sui
  endpoint answers JSON-RPC with "deprecated, use gRPC or GraphQL"; Sui
  retired JSON-RPC on public fullnodes.
- **ENA on TON is right.** CoinGecko lists `EQAPh9RC…`, which is Ethena's
  LayerZero contract on TON, not a jetton; Ethena's key addresses name the
  catalog's `EQAFhZFZ…` (`0:0585…`).
- **LINK on BNB Chain is right.** Chainlink names `0x404460c6…`; CoinGecko's
  `0xf8a0bf9c…` is Binance-Peg LINK, which Chainlink says is not ERC-677 and
  converts through PegSwap.
- **USDC on Linea is Circle's own now**; Circle's table lists the address.
  USDC on Scroll and Mantle is bridged USDC.e from each network's bridge.
- DAI on Arbitrum, Optimism and Scroll, which no earlier record sourced, is
  named by each network's canonical token list.
- **CoinPaprika:** Gram's `ton-tontoken` was an unrelated Ethereum token
  (now `toncoin-the-open-network`); `dym-dymension` does not exist (now
  `dym-dymension-iou`, CoinPaprika's only active Dymension); USD Blast's
  `usdb-usdb` is inactive (now empty).

## Additions

| Token | | Networks | Sources |
| --- | --- | --- | --- |
| 1INCH | new | ethereum, bnb, base, avalanche, polygon | 1inch.com; Polygon, Avalanche Bridge and Superchain token lists |
| APE | new | ethereum, arbitrum, bnb, polygon, solana, hyperliquid, robinhood-chain | ApeChain docs; LayerZero OFT metadata; Polygon token list |
| AXS | new | ethereum, bnb, polygon | Ronin support; Polygon token list |
| BAT | new | ethereum, solana, polygon, avalanche | basicattentiontoken.org; Brave support; Polygon and Avalanche Bridge lists |
| CBETH | new | ethereum, base, arbitrum, optimism, polygon | coinbase.com; Superchain, Arbitrum and Polygon token lists |
| CETUS | new | sui | Cetus docs and SDK |
| COMP | new | ethereum, base, arbitrum, polygon, optimism, mantle, avalanche | compound-finance/comet; Polygon, Avalanche Bridge and Superchain lists |
| CRVUSD | new | ethereum, arbitrum, optimism, base, polygon, sonic, zksync-era | curvefi/docs; Superchain and Polygon lists |
| DEEP | new | sui | MystenLabs/sui-docs |
| EIGEN | new | ethereum, arbitrum | Layr-Labs/eigenlayer-contracts; Arbitrum token list |
| FET | new | ethereum, polygon | fetchai bridge repository; Polygon token list |
| FLOKI | new | ethereum, bnb | floki.com |
| GALA | new | ethereum | news.gala.com |
| GMX | new | arbitrum, avalanche, solana | docs.gmx.io |
| GNO | new | ethereum, arbitrum, polygon | gnosischain/documentation; Arbitrum and Polygon lists |
| GRT | new | ethereum, arbitrum, polygon | graphprotocol/docs; Arbitrum and Polygon lists |
| HNT | new | solana | helium/docs |
| IMX | new | ethereum | immutable/zkevm-bridge-contracts |
| JST | new | tron | docs.justlend.org; justlend-interface |
| JTO | new | solana | jito-foundation/jito-omnidocs and distributor |
| MANA | new | ethereum, polygon | decentraland-transactions; Polygon list |
| NEXO | new | ethereum, polygon, arbitrum | nexo.com; NEXO-Token; Arbitrum and Polygon lists |
| NOT | new | ton | Notcoin's MiCA white paper |
| QNT | new | ethereum, arbitrum, polygon | quantnetwork's treasury repository; Arbitrum and Polygon lists |
| RENDER | new | solana | know.rendernetwork.com |
| SAFE | new | ethereum | safefoundation.org; safe-token |
| SAND | new | ethereum, polygon, base, bnb | sandbox-smart-contracts; LayerZero OFT metadata |
| SNEK | new | cardano | snek.com (archived); Cardano token registry |
| SNX | new | ethereum, optimism, polygon | docs.synthetix.io; Superchain and Polygon lists |
| SPX | new | ethereum, base, solana, sui | spx6900.com; Superchain list |
| SUN | new | tron | docs.sun.io; docs.justlend.org |
| TWT | new | bnb | trustwallet.com; trustwallet/assets |
| VIRTUAL | new | base, ethereum, solana, robinhood-chain, arc | whitepaper.virtuals.io; Chainlink CCIP directory |
| W | new | solana, ethereum, arbitrum, optimism, base | wormhole-foundation/connect-w and wormhole-dashboard |
| WAL | new | sui | MystenLabs/walrus |
| WBETH | new | ethereum, bnb | binance.com |
| WEETH | new | ethereum, arbitrum, base, optimism, bnb, linea, avalanche, hyperliquid, ink, katana, monad, plasma, stable, unichain | ether.fi docs; LayerZero OFT metadata; Arbitrum list |
| BONK | new, no icon | solana | bonkcoin.com (archived) |
| DOGS | new, no icon | ton | dogs.dev |
| PENGU | new, no icon | ethereum, solana, bnb | pudgypenguins.com (archived); LayerZero OFT metadata |
| TRUMP | new, no icon | solana, tron | gettrumpmemes.com |
| WIF | new, no icon | solana | dogwifcoin.org (archived) |
| ASTER | new, no icon | bnb | docs.asterdex.com |
| ORCA | new, no icon | solana | docs.orca.so |
| AAVE | networks | avalanche, starknet | Avalanche Bridge; aave-address-book; StarkGate |
| CAKE | networks | aptos | pancake-developer |
| CHZ, INJ, METIS, OKB, STRK | networks | ethereum | each project's own docs or repository |
| ENA | networks | avalanche, hyperliquid | LayerZero OFT metadata (run by Ethena) |
| EURC | networks | stellar, world-chain | Circle |
| GHO | networks | ink, plasma, monad, mantle | aave-address-book |
| LBTC | networks | sui, avalanche, monad | docs.lombard.finance; lombard-finance repositories |
| LINK | networks | base, optimism, polygon, scroll, zksync-era, celo, metis, robinhood-chain, 0g | Chainlink documentation |
| MORPHO | networks | katana | docs.morpho.org |
| PAXG | networks | base, solana | docs.paxos.com |
| PENDLE | networks | optimism, sonic, berachain, hyperliquid, monad, robinhood-chain, x-layer, mantle, plasma | pendle-core-v2-public; LayerZero metadata; Superchain and Mantle lists |
| PYUSD | networks | stellar, polygon, x-layer, stable | docs.paxos.com; Paxos's stellar.toml |
| RETH | networks | unichain, polygon, zksync-era, scroll, starknet | docs.rocketpool.net; network token lists |
| RLUSD | networks | xrp, base | docs.ripple.com |
| SUSDE | networks | avalanche, monad, plasma, hyperliquid, bnb | docs.ethena.fi; LayerZero metadata |
| UNI | networks | arbitrum, optimism, polygon, base, bnb, avalanche, celo, linea, scroll, starknet | Uniswap default list; network token lists |
| USAT | networks | avalanche | usat.io; LayerZero metadata |
| USD1 | networks | mantle, monad | docs.worldlibertyfinancial.com |
| USDC | networks | stellar, xrp, starknet | Circle; Circle's stellar.toml |
| USDE | networks | avalanche, manta-pacific, monad, plasma, hyperliquid, robinhood-chain, ink | docs.ethena.fi; LayerZero metadata |
| USDG | networks | ink, x-layer, mantle, arbitrum | docs.paxos.com |
| USDT0 | networks | monad | docs.usdt0.to |
| WBTC | networks | sei, monad, avalanche, sonic, berachain, bnb, unichain, optimism, arbitrum, polygon, linea, scroll, solana, tron, kava-evm, 0g, mantle, celo | wbtc.network; network token lists |
| WETH | networks | arbitrum, optimism, base, unichain, ink, world-chain, polygon, linea, scroll, zksync-era, mantle, avalanche, celo | OP Stack predeploys; network token lists |
| WSTETH | networks | arbitrum, optimism, base, polygon, linea, scroll, zksync-era, mantle, bnb, starknet | docs.lido.fi; StarkGate |
| XAUT0 | networks | monad, plasma | docs.usdt0.to |
| ZRO | networks | polygon, optimism, avalanche | LayerZero OFT metadata |

## What was left out

- **A different copy where the issuer names one.** WBTC.e on Avalanche, the
  OP-bridge WBTC on Unichain and StarkGate's on Starknet (wbtc.network names
  the LayerZero WBTC), the canonical-bridge LINK on Celo, Unichain, Linea and
  Polygon (Chainlink names another), and Binance-Peg LINK on BNB Chain.
- **Third-party copies the issuer does not list.** EURC on Optimism (ether.fi's
  LayerZero copy), PYUSD0 on Ink, CCIP and LayerZero copies of wstETH, rETH and
  cbBTC on newer networks, BONK's Wormhole wrappers, WIF's LayerZero copies,
  and FLOKI, GALA and SNX copies their issuers disown or no longer support.
- **Deprecated deployments.** LBTC on Katana, Berachain, Sonic and Starknet;
  weETH on Scroll, zkSync Era, Blast, Berachain and Sonic; the pre-2021 AXS,
  GALA v1, RNDR on Ethereum (upgraded one-way to RENDER on Solana) and SUNOLD,
  whose contract also reports the symbol SUN.
- **tsUSDe on TON**, which Ethena lists in its sUSDe column but is a different
  token.
- **Deployments with under $1,000 in circulation**, 78 of them: issuer-listed
  but held by almost no one (LINK on seven networks has none in circulation).
- **Fartcoin and Kamino**, for which no issuer source names the mint
  (Fartcoin has no issuer at all).

## Findings worth retaining

- BONK, WIF, PENGU and SNEK rest on archived copies of the issuer's own page:
  each live site blocks automated reads or renders its addresses in script.
- Precision differs by deployment: USD1 has 6 decimals on Monad and 18 on
  Mantle; W has 6 on Solana and 18 on EVM networks; Solana APE and GMX have 9,
  Solana BAT 8, against 18 on EVM.
- The SAND address on Base and BNB Chain is The Sandbox's LayerZero token; on
  Ethereum the same address is the adapter, not a token.
- XRP Ledger RLUSD and USDC are 40-hex currency codes. Stellar PYUSD's issuer
  has clawback enabled. Paxos lists only the Soroban wrapper for Stellar PYUSD;
  its stellar.toml names the classic asset.
- FLOKI charges 0.3% on DEX buys and sells.
- ORCA's mint authority is still set, though Orca's docs call its supply fixed
  at 75,000,000.
- Many contracts report a bridge suffix (`DAI.e`, `LINK.e`, `UNI.e`, `fxcbETH`)
  or a different case (`wBETH`, `Cake`, `Metis`); the catalog's symbol is the
  token's.

No transaction was signed or broadcast during this audit.
