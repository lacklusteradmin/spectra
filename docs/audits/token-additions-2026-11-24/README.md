# Eight stablecoin deployments added on 2026-11-24

USDT and USDC on BNB Smart Chain, USDT0 on Plasma, USDC on Sei, USDT on Celo,
USDT on opBNB, and USDT and USDC on Polkadot Asset Hub were added to
`core/data/tokens.toml`. Exchanges list deposits and withdrawals on each of
these networks; the catalog had none of them.

**All eight are named, at the exact address or asset ID, by their issuer or by
the operator of the bridge that mints them. The six EVM deployments pass a
read-only chain-ID, bytecode and decimals check at a fixed block; the two
Asset Hub assets are live, sufficient and state six decimals on each built-in
Asset Hub node.**

## Evidence

- [Identity sources](sources.json): for each deployment, the issuer's or the
  bridge operator's list, an excerpt, and who mints it. GitHub sources are
  pinned to a commit; the Binance, Tether, Circle and USDT0 pages are live and
  were read on 2026-11-24.
- [Fixed-block chain reads](rpc-metadata.json): chain ID, bytecode size and
  hash, `decimals()`, `name()` and `symbol()` from a built-in endpoint of each
  network, and for opBNB the bridge mapping (`REMOTE_TOKEN()`, `BRIDGE()`).
- [Asset Hub reads](asset-hub-reads.json): assets 1984 and 1337's `Asset` and
  `Metadata` entries at the finalized head of each built-in Polkadot and
  Westend Asset Hub node, decoded by that node's runtime metadata. They also
  show each node serves the `Assets` storage its new `token-balance`
  capability claims.

## Deployments

| Asset | Network | Contract | Decimals | Minted by | Primary source |
| --- | --- | --- | --- | --- | --- |
| USDT | bnb | `0x55d398326f99059ff775485246999027b3197955` | 18 | Binance (Binance-Peg) | [Binance proof of collateral](https://www.binance.com/en/proof-of-collateral), [BNB Chain](https://raw.githubusercontent.com/bnb-chain/bcfusion/26b36d15803eb30edfa1db08290c0fd8d247c42c/README.md) |
| USDC | bnb | `0x8ac76a51cc950d9822d68b83fe1ad97b32cd580d` | 18 | Binance (Binance-Peg) | [Binance proof of collateral](https://www.binance.com/en/proof-of-collateral), [BNB Chain](https://raw.githubusercontent.com/bnb-chain/bcfusion/26b36d15803eb30edfa1db08290c0fd8d247c42c/README.md) |
| USDT0 | plasma | `0xb8ce59fc3717ada4c02eadf9682a9e934f625ebb` | 6 | USDT0 | [USDT0 deployments](https://docs.usdt0.to/api/deployments) |
| USDC | sei | `0xe15fc38f6d8c56af07bbcbe3baf5708a2bf42392` | 6 | Circle | [Circle](https://developers.circle.com/stablecoins/usdc-contract-addresses) |
| USDT | celo | `0x48065fbbe25f71c9282ddf5e1cd6d6a887483d5e` | 6 | Tether | [Tether](https://tether.to/en/supported-protocols/), [Celo docs](https://raw.githubusercontent.com/celo-org/docs/dc39161794c0d764571a85d4d0daa8b36c9ee4a1/build/tools/contracts/stablecoin-contracts.mdx) |
| USDT | opbnb | `0x9e5aac1ba1a2e6aed6b32689dfcf62a509ca96f3` | 18 | opBNB standard bridge, from the BNB Smart Chain USDT | [opBNB bridge tokens](https://raw.githubusercontent.com/bnb-chain/opbnb-bridge-tokens/21699c666d8a337b54d8e181b4fb6088f4172f9d/data/USDT/data.json) |
| USDT | polkadot | asset `1984` | 6 | Tether | [Tether](https://tether.to/en/supported-protocols/) |
| USDC | polkadot | asset `1337` | 6 | Circle | [Circle](https://developers.circle.com/stablecoins/usdc-contract-addresses) |

## Findings worth retaining

- The BNB Smart Chain USDT and USDC are Binance's wrapped tokens, not Tether's
  or Circle's: Tether lists only XAU₮ on BNB Smart Chain, and Circle lists no
  USDC there. Their contracts name themselves Tether USD and USD Coin, and
  Binance publishes the Ethereum holdings that back them. They are listed
  under `tether` and `usd-coin`, as bridge wrappers such as Avalanche's DAI.e
  are under the token they wrap. opBNB's USDT is a second wrap: the opBNB
  standard bridge's token for the BNB Smart Chain USDT.
- BNB Smart Chain and opBNB tokens have 18 decimals, not the 6 of the
  issuers' own deployments; precision belongs to the deployment.
- Celo's USDT contract calls itself `USD₮`. Paying Celo gas in USDT uses the
  fee-currency adapter `0x0e2a3e05bc9a16f5292a6170456a710cb89c6f72`, which is
  not the token.
- USDT0 deploys its token at the same address on several networks; the Plasma
  entry quoted is the one with chain ID 9745 under `usdt0.native`.
- Asset Hub USDT and USDC have a minimum balance of 10,000 units (0.01): a
  holding left with less is removed. On Westend, 1984 is a test asset and 1337
  another asset entirely (six and three decimals); Circle's Westend USDC is
  31337. No Westend asset is added.
