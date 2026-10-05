# Supported chain scope

Spectra has 50 mainnets and 40 test networks. A chain entry identifies a network;
it does not promise every asset protocol or an address indexer. Chain facts live
in `registry::Chain`, provider claims in `core/data/endpoints.toml`, and every
operation refuses missing capabilities before signing or changing holdings.

## Wallets and transfers

All catalog networks have native-transfer signing paths. Cardano signs its
CIP-1852 extended Ed25519 key; Bittensor and Polkadot use verified genesis and
live runtime metadata rather than hard-coded pallet indices. Cardano ADA-only
transfers select inputs with an explicitly empty native-asset list; token-bearing
inputs are left untouched. A wallet with insufficient pure-ADA inputs cannot send
an ADA-only transfer.

Raw private-key import covers 49 mainnets and their corresponding test networks.
Ed25519 chains accept 32-byte seeds, Substrate chains accept sr25519 seeds, and
secp256k1 chains accept their validated scalar. Cardano accepts a validated
64-byte extended key. Monero's spend/view key model uses mnemonic import instead
of a generic 32-byte key. Watch-only addresses are available on supported
mainnets and test networks;
Monero requires wallet scan keys. Bitcoin account xpub import validates the
concrete network, including testnet 3, testnet 4 and signet.

BTC, BCH, BSV, LTC, DOGE and PPC support account address discovery and multiple owned
inputs. ZEC, BTG, DCR, KAS and DASH wallets have one derived address per network;
receive and send share that address. They do not offer an account-wide gap scan
or hand out child addresses that their signer cannot spend.

Peercoin mainnet and testnet support BIP-44 legacy, BIP-49 nested SegWit,
BIP-84 native SegWit and BIP-86 Taproot accounts, raw scalar and watch-only imports, account
discovery and receive rotation. Balances aggregate owned receive and change
addresses, using six-decimal PPC amounts. Ordinary version-3 transfers retain
the actual P2PK/P2PKH/SegWit/Taproot prevout script and derive each signing key from
its owned source. Miner and coinstake rewards remain in the total balance but
are excluded from spendable inputs until 500 mainnet or 60 testnet confirmations.
Recipient and change outputs follow Peercoin Core's 0.01 PPC wallet minimum;
smaller change joins the reviewed fee. Fees reserve the full serialized size,
including witness bytes, at 0.01 PPC/kB with a 0.001 PPC floor. Transfers
choose the largest inputs needed; MAX uses a deterministic set within
the single-transaction money bound and a conservative 100 kB serialized-size limit.
Aggregate wallet balances have no single-transaction money cap. The current
Blockbook network and precision must match before reads or submission.
Peercoin minting requires an online node and is outside this transfer integration;
the app does not advertise a minting/staking action. The [dated audit](audits/peercoin-2026-10-04.md)
records protocol sources, verified keyless reads and submission-route checks;
no funded live transaction was broadcast during verification.

## Fungible assets

| Network family | Supported token transfer protocol |
| --- | --- |
| Every EVM network, including Ethereum Classic and HyperEVM | ERC-20 and the network's registry label |
| Solana | Standard SPL and ordinary Token-2022 transfers |
| Tron | TRC-10 asset IDs and TRC-20 contracts |
| Sui | `Coin<T>` programmable transactions with separate SUI gas |
| Aptos | Legacy Move coin and primary fungible store transfers |
| TON | TEP-74 jettons through the owner's verified jetton wallet |
| NEAR | NEP-141 |

Token precision comes from validated metadata. TON master identity uses the raw
account address, with friendly-address network flags checked first. Raw,
bounceable and non-bounceable aliases cannot create duplicate holdings.
Jetton amounts use TEP-74's 120-bit integer range, including amounts beyond
64-bit units; values outside the wire format are refused before signing.

A network lists all supported standards in `token_standards`; each deployment
owns its actual protocol. Tron accepts TRC-10 and TRC-20, and Aptos accepts both
legacy Move coins and AIP-21 fungible assets. No caller chooses a protocol from
the first entry of a chain list. TRC-10 validates a canonical numeric asset ID,
on-chain precision and balances before creating or signing a token transfer.
TransferAssetContract bytes and signatures match independent TronWeb fixtures;
mainnet and Nile retain distinct network identity.

XRP issued currencies, Stellar issued assets and
Cardano native assets are outside the tracked-token interface. Zcash supports
transparent transfers; shielded addresses and transfers are excluded. Litecoin
supports transparent SegWit; MWEB is excluded. Solana transfer-fee, transfer-hook
and unknown Token-2022 extensions are refused until their semantics can be
reviewed and signed correctly.

The [follow-up source audit](audits/token-verification-2026-10-04/README.md)
verifies and reactivates 58 of the 59 preserved deployments, along with all 21
previously commented identities and wiki descriptions. The reactivated records
retain their original addresses, standards and precision. Primary issuer/protocol
documents, official bridge lists and exact bridge mappings establish identity; all 59 original
contracts also pass fixed-block chain-ID, bytecode and decimals reads.
The unresolved Avalanche PEPE deployment was removed at the user's explicit
request; its underlying Ethereum PEPE and reciprocal bridge relationship are
proved, but issuer or official bridge operator identity remains unresolved.
Ethereum, Arbitrum and BNB PEPE remain active.
The [original audit list](audits/chain-support-2026-10-04/removed-token-deployments.json)
retains the earlier decision as history. Explicit custom assets remain available.
Verified legacy BTTOLD (TRC-10 ID `1002000`)
is separate from the redenominated TRC-20 BTT; it cannot inherit BTT's market
quote or conflate their 1:1000 denomination.

## History and confirmation

