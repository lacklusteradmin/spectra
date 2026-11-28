# Supported chain scope

Spectra has 90 mainnets and 40 test networks. A chain entry identifies a network;
it does not promise every asset protocol or an address indexer. Chain facts live
in `registry::Chain`, provider claims in `core/data/endpoints.toml`, and every
operation refuses missing capabilities before signing or changing holdings.

## Wallets and transfers

All catalog networks have native-transfer signing paths. Cardano signs its
CIP-1852 extended Ed25519 key. A Cardano phrase holds the CIP-19 base address
mainstream wallets restore: the payment key at
`m/1852'/1815'/{account}'/{role}/{index}` (role 0 or 1) with its account's
stake key at `m/1852'/1815'/{account}'/2/0`; a path of any other shape names no
stake key and is refused. Balance, inputs and history are read there, change
returns there, and spending needs only the payment key's witness. A raw
extended key holds no stake key and keeps its enterprise address. A base
address's network account shows the stake address its stake credential names
(CIP-19 type 14, or 15 for a script's) with the registration, available rewards
and pool and DRep delegation Koios reports for it; an enterprise address has
none. The Substrate networks (Polkadot, Kusama Asset Hub, Astar and Bittensor)
use verified genesis and live runtime metadata rather than hard-coded pallet
indices. A Cardano transfer
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
restore. Algorand reads and creates its 25-word mnemonic, one account's key,
and also reads a BIP-39 phrase, whose accounts derive along ARC-52
(`m/44'/283'/{account}'/0/0`, BIP32-Ed25519 as Pera's Universal Wallet and
Lute derive it); a 25-word mnemonic takes no path or passphrase. A Monero wallet's restore height is fixed at import: typed, from a
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
and the older coin type 0; Fractal Bitcoin Bitcoin's four at coin type 0;
DigiByte all four at its coin type 20; Qtum legacy, native and nested SegWit
at coin type 88; eCash its coin type 1899 and the older 899; Hedera Ed25519
SLIP-10 `m/44'/3030'/0'/0'/{account}'` (default), ECDSA
`m/44'/3030'/0'/0/{account}` and EVM `m/44'/60'/0'/0/{account}`; Solana
`m/44'/501'/{account}'/0'` and the older `m/44'/501'/{account}'`; every other
chain with a path its one standard profile. A Starknet key is derived as
Argent X derives it: the Ethereum key at `m/44'/60'/0'/0/0` seeds a second
BIP-32 tree whose `m/44'/9004'/0'/0/{account}` key is ground into a Stark key. NEAR walks SLIP-10 along `m/44'/397'/{account}'` as near-seed-phrase
does. The Internet Computer derives the BIP-32 secp256k1 key at
`m/44'/223'/{account}'/0/0`, as quill, dfx and DFINITY's agent libraries do:
its principal is the self-authenticating one of the key's DER encoding with
the uncompressed point, and every ingress is signed with ECDSA over SHA-256. Monero and TON derive without a path and refuse one. Polkadot, its
Westend testnet, Kusama Asset Hub, Astar and Bittensor derive along a Substrate path of hard (`//`)
and soft (`/`) junctions under the phrase's sr25519 root, as subkey and
polkadot.js read a secret URI (none is the root key), checked against
polkadot.js; its `///password` is the passphrase, entered as one. A junction
the two read differently is refused: `0x` hex, which polkadot.js reads as
bytes, and a number past 64 bits. A key whose path has a soft junction has
no seed, so its wallet exports only its phrase.
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
For a recipient that shares one account among many, such as an exchange's
deposit address, an XRP payment or account deletion carries a destination
tag, a Stellar payment or merge a text memo of up to 28 bytes or an ID
memo, of XRP and issued assets alike, and a TON transfer or multisig order
a text comment of up to 123 bytes, one cell; a jetton transfer forwards it
to the recipient as its forward payload; a Cosmos transfer a text memo of up
to the network's `max_memo_characters` bytes (512 on Cosmos Hub, Terra, Kava
and BandChain, 256 elsewhere); and a Hedera transfer a memo of up to 100
bytes, with no zero byte. It is reviewed with the transaction and
bound into its review digest. A destination that asks for one — XRP's
`lsfRequireDestTag`, Stellar's SEP-29 `config.memo_required` — is refused
without one, read from a verified node at build and again before signing;
TON, the Cosmos SDK and Hedera have no such flag. A scanned code's tag or memo comes with its address
(XRP's `dt`, SEP-7's `memo`, a `ton://transfer` code's `text`); one Spectra
cannot send, such as a TON code's `bin` message body, refuses the code.
Destinations are classic `r…` and `G…` accounts; XRP X-addresses and Stellar
muxed `M…` accounts, which carry the tag in the address, are not accepted.

An EVM transfer is an EIP-1559 transaction priced from the network's fee
history, asked for with its block count as a hex quantity, which XDC and
Somnia nodes require: the max fee is twice the next base fee plus the last
block's lowest sampled tip, raised to the network's floor where it has one
(1 gwei on Chiliz, 4 gwei on 0G, whose blocks fill with transactions that tip
nothing). Ethereum Classic (and Mordor), Kava EVM, Merlin and Metis nodes
take only legacy transactions: a
send there is an EIP-155 transaction at the node's `eth_gasPrice`, which fills
both reviewed fee fields, and carries no access list. Rollup data fees follow
each network's oracle: Manta Pacific, Katana and MegaETH read Fjord's upper
bound and the operator fee, Metis Bedrock's `getL1Fee`, and Robinhood Chain,
an Arbitrum Orbit chain, folds its L1 cost into gas. Every new EVM network
derives along `m/44'/60'`. An EVM address written in mixed case is read
with its EIP-55 checksum, wherever it is typed, scanned, resolved from ENS or
kept in the address book, and refused when the checksum fails; one written
in a single case carries none. An XDC address is also read with the `xdc`
prefix its wallets write, and stored and sent as `0x`.

Sui is read and written through Sui GraphQL, Mysten's and ZAN's public
endpoints built in; Sui's JSON-RPC is not spoken. An endpoint is the network
whose checkpoint-zero digest it answers as its chain identifier. A Sui
balance, SUI's or a token's, counts the address's coin objects and its address
balance together. A transfer, coin merge or stake spends coin objects only and
pays gas with SUI coin objects, so what the address balance holds is shown
but cannot be sent from Spectra.

A Cosmos SDK network (Cosmos Hub, Celestia, dYdX Chain, Terra, Kava, Injective,
Akash, Babylon Genesis, Osmosis, Dymension, Saga, Allora and BandChain) sends
its own coin with one bank `MsgSend`, signed in `SIGN_MODE_DIRECT`. Its keys
are BIP-32 secp256k1 along `m/44'/{coin}'/{account}'/0/0`, at coin type 118,
Terra's 330, Kava's 459, BandChain's 494 and 60 on Injective and Dymension,
whose Ethermint keys take the key's Ethereum address as the account and sign
over Keccak-256 of the sign document; the others take RIPEMD-160 of SHA-256
of the compressed key and sign over SHA-256. An address is classic bech32
under the network's prefix, in one case, of 20 bytes or a module's 32; another
network's prefix is refused. The gas a node's simulation of the transfer uses,
with two fifths more, is priced at the network's gas price: chain-registry's
`low` price, or a fee market's live price and a fifth more where that is
higher (Skip's `x/feemarket` on Cosmos Hub and Allora, the `txfees` base fee
on Osmosis and Injective). A transaction is valid for about ten minutes of
blocks (its `timeout_height`), from an account the network has, at its next
sequence, with the amount and fee in its spendable balance (vesting coins
excluded); each is read again before signing. The sign document binds the
chain id a verified node's newest block names. The public key in its
`AuthInfo` is the signing key's, written at signing: a wallet stores none.
Tokens (IBC denominations, CW-20s), staking and multisig are not offered.
Evidence is in [the 2026-11-24 audit](audits/cosmos-2026-11-24/README.md).

Kusama is its Asset Hub, where KSM balances now live, and Astar its own
chain; each sends its coin with `Balances.transfer_keep_alive`, encoded from
the runtime's metadata as on Polkadot. Every Substrate network reads an
address only under its own SS58 prefix (0 on Polkadot, 2 on Kusama, 5 on
Astar, 42 on Westend and Bittensor), so a generic `5…` address is refused on
Polkadot. Kusama Asset Hub pays its fees through `ChargeAssetTxPayment` and
Astar through `ChargeTransactionPayment`; metadata naming another fee
extension is refused. Staking, multisig and Kusama's `Assets` pallet are not
offered. Evidence is in [the 2026-11-24 audit](audits/substrate-mainnets-2026-11-24/README.md).

An Algorand send is one `pay` transaction in canonical msgpack, signed with
Ed25519 over `TX` and its bytes. Its fee is the network's minimum or its
per-byte fee for the signed size, whichever is more, and it is valid for
1,000 rounds from the node's last round; the genesis id and hash the node
names beside that round are bound in. Refused before signing: a rekeyed
sender, an amount and fee more than the balance above the account's minimum
balance, a recipient left below 0.1 ALGO, the least an account holds, and
assets or notes.

A VeChainThor send moves VET, VTHO or a tracked VIP-180 token in one clause of
a dynamic-fee transaction (type `0x51`), signed over BLAKE2b-256. Its fee is
paid in VTHO, a VET transfer's too. The gas is the clause's intrinsic gas and
what a node's simulation at the next block uses, a fifth more (at least
15,000) when code runs; the max fee is twice the base fee plus the node's
priority fee, and the VTHO balance must cover the gas at the base fee and that
tip, as nodes require. A transaction refers to the best block and expires 60
blocks later. A mixed-case address needs its EIP-55 checksum. Fee delegation
(VIP-191), several clauses, contract calls and memos are not offered.

