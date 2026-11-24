# Thirteen Cosmos SDK mainnets, 2026-11-24

Read-only checks behind Cosmos Hub, Celestia, dYdX Chain, Terra, Kava, Injective,
Akash, Babylon Genesis, Osmosis, Dymension, Saga, Allora and BandChain in the
registry (`core/src/registry/cosmos.rs`) and their `cosmos-rest` endpoints.
Nothing was signed or broadcast; a simulation carried no signature.

| File | What it holds |
|---|---|
| `network-facts.json` | Per network: chain id, SDK version and application from `node_info`; bech32 prefix, SLIP-44 coin type and path (chain-registry `a23d0248`, Keplr registry `8cee9a15`); key type, public-key type URL and the hash a direct-mode signature is over, cited to source; fee denomination, decimals and CoinGecko id; chain-registry gas prices and each endpoint's `minimum_gas_price`; fee-market routes and a live answer; `auth` and `bank` parameters; block time from two headers ~1,000 apart; whether `timeout_height` is enforced. Per endpoint (78 checked): `node_info`, balances, accounts, the simulate route, transaction search in each form and order, received-transfer search, a transaction by hash, and how far back blocks are kept. |
| `client-probes.json` | Every catalogued endpoint read through Spectra's `CosmosClient`: network, latest height, account, balance and spendable balance, fee-market price, a simulated 1-unit self-send with a memo, a known and an unknown transaction's status, and the first history page. |

## Findings the registry encodes

- **Keys.** Injective and Dymension use Ethermint's `eth_secp256k1` at coin
  type 60: the address is the key's Ethereum address under the network's
  prefix, and a direct-mode signature is over Keccak-256 of the sign document,
  with the public key typed `/injective.crypto.v1beta1.ethsecp256k1.PubKey` and
  `/ethermint.crypto.v1.ethsecp256k1.PubKey`. Every other network uses the SDK's
  secp256k1 at coin type 118, except Terra (330), Kava (459) and Band (494).
  Injective's chain source is private; its signing rule is cited from
  InjectiveLabs/sdk-go.
- **Gas prices.** Each network's floor is chain-registry's `low` price.
  Cosmos Hub and Allora run Skip's `x/feemarket`
  (`/feemarket/v1/gas_price/{denom}`), Osmosis and Injective a `txfees`
  EIP-1559 base fee (`/osmosis/…/cur_eip_base_fee`,
  `/injective/…/cur_eip_base_fee`). Celestia's registry `fixed_min` (0.002)
  is below the 0.004 nodes apply by default, so its floor is `low` (0.01).
  Allora's `uallo` has 18 decimals.
- **Memos.** `max_memo_characters` is 512 on Cosmos Hub, Terra, Kava and
  BandChain and 256 elsewhere.
- **Timeouts.** Every network with public source enforces `timeout_height`;
  block times run from 0.59 s (Injective) to 9.5 s (Babylon).
- **Transaction search.** Kava (SDK 0.47.15) takes `events=` and refuses
  `query=`; the others take `query=`. A search must run newest first: nodes
  that index transactions older than the blocks they keep fail an
  oldest-first search. Injective and Osmosis answer pages shorter than asked
  and count index hits in `total`, so only an empty page ends a search.
- **Gateway errors.** The gateway answers an application error (an unknown
  account, a failed simulation, a pruned height) with HTTP 500 and the reason
  in the body.
- **Endpoints.** Two or more keyless REST endpoints from different operators
  per network. Celestia's Polkachu gateway and Saga's NodeStake gateway fail
  history searches, and Numia's Celestia gateway serves no `node_info`; the
  catalog claims only what each answered. Allora Labs' own gateway answers 401
  on the fee-market route. History reaches back days on Osmosis (6–12),
  Band (4–14) and Injective (under 16), and months to years elsewhere; Kava
  Labs and Numia keep it all.
- **Explorers.** Mintscan for eleven networks; it no longer serves Dymension
  (ezstaking.app instead) or Allora (Allora's own explorer). Allora's
  explorer links an address to `/profile/{address}`, which redirected to its
  502 error page for every address tried on 2026-11-24; it is the only address
  page the network has, and is linked.
