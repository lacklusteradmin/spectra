# Kusama and Astar, 2026-11-24

Read-only checks behind Kusama (on Kusama Asset Hub) and Astar (its Substrate
side) in the registry. Nothing was signed with a real key or broadcast; fee
quotes were asked for a transfer carrying a fixed, invalid signature.

| File | What it holds |
|---|---|
| `network-probes.json` | Per RPC: `chain_getBlockHash(0)`, `system_chain`, `system_name`, `system_version`, `system_properties` (SS58 format, decimals, symbol), the finalized head, `state_getRuntimeVersion`, and whether `rpc_methods` lists `payment_queryInfo`, `state_call`, `author_submitExtrinsic`, `system_accountNextIndex`, `state_getMetadata` and `chain_getBlock`. |
| `metadata-and-accounts.json` | Each network's metadata at one finalized block (spec and transaction version, size, SHA-256, and whether a second endpoint served the same bytes), and at another block every endpoint's metadata hash and its `System.Account` record for the treasury (`modlpy/trsry`). |
| `fee-quotes.json` | `payment_queryInfo` and `TransactionPaymentApi_query_info` for a signed `transfer_keep_alive`, with best and finalized heights and a nonce. |

The runtime metadata itself is `core/tests/fixtures/asset-hub-kusama-metadata.scale`
and `astar-metadata.scale`, and the finalized blocks whose events the
dispatch-outcome test reads are `core/tests/fixtures/substrate-blocks.json`.

## Findings the registry encodes

- **Identity.** Kusama Asset Hub's genesis is
  `0x48239ef6…2eda771a` (`specName` `statemine`, chain "Kusama Asset Hub",
  SS58 format 2, KSM at 12 decimals) on the Parity, Dwellir and Stakeworld
  RPCs. Astar's is `0x9eb76c51…ec4029c6` (`specName` `astar`, SS58 format
  5, ASTR at 18 decimals) on the Astar, Dwellir, OnFinality and 1RPC RPCs.
  The SS58 registry (`@polkadot/networks`) gives the same prefixes and
  decimals, and Astar's genesis.
- **Transaction extensions.** Kusama Asset Hub (spec 2005003, transaction
  version 15) has Polkadot Asset Hub's thirteen in the same order, fees
  through `ChargeAssetTxPayment` (a tip and an optional XCM location).
  Astar (spec 2500, transaction version 3) has eight: the `frame_system`
  checks, `ChargeTransactionPayment` (a compact `u128` tip) and
  `CheckMetadataHash`. Both use `MultiAddress` and `MultiSignature`, a `u128`
  `Balance` and `AccountData` of free, reserved, frozen and flags.
- **Transfers.** `Balances.transfer_keep_alive` is pallet 10, call 3 on
  Kusama Asset Hub and pallet 31, call 3 on Astar. The existential deposit
  is 3 333 333 planck (0.000003333333 KSM) and 10^6 wei (10^-12 ASTR).
- **Fees.** Every endpoint quoted the same fee through both methods:
  59 163 625 planck (about 0.000059 KSM) and 39 656 489 331 791 367 wei
  (about 0.0397 ASTR) for the transfer.
- **Finality.** Both finalize through their relay chain; finalized heads
  trailed the best block by about 16 blocks on Kusama Asset Hub and 5 on
  Astar.
- **Astar endpoints disagree on metadata.** Dwellir's Astar node serves
  runtime 2500 metadata 24 bytes longer than the Astar and OnFinality nodes
  at the same block, though Astar's own node runs the same client as
  Dwellir's (5.51.2) and OnFinality's runs 5.50.0: Dwellir's `System.Version`
  constant lists 22 runtime APIs instead of 20. Spectra binds the metadata
  hash into a reviewed transfer, so a node that answers differently would
  refuse a transfer built on another; Dwellir is left out for Astar. The
  1RPC Astar endpoint answered intermittently with HTTP 522, Blast's Astar
  endpoint is discontinued, and OnFinality's answered intermittently with
  HTTP 504 later the same day.
- **History.** Subscan's API refuses requests without a key ("Unauthenticated
  access is disabled") on both networks. Astar's Blockscout indexes only the
  EVM side's `0x` accounts. No keyless address history was found for either.