A Filecoin wallet is an `f1` secp256k1 account at `m/44'/461'/{account}'/0/0`
and pays `f0`, `f1`, `f2`, `f3` and `f410f` addresses: an `f410f` address, or
an `f0` or `f2` actor a verified node resolves to one, with `InvokeEVM`,
anything else with method 0. A `t…`, `0x…`, mixed-case or non-canonical
address is refused. The message is DAG-CBOR, signed with recoverable
secp256k1 over BLAKE2b-256 of its CID. Gas is the node's estimate, its gas
limit times fee cap, the reviewed fee, at most 0.07 FIL; the nonce is the
node's next, read again before signing and broadcasting. The node must name
the mainnet and its genesis before a build, a signature, a status read or a
broadcast.

A Tezos wallet is a `tz1` Ed25519 account at SLIP-10
`m/44'/1729'/{account}'/0'`, as Temple, Kukai and Umami derive it. A send is
one transaction operation, with a reveal before it while the account's key is
unrevealed, forged here for the current protocol (Ushuaia) and signed over
BLAKE2b-256 of its watermarked bytes; the operation hash computed here must be
the node's. It pays `tz1` to `tz4` and `KT1` addresses (`tz5` is refused).
Gas and storage come from simulating the operation, its fee from the
protocol's minimal-fee rule and 100 mutez more; the reviewed fee includes the
most its storage can burn (0.06425 XTZ to an empty account). Before signing,
the chain id, protocol, branch, counter and reveal state are read again and
the exact batch is simulated with the signing key; a protocol switch at the
next block refuses it. The balance is the spendable one: staked tez are not
counted. FA1.2 and FA2 tokens, staking and memos are not offered. Evidence is
in [the 2026-11-24 audit](audits/tezos-2026-11-24.md).

