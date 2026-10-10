# Open items

Engineering work that remains. Rule 0 in [AGENTS.md](../AGENTS.md) applies.
Delete an item once it is done; what changed belongs in
[BEHAVIOUR-CHANGES.md](BEHAVIOUR-CHANGES.md).

## Tasks

- [ ] **Audit endpoints across all supported chains and networks.** Review
  the endpoint catalog, default selection and custom endpoint routing. Verify
  each endpoint's concrete network identity, `EndpointApi`, advertised
  capabilities, access requirements and availability; confirm built-in defaults
  need no API key. Cover reads, history/token discovery, fees, transaction status
  and broadcast routing where advertised.
  Check URL validation, health probes, failure reporting and fallback behaviour
  against the same core rules. Record dated, reproducible evidence per endpoint,
  remove or correct unsupported entries, and prove routing/refusal paths through
  offline CLI fixtures. Keep live probes read-only; any resulting behaviour
  changes need `BEHAVIOUR-CHANGES.md` entries and `make verify`.

## Supported chain scope

[CHAIN-SUPPORT.md](CHAIN-SUPPORT.md) defines supported protocols, import formats,
provider coverage, staking actions and explicit exclusions. The [follow-up asset
audit](audits/token-verification-2026-10-04/README.md) reactivates 58 deployments
and all associated identity/wiki records without changing original fields.

The following gaps were identified in the 2026-10-04 code review. An absent
feature is not a supported capability; add its registry capability only when
core owns the complete operation and the CLI can prove it. Protocol and
funds/key changes require independent vectors, CLI acceptance, the relevant
refusal paths and `make verify`, with behaviour changes recorded separately.

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

### Multisig accounts

Every network with a multisig scheme has its multisig account and sessions
([CHAIN-SUPPORT.md](CHAIN-SUPPORT.md#multisig-accounts), and the 2026-10-08
entry in [BEHAVIOUR-CHANGES.md](BEHAVIOUR-CHANGES.md)): a policy core reads and
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

- [ ] **Add built-in EVM history and token-discovery providers.** BNB,
  HyperEVM, Linea, Sei, Cronos, opBNB, Sonic, Berachain, X Layer and Monad
  currently require a custom Blockscout-compatible address indexer. Add only
  independently verified keyless sources for the concrete network, including
  the corresponding test networks where coverage is absent. Verify network
  identity, paginated native/token history, discovery and failure reporting;
  an ordinary RPC does not supply complete address history.
- [ ] **Implement Polkadot/Westend and Bittensor address-history adapters.**
  Their current Substrate RPC path supports balances, transfers and submitted
  finality, but no complete address history. A custom RPC cannot fill the gap:
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
- [ ] **Implement Peercoin minting if it is brought into product scope.**
  Minting is explicitly excluded today. It needs an online node and its own
  authority, key-access, process-lifetime and recovery model; a staking catalog
  entry alone is not an implementation. Resolve those requirements in core
  and prove the lifecycle through the CLI before exposing the action.
