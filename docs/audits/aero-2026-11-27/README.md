# AERO replaced by the Aero token, 2026-11-27

Aerodrome and Velodrome merged into Aero on 2026-10-21 (2026-10-22 00:00 UTC).
Holders upgrade one way through a portal: 1 legacy AERO to 1 new AERO, 1 VELO
to 0.044 AERO. Legacy AERO and VELO emissions ended at launch. The catalog
listed the legacy Aerodrome AERO on Base; it now lists the new AERO instead.

**The new AERO is named, at the exact address, by Aero's docs security page,
and is a live contract on Base with 18 decimals, name "Aero" and symbol
"AERO". Aero states AERO exists only on Base. CoinGecko lists it as `aero`
with the same Base contract; CoinPaprika has no listing for it.**

## Evidence

- [Identity sources](sources.json): Aero's security page, its AERO and sAERO
  page and the upgrade hub, with verbatim excerpts, and CoinGecko's listings
  of the new and legacy tokens. Live pages, read on the date above.
- [Chain reads](reads.json): chain ID, bytecode, decimals, name, symbol,
  supply and the EIP-1967 implementation slot of the new AERO, the legacy
  AERO and each xAERO, at one block per network from a built-in endpoint.
- [Icon source](icons.json): the brand-kit token mark, its SHA-256, the
  layout onto the library's disc and the CoreSVG comparison.

## Catalog change

| | Token | Network | Contract | Decimals |
| --- | --- | --- | --- | --- |
| added | `aero`, AERO, "Aero" | base | `0x3f5bf3edac59d650d8fdf9657a71f2f904a3a4ee` | 18 |
| removed | `aerodrome-finance`, AERO, "Aerodrome Finance" | base | `0x940181a94a35a4569e4529a3cdfb74e38fd98631` | 18 |

- The new token is not behind an EIP-1967 proxy. Its supply at Base block
  `0x3c3bb9c` is 2,154,914,347.6 AERO.
- The legacy token's supply is 2,008,752,775.3, the same as the 2026-11-24
  read: no emissions since launch. It still trades, and a holder can add it
  as a custom token and upgrade it on aerodrome.finance at any time.
- The artwork is Aero's token mark, a pinwheel on a light disc
  (`icons/crypto/aero.svg`); the legacy Aerodrome mark is removed.

## What was left out

- **xAERO** on Optimism, Arbitrum, Ethereum, Ink, Robinhood Chain and Arc. It
  is the receipt token Aero pays rewards in off Base: symbol `xAERO` on chain,
  redeemable 1:1 for AERO on Base and movable only back to Base. Each holds
  between 16,341 and 262,680 xAERO. Aero's security page also lists a Base
  xAERO, which holds dust.
- **The legacy AERO beside the new one.** It is replaced by a one-way
  upgrade, as RNDR was by RENDER in the 2026-11-24 audit.

No transaction was signed or broadcast during this audit.