A Starknet wallet is a Ready (Argent X) account of class 0.4.0 whose address
is that class deployed with the wallet's Stark key. It sends STRK and ERC-20
tokens (the catalog lists ETH) in v3 invoke transactions, hashed with Poseidon
and signed with Stark ECDSA; an account not yet deployed deploys itself in the
same review, at the next nonce before the transfer's. Resource bounds and
prices carry half again the node's estimate and are paid in STRK. Another
account class, a guardian, a changed owner key and memos are refused.
Evidence is in [the 2026-11-24 audit](audits/starknet-2026-11-24.md).

A Hedera wallet is an existing account `0.0.N`: a key alone has none. The
account is found by its key on a verified mirror node, or named at import, and
accepted only when that one key is all it holds (key lists and threshold keys
are refused); no phrase is created for Hedera. A send is a `CryptoTransfer` of
HBAR whose fee bound is ten times the $0.0001 base fee, in tinybars at the
better of the current and next exchange rates. It is valid for 120 seconds
from a start set at signing, signed for three nodes of the address book and
sent over gRPC TLS pinned to each node's certificate hash. A deleted
recipient, or one requiring its own signature, is refused. HTS tokens,
staking, contracts and account creation are not offered.

Raw private-key import covers every mainnet but Monero, 90 in all, and their
corresponding test networks.
Ed25519 chains accept 32-byte seeds, Substrate chains accept sr25519 seeds, and
secp256k1 chains accept their validated scalar, each as 64 hex digits. Cardano
accepts a validated 64-byte extended key. Each chain also reads the encoding
its own wallets export: compressed WIF on the Bitcoin family (BTC, BCH, BSV,
BTG, LTC, DOGE, DASH, ZEC, PPC, QTUM, FB, XEC and DGB) with the network's own
version byte, a base58 or Solana-CLI JSON keypair on Solana, an `S…` secret
seed on Stellar, `suiprivkey1…` (Ed25519) on Sui, AIP-80 `ed25519-priv-0x…`
on Aptos, an `ed25519:…` key string on NEAR, an `edsk…` key on Tezos (the
seed, or the 64-byte key whose public half is checked) and a DER key on
Hedera, where a bare 64-hex key is tried as Ed25519 and as ECDSA against the
network. Starknet takes only its Stark key, up to 64 hex digits below the
curve order. A keypair whose public half is not its
secret's, a WIF for another network and an uncompressed WIF are refused: an
uncompressed key owns a different P2PKH address than the compressed key
Spectra signs with. A Cardano phrase wallet exports only its phrase: its
payment key alone imports as the enterprise address, not the base address
that holds its funds. Monero's spend/view key model uses mnemonic import instead
of a generic 32-byte key. Watch-only addresses are available on supported
mainnets and test networks;
Monero is watched from its scan keys: a view-only wallet is its primary
address and private view key, refused unless the key is the one the
address's view key was made from (a subaddress or an integrated address is
refused). It scans with the view key alone and takes no password, but no
key image is known without the spend key: its balance is what it received,
and a spend made elsewhere does not lower it (`MoneroSyncStatus.spends_known`
says so), nor does it record outgoing transfers. It cannot send. Importing
the wallet's phrase upgrades it in place and scans again with the spend key.

