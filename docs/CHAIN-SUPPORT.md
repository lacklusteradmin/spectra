# Supported chain scope

Spectra has 50 mainnets and 40 test networks. A chain entry identifies a network;
it does not promise every asset protocol or an address indexer. Chain facts live
in `registry::Chain`, provider claims in `core/data/endpoints.toml`, and every
operation refuses missing capabilities before signing or changing holdings.

## Wallets and transfers

All catalog networks have native-transfer signing paths. Cardano signs its
CIP-1852 extended Ed25519 key; Bittensor and Polkadot use verified genesis and
live runtime metadata rather than hard-coded pallet indices. A Cardano transfer
spends any of the address's outputs, those holding native assets included, and
its change returns every asset the inputs held that the transfer does not
send: outputs of the sent asset first, then ADA alone, then other
token-bearing outputs, each largest first. Every output holds the protocol's
minimum ADA for its size, the fee is the protocol's linear fee for the signed
size, both from the latest epoch's parameters, and change too small to stand
as an output joins the fee.

Each network restores the phrase formats its own wallets write and creates
in one every wallet for it restores. Most networks read and create BIP-39.
Monero reads its 25-word seed in all thirteen Monero wordlists and the 16-word
Polyseed in all ten of its lists, and creates the 25-word seed; a Polyseed's
birthday becomes the wallet's restore height, and an encrypted Polyseed is
refused because Monero imports take no passphrase. TON reads and creates
ton-crypto's 24-word mnemonic, including password-protected ones opened with
the derivation passphrase. Neither reads BIP-39, which none of their wallets
restore. A Monero wallet's restore height is fixed at import: typed, from a
Polyseed's birthday, near now for a created wallet (from monthly checkpoints
read off two public daemons), or the start of the chain. A Zcash wallet
restored from a phrase has one too, where its scan for shielded funds starts:
typed, near now for a created wallet (a recent block of each network and the
75-second target spacing, less a week), or Sapling's activation, the first
block a ZIP-32 key could receive at.

A phrase wallet derives along one of its network's derivation profiles at an
account index, or a custom path. Bitcoin and its test networks offer native
SegWit (default), legacy, nested SegWit and Taproot; Litecoin legacy (default),
native and nested SegWit; Peercoin all four; Bitcoin Cash its BIP-44 coin type
and the older coin type 0; Solana `m/44'/501'/{account}'/0'` and the older
`m/44'/501'/{account}'`; every other chain with a path its one standard
profile. NEAR walks SLIP-10 along `m/44'/397'/{account}'` as near-seed-phrase
does. Monero, TON, Polkadot and Bittensor derive without a path and refuse one.
A TON phrase or key holds one account per wallet contract: W5 (wallet v5r1,
the default and what a created wallet uses) or v4R2, chosen at import and
found by the used-account search. W5 folds the network's global id into its
wallet id, so its testnet account is not its mainnet one; v4R2's is the same
on both. Nothing stores the version: a send signs as the version whose
account the stored address is for the key, and refuses an address no version
gives it.
Every profile is checked at two accounts against independent implementations.
A NEAR phrase or key may hold a named account instead of its implicit one,
once a verified node lists the key among the account's full-access keys.
XRP and Stellar accounts exist only once they hold the network's reserve,
which the receive screen reads from the network while a wallet is empty.

Raw private-key import covers 49 mainnets and their corresponding test networks.
Ed25519 chains accept 32-byte seeds, Substrate chains accept sr25519 seeds, and
secp256k1 chains accept their validated scalar, each as 64 hex digits. Cardano
accepts a validated 64-byte extended key. Each chain also reads the encoding
its own wallets export: compressed WIF on the Bitcoin family (BTC, BCH, BSV,
BTG, LTC, DOGE, DASH, ZEC and PPC) with the network's own version byte, a
base58 or Solana-CLI JSON keypair on Solana, an `S…` secret seed on Stellar,
`suiprivkey1…` (Ed25519) on Sui, AIP-80 `ed25519-priv-0x…` on Aptos and an
`ed25519:…` key string on NEAR. A keypair whose public half is not its
secret's, a WIF for another network and an uncompressed WIF are refused: an
uncompressed key owns a different P2PKH address than the compressed key
Spectra signs with. Monero's spend/view key model uses mnemonic import instead
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
| Solana | SPL and Token-2022, including transfer fees and transfer hooks |
| Tron | TRC-10 asset IDs and TRC-20 contracts |
| Sui | `Coin<T>` programmable transactions with separate SUI gas |
| Aptos | Legacy Move coin and primary fungible store transfers |
| TON | TEP-74 jettons through the owner's verified jetton wallet |
| NEAR | NEP-141 |
| XRP Ledger | Issued currencies on trust lines (`CODE.rIssuer`) |
| Stellar | Credit assets on trustlines (`CODE:ISSUER`) |
| Cardano | Native assets (`POLICY.NAME` in hex) |

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

