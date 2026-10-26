//! The chain registry: `Chain` and every per-chain fact core decides with.
//!
//! The canonical `Chain` enum crosses domain and FFI boundaries. Storage and
//! text input use its stable string ids (e.g. `"bitcoin"`, `"ethereum"`),
//! written by `Chain::str_id()` and parsed by `Chain::from_str_id()`.
//!
//! This file holds the enum, its identity and the types its answers use; the
//! `impl Chain` methods live in one sibling per topic (`keys`, `address`,
//! `endpoints`, `tokens`, `send`, `staking`, `utxo`, `networks`, `evm`).

mod address;
mod endpoints;
mod evm;
mod keys;
mod networks;
mod send;
mod staking;
#[cfg(test)]
mod tests;
mod tokens;
mod utxo;

use crate::api::endpoint::{EndpointApi, EndpointCapability};

/// Declares [`Chain`] and `ALL_CHAINS` from one list, so the enum and the
/// iteration order cannot disagree.
macro_rules! declare_chains {
    ($($variant:ident,)*) => {
        /// Every chain Spectra knows about.
        ///
        /// This crosses the FFI boundary as the one chain type every front end uses.
        /// Ordered as declared, which is the catalog's order.
        #[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, uniffi::Enum)]
        pub enum Chain {
            $($variant,)*
        }

        /// Every variant, in declaration order.
        const ALL_CHAINS: &[Chain] = &[$(Chain::$variant,)*];
    };
}

declare_chains! {
    Bitcoin,
    Ethereum,
    Solana,
    Dogecoin,
    Xrp,
    Litecoin,
    BitcoinCash,
    Tron,
    Stellar,
    Cardano,
    Polkadot,
    Arbitrum,
    Optimism,
    Avalanche,
    Sui,
    Aptos,
    Ton,
    Near,
    Icp,
    Monero,
    Base,
    EthereumClassic,
    BitcoinSV,
    BnbChain,
    Hyperliquid,
    Polygon,
    Linea,
    Scroll,
    Blast,
    Mantle,
    Zcash,
    BitcoinGold,
    Decred,
    Kaspa,
    Sei,
    Celo,
    Cronos,
    OpBnb,
    ZkSyncEra,
    Sonic,
    Berachain,
    Unichain,
    Ink,
    Dash,
    XLayer,
    Bittensor,
    Plasma,
    Monad,
    WorldChain,
    Peercoin,

    // ── Testnets ─────────────────────────────────────────────────────────────
    BitcoinTestnet,
    BitcoinTestnet4,
    BitcoinSignet,
    LitecoinTestnet,
    BitcoinCashTestnet,
    BitcoinSVTestnet,
    DogecoinTestnet,
    ZcashTestnet,
    DecredTestnet,
    KaspaTestnet,
    DashTestnet,
    PeercoinTestnet,
    EthereumSepolia,
    EthereumHoodi,
    ArbitrumSepolia,
    OptimismSepolia,
    BaseSepolia,
    BnbChainTestnet,
    AvalancheFuji,
    PolygonAmoy,
    HyperliquidTestnet,
    LineaSepolia,
    CeloSepolia,
    CronosTestnet,
    ZkSyncEraSepolia,
    SonicTestnet,
    InkSepolia,
    XLayerTestnet,
    EthereumClassicMordor,
    TronNile,
    SolanaDevnet,
    XrpTestnet,
    StellarTestnet,
    CardanoPreprod,
    SuiTestnet,
    AptosTestnet,
    TonTestnet,
    NearTestnet,
    PolkadotWestend,
    MoneroStagenet,
}

/// Stored and printed as its catalog id (`"bitcoin"`). An id the catalog does
/// not know fails to deserialize: a row naming no chain is refused where it is
/// read, not carried as a string until something parses it.
impl serde::Serialize for Chain {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.str_id())
    }
}

impl<'de> serde::Deserialize<'de> for Chain {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        Chain::parse(&id).map_err(serde::de::Error::custom)
    }
}