A Monero wallet, from its phrase or its view key, watches its subaddresses
with wallet2's default lookahead: fifty accounts past the highest one an
output arrived in, and two hundred addresses past each account's highest
used, widened while it scans — a block is scanned again when its own
outputs widen the window. What was used is kept in the encrypted scan cache,
so a restart watches the same subaddresses. The balance sums every account;
a send spends outputs of any account and returns change to the primary
address. The receive address is account 0's: the primary address until an
output arrives there, then the first subaddress past the highest used or
handed out. The private view key is stored at import outside the password,
so a fresh subaddress needs none.

An account public key is watched on every
account UTXO network below, in the encodings its wallets write
(`Chain::account_key_versions`: `xpub`/`ypub`/`zpub` on Bitcoin, Fractal
Bitcoin, Peercoin, Qtum and DigiByte, `Ltub`,
`Mtub` and `zpub` on Litecoin, `dgub` on Dogecoin, `xpub` or `drkp` on Dash,
`dpub` on Decred, `kpub` on Kaspa, `xpub` elsewhere, and each test
network's own), only at an account's depth and only on its concrete
network: a testnet key is refused on mainnet, and testnet 3, testnet 4 and
signet each read their own.

On every UTXO network but Cardano and Monero — BTC, BCH, BSV, LTC, DOGE,
PPC, ZEC (its transparent funds), BTG, DCR, KAS, DASH, QTUM, FB, XEC and DGB,
and their test networks — a phrase wallet is a BIP-44 account. It stores the account's
public key, so a gap scan (twenty unused addresses past the last used one,
on the receive and the change branch) and a fresh receive address need no
password. The scan runs once on its own before a restored or watched
account's balance or history is first read, and again on request; what it
finds is stored. The balance and history sum every address the account is
known to hold, each read from the network's own indexer (Esplora, Blockbook,
Blockcypher, Whatsonchain, Insight, Bitcoin Cash's REST v2, Kaspa's REST API,
eCash's Chronik or UniSat's Open API on Fractal Bitcoin). A send spends the
largest confirmed outputs it needs (on Litecoin, every confirmed output),
across addresses, each signed with its own address's key derived along its
stored path, and returns change to the
wallet's own address; an output gone or changed after review is refused at
signing. A phrase is imported only at a path its profile's account names: a
path no account of the network's profiles holds is refused. A private key or
a watched address is the account's one address. The coin listing, address by
address with confirmations, covers the networks whose indexer counts them,
all but Decred and Kaspa. A Kaspa coinbase output is not spent, since the
indexer does not say when it matures. Zcash shields transparent funds at the
addresses librustzcash's own gap window tracks; transparent sends spend
every address the scan found.