A Solana token transfer is one `TransferChecked` from the owner's associated
account, after an idempotent create of the recipient's. Token-2022 rules are
read from the mint before building and again before signing. A transfer fee
is computed for the current epoch as the program computes it (basis points of
the amount, rounded up and capped) and stated with `TransferCheckedWithFee`, so
a fee changed after review fails the transfer rather than withholding more;
the review shows the fee and what the recipient receives, and a fee that
changes in an epoch about to start is refused near the boundary. A transfer
hook's extra accounts are resolved from its validation account exactly as
spl-transfer-hook-interface resolves them (literal, PDA, instruction-data,
account-data, external-program and pubkey-data entries, read-only accounts
kept read-only), the transfer is simulated, and the review names the hook
program. Refused before anything is built: a hook asking for a signature, a
missing or foreign validation account, a simulation the hook fails, a paused
or non-transferable mint, a frozen sending or receiving account, a recipient
account that requires a memo or accepts only confidential transfers, a
recipient without an account when new accounts start frozen, an insufficient
associated-account balance, interest-bearing and scaled-amount mints (their
displayed amount is not the amount moved), and any extension not listed.

An XRP Ledger issued currency and a Stellar credit asset are identified by
their code and issuer together, so two assets with one code are two tokens in
the list, the balances, discovery and history; an XRP Ledger code is kept as
its three characters or 40 uppercase hex digits, a Stellar issuer in
capitals, and codes keep their case. Stellar amounts have seven places; an
XRP Ledger value has 16 significant digits at a floating exponent, which
Spectra keeps at 15 places, rounding a balance down, and a send refuses an
amount the ledger cannot hold exactly. A node's `account_lines` and an
account's Horizon balances list every line, history rows come from each
line's own change in the transaction, and a row of a listed token is named
as the list names it. Trust lines are a wallet operation: opening one sets the
largest limit (no rippling through the wallet on the XRP Ledger) and locks a
reserve, which the XRP Ledger waives for an account's first two objects;
removing an empty one frees it. A payment is refused before building when the
sender or recipient has no line, the issuer has not authorized it or has
frozen it, the recipient's line has no room, the recipient does not exist or
does not accept the sender's deposits, the issuer lets its currency ripple
on neither line, or the balance is short. An XRP Ledger issuer's transfer rate
is paid with `SendMax` (the amount times the rate, rounded up), reviewed as
the asset's transfer terms and read again before signing; paying the issuer
back takes none. A wallet that is an asset's issuer does not send it.

A Cardano native token (`Cardano Native Token`) is its policy and name,
written `POLICY.NAME` in lowercase hex (the policy alone for an empty name).
Its decimals are its CIP-68 fungible-token datum's, else the token
registry's, else none. Balances and discovery sum the address's outputs,
history gives each token its own row from the address's net change, and a
transfer of a token carries the minimum ADA its output needs, reviewed as ADA
sent with it. A CIP-113 programmable token is a native token held at a shared
script address under its owner's credential, which the wallet's address does
not hold; Spectra neither shows nor sends one. A Koios endpoint's genesis
network magic is checked before it receives a transaction.