Indexed native and token history use bounded provider pages. Exhaustion follows
the raw provider page, including pages that contain no matching transfers.
Continuation is persisted after the transaction merge; a restart can repeat a
page safely but cannot skip it. A failed refresh retains the old continuation;
a successful refresh starts at the newest page while keeping older saved rows.

Aptos uses the public address Indexer for received, submitted and orderless
transactions, with fullnode network verification. Sui, Aptos, TON and NEAR token
transfers are included alongside native history. Provider failures are reported;
they do not produce an empty successful history.

Submitted transfers have confirmation polling. Account chains read the exact
transaction's execution result, including failed transactions omitted by transfer
history. TON follows the external message into its execution trace; jetton
confirmation also checks the actual token-delivery action. The UTXO families read
their transaction status; Substrate transfers resolve finalized extrinsic events
to success or failure. OP Stack actual fees include execution, L1 data and the
applicable historical operator charge. The oracle is read at the mined block;
missing or malformed components leave the total unknown while preserving the
confirmed execution outcome.

Only verified keyless public indexers are built in. BNB, HyperEVM, Linea, Sei,
Cronos, opBNB, Sonic, Berachain, X Layer and Monad require a compatible custom
address indexer for history and automatic token discovery. A Blockscout-compatible
source can be configured on any EVM network. RPC-only access still supports
native transfers, tracked ERC-20 balances/transfers and submitted receipts.
Polkadot/Westend and Bittensor have no built-in keyless address-history indexer;
their verified RPCs support balances, transfers and finality.

BCH, DOGE, ZEC, DCR and DASH test networks have no verified built-in keyless
provider. Their supported custom API must be configured before network reads or
transfers. Failed providers are not retained as fallback entries. Mainnet Monero
and stagenet defaults support local scanning; Avalanche defaults read ERC-20
balances. [Dated read-only endpoint checks](audits/chain-support-2026-10-04/endpoint-probes.json)
verify network identity and selected reads, not live transaction broadcast.

## Staking positions and execution

The Solana, Sui, Aptos, NEAR, Polkadot and ICP mainnets expose owned live
positions and real transaction preparation, signing and explicit broadcast. Core derives each
action from current ownership, stake and unlock state. Watch-only wallets cannot
sign. Unknown yield or reward amounts remain unknown. Reviews include exact
amounts, fees or fee budgets, SOL account rent and explicit ICP lock periods.
Artifacts, signed payloads and execution receipts survive a restart; late UI
reads cannot switch wallet identity or overwrite a newer review.

| Network | Executed actions and ownership |
| --- | --- |
| Solana | Wallet-controlled stake account creation/delegation, full deactivation, unlocked withdrawal and simulated excess-lamport withdrawal. Staker, withdrawer and lockup authority are checked. |
| Sui | System staking and whole withdrawal of owned StakedSui objects, including the chain's reward payout in that withdrawal. No artificial unlock delay. |
| Aptos | Delegation-pool stake, unlock and withdrawal using actual delegator balances and Move simulation; validator identities alone do not prove delegation pools. |
| NEAR | Verified delegation-pool deposit_and_stake, unstake and withdraw with full-access-key authority, storage and gas budgets. Existing pool discovery uses Fastnear plus durable core targets. |
| Polkadot | Asset Hub nomination-pool join/top-up, unbond, withdraw and claim, using live runtime metadata, pool points, migration and slashing state. Required prerequisites execute atomically. |
| ICP | Controller-owned NNS neuron funding/claim, explicit dissolve delay and following, whole-neuron dissolve, disbursement and delayed maturity payout. Hot keys cannot withdraw another controller's neuron. |

ICP signs each funding and governance ingress locally. Each successful step
requires a certificate verified against the pinned IC root key, exact canister
and request ID, then persists that proof immediately. A transfer alone cannot
confirm the whole operation. Interrupted new-neuron setup can prepare a recovery
review only after the original transfer is proved; it retains the same
controller, nonce and derived subaccount and never creates another funding
call. Recovery requires a new review, signature and explicit broadcast. Pending
ingress cannot be replaced while it may still execute. The application accepts
new dissolve delays up to two years, counted from the configuration execution;
this is an explicit application bound, not an assertion that every existing
neuron has that maximum. Maturity claims require at least 1 ICP of maturity, a
non-spawning neuron and fewer than ten pending payouts. The zero-fee minting payout is queued for seven
days, subject to governance modulation, and displayed separately from liquid ICP.
Principal disbursement fees are deducted from the reviewed amount once;
full disbursement does not subtract already-accounted neuron fees again.

Staking history distinguishes stake, unstake, withdraw and reward claims.
The four account chains compound rewards or pay them during withdrawal rather
than exposing a separate reward-claim operation; DOT and ICP have explicit claims.
NEAR native, token and staking fee budgets use live protocol action/receipt
overhead and gas prices, including the protocol purchase minimum for prepaid
gas. Creating an implicit account includes its account and access-key costs.
NEP-141 includes the transfer call and attached deposit. Storage reserve, funds
and the reviewed budget are checked again before signing or submitting; pending
retries also check the current budget because signed bytes do not cap gas price. Four account chains resolve exact transaction execution outcomes;
Substrate resolves finalized extrinsic events. Manual status checks bypass background
poll backoff. NEAR native, token and staking transactions validate their canonical
reference block against the live protocol validity period, rather than an
arbitrary wall-clock timeout. Native and NEP-141 sends retain the local hash
checked against official SDK vectors; a foreign node receipt remains uncertain. No age, provider failure or node acceptance alone
proves success or failure. Official SDK fixtures and loopback CLI checks exercise signing,
reopen, uncertain submission, final outcomes and refusal paths without sending
real funds. Dated Solana mainnet simulations validate current instruction layouts
with zero signatures and no broadcast.