A custom indexer on Bitcoin, Litecoin, Qtum, Fractal Bitcoin, eCash, DigiByte
or Zcash broadcasts only once it names its network's identity block: its genesis, or a
later block where networks share one (block 1 on Bitcoin and Fractal Bitcoin,
block 896,800 on eCash). Fractal Bitcoin's addresses and keys are Bitcoin's,
so a Fractal send checks every indexer it read its inputs from for that block
before it builds and again before it signs. Multisig accounts are
[their own section](#multisig-accounts).

Fees follow each network's rule for the transaction's size: Bitcoin's and
Litecoin's sat/vB rate over the estimated virtual size, and Fractal
Bitcoin's and DigiByte's at least 1 and 100 sat/vB, Qtum Core's 400 sat/vB
(its indexer estimates none), Peercoin's per kilobyte, the network's static
fee for each started kilobyte on Bitcoin Cash, Bitcoin SV, Dogecoin, Dash,
Bitcoin Gold and eCash, ZIP-317's 5,000 zatoshis
per logical action (at least two) on Zcash, at least dcrd's 10 atoms a byte
on Decred and at least the transaction's mass on Kaspa, each never below the
network's static fee. A recipient amount below the network's dust threshold
is refused, and change below it joins the fee: 546 units, DigiByte's 5,460
and Qtum's 72,800 satoshis, Decred's 6,030 atoms, Kaspa's 1,000 sompi, and Dogecoin Core's dust limit of 0.01 DOGE, below
which an output pays that much again in fee and below a tenth of which it is
not relayed. A Kaspa spend also keeps within the 100,000-gram standard limit
on its KIP-9 storage mass, which charges an output small beside the coins it
spends: selection takes more coins while its change is too small to stand,
and a send no selection fits, such as a tenth of a kaspa or less from a large
coin, is refused.

A Decred send pays the script its recipient names on the wallet's own
network: a secp256k1 pubkey hash (`Ds…`, `Ts…`) or a script hash (`Dc…`,
`Tc…`). Decred's pay-to-pubkey (`Dk…`), Ed25519 (`De…`) and Schnorr (`DS…`)
pubkey-hash addresses, and every address of the other network, are refused
before an input is read.

Qtum, Fractal Bitcoin, eCash and DigiByte read their own address forms:
Qtum's `Q…` and `M…` and version-0 `qc` SegWit (Taproot's activation there is
unconfirmed); Fractal Bitcoin's, which are Bitcoin's, Taproot included;
eCash's CashAddr under `ecash:` alone (a legacy or `bitcoincash:` address is
refused), signed with Bitcoin Cash's digest at fork id 0; and DigiByte's `D…`,
`S…` and older `3…`, and `dgb` SegWit and Taproot. A Qtum staking reward is
spent after 2,000 confirmations and an eCash coinbase output after 100; Qtum's
pay-to-pubkey coinstake outputs are not spent. Qtum's QRC-20 tokens, eCash's
eTokens, Fractal's inscriptions and runes, and multisig on these four are not
offered.

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

## Multisig accounts