/// A chain column holds the catalog id, and a row naming no known chain fails
/// to read rather than surfacing as a string nothing can use.
impl rusqlite::types::ToSql for Chain {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(self.str_id().into())
    }
}

impl rusqlite::types::FromSql for Chain {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        let id = value.as_str()?;
        Chain::parse(id).map_err(|e| rusqlite::types::FromSqlError::Other(e.into()))
    }
}

/// A registry question with no answer for this chain.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    #[error("Unknown network: {0}")]
    UnknownChain(String),
    /// A fact asked of a chain outside the family that has it.
    #[error("{chain} is not a {family} network")]
    NotIn { chain: Chain, family: &'static str },
    #[error("Zcash V5 is not active at height {0}")]
    ZcashV5Inactive(u32),
}

/// The fee-oracle methods deployed by each supported OP Stack network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpStackFeeModel {
    FjordWithOperator,
    Fjord,
    Bedrock,
}

impl From<RegistryError> for crate::error::SpectraBridgeError {
    fn from(error: RegistryError) -> Self {
        Self::InvalidInput {
            message: error.to_string().into(),
        }
    }
}

impl From<RegistryError> for crate::api::error::ApiError {
    fn from(error: RegistryError) -> Self {
        Self::InvalidInput(error.to_string())
    }
}

impl std::fmt::Display for Chain {
    /// The catalog id, as a log line or an error message names the chain.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.str_id())
    }
}

/// A fungible-token protocol: what a token deployment is on the network that
/// hosts it. Stored, printed and written in the catalogs as its name
/// (`"ERC-20"`, `"Sui Coin"`), which [`TokenStandard::as_str`] spells.
///
/// A network's own coin has no standard: a holding or deployment of it
/// carries `None`.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    uniffi::Enum,
)]
pub enum TokenStandard {
    #[serde(rename = "ERC-20")]
    Erc20,
    #[serde(rename = "BEP-20")]
    Bep20,
    #[serde(rename = "ARC-20")]
    Arc20,
    #[serde(rename = "SPL")]
    Spl,
    #[serde(rename = "TRC-10")]
    Trc10,
    #[serde(rename = "TRC-20")]
    Trc20,
    #[serde(rename = "TEP-74")]
    Tep74,
    #[serde(rename = "NEP-141")]
    Nep141,
    #[serde(rename = "Sui Coin")]
    SuiCoin,
    #[serde(rename = "Aptos Coin")]
    AptosCoin,
    #[serde(rename = "AIP-21")]
    Aip21,
    #[serde(rename = "Trust Line Token")]
    TrustLineToken,
    #[serde(rename = "Stellar Asset")]
    StellarAsset,
    #[serde(rename = "Cardano Native Token")]
    CardanoNativeToken,
}

/// The signature scheme a chain's raw private key belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyScheme {
    /// A secp256k1 scalar: the Bitcoin family, every EVM network, Tron, XRP,
    /// Kaspa, Decred and the Internet Computer.
    Secp256k1,
    /// A 32-byte Ed25519 seed.
    Ed25519,
    /// A Substrate sr25519 mini secret.
    Sr25519,
    /// Cardano's 64-byte BIP32-Ed25519 extended key.
    Ed25519Extended,
}

/// Where an EVM chain's transaction history can be read from.
///
/// Only keyless explorer sources are configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvmHistorySource<'a> {
    /// A keyless endpoint. The base already identifies the chain, so no
    /// `chainid` is sent.
    Open(&'a str),
    /// No keyless indexer is configured. Asking is an error, not an empty list.
    Unavailable,
}

