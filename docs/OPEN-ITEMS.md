# Open items

Engineering work that remains. Delete an item once it is done. Rule 0 and
[Funds and keys](../AGENTS.md#funds-and-keys) in `AGENTS.md` apply to all of it.

## Tasks

- [ ] **Verify an endpoint's network before reading from it.** Most XRP
  Ledger, Tron and Stellar reads, and a few Cardano, NEAR and TON ones, ask a
  node its network first; every other endpoint is asked only by the health
  probe and before a broadcast, so a custom endpoint on another network (a
  Sepolia RPC saved as Ethereum's) answers balances and history beside the
  built-in ones. Ask each endpoint once per process through
  `api::network::verify_network`, before any read is raced to it, and drop
  the per-read checks.
- [ ] **Follow Zcash's next network upgrade.** On 2026-11-26 Zebra 7.0.0 nodes
  at height 3,613,715 reported consensus branch `0x77190ad9`, which
  `zcash_protocol` 0.10.6 does not know: core refuses every transparent and
  shielded Zcash send ("update before sending"). Move the Zcash crates to a
  release that names the upgrade and its activation height, with SDK vectors
  for both pools.
- [ ] **Price opBNB and BNB testnet transfers when the fee history says
  zero.** Their nodes' fee history sometimes gives a zero base fee and a zero
  lowest tip (2026-11-26), and core refuses the zero fee, so a send fails
  until a block holds a tipping transaction. Give each network the tip floor
  its nodes include, from the network's own documentation, as Chiliz and 0G
  have.
- [ ] **Read Cosmos history from pruned nodes.** A transaction search on a
  node that no longer keeps the blocks of some matching transactions fails
  whole (HTTP 500 "height … is not available") instead of answering the newer
  ones, as Injective's and BandChain's own nodes did on 2026-11-26. Bound the
  search by the node's lowest kept height and say where the history stops.

## Supported chain scope

[CHAIN-SUPPORT.md](CHAIN-SUPPORT.md) defines supported protocols, import formats,
provider coverage, staking actions and explicit exclusions. These are the gaps
in it that the 2026-10-04 code review identified. An absent feature is not a
supported capability.

### Asset and privacy protocols

- [ ] **Support CIP-113 programmable tokens on Cardano.** CIP-0113
  (Proposed; merged on 2026-09-29 and launched by the Cardano Foundation on
  2026-10-07) holds a programmable token at the shared `programmableLogicBase`
  script address under its owner's stake credential, which a wallet sets to
  its payment key hash. An on-chain registry keyed by policy ID marks a
  policy programmable, and a transfer spends from the script address with the
  protocol-parameters UTxO and the registry's nodes as reference inputs,
  running the global and the token's own transfer logic as zero-amount
  withdrawals; the issuer's logic can also freeze, seize or deny holders.
  Spectra reads only the wallet's own address, so these tokens are invisible
  to it today, and no plain transaction moves one. Derive the wallet's
  programmable address from its payment key hash, read balances and history
  there, identify a token by the registry, and give it its own standard,
  `CIP-113`, beside `Cardano Native Token`. Build transfers with the script
  executions, their execution units, collateral and reference inputs; refuse a
  policy the registry does not list and a transfer its logic rejects, and say
  in the review what the issuer's logic can do. Prove the address, the
  registry proofs and the transaction bytes against the reference
  implementation (`cardano-foundation/cip113-programmable-tokens`), and the
  receive, transfer, refusal and recovery paths through the CLI. The standard
  is Proposed and its reference substandards are marked for testnets: build
  against what issuers deploy on mainnet.
- [ ] **Spend Sui address balances.** Sui holds an address's funds in coin
  objects and in its address balance (the balance accumulator), and mainnet
  addresses already hold SUI and tokens in the latter. Spectra shows both but
  builds transactions from coin objects alone, so a send, merge or stake
  cannot reach what the address balance holds. Build the withdrawal from the
  address balance, and gas paid from it, as `@mysten/sui` does; prove the bytes
  and digests against SDK vectors, select between coin objects and the address
  balance in core, simulate before review, and drive the send, merge and
  stake paths through the CLI.

### Multisig accounts

Every network with a multisig scheme has its multisig account and sessions
([CHAIN-SUPPORT.md](CHAIN-SUPPORT.md#multisig-accounts)): a policy core reads and
checks itself, a review digest a signature is given for, signatures verified
before they count, and sessions that survive a restart. None depends on a
coordination service that needs an API key; signatures travel between
cosigners as data, as PSBTs do. What remains:

- [ ] **Set up and change multisig signer sets.** Every multisig account is
  spent from, never created or changed: deploying a Safe and changing its
  owners or threshold, Tron's `AccountPermissionUpdate` (which replaces the
  whole permission set for a fee), XRP's `SignerListSet`, Stellar's
  `SetOptions` signers and thresholds, and TON's `update_multisig_params`
  order. Each needs a review as strict as a send's — on most of these
  networks a wrong signer set locks the account — and should state what
  the account can no longer do afterwards. Policies that derive the account
  (UTXO descriptors, Sui, Aptos MultiKey, Cardano scripts, Substrate
  signatories) change only by moving the funds to a new account, which is a
  spend.
- [ ] **Spend tokens from multisig accounts.** Sessions pay the network's own
  coin only. ERC-20 from a Safe, TRC-10/TRC-20 from a Tron account, issued
  currencies and Stellar assets, Cardano native assets and TON jettons each
  need their transfer encoded in the session, decoded back on review and
  checked against the token's identity, as single-key sends already do.
- [ ] **Cancel a pending Substrate multisig operation.** A `pallet-multisig`
  operation whose first approval is on chain keeps its depositor's deposit
  reserved until it executes or the depositor cancels it (`cancel_as_multi`).
  Discarding the session here leaves it pending; offer the cancel to the
  depositor's wallet, with the timepoint read from the chain.

### History and provider coverage

- [ ] **Add built-in EVM history and token-discovery providers.** Arbitrum,
  Base, Polygon, Arbitrum Sepolia, Plasma, BNB, HyperEVM, Linea, Sei, Cronos,
  opBNB, Sonic, Berachain, X Layer, Monad, Kaia, XDC Network, Robinhood Chain,
  Kava EVM, Stable, Arc, Katana, peaq and 0G currently require a custom
  Blockscout-compatible address indexer. Add only independently verified
  keyless sources for the concrete network, including the corresponding test
  networks where coverage is absent. Verify network identity, paginated
  native/token history, discovery and failure reporting; an ordinary RPC does
  not supply complete address history. Robinhood Chain's and Arc's Blockscout
  instances answer non-browser clients with a Cloudflare challenge, and 0G's
  `chainscan.0g.ai/open/api` speaks a variant of the Etherscan API
  (`timestamp`, hex `gas` and `gasUsed`, `addresstokenbalance`) that the
  Blockscout adapter does not read (docs/audits/evm-mainnets-2026-11-24).
  Arbitrum's, Base's, Polygon's and Arbitrum Sepolia's Blockscout instances
  began answering every client with a Cloudflare challenge, and Routescan
  stopped serving Plasma (docs/audits/endpoints-2026-11-26).
- [ ] **Implement Substrate address-history adapters** for Polkadot/Westend,
  Kusama Asset Hub, Astar and Bittensor. Their current Substrate RPC path
  supports balances, transfers and submitted finality, but no complete
  address history (Subscan needs an API key). A custom RPC cannot fill the gap:
  add an independently verified indexer API, registry capabilities, pagination
  and durable merge/continuation handling. Prove received and externally
  submitted transactions through CLI fixtures, not only local submissions.
- [ ] **Restore verified default providers for BCH, DOGE, ZEC, DCR and DASH
  testnets.** These networks currently require custom endpoints before reads
  or transfers. Verify the concrete network and every advertised capability,
  including balance, UTXOs, history, fee/status reads and submission routing.
  CLI checks must distinguish missing configuration from provider failure;
  retain no failed endpoint as a fallback.

### Staking coverage

- [ ] **Extend staking beyond the six supported mainnets and enable supported
  testnets.** Only SOL, SUI, APT, NEAR, DOT and ICP mainnets currently have
  adapters; all testnets are refused. Add other staking integrations one
  protocol at a time, with owned positions, exact action prerequisites,
  preparation/signing, explicit broadcast, final execution proof and durable
  recovery. Prove the whole action lifecycle and wrong-owner/network refusal
  through the CLI before enabling a network in the registry.
- [ ] **Read Sui staking rewards and departed validators.** Sui GraphQL
  reports neither a stake's reward estimate nor the validator of a pool that
  left the active set, so a Sui position's reward is unknown and such a
  position is named by its pool's id. Compute the reward from the pool's
  exchange rates at the stake's activation epoch and now, and read a departed
  pool's validator from the system state's inactive validators, checked
  against `@mysten/sui`'s staking reads.
- [ ] **Implement Peercoin minting if it is brought into product scope.**
  Minting is explicitly excluded today. It needs an online node and its own
  authority, key-access, process-lifetime and recovery model; a staking catalog
  entry alone is not an implementation. Resolve those requirements in core
  and prove the lifecycle through the CLI before exposing the action.