A multisig account is one wallet whose spends several signers sign. Core
owns each network's scheme: it reads the policy from the network or derives
the account from it, shows the signers with their weights and thresholds,
and keeps each spend as a session (`multisig_sessions`) that survives a
restart. Every session has a review digest a signature names; every
signature is verified before it counts; what the network holds (inputs,
nonces, sequences, signer sets, deadlines) is read again before a signature
and before a submission; and data read from another signer or coordinator
is decoded and encoded again, refused unless it comes out byte for byte the
same. Signatures travel between signers as data in the network's own form:
no coordination service is used.

| Networks | Account | Policy | Signers sign with | Reaches the network |
|---|---|---|---|---|
| Bitcoin, Litecoin (and test networks) | P2WSH `sortedmulti` | `wsh(sortedmulti(k, …))` descriptor (`<0;1>` or `/0/*` keys with origins), up to 20 keys | a cosigner's phrase added to the account wallet | PSBT (BIP-174), finalized and broadcast |
| Bitcoin Cash, Dogecoin | P2SH `sortedmulti` | `sh(sortedmulti(k, …))` descriptor, up to 15 keys | a cosigner's phrase added to the account wallet | BCHN PSBT (SIGHASH_ALL\|FORKID); Dogecoin Core's partially signed transaction (legacy digest) |
| EVM networks | Safe 1.3.0 / 1.4.1 proxy | read from the contract: owners, threshold, nonce, modules, guard | owner wallets (EIP-712 `SafeTx`) | an owner wallet executes it and pays the gas |
| Tron | account permissions | read: owner and active permissions, weighted keys | wallets holding the keys | the transaction names the permission (`Permission_id`) |
| XRP Ledger | signer list | read: signers, weights, quorum | signer accounts' master keys | multi-signed payment, fee N+1 base fees, `LastLedgerSequence` |
| Stellar | account signers | read: signers, weights, low/medium/high thresholds | signer keys' wallets | envelope with every signature and time bounds |
| Sui | `MultiSig` address | `{"threshold", "publicKeys": [{"publicKey", "weight"}]}`, up to 10 keys | Ed25519 wallets | one combined `MultiSig` signature |
| Aptos | `MultiKey` account | `{"signaturesRequired", "publicKeys": ["ed25519-pub-0x…", …]}`, up to 32 keys | Ed25519 wallets | one MultiKey authenticator, submitted as BCS |
| Cardano | native script address | cardano-cli's native-script JSON (`sig`, `all`, `any`, `atLeast`, `after`, `before`) | phrase wallets' CIP-1854 keys (`m/1854'/1815'/0'/0/i`) | transaction carrying the script and every vkey witness |
| Polkadot Asset Hub, Bittensor | `pallet-multisig` account | `{"threshold", "signatories": [SS58 …]}`, threshold at least 2 | signatories' wallets, each submitting its approval | the approval meeting the threshold executes the call |
| TON | multisig v2 contract | read from the contract (its code hash checked): signers, threshold | signers' wallets: the first proposes the order, the others approve it | the approval meeting the threshold executes the order |