impl Chain {
    /// This chain's row in the catalog.
    ///
    /// The enum is declared in `chains.toml` order, so a variant *is* an index
    /// into the catalog and every column it holds can be read without a second
    /// table. `chain_order_matches_the_catalog` fails the build if they drift.
    pub fn entry(self) -> &'static crate::catalog::chains::ChainEntry {
        crate::catalog::chains::catalog()
            .get(self as usize)
            .expect("enum declaration order is the catalog's order")
    }

    /// Stable string id — the catalog's `id`.
    pub fn str_id(self) -> &'static str {
        crate::catalog::chains::catalog_id(self as usize)
    }

    /// Parse a string id (from `chains.toml`, storage or a command line).
    pub fn from_str_id(id: &str) -> Option<Self> {
        static BY_ID: std::sync::LazyLock<std::collections::HashMap<&'static str, Chain>> =
            std::sync::LazyLock::new(|| Chain::all().map(|c| (c.str_id(), c)).collect());
        BY_ID.get(id).copied()
    }

    /// [`Chain::from_str_id`], with the refusal a caller reports.
    pub fn parse(id: &str) -> Result<Self, RegistryError> {
        Self::from_str_id(id).ok_or_else(|| RegistryError::UnknownChain(id.to_string()))
    }

    /// Returns `true` for chains that are testnets.
    pub fn is_testnet(self) -> bool {
        crate::catalog::chains::declared(self).mainnet.is_some()
    }

    /// A test network's faucet page, where its coins are free; `None` on
    /// mainnets and on test networks without a working one.
    pub fn faucet_url(self) -> Option<&'static str> {
        crate::catalog::chains::declared(self).faucet.as_deref()
    }

    /// Maps a testnet variant to its mainnet counterpart. Returns `self` for mainnets.
    ///
    /// Read from the declared row rather than [`Chain::entry`]: building the
    /// catalog's entries asks this of every chain.
    pub fn mainnet_counterpart(self) -> Chain {
        crate::catalog::chains::declared(self)
            .mainnet
            .as_deref()
            .map_or(self, |mainnet| {
                Chain::from_str_id(mainnet).expect("the catalog checks each mainnet")
            })
    }

    /// `true` for every EVM-compatible chain (mainnet or testnet): the ones
    /// with an EIP-155 chain id. Independent of the catalog, which calls this
    /// to project the same fact to the CLI and the app.
    pub fn is_evm(self) -> bool {
        self.evm_chain_id().is_ok()
    }

    fn not_in(self, family: &'static str) -> RegistryError {
        RegistryError::NotIn {
            chain: self,
            family,
        }
    }

    // ── Native-coin metadata

    /// A zero-balance holding of this network's native coin: what a wallet
    /// starts with before its first refresh.
    pub fn native_holding_template(self) -> crate::store::wallet_domain::AssetHolding {
        crate::store::wallet_domain::AssetHolding {
            id: String::new(),
            name: self.coin_name().to_string(),
            symbol: self.coin_symbol().to_string(),
            coingecko_id: self.coingecko_id().to_string(),
            chain_id: self,
            token_standard: None,
            contract_address: None,
            amount: "0".to_string(),
        }
        .identified()
    }

    pub fn coin_name(self) -> &'static str {
        self.entry().native_asset_display_name.as_str()
    }

    /// The native token's symbol, joined through the network's deployment reference.
    pub fn coin_symbol(self) -> &'static str {
        self.entry().gas_token_symbol.as_str()
    }

    /// The name shown to a user — the catalog's `name`.
    pub fn chain_display_name(self) -> &'static str {
        self.entry().name.as_str()
    }

    pub fn native_decimals(self) -> u8 {
        self.entry().native_decimals as u8
    }

    pub fn coingecko_id(self) -> &'static str {
        self.entry().native_coingecko_id.as_str()
    }

    /// Iterator over every known chain.
    pub fn all() -> impl Iterator<Item = Self> {
        ALL_CHAINS.iter().copied()
    }

    /// Iterator over only mainnet chains.
    #[cfg(test)]
    pub(crate) fn mainnets() -> impl Iterator<Item = Self> {
        Self::all().filter(|c| !c.is_testnet())
    }

    /// Resolve a chain from the display name used on the boundary.
    ///
    /// No special cases: the enum and `chains.toml` agree on every name, and
    /// `every_catalog_name_resolves` fails if they ever stop.
    pub fn from_display_name(name: &str) -> Option<Self> {
        Chain::all().find(|c| c.chain_display_name() == name)
    }
}