Zcash holds transparent and [shielded funds](#zcash-shielded-funds).
Litecoin holds transparent and [MWEB funds](#litecoin-mweb-funds).

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

## Non-fungible tokens

Every EVM network holds ERC-721 and ERC-1155 tokens, each one a collection's
contract and a token id, and an ERC-1155 holding a whole-number quantity of
it. Neither is a balance: no token id or quantity is ever scaled by decimals,
and no NFT is a holding, a price or part of the portfolio total.

| What | Source |
| --- | --- |
| The NFTs a wallet holds | A Blockscout explorer's inventory (`/api/v2/addresses/{address}/nft`), read to its end |
| The standard | The contract's own ERC-165 answer |
| Ownership and quantity | `ownerOf` and ERC-1155 `balanceOf`, read live before building and again before signing |
| History | `tokennfttx` and `token1155tx`, from Blockscout or Routescan |
| Transfer | The collection's `safeTransferFrom`, from the wallet, with no value |

A network whose only explorer is Routescan has NFT history but no inventory,
so listing a wallet's NFTs there is refused rather than answered empty; a
Blockscout endpoint added for it lists them. A transfer is refused before
anything is built when the contract reports neither standard, reports both,
or claims every interface; when the wallet does not own the ERC-721 token or
holds fewer of the ERC-1155 id than it sends; for an ERC-721 quantity other
than one, a zero quantity, the wallet's own address or the zero address. A
contract recipient must accept the token or the transfer reverts. History
gives each token its own asset (`ethereum:erc-721:0xcontract:1234`), its
quantity the amount. A contract that reports an NFT standard is never read as
a fungible token: its ERC-20 metadata read is refused, so it cannot be sent as
one, and a tracked token whose contract turns out to be a collection holding
something has its balance refused. Token images are not loaded, since a
token's metadata can point at any host. Contracts that predate ERC-165's
ERC-721 interface, such as CryptoKitties and CryptoPunks, are not NFTs here.

## Zcash shielded funds

A Zcash wallet restored from its phrase holds a shielded account: the ZIP-32
account its standard path names (`m/44'/133'/account'/0/0`, coin type 1 on
testnet), so its transparent receiver at index 0 is the wallet's address. A
wallet added from a key, watched, or derived along a custom path or HMAC key
has none. The account receives at a unified address with Orchard and Sapling
receivers and no transparent one, so the wallet's public address is not tied
to it. librustzcash (`zcash_client_backend`, `zcash_client_sqlite`) does the
protocol work over a wallet database of the account's own beside Spectra's,
holding viewing keys, notes and note commitment trees and never a spending
key; it is deleted with the wallet.

| What | How |
| --- | --- |
| Scan | Compact blocks from a lightwalletd server, from the restore height, a bounded batch per call; each batch is kept, so a cancelled scan resumes |
| Trees | The server's tree state at the restore height, and the completed subtree roots of the Sapling, Orchard and Ironwood trees |
| Memos and spends | Each transaction the scan finds, read whole and decrypted on the device |
| Balance | Spendable and pending shielded ZEC, added to the wallet's ZEC holding; transparent ZEC the account can shield |
| History | Received into the shielded pools, sent out of them, and shielded, one row each |
| Fees | ZIP-317, as librustzcash's proposal chooses inputs and change |
| Signing | Proofs on the device: Orchard needs no parameters; a Sapling spend or output needs the published Sapling parameters, downloaded once from `download.z.cash` and used only when each file matches its pinned BLAKE2b-512 hash |
| Broadcast | The network's lightwalletd servers |

Shielding moves every transparent output of the wallet into its own shielded
pool, paying no one; a shielded payment pays one unified, Sapling or
transparent address, with a memo of up to 512 bytes for a shielded recipient.
Since NU6.3, shielded value and payments to an Orchard receiver land in the
Ironwood pool. A payment to a transparent address spends the Sapling pool
first when it covers the amount, as librustzcash prefers. The proposal is what
is reviewed: its encoding and what it pays, its memo and fee are bound into
the review digest, and signing decodes it against the database again and
refuses it if anything changed. Before signing, the server's consensus branch
must be the one librustzcash gives for the next block; a network past the
upgrades this build knows (the test network's NU7) scans but does not sign. A
TEX address (ZIP-320) takes only transparent funds, so the transparent send
pays it, the P2PKH script of its key hash, and the shielded one refuses it;
the transparent send refuses a Sapling or unified address. Every Zcash address
form is a valid address on its network for the address book and the send
screen. A restored wallet recovers a payment it sent from the chain with its
outgoing viewing key, which gives the receiver the payment reached, not the
whole unified address it was sent to.

A session reads one server that says it is on the wallet's network, with
Sapling's activation where the network has it; a server on another network is
refused. lightwalletd learns the restore height, which transactions the
wallet reads whole, and the transparent address it asks about. Every gRPC
connection goes through the same Tor kill switch, SOCKS5 proxy and
loopback-only guard as HTTP. [Dated read-only checks](audits/zcash-shielded-endpoints-2026-10-07.json)
of the built-in servers record their network, branch, trees and subtree roots.

## Litecoin MWEB funds

A Litecoin wallet restored from its phrase holds the phrase's MWEB funds,
whatever path its transparent address is at: the scan key is the BIP-32 key
at `m/1000'/0'` of its seed and the spend key the one at `m/1000'/1'`, where
Cake Wallet and mwebd derive them, so a phrase restored here finds what it
received there. One wallet per phrase and network holds them; the first to
sync claims them, and a second wallet of the same phrase is refused. A key, a
watched address or another network's wallet has none. The wallet receives at
its stealth address 2 (`ltcmweb1…`, `tmweb1…` on testnet); address 0 takes
change and address 1 peg-ins, as Litecoin Core keeps them, and a scan
recognizes the first 1,000. An MWEB address is a Litecoin address for the
send screen and the address book, and never a wallet's own: a watch import
refuses one, as nothing public shows what it holds.

| What | How |
| --- | --- |
| Anchor | The block the wallet's indexer names as its tip; a Litecoin node's headers after it are checked, each by scrypt proof of work and Litecoin's retarget, the retarget window's first time read the same way |
| Scan | Over the peer-to-peer protocol (`NODE_MWEB_LIGHT_CLIENT`): the node's tip's MWEB header, proved through the block's HogEx by its merkle root; the leafset, proved by the header's leafset root; pages of 4,096 unspent outputs, proved into the output root; eight pages a batch, each kept, so a cancelled scan resumes |
| Recognition | Each output's view tag, then its key exchange with the scan key, on the device; the scan key and what it finds stay there, the cache encrypted under a key derived from it |
| Balance | Unspent outputs, less what a signed payment spends, with what broadcast payments return to the wallet; added to the wallet's LTC holding |
| History | Outputs received at addresses the wallet gives out, dated by the scan that found them; payments this device sent, confirmed once their inputs are spent; peg-ins as `shield` |
| Fees | 100 litoshis per unit of MWEB weight (3 a kernel, 18 an output, a peg-out's script a unit per 42 bytes), as Litecoin Core weighs them; a peg-out's canonical bytes at 10,000 litoshis per kB |
| Signing | Inputs, outputs with their range proofs, and the kernel, made on the device from the seed; the transaction checked as a node checks one before it is stored |
| Broadcast | The wallet's Litecoin indexers |

A payment out of MWEB funds spends the largest outputs first and pays an MWEB
address inside MWEB, or any other Litecoin address by a peg-out whose amount
and script the kernel names; its change goes to address 0, unless what is left
would not pay for its own output, when it joins the fee. Litecoin Core relays
at most 1,000 inputs in one transaction. Moving transparent LTC into MWEB is a
peg-in to address 1: a canonical transaction from the wallet's transparent
outputs pays the script its kernel makes, the amount and the kernel's fee,
with the canonical fee beside it; a send from transparent funds to any MWEB
address is the same peg-in to that address. Litecoin Core's relay rules are
the builder's: a peg-in kernel is the one its canonical output names, a
transaction with a canonical part has no other kernel, and every output,
peg-in and peg-out alike, clears the dust threshold. What is reviewed is the
inputs, the recipient, the amount, the change and the fee; signing finds each
input among the wallet's unspent outputs again and refuses a review that does
not balance.

A light client reads unspent outputs only, so what was received and spent
before a wallet's first scan is not in its history, and a restored wallet's
change and peg-ins are its own outputs, not receipts. Nodes serve leafsets and
outputs only for blocks within ten of their tip, and only 32 such requests at
once, then one every two seconds, across every client they serve: requests
from here keep to that allowance per node, and one left unanswered fails the
batch, which can be run again. A chain that no longer holds the last scan's
tip is scanned again whole; the whole unspent set is a few dozen pages. The
node learns that a light client asked for its tip's outputs, nothing about
which are the wallet's; the indexer learns what the transparent address
always told it. [Dated read-only checks](audits/litecoin-mweb-nodes-2026-10-08.json)
of the built-in nodes record their service bits, heights and the activation
blocks.

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

BCH, DOGE, ZEC (transparent), DCR and DASH test networks have no verified
built-in keyless provider; Zcash testnet's shielded funds read a built-in
lightwalletd. Their supported custom API must be configured before network reads or
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