On a UTXO network the account's addresses are derived per index (BIP-67
sorted keys), so discovery, the balance, history and receive rotation are an
account's; a spend uses its largest confirmed outputs not held by another
open session, returns change to its next change address, and signs only
after the indexer confirms each input unspent and of the amount reviewed. A
Dogecoin transaction carries no amounts, so a session read from elsewhere
must spend the account's unspent outputs. A custom Bitcoin Cash or Dogecoin
indexer cannot prove its network (Bitcoin Cash shares Bitcoin's genesis), so
a session is submitted only through a catalog endpoint.

An ordinary send from a Tron, XRP Ledger or Stellar account is refused before
signing when the wallet's key alone cannot authorize it, saying why: no
permission it meets alone covers the contract, the master key is disabled, or
its weight is below the operation's threshold. A Safe shows its modules and
guard, which can move or block funds outside the threshold.

Not supported: setting up or changing a signer set (Safe deployment, Tron's
`AccountPermissionUpdate`, `SignerListSet`, Stellar's `SetOptions`, TON's
`update_multisig_params`), multisig spends of tokens, Safe delegate calls,
P2SH-wrapped, Taproot and non-sorted Bitcoin multisig, Bitcoin Cash Schnorr
multisig signatures, Aptos's on-chain `0x1::multisig_account`, a Substrate
threshold of one, cancelling a pending Substrate operation
(`cancel_as_multi`), and TON multisigs that take orders by arbitrary number.

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
| Polkadot and Westend (Asset Hub) | `Assets` pallet assets (`Asset Hub Asset`, by asset ID) |
| VeChainThor | VIP-180 (VTHO and tracked tokens), fees in VTHO |
| Starknet | ERC-20, including STRK, the fee token |

Token precision comes from validated metadata. TON master identity uses the raw
account address, with friendly-address network flags checked first. Raw,
bounceable and non-bounceable aliases cannot create duplicate holdings.
Jetton amounts use TEP-74's 120-bit integer range, including amounts beyond
64-bit units; values outside the wire format are refused before signing.

On Stable and Arc the gas coin, USDT0 and USDC at 18 places, is also an
ERC-20 at 6 places over the same balance, as Celo's CELO, Metis's METIS
(`0xdead…0000`) and peaq's PEAQ (its balances precompile) are. That face is
the network's coin row and never a token: discovery leaves it out of an
indexer's listing, and adding it as a custom token is refused as a duplicate.

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

An Asset Hub asset (`Asset Hub Asset`) is an `Assets` pallet asset, named
by its `u32` ID in decimal: USDT is 1984 and USDC 1337 on Polkadot. Its
decimals are its pallet metadata's, and an asset with no metadata is refused.
Each tracked asset's balance is the account's `Account` entry, every asset
read from one pinned state; nothing lists an account's assets, so none is
discovered. A transfer is `Assets.transfer`, encoded from the runtime's
metadata with its fee paid and quoted in DOT as that transfer; all of a
holding may be sent. Refused at build and again before signing: a frozen or
destroying asset, a frozen holding or one short of the amount, a remainder
below the asset's minimum balance (the runtime removes such a holding), a
recipient whose holding is blocked, less than the minimum balance to a
recipient that holds none, and an asset that cannot open an account (not
sufficient) to a recipient without DOT of its own; before signing, also an
asset whose decimals changed since the review.

Zcash holds transparent and [shielded funds](#zcash-shielded-funds).
Litecoin holds transparent and [MWEB funds](#litecoin-mweb-funds).

The [follow-up source audit](audits/token-verification-2026-10-04/README.md)
verifies and reactivates 58 of the 59 preserved deployments, along with all 21
previously commented identities and their descriptions. The reactivated records
retain their original addresses, standards and precision. Primary issuer/protocol
documents, official bridge lists and exact bridge mappings establish identity; all 59 original
contracts also pass fixed-block chain-ID, bytecode and decimals reads.
The unresolved Avalanche PEPE deployment was removed at the user's explicit
request; its underlying Ethereum PEPE and reciprocal bridge relationship are
proved, but issuer or official bridge operator identity remains unresolved.
Ethereum, Arbitrum and BNB PEPE remain active.
The [original audit list](audits/chain-support-2026-10-04/removed-token-deployments.json)
retains the earlier decision as history. Explicit custom assets remain available.
Eight stablecoin deployments added on 2026-11-24 — USDT and USDC on BNB
Smart Chain and Asset Hub, USDT0 on Plasma, USDC on Sei, USDT on Celo and
opBNB — are each named by their issuer or the bridge operator that mints
them, and pass fixed-block chain reads ([record](audits/token-additions-2026-11-24/README.md)).
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
transactions, with fullnode network verification. Sui's history is every
transaction Sui GraphQL lists as affecting the address, each coin type it
moved read from the transaction's balance changes, gas aside. Sui, Aptos, TON and NEAR token
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

Only verified keyless public indexers are built in. Arbitrum, Base, Polygon,
Arbitrum Sepolia, Plasma, BNB, HyperEVM, Linea, Sei, Cronos, opBNB, Sonic,
Berachain, X Layer, Monad, Kaia, XDC Network, Robinhood Chain, Kava EVM, Stable,
Arc, Katana, peaq and 0G require a compatible custom address indexer for
history and automatic token discovery. A Blockscout-compatible
source can be configured on any EVM network. RPC-only access still supports
native transfers, tracked ERC-20 balances/transfers and submitted receipts.
Polkadot/Westend, Kusama Asset Hub, Astar, Bittensor and Starknet have no
built-in keyless address-history indexer; their verified RPCs support balances,
transfers and finality.
Algorand's history is its indexer's payments and close-outs, inner
transactions included; VeChainThor's the node's transfer logs for VET and
`Transfer` events for VTHO and tracked tokens; Filecoin's Filfox's transfer
list; Tezos's TzKT's transactions, one row per operation group; Hedera's the
mirror node's crypto transfers net of the fee and staking rewards. Each leaves
fees aside; Tezos lists a failed send at the amount it asked for, the others
no failed transfer.
A Cosmos network's history is its nodes' transaction search, newest first,
over the transactions the address signed and the transfers it received
(Kava's SDK 0.47 node takes the search as `events=`, the others as `query=`),
each shown as what its messages moved in the network's coin, the fee aside.
Nodes keep only recent blocks: days on Osmosis, BandChain and Injective,
months to years elsewhere, and all of it at Kava Labs' and Numia's (Celestia).

BCH, DOGE, ZEC (transparent), DCR and DASH test networks have no
verified built-in keyless provider; Zcash testnet's shielded funds read a
built-in lightwalletd. Their supported custom API must be configured before
network reads or transfers. TON testnet has no built-in history: its TON Center
archive answered no request on 2026-11-26. Failed providers are not retained as
fallback entries. Mainnet Monero and stagenet defaults support local scanning;
Avalanche defaults read ERC-20 balances.

Every endpoint is asked its network through its API's own client, by the
health probe and before a broadcast, wherever the API can say (an EVM chain
id, a genesis or identity block, a Cosmos block's chain id, a network
passphrase and the like). Blockscout and Routescan, TronGrid, NearBlocks,
FastNEAR, Filfox, the IC replica, Insight, Kaspa's REST API, WhatsOnChain, the
BCH REST API, and the indexers of Bitcoin Cash, Bitcoin Gold, Dash and Dogecoin
cannot be asked; a broadcast goes through such an endpoint only when it is
built in. [The 2026-11-26 audit](audits/endpoints-2026-11-26/README.md) records
every built-in endpoint's network and each capability it claims.

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
| Sui | System staking and whole withdrawal of owned StakedSui objects, including the chain's reward payout in that withdrawal. No artificial unlock delay. A position's reward is unknown, as Sui GraphQL estimates none; a position in the pool of a validator that has left the active set is inactive, named by its pool's id, and can still be withdrawn. |
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
NEP-141 includes the transfer call and attached deposit. A NEP-141 send reads
the recipient's NEP-145 `storage_balance_of`; when the token has not
registered it, the transaction calls `storage_deposit` with `registration_only`
and `storage_balance_bounds().min` before `ft_transfer`, its budget covers both
calls and the deposit is reviewed as part of the cost. A contract that answers
neither query is refused. Storage reserve, funds, the recipient's registration
and the reviewed budget are checked again before signing or submitting; pending
retries also check the current budget because signed bytes do not cap gas price.
A NEAR wallet lists the token contracts its NearBlocks inventory, holdings and
history name that hold a NEP-145 deposit while the account holds none of their
token, and returns one deposit per transaction with `storage_unregister`:
empty arguments, so never `force`, and one yoctoNEAR. A held balance or a
missing deposit is refused, and both are read again before signing or
submitting. Four account chains resolve exact transaction execution outcomes;
Substrate resolves finalized extrinsic events. Manual status checks bypass background
poll backoff. NEAR native, token and staking transactions validate their canonical
reference block against the live protocol validity period, rather than an
arbitrary wall-clock timeout. Native and NEP-141 sends retain the local hash
checked against official SDK vectors; a foreign node receipt remains uncertain. No age, provider failure or node acceptance alone
proves success or failure. Official SDK fixtures and loopback CLI checks exercise signing,
reopen, uncertain submission, final outcomes and refusal paths without sending
real funds. Dated Solana mainnet simulations validate current instruction layouts
with zero signatures and no broadcast.