/// A payment request's format, by the scheme its wallets read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaymentUriFormat {
    /// BIP-21 and its descendants: `scheme:address?amount=<whole units>`.
    Bip21(&'static str),
    /// EIP-681: `ethereum:address@<chain id>?value=<wei>`.
    Eip681,
    /// Solana Pay: `solana:address?amount=<whole units>`.
    SolanaPay,
    /// `ripple:address?amount=<whole units>&dt=<destination tag>`.
    Xrpl,
    /// SEP-7: `web+stellar:pay?destination=…&amount=…&memo=…&memo_type=…`.
    Sep7,
    /// `monero:address?tx_amount=<whole units>`.
    MoneroUri,
    /// `ton://transfer/address?amount=<nanotons>`.
    TonTransfer,
}

/// How a payment names whose deposit it is at an account many share: the
/// field a network's payments carry for it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, uniffi::Enum,
)]
#[serde(rename_all = "camelCase")]
pub enum PaymentMemoKind {
    /// XRP Ledger `DestinationTag`: an unsigned 32-bit integer.
    DestinationTag,
    /// Stellar `MEMO_TEXT`: up to 28 bytes of UTF-8.
    MemoText,
    /// Stellar `MEMO_ID`: an unsigned 64-bit integer.
    MemoId,
}

/// How a chain's estimated fee enters its signing request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SendFeeField {
    /// Sui: the fee becomes the transaction's gas budget.
    GasBudget,
    /// The reviewed fee or gas budget is passed as an explicit amount.
    FeeAmount,
    /// UTXO chains: the fee is converted to satoshis.
    FeeSats,
    /// The chain computes its own fee at signing time.
    None,
}

/// How a chain's fee enters a send, beyond the amount and the destination.
#[derive(Debug, Clone, Copy)]
pub struct SendExecutionShape {
    pub fee_field: SendFeeField,
    /// Fee to assume when no preview is available, as an exact decimal in
    /// native units. `None` where the chain always has a preview by the time
    /// a send is submitted.
    pub fee_fallback: Option<&'static str>,
}

/// How a chain's pending transactions are polled for confirmation.
///
/// A per-chain fact, so it lives here rather than in the shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingStatusPoll {
    /// Ask the chain's own status endpoint for a txid.
    Utxo {
        /// Only sends are tracked; receives confirm on their own.
        require_send_kind: bool,
    },
    /// Read this exact transaction's committed execution result from the API.
    TransactionStatus(EndpointApi),
    /// Scan finalized Substrate blocks and their System.Events for an exact
    /// extrinsic hash, since the node has no address-history index.
    SubstrateFinality,
    /// Receipt-based, through the EVM history path.
    EvmReceipt,
    /// Not polled.
    None,
}

// ── FFI surface ──────────────────────────────────────────────────────────

/// The canonical form an address is folded to before storage or comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressNormalization {
    /// Case and shape are significant — a Bitcoin or Solana address is used
    /// exactly as the user typed it.
    None,
    Lowercase,
    /// Stellar StrKey uses canonical uppercase RFC 4648 base32.
    Uppercase,
    /// Lowercase, and prefixed with `0x` when the input omitted it.
    LowercaseHexPrefixed,
}

/// One encoding of an account public key on a network: its four version
/// bytes, the prefix they spell, and the script the account's addresses pay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AccountKeyVersion {
    pub version: [u8; 4],
    pub prefix: &'static str,
    pub script: crate::derivation::types::BitcoinScriptType,
}

/// History service strategy; callers never select a protocol implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HistoryRefreshKind {
    Evm,
    Utxo,
    Normalized,
}
