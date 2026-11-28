# Icon library audit, 2026-11-27

Each of the 185 icons in `icons/crypto/` at Beta Commit 263 was compared with
the mark its project publishes now: the brand, press or media kit, an official
GitHub media repository, the logo or token icon the official site serves, or
the issuer's token list. web3icons
(`0xa3k5/web3icons@172f57f38fca75007002a4d707e08f33d82ba3a5`) was a reference
only; it lags several rebrands. Every replacement source was downloaded again
and its SHA-256 checked against the one recorded. Each changed icon was
rendered in WebKit (Quick Look) and in CoreSVG (`NSImage`, the renderer behind
spectra-apple's asset catalog) and compared with the official artwork.

**151 icons show their project's current mark. 47 icons change: 40 are redrawn
from the project's current vector artwork, 6 take the colours of an official
icon that exists only as PNG or of a changed palette, and Sun's broken face is
repaired. Four icons stay outdated because no usable vector or no clear token
mark exists. The normalization no longer rewrites curves as arcs, which had
broken the ASI Alliance mark.**

## Evidence

- [Review](review.json): all 185 icons, with the batch verdict (`current`,
  `outdated`, `wrong`, `unverified`), what ours drew, what the project
  publishes, the pages checked and the outcome.
- [Icon sources](icons.json): for each of the 47 changed icons, the source file
  or zip member, the page it was found on, its terms, its SHA-256 and how it is
  laid on the disc.

## Changed

| Icon | Before | Now |
|---|---|---|
| 1inch | unicorn on navy | the 2025 token icon: black square on white |
| akash | three-piece sign, translucent foot, #db4533 | 2026 solid sign, white on Akash Red #ff4141 |
| apecoin | gorilla with outline, ripples and 9 blur filters (64 KB) | flat white gorilla on #0054fa (3.7 KB) |
| asi | lobes broken into slivers by arc conversion | the mark as published |
| astar | flat white redraw on cyan | the gradient knot (shadows kept, overlay highlights left out) |
| avalanche | #ff394a | #e6212f |
| bat | single-colour triangle on orange | tricolour triangle on white |
| cbeth | white diamond on blue | Coinbase's white disc, blue ring and two-tone diamond |
| celestia | #7a2bf9 | Indigo #5640d1 |
| cetus | white emblem on mint | two-tone emblem on black |
| chainlink | #2a5ada | #0847f7 |
| cosmos | flat atom on lavender | chain-registry ATOM icon on navy |
| cronos | navy hexagon C | gradient ribbon C on #090909 (cronos.com/brand) |
| crv | transform on `<mask>`: CoreSVG drew a different shape (218 KB) | the official logo, same in both renderers (113 KB) |
| crvusd | Curve's CRV torus | crvUSD's white wireframe on green |
| decentraland | flat yellow | full-colour logo |
| digibyte | flat blue disc | blue ring around a navy disc |
| dydx | white X on purple | dark disc with a purple leg |
| ecash | #1355ae | #0074c2 round icon |
| fdusd | 2023 bracket mark | 2024 slanted F on #101010 |
| gmx | white glyph on gradient disc | gradient glyph on #192631 |
| gnosis | teal owl | 2026 purple #3f2acd |
| kaspa | dark K on a teal disc | 2023 teal blob with white K |
| katana | flat navy disc | the token's gradient disc |
| kcs | #24ae8f | #41e6af |
| metis | teal head on black | cyan disc, black head |
| nexo | 3D blue N | 2024 line-N on near-black |
| osmosis | home-made white line flask | the full-colour flask, neck inside the disc |
| paxg | yellow blob | white ring mark on gold #dbba08 |
| pendle | pendulum's bob cut off by the disc | the whole mark |
| plasma | dark green | warm black #251d1d |
| pyusd | blue #0071f3 | Paxos's token logo, white on black |
| qtum | geodesic sphere | 2025 blue chip Q |
| rocketpooleth | tiled texture | smooth disc with lighter ring |
| safe | white symbol on #049f67 | black symbol on green-to-cyan |
| sei | #9e1f19 | Maroon #600014 |
| solana | #cb4ee8 to #10f4b1 | #9945ff to #19fb9b |
| sui | #4da2ff | Sui Blue #298dff |
| sun | right-half face covering the yellow half, art shrunk | split face, rays to the edge |
| synthetix | white S monogram | cyan X on navy |
| tezos | #2e84fd | #0d61ff token |
| thegraph | purple glyph on a white inner disc | white glyph on #6f4cff |
| trustwallet | 2020 outline shield | 2023 half-solid #0500ff shield |
| vechain | #582974 | VeChain Purple #7266ff |
| vtho | blue bolt on black | 2025 purple bolt on #eef3f7 |
| wbeth | grey | Binance's gold |
| zcash | black ring, gold disc, black Z | 2023 yellow #f3b724 disc, white Z |

## Kept

- **Outdated, left as they are:** Polkadot (polkadot.com, now the Polkadot
  app's site, ships a new black-and-white mark and no token icon), Bitcoin SV
  (the BSV Association shows a "BSV" wordmark tile; exchanges keep the gold
  B), SNEK (new mascot, PNG only) and SPX6900 (gold coin, PNG only).
- **Aerodrome is right for the catalog's token.** The catalog lists Aerodrome's
  AERO (`0x940181a94a35a4569e4529a3cdfb74e38fd98631` on Base). Aero, launched
  2026-10-21, is a new AERO that AERO and VELO holders upgrade to; its mark
  belongs to that token.
- **BERA** shares Berachain's network logo; BERA's own token-list icon (PNG) is
  a bear face.
- **Blast and USDB** show Blast's last mark. Blast took its L2 down on
  2026-11-04; blast.io is a fund-recovery page without a logo.
- **WETH** has no issuer; ours matches the icon Uniswap's token list uses.
- Same mark with a small colour difference, not changed: Kava, JUST, TRON,
  Tether Gold, XDC, MegaETH, Pyth, Wrapped Bitcoin, weETH, Uniswap (the brand
  kit's #f50db4 against #ff007a in Uniswap Labs' own token list), ether.fi
  (its site refused automated reads), Shiba Inu (shib.io's redrawn head breaks
  out of the circle) and Walrus (its full-colour variant is PNG only).

## Renderers

- CoreSVG ignores `mix-blend-mode`. Astar's official symbol relies on it: its
  multiply shadows are redrawn as black at the opacity that composites the
  same, and its overlay highlights, which have no such equivalent, are left
  out.
- CoreSVG ignores `transform` on `<mask>`, which SVG does not define; the old
  `crv.svg` relied on it.
- ImageMagick's built-in SVG renderer drops gradients and mask transforms, so
  it is no guide to how an icon looks in the apps.
