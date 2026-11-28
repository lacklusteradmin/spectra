# Blast shutdown evidence — 2026-11-27

Spectra catalogued Blast (`blast`, EIP-155 81457) until this record. Every
reading below was taken on 2026-11-27 from one Mac; each probe was read-only
and nothing was submitted.

## Blast's own services

| Host | Result |
|---|---|
| `rpc.blast.io` | Does not resolve (NXDOMAIN) |
| `docs.blast.io` | Does not resolve (NXDOMAIN) |
| `blast.io` | "Blast Fund Recovery": ETH, WETH and USDB held on Blast, and withdrawals that never finished, are claimed on Ethereum mainnet from recovery contracts; Blast Mobile users import the mobile seed phrase into an EVM wallet to claim. The page and its script state no deadline. |
| `blastscan.io` | "Blastscan.io has been discontinued"; "Blast Mainnet API support ended October 31, 2026." |

The recovery page's script names one contract on Ethereum mainnet,
`0x9D31771458B421C7EA8A06CB8D12663E91617869`. Read through
`ethereum-rpc.publicnode.com`: 212 bytes of code (a proxy), a balance of
about 8,678.67 ETH. The script's ABI for it is `roots`, `claimed`,
`isApprovedRedeemer` and `claim` (a Merkle proof per claim). The script also
names DAI (`0x6B175474E89094C44Da98b954EedeAC495271d0F`).

## Public Blast endpoints

Each was asked `eth_chainId` and `eth_getBlockByNumber("latest")`.

| Endpoint | Result |
|---|---|
| `blast-rpc.publicnode.com` | HTTP 403, "unsupported platform" |
| `blast.drpc.org` | "chain is not available on free plan" |
| `rpc.ankr.com/blast` | HTTP 403, API key required |
| `81457.rpc.thirdweb.com` | `eth_chainId` answers `0x13e31`; every block and balance read fails with -32603 |
| `blast.blockpi.network/v1/rpc/public` | HTTP 503, "Apikey not found" |
| `blast.gateway.tenderly.co` | HTTP 404 |
| `rpc.envelop.is/blast` | HTTP 404 |
| `endpoints.omniatech.io/v1/blast/mainnet/public` | HTTP 521 |
| `blastl2-mainnet.public.blastapi.io` | "Blast API is no longer available" |
| `blast.din.dev/rpc`, `blast.gasswap.org`, `blast.rpc.subquery.network/public` | Do not resolve |
| Routescan, chain 81457 | "chain not supported" |

No endpoint returned a block.

## Announcements

Press coverage of Blast's posts on X ([The Block](https://www.theblock.co/news/business/2026-10-02-blast-ethereum-layer-2-shutting-down-417583),
[Decrypt](https://decrypt.co/379972/ethereum-layer-2-blast-shutting-down),
[Cryptopolitan](https://www.cryptopolitan.com/blast-users-year-reclaim-funds-layer-2-dark/)):
the wind-down was announced on 2026-10-02 because costs exceeded revenue;
the usual withdrawal interface ran until 2026-10-26; the network was shut
down on 2026-11-04, and the recovery site runs for 12 months, to 2027-11-04.
The deadline comes from these reports alone.
