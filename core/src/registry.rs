//! Central chain + token registry.
//!
//! The canonical `Chain` enum crosses domain and FFI boundaries. Storage and
//! text input use its stable string ids (e.g. `"bitcoin"`, `"ethereum"`),
//! written by `Chain::str_id()` and parsed by `Chain::from_str_id()`.

use crate::EndpointApi;

/// Every chain Spectra knows about.
///
/// This crosses the FFI boundary as the one chain type every front end uses.
/// Ordered as declared, which is the catalog's order.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, uniffi::Enum)]
pub enum Chain {
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

impl From<RegistryError> for crate::SpectraBridgeError {
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

// All variants in stable order. Used by Chain::all().

const ALL_CHAINS: &[Chain] = &[
    Chain::Bitcoin,
    Chain::Ethereum,
    Chain::Solana,
    Chain::Dogecoin,
    Chain::Xrp,
    Chain::Litecoin,
    Chain::BitcoinCash,
    Chain::Tron,
    Chain::Stellar,
    Chain::Cardano,
    Chain::Polkadot,
    Chain::Arbitrum,
    Chain::Optimism,
    Chain::Avalanche,
    Chain::Sui,
    Chain::Aptos,
    Chain::Ton,
    Chain::Near,
    Chain::Icp,
    Chain::Monero,
    Chain::Base,
    Chain::EthereumClassic,
    Chain::BitcoinSV,
    Chain::BnbChain,
    Chain::Hyperliquid,
    Chain::Polygon,
    Chain::Linea,
    Chain::Scroll,
    Chain::Blast,
    Chain::Mantle,
    Chain::Zcash,
    Chain::BitcoinGold,
    Chain::Decred,
    Chain::Kaspa,
    Chain::Sei,
    Chain::Celo,
    Chain::Cronos,
    Chain::OpBnb,
    Chain::ZkSyncEra,
    Chain::Sonic,
    Chain::Berachain,
    Chain::Unichain,
    Chain::Ink,
    Chain::Dash,
    Chain::XLayer,
    Chain::Bittensor,
    Chain::Plasma,
    Chain::Monad,
    Chain::WorldChain,
    Chain::Peercoin,
    // Testnets
    Chain::BitcoinTestnet,
    Chain::BitcoinTestnet4,
    Chain::BitcoinSignet,
    Chain::LitecoinTestnet,
    Chain::BitcoinCashTestnet,
    Chain::BitcoinSVTestnet,
    Chain::DogecoinTestnet,
    Chain::ZcashTestnet,
    Chain::DecredTestnet,
    Chain::KaspaTestnet,
    Chain::DashTestnet,
    Chain::PeercoinTestnet,
    Chain::EthereumSepolia,
    Chain::EthereumHoodi,
    Chain::ArbitrumSepolia,
    Chain::OptimismSepolia,
    Chain::BaseSepolia,
    Chain::BnbChainTestnet,
    Chain::AvalancheFuji,
    Chain::PolygonAmoy,
    Chain::HyperliquidTestnet,
    Chain::LineaSepolia,
    Chain::CeloSepolia,
    Chain::CronosTestnet,
    Chain::ZkSyncEraSepolia,
    Chain::SonicTestnet,
    Chain::InkSepolia,
    Chain::XLayerTestnet,
    Chain::EthereumClassicMordor,
    Chain::TronNile,
    Chain::SolanaDevnet,
    Chain::XrpTestnet,
    Chain::StellarTestnet,
    Chain::CardanoPreprod,
    Chain::SuiTestnet,
    Chain::AptosTestnet,
    Chain::TonTestnet,
    Chain::NearTestnet,
    Chain::PolkadotWestend,
    Chain::MoneroStagenet,
];

/// The signature scheme a chain's raw private key belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyScheme {
    /// A secp256k1 scalar: the Bitcoin family, every EVM network, Tron, XRP,
    /// Kaspa and Decred.
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
    pub fn supports_derivation_passphrase(self) -> bool {
        self.mainnet_counterpart() != Self::Monero
    }
    pub fn supports_derivation_hmac_override(self) -> bool {
        matches!(
            self.mainnet_counterpart(),
            Self::Solana | Self::Stellar | Self::Polkadot
        )
    }

    /// Named accounts authorize keys on chain instead of encoding the key in their address.
    pub fn supports_named_sender_accounts(self) -> bool {
        matches!(self, Self::Near | Self::NearTestnet)
    }

    /// This chain's row in the catalog.
    ///
    /// The enum is declared in `chains.toml` order, so a variant *is* an index
    /// into the catalog and every column it holds can be read without a second
    /// table. `chain_order_matches_the_catalog` fails the build if they drift.
    pub fn entry(self) -> &'static crate::chains::ChainEntry {
        crate::chains::catalog()
            .get(self as usize)
            .expect("enum declaration order is the catalog's order")
    }

    /// Stable string id — the catalog's `id`.
    pub fn str_id(self) -> &'static str {
        crate::chains::catalog_id(self as usize)
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

    /// Key under which this chain's address is stored during wallet import.
    ///
    /// Most chains own their address, so the slot is just [`Chain::str_id`].
    /// The exceptions are the EVM family: one derived secp256k1 address serves
    /// Ethereum, Arbitrum, Base and the rest, so they all share the
    /// `"ethereum"` slot rather than each carrying a copy.
    ///
    /// The slot describes address encoding, not network or derivation identity.
    /// Every EVM network, including Ethereum Classic and testnets, uses it;
    /// each wallet separately records its concrete network and derivation path.
    /// Other networks keep separate slots when their address encoding differs.
    pub fn address_slot(self) -> &'static str {
        match self {
            _ if self.is_evm() => Chain::Ethereum.str_id(),
            _ => self.str_id(),
        }
    }

    /// The signature scheme a raw key on this chain is a secret of, or `None`
    /// where a key alone yields no address. Two chains with one scheme read
    /// the same key as the same secret; across schemes the same bytes are a
    /// different key, which a wallet's key must not become.
    pub fn key_scheme(self) -> Option<KeyScheme> {
        if !self.derives_from_private_key() {
            return None;
        }
        Some(match self.mainnet_counterpart() {
            Self::Cardano => KeyScheme::Ed25519Extended,
            Self::Polkadot | Self::Bittensor => KeyScheme::Sr25519,
            Self::Solana
            | Self::Stellar
            | Self::Sui
            | Self::Aptos
            | Self::Ton
            | Self::Near
            | Self::Icp => KeyScheme::Ed25519,
            _ => KeyScheme::Secp256k1,
        })
    }

    /// Whether a raw private key yields an address on this chain.
    /// Shared by derivation and import eligibility. Testnets follow their
    /// mainnet; derivation handles network-specific address encoding.
    pub fn derives_from_private_key(self) -> bool {
        self.mainnet_counterpart() != Self::Monero
    }

    /// The private-key encodings an import on this chain accepts, or none
    /// where a key alone yields no address.
    pub fn private_key_formats(self) -> Vec<crate::derivation::setup::WalletSecretFormat> {
        use crate::derivation::setup::WalletSecretFormat as F;
        if !self.derives_from_private_key() {
            return Vec::new();
        }
        let native = match self.mainnet_counterpart() {
            Self::Cardano => return vec![F::CardanoExtendedKey],
            _ if self.wif_version().is_some() => Some(F::Wif),
            Self::Solana => Some(F::SolanaKeypair),
            Self::Stellar => Some(F::StellarSecretSeed),
            Self::Sui => Some(F::SuiPrivateKey),
            Self::Aptos => Some(F::AptosPrivateKey),
            Self::Near => Some(F::NearSecretKey),
            _ => None,
        };
        std::iter::once(F::HexSecret32).chain(native).collect()
    }

    /// The version byte a Wallet Import Format key carries on this network,
    /// for the Base58Check-WIF chains: the Bitcoin family's own, and `0xEF`
    /// (Dogecoin `0xF1`) on their test networks.
    pub fn wif_version(self) -> Option<u8> {
        let mainnet = match self.mainnet_counterpart() {
            Self::Bitcoin
            | Self::BitcoinCash
            | Self::BitcoinSV
            | Self::BitcoinGold
            | Self::Zcash => 0x80,
            Self::Litecoin => 0xb0,
            Self::Dogecoin => 0x9e,
            Self::Dash => 0xcc,
            Self::Peercoin => 0xb7,
            _ => return None,
        };
        Some(match (self.is_testnet(), self.mainnet_counterpart()) {
            (false, _) => mainnet,
            (true, Self::Dogecoin) => 0xf1,
            (true, _) => 0xef,
        })
    }

    /// The phrase encodings an import on this chain restores from: the ones
    /// its own wallets write. Monero and TON wallets do not read BIP-39.
    pub fn phrase_formats(self) -> Vec<crate::derivation::setup::WalletSecretFormat> {
        use crate::derivation::setup::WalletSecretFormat as F;
        match self.mainnet_counterpart() {
            Self::Monero => vec![F::MoneroPhrase, F::Polyseed],
            Self::Ton => vec![F::TonMnemonic],
            _ => vec![F::Bip39Phrase],
        }
    }

    /// The phrase encoding a wallet created on this chain is generated in:
    /// one every wallet for the chain restores. For Monero that is the
    /// 25-word seed, which older wallets read and Polyseed's do not replace.
    pub fn created_phrase_format(self) -> crate::derivation::setup::WalletSecretFormat {
        use crate::derivation::setup::WalletSecretFormat as F;
        match self.mainnet_counterpart() {
            Self::Monero => F::MoneroPhrase,
            Self::Ton => F::TonMnemonic,
            _ => F::Bip39Phrase,
        }
    }

    /// The decode shape this chain's send preview comes back in, for the
    /// chains core estimates through one entry point.
    ///
    /// `None` means the chain has a preview path of its own — the UTXO family,
    /// Dogecoin, Tron and the EVM family each take different inputs and return
    /// a different record, which is why they are not one call.
    ///
    /// Through `mainnet_counterpart` because the shape is what decoding needs
    /// and a testnet decodes like its mainnet; which network is reached is the
    /// chain id's business.
    pub fn simple_preview_chain(self) -> Option<crate::send::preview_decode::SimpleChain> {
        use crate::send::preview_decode::SimpleChain;
        Some(match self.mainnet_counterpart() {
            Chain::Solana => SimpleChain::Solana,
            Chain::Xrp => SimpleChain::Xrp,
            Chain::Stellar => SimpleChain::Stellar,
            Chain::Monero => SimpleChain::Monero,
            Chain::Cardano => SimpleChain::Cardano,
            Chain::Sui => SimpleChain::Sui,
            Chain::Aptos => SimpleChain::Aptos,
            Chain::Ton => SimpleChain::Ton,
            Chain::Icp => SimpleChain::Icp,
            Chain::Near => SimpleChain::Near,
            Chain::Polkadot => SimpleChain::Polkadot,
            Chain::Bittensor => SimpleChain::Bittensor,
            _ => return None,
        })
    }

    /// The APIs the chain's own clients speak: what its balance, fee and
    /// broadcast reads use. Every URL speaking any of them is asked at once,
    /// whichever API it speaks; indexers and secondary services are asked for
    /// by API.
    ///
    /// More than one API is only possible where one client answers in all of
    /// them — the UTXO family, through `api::utxo`.
    pub fn endpoint_apis(self) -> &'static [crate::EndpointApi] {
        use crate::EndpointApi as Api;
        match self.mainnet_counterpart() {
            c if c.is_evm() => &[Api::EvmJsonRpc],
            Chain::Bitcoin => &[Api::Esplora, Api::Blockcypher],
            Chain::Litecoin => &[Api::Blockbook, Api::Esplora, Api::Blockcypher],
            Chain::Dogecoin => &[Api::Blockcypher, Api::Blockbook],
            Chain::Dash => &[Api::Blockbook, Api::Blockcypher],
            Chain::BitcoinCash => &[Api::Blockbook, Api::BchRestV2],
            // Zcash's transparent builder asks Blockbook for its consensus
            // branch, which no other indexer reports.
            Chain::BitcoinGold | Chain::Zcash | Chain::Peercoin => &[Api::Blockbook],
            Chain::BitcoinSV => &[Api::Whatsonchain],
            Chain::Solana => &[Api::SolanaJsonRpc],
            Chain::Tron => &[Api::TronHttp],
            Chain::Stellar => &[Api::Horizon],
            Chain::Xrp => &[Api::XrplJsonRpc],
            Chain::Cardano => &[Api::Koios],
            Chain::Polkadot | Chain::Bittensor => &[Api::SubstrateJsonRpc],
            Chain::Sui => &[Api::SuiJsonRpc],
            Chain::Aptos => &[Api::AptosRest],
            Chain::Ton => &[Api::ToncenterV2],
            Chain::Near => &[Api::NearJsonRpc],
            Chain::Icp => &[Api::IcpRosetta],
            Chain::Monero => &[Api::MoneroDaemonRpc],
            Chain::Decred => &[Api::Insight],
            Chain::Kaspa => &[Api::KaspaRest],
            _ => &[],
        }
    }

    /// Primary transports and implemented auxiliary indexers this network accepts.
    pub fn compatible_endpoint_apis(self) -> Vec<crate::EndpointApi> {
        use crate::EndpointApi as Api;
        let auxiliary: &[Api] = match self.mainnet_counterpart() {
            chain if chain.is_evm() => &[Api::Blockscout],
            Chain::Aptos => &[Api::AptosIndexer],
            Chain::Icp => &[Api::IcpReplica],
            Chain::Near => &[Api::Nearblocks, Api::Fastnear],
            Chain::Tron => &[Api::TrongridV1],
            Chain::Ton => &[Api::ToncenterV3],
            // Shielded funds are scanned from a lightwalletd server.
            Chain::Zcash => &[Api::Lightwalletd],
            // MWEB funds are scanned from a Litecoin node, peer to peer.
            Chain::Litecoin => &[Api::LitecoinP2p],
            _ => &[],
        };
        self.endpoint_apis()
            .iter()
            .chain(auxiliary)
            .copied()
            .collect()
    }

    /// The byte width of a Substrate chain's `Balance` type, which sizes its
    /// `System.Account` record: Polkadot's is `u128`, subtensor's `u64`.
    pub fn substrate_balance_bytes(self) -> Option<usize> {
        match self.mainnet_counterpart() {
            Chain::Polkadot => Some(16),
            Chain::Bittensor => Some(8),
            _ => None,
        }
    }

    /// How a configured URL the endpoint directory does not know is read.
    /// It picks nothing about which URLs are used.
    pub fn default_api(self) -> Option<crate::EndpointApi> {
        self.endpoint_apis().first().copied()
    }

    /// The chain's reads go through `api::utxo::UtxoClient`, which answers
    /// the same way in every UTXO indexer API.
    pub fn uses_utxo_client(self) -> bool {
        let apis = self.endpoint_apis();
        !apis.is_empty() && apis.iter().all(|api| api.is_utxo_indexer())
    }

    pub fn has_send_preview(self) -> bool {
        // The EVM family and the chains with a preview path of their own —
        // everything that does not go through the generic submit — always have
        // one. The rest need either a shared-path shape or a fee fallback.
        self.is_evm()
            || self.uses_account_utxo()
            || !self.uses_generic_send_submit()
            || self.simple_preview_chain().is_some()
            || self.send_execution_shape().fee_fallback.is_some()
    }

    /// This chain's send builder can sign a transaction and stop, without
    /// putting it on the chain.
    ///
    /// Capabilities follow the staged builder; unsupported protocols refuse early.
    pub fn supports_sign_only(self) -> bool {
        self.has_send_preview() && self.transparent_send_unavailable_reason().is_none()
    }

    /// Staking reads live state through the network's declared transport.
    pub fn staking_uses_endpoint(self) -> bool {
        self.supports_staking()
    }

    /// Whether this network supports any token protocols.
    pub fn hosts_tokens(self) -> bool {
        !self.token_standards().is_empty()
    }

    /// Protocols supported by this concrete network. A deployment carries its
    /// own standard, and no member of this list is a network-wide default.
    pub fn token_standards(self) -> &'static [String] {
        &crate::chains::declared(self).token_standards
    }

    pub fn allows_token_standard(self, standard: &str) -> bool {
        self.token_standards().iter().any(|s| s == standard)
    }

    /// Infer a protocol only where identifier shapes distinguish it. Explicit
    /// deployments keep their recorded standard, including EVM aliases.
    pub fn token_standard_for_identifier(self, identifier: &str) -> &'static str {
        match self {
            chain if chain.is_evm() => "ERC-20",
            Self::Solana | Self::SolanaDevnet => "SPL",
            Self::Tron | Self::TronNile if identifier.bytes().all(|b| b.is_ascii_digit()) => {
                "TRC-10"
            }
            Self::Tron | Self::TronNile => "TRC-20",
            Self::Ton | Self::TonTestnet => "TEP-74",
            Self::Near | Self::NearTestnet => "NEP-141",
            Self::Sui | Self::SuiTestnet => "Sui Coin",
            Self::Aptos | Self::AptosTestnet if identifier.contains("::") => "Aptos Coin",
            Self::Aptos | Self::AptosTestnet => "AIP-21",
            Self::Xrp | Self::XrpTestnet => "Trust Line Token",
            Self::Stellar | Self::StellarTestnet => "Stellar Asset",
            Self::Cardano | Self::CardanoPreprod => "Cardano Native Token",
            _ => "",
        }
    }

    /// Whether core has a balance/metadata reader for this actual protocol.
    pub fn reads_token_standard(self, standard: &str) -> bool {
        self.allows_token_standard(standard)
    }

    /// Whether core has a transfer builder for this actual protocol.
    pub fn sends_token_standard(self, standard: &str) -> bool {
        self.reads_token_standard(standard) && self.sends_tokens()
    }

    pub fn supports_staking(self) -> bool {
        matches!(
            self,
            Chain::Solana | Chain::Sui | Chain::Aptos | Chain::Near | Chain::Polkadot | Chain::Icp
        )
    }

    pub(crate) fn solana_stake_program(self) -> Result<&'static str, RegistryError> {
        match self.mainnet_counterpart() {
            Self::Solana => Ok("Stake11111111111111111111111111111111111111"),
            _ => Err(RegistryError::NotIn {
                chain: self,
                family: "Solana",
            }),
        }
    }

    pub(crate) fn sui_staking_system(
        self,
    ) -> Result<(&'static str, &'static str, u64), RegistryError> {
        match self.mainnet_counterpart() {
            Self::Sui => Ok(("0x3", "0x5", 1)),
            _ => Err(RegistryError::NotIn {
                chain: self,
                family: "Sui",
            }),
        }
    }

    pub(crate) fn near_staking_whitelist(self) -> Result<&'static str, RegistryError> {
        match self {
            Self::Near => Ok("lockup-whitelist.near"),
            _ => Err(RegistryError::NotIn {
                chain: self,
                family: "NEAR mainnet",
            }),
        }
    }

    /// Whether this chain's derivation reads a BIP-32 path.
    ///
    /// Read from the catalog rather than restated: `derivation_path = []` is
    /// how a row says its keys do not come from a BIP-32 path. Monero's spend
    /// and view keys come from the seed directly, TON's from its mnemonic,
    /// and Polkadot's and Bittensor's from Substrate junctions instead
    /// (`derives_along_junctions`).
    ///
    /// For those "no path" is the answer, not an error. See
    /// `default_path_from_catalog`.
    pub fn uses_derivation_path(self) -> bool {
        crate::chains::default_derivation_path_template(self).is_some()
    }

    /// Whether a phrase wallet on this chain derives along a Substrate path
    /// of hard (`//`) and soft (`/`) junctions under its sr25519 root key,
    /// as subkey and polkadot.js do (`derivation::substrate_path`). No path
    /// is the root key; there are no profiles or account indices.
    pub fn derives_along_junctions(self) -> bool {
        self.key_scheme() == Some(KeyScheme::Sr25519)
    }

    /// The derivation profiles a phrase wallet on this chain can use, the
    /// default first. Empty where the chain derives without a path (Monero,
    /// TON, Polkadot, Bittensor), which also means it has no account index.
    pub fn derivation_profiles(self) -> Vec<crate::chains::DerivationProfile> {
        let entries = &self.entry().derivation_path;
        entries
            .iter()
            .filter(|entry| entry.is_default)
            .chain(entries.iter().filter(|entry| !entry.is_default))
            .map(|entry| entry.profile)
            .collect()
    }

    /// The path `profile` derives at account `account` on this chain, or
    /// `None` when the chain does not offer the profile or the index does not
    /// fit a hardened segment.
    pub fn derivation_profile_path(
        self,
        profile: crate::chains::DerivationProfile,
        account: u32,
    ) -> Option<String> {
        if account >= 1 << 31 {
            return None;
        }
        self.entry()
            .derivation_path
            .iter()
            .find(|entry| entry.profile == profile)
            .map(|entry| entry.path.replace("{account}", &account.to_string()))
    }

    /// Returns `true` for chains that are testnets.
    pub fn is_testnet(self) -> bool {
        crate::chains::declared(self).environment == "testnet"
    }

    /// A test network's faucet page, where its coins are free; `None` on
    /// mainnets and on test networks without a working one.
    pub fn faucet_url(self) -> Option<&'static str> {
        crate::chains::declared(self).faucet.as_deref()
    }

    /// Maps a testnet variant to its mainnet counterpart. Returns `self` for mainnets.
    pub fn mainnet_counterpart(self) -> Chain {
        Chain::from_str_id(&self.entry().family).expect("validated network family")
    }

    /// `true` for every EVM-compatible chain (mainnet or testnet).
    /// Registry-owned and independent of the catalog: catalog initialization
    /// calls this to project the same fact to CLI and platform clients.
    pub fn is_evm(self) -> bool {
        matches!(
            self,
            Chain::Ethereum
                | Chain::Arbitrum
                | Chain::Optimism
                | Chain::Avalanche
                | Chain::Base
                | Chain::EthereumClassic
                | Chain::BnbChain
                | Chain::Hyperliquid
                | Chain::Polygon
                | Chain::Linea
                | Chain::Scroll
                | Chain::Blast
                | Chain::Mantle
                | Chain::Sei
                | Chain::Celo
                | Chain::Cronos
                | Chain::OpBnb
                | Chain::ZkSyncEra
                | Chain::Sonic
                | Chain::Berachain
                | Chain::Unichain
                | Chain::Ink
                | Chain::XLayer
                | Chain::Plasma
                | Chain::Monad
                | Chain::WorldChain
                | Chain::EthereumSepolia
                | Chain::EthereumHoodi
                | Chain::ArbitrumSepolia
                | Chain::OptimismSepolia
                | Chain::BaseSepolia
                | Chain::BnbChainTestnet
                | Chain::AvalancheFuji
                | Chain::PolygonAmoy
                | Chain::HyperliquidTestnet
                | Chain::LineaSepolia
                | Chain::CeloSepolia
                | Chain::CronosTestnet
                | Chain::ZkSyncEraSepolia
                | Chain::SonicTestnet
                | Chain::InkSepolia
                | Chain::XLayerTestnet
                | Chain::EthereumClassicMordor
        )
    }

    /// Why the current protocol adapter cannot safely expose separate stages.
    fn not_in(self, family: &'static str) -> RegistryError {
        RegistryError::NotIn {
            chain: self,
            family,
        }
    }

    pub fn transparent_send_unavailable_reason(self) -> Option<&'static str> {
        None
    }

    /// Legacy output address versions: P2PKH, followed by accepted P2SH aliases.
    pub(crate) fn fixed_utxo_address_versions(self) -> Result<(u8, &'static [u8]), RegistryError> {
        match self {
            Self::Bitcoin => Ok((0x00, &[0x05])),
            Self::BitcoinTestnet | Self::BitcoinTestnet4 | Self::BitcoinSignet => {
                Ok((0x6f, &[0xc4]))
            }
            Self::BitcoinCash | Self::BitcoinSV => Ok((0x00, &[0x05])),
            Self::BitcoinCashTestnet | Self::BitcoinSVTestnet => Ok((0x6f, &[0xc4])),
            Self::Dogecoin => Ok((0x1e, &[0x16])),
            Self::DogecoinTestnet => Ok((0x71, &[0xc4])),
            Self::Litecoin => Ok((0x30, &[0x32, 0x05])),
            Self::LitecoinTestnet => Ok((0x6f, &[0x3a, 0xc4])),
            Self::Dash => Ok((0x4c, &[0x10])),
            Self::DashTestnet => Ok((0x8c, &[0x13])),
            Self::BitcoinGold => Ok((0x26, &[0x17])),
            Self::Peercoin => Ok((0x37, &[0x75])),
            Self::PeercoinTestnet => Ok((0x6f, &[0xc4])),
            _ => Err(self.not_in("Base58 UTXO")),
        }
    }

    /// The fork id a network's SIGHASH_FORKID signatures carry: Bitcoin
    /// Gold's 79, and 0 on Bitcoin Cash and Bitcoin SV, whose replay
    /// protection is the flag alone.
    pub(crate) fn sighash_fork_id(self) -> Result<u32, RegistryError> {
        match self.mainnet_counterpart() {
            Self::BitcoinCash | Self::BitcoinSV => Ok(0),
            Self::BitcoinGold => Ok(79),
            _ => Err(self.not_in("SIGHASH_FORKID")),
        }
    }

    /// Decred's two-byte address versions, `(P2PKH, P2SH)`, from dcrd's
    /// chaincfg: secp256k1 ECDSA pubkey hash (`Ds`/`Ts`) and script hash
    /// (`Dc`/`Tc`).
    pub(crate) fn decred_address_versions(self) -> Result<([u8; 2], [u8; 2]), RegistryError> {
        match self {
            Self::Decred => Ok(([0x07, 0x3f], [0x07, 0x1a])),
            Self::DecredTestnet => Ok(([0x0f, 0x21], [0x0e, 0xfc])),
            _ => Err(self.not_in("Decred")),
        }
    }

    /// A wallet on this chain is a BIP-44 account rather than one address:
    /// a phrase wallet stores its account public key, a gap scan finds the
    /// receive and change addresses it has used, receive addresses rotate,
    /// its balance and history sum every one of them, and a send spends from
    /// each with that address's own key. A private-key or watched-address
    /// wallet is the account's one address.
    pub fn uses_account_utxo(self) -> bool {
        matches!(
            self.mainnet_counterpart(),
            Self::Bitcoin
                | Self::BitcoinCash
                | Self::BitcoinSV
                | Self::Litecoin
                | Self::Dogecoin
                | Self::Peercoin
                | Self::Zcash
                | Self::BitcoinGold
                | Self::Decred
                | Self::Kaspa
                | Self::Dash
        )
    }

    /// Whether a wallet's coins can be listed address by address with their
    /// confirmations: an account on a UTXO indexer that reports both.
    /// Decred's Insight and Kaspa's REST API do not count confirmations
    /// against a tip, so their accounts list no coins.
    pub fn lists_account_coins(self) -> bool {
        self.uses_account_utxo() && self.uses_utxo_client()
    }

    /// Peercoin Core amount.h: amount range sanity bound, not a supply cap.
    pub(crate) fn peercoin_max_money(self) -> Result<u64, RegistryError> {
        match self.mainnet_counterpart() {
            Self::Peercoin => Ok(21_000_000 * 1_000_000),
            _ => Err(self.not_in("Peercoin")),
        }
    }

    /// Standard recipient and retained-change floor: CENT, or 0.01 PPC.
    pub(crate) fn peercoin_min_output_units(self) -> Result<u64, RegistryError> {
        match self.mainnet_counterpart() {
            Self::Peercoin => Ok(10_000),
            _ => Err(self.not_in("Peercoin")),
        }
    }

    pub(crate) fn peercoin_min_fee_units(self) -> Result<u64, RegistryError> {
        match self.mainnet_counterpart() {
            Self::Peercoin => Ok(1_000),
            _ => Err(self.not_in("Peercoin")),
        }
    }

    /// Consensus fees charge complete serialized bytes, including witness bytes.
    pub(crate) fn peercoin_fee_per_kb_units(self) -> Result<u64, RegistryError> {
        match self.mainnet_counterpart() {
            Self::Peercoin => Ok(10_000),
            _ => Err(self.not_in("Peercoin")),
        }
    }

    /// Both proof-of-work coinbase and proof-of-stake coinstake outputs mature.
    pub(crate) fn peercoin_generated_output_maturity(self) -> Result<u32, RegistryError> {
        match self {
            Self::Peercoin => Ok(500),
            Self::PeercoinTestnet => Ok(60),
            _ => Err(self.not_in("Peercoin")),
        }
    }

    /// The network's genesis block, as explorers write its hash: what an
    /// indexer on it names at height 0.
    /// The hash of the network's genesis block, as an indexer names the
    /// block at height 0: what a custom Bitcoin or Litecoin indexer is
    /// checked against before it broadcasts.
    pub(crate) fn genesis_block_hash(self) -> Result<String, RegistryError> {
        match self {
            Self::Litecoin => {
                Ok("12a765e31ffd4059bada1e25190f6e98c99d9714d334efa41a195a7e7e04bfe2".into())
            }
            Self::LitecoinTestnet => {
                Ok("4966625a4b2851d9fdee139e56211a0d88575f59ed816ff5e6a63deb4e3e29a0".into())
            }
            _ => self
                .bitcoin_network()
                .map(|network| {
                    bitcoin::blockdata::constants::genesis_block(network)
                        .block_hash()
                        .to_string()
                })
                .ok_or_else(|| self.not_in("Bitcoin or Litecoin")),
        }
    }

    pub(crate) fn litecoin_max_money(self) -> Result<u64, RegistryError> {
        match self {
            Self::Litecoin | Self::LitecoinTestnet => Ok(84_000_000 * 100_000_000),
            _ => Err(self.not_in("Litecoin")),
        }
    }

    /// Litecoin Core's default dust relay fee, in litoshis per virtual kilobyte.
    pub(crate) fn litecoin_dust_relay_fee_per_kvb(self) -> Result<u64, RegistryError> {
        match self {
            Self::Litecoin | Self::LitecoinTestnet => Ok(30_000),
            _ => Err(self.not_in("Litecoin")),
        }
    }

    pub(crate) fn fixed_utxo_segwit_hrp(self) -> Option<&'static str> {
        match self {
            Self::Bitcoin => Some("bc"),
            Self::BitcoinTestnet | Self::BitcoinTestnet4 | Self::BitcoinSignet => Some("tb"),
            Self::Litecoin => Some("ltc"),
            Self::LitecoinTestnet => Some("tltc"),
            Self::BitcoinGold => Some("btg"),
            Self::Peercoin => Some("pc"),
            Self::PeercoinTestnet => Some("tpc"),
            _ => None,
        }
    }

    pub(crate) fn cashaddr_prefix(self) -> Option<&'static str> {
        match self {
            Self::BitcoinCash => Some("bitcoincash"),
            Self::BitcoinCashTestnet => Some("bchtest"),
            _ => None,
        }
    }

    pub(crate) fn fixed_utxo_supports_witness(self, version: u8, program_length: usize) -> bool {
        match self {
            Self::Bitcoin | Self::BitcoinTestnet | Self::BitcoinTestnet4 | Self::BitcoinSignet => {
                (version == 0 && matches!(program_length, 20 | 32))
                    || (version == 1 && program_length == 32)
            }
            Self::Litecoin | Self::LitecoinTestnet => {
                (version == 0 && matches!(program_length, 20 | 32))
                    || (version == 1 && program_length == 32)
            }
            Self::Peercoin | Self::PeercoinTestnet => {
                (version == 0 && matches!(program_length, 20 | 32))
                    || (version == 1 && program_length == 32)
            }
            // BTG's Taproot deployment is not an established active consensus rule.
            Self::BitcoinGold => version == 0 && matches!(program_length, 20 | 32),
            _ => false,
        }
    }

    /// Minimum retained change for the legacy-format P2PKH networks.
    pub(crate) fn legacy_change_dust(self) -> Result<u64, RegistryError> {
        match self.mainnet_counterpart() {
            Self::BitcoinCash
            | Self::BitcoinSV
            | Self::BitcoinGold
            | Self::Dogecoin
            | Self::Dash => Ok(546),
            _ => Err(self.not_in("fixed-fee P2PKH")),
        }
    }

    pub(crate) fn monero_network_name(self) -> Result<&'static str, RegistryError> {
        match self {
            Self::Monero => Ok("mainnet"),
            Self::MoneroStagenet => Ok("stagenet"),
            _ => Err(self.not_in("Monero")),
        }
    }

    pub(crate) fn icp_ledger_id(self) -> Result<&'static str, RegistryError> {
        match self {
            Self::Icp => Ok("00000000000000020101"),
            _ => Err(self.not_in("ICP ledger")),
        }
    }

    pub(crate) fn icp_governance_id(self) -> Result<&'static str, RegistryError> {
        match self {
            Self::Icp => Ok("rrkah-fqaaa-aaaaa-aaaaq-cai"),
            _ => Err(self.not_in("ICP governance")),
        }
    }

    /// NNS governance/disburse_maturity.rs MINIMUM_DISBURSEMENT_E8S and
    /// MAX_NUM_DISBURSEMENTS. Maturity is minted without a ledger transfer fee.
    pub(crate) fn icp_maturity_disbursement_limits(self) -> Result<(u64, usize), RegistryError> {
        match self {
            Self::Icp => Ok((100_000_000, 10)),
            _ => Err(self.not_in("ICP maturity disbursement")),
        }
    }

    /// Mainnet root of trust, from DFINITY agent-rs IC_ROOT_KEY. Never fetched
    /// from a configurable provider, which could otherwise replace the network.
    pub(crate) fn icp_root_key(self) -> Result<Vec<u8>, RegistryError> {
        match self {
            Self::Icp => Ok(hex::decode("308182301d060d2b0601040182dc7c0503010201060c2b0601040182dc7c05030201036100814c0e6ec71fab583b08bd81373c255c3c371b2e84863c98a4f1e08b74235d14fb5d9c0cd546d9685f913a0c0b2cc5341583bf4b4392e467db96d65b9bb4cb717112f8472e0d5a4d14505ffd7484b01291091c5f87b98883463f98091a0baaae").expect("IC mainnet root key")),
            _ => Err(self.not_in("ICP root key")),
        }
    }

    /// The consensus parameters librustzcash builds and scans this network
    /// with: its upgrade schedule is the one schedule Spectra uses.
    pub(crate) fn zcash_network(self) -> Result<zcash_protocol::consensus::Network, RegistryError> {
        match self {
            Self::Zcash => Ok(zcash_protocol::consensus::Network::MainNetwork),
            Self::ZcashTestnet => Ok(zcash_protocol::consensus::Network::TestNetwork),
            _ => Err(self.not_in("Zcash")),
        }
    }

    /// The consensus branch at `height`, from librustzcash's upgrade schedule.
    /// Before NU5 no V5 transaction is valid, so there is none to build for.
    pub(crate) fn zcash_consensus_branch(self, height: u32) -> Result<u32, RegistryError> {
        use zcash_protocol::consensus::{BlockHeight, BranchId, NetworkUpgrade, Parameters};
        let network = self.zcash_network()?;
        if !network.is_nu_active(NetworkUpgrade::Nu5, BlockHeight::from_u32(height)) {
            return Err(RegistryError::ZcashV5Inactive(height));
        }
        Ok(u32::from(BranchId::for_height(
            &network,
            BlockHeight::from_u32(height),
        )))
    }

    pub(crate) fn zcash_genesis(self) -> Result<&'static str, RegistryError> {
        match self {
            Self::Zcash => Ok("00040fe8ec8471911baa1db1266ea15dd06b4a8a5c453883c000b031973dce08"),
            Self::ZcashTestnet => {
                Ok("05a60a92d99d85997cce3b87616c089f6124d7342af37106edc76126334a2c38")
            }
            _ => Err(self.not_in("Zcash")),
        }
    }

    /// What a payment on this network can say about whose deposit it is,
    /// the first the default: XRP's destination tag, Stellar's text and ID
    /// memos. Empty where payments carry none.
    /// How a payment request to an address on this network is written for
    /// a QR code: the format wallets read, with the amount and memo in it.
    /// `None` where no format is widely read, and the address is shared alone.
    pub fn payment_uri_format(self) -> Option<PaymentUriFormat> {
        use PaymentUriFormat::*;
        if self.is_evm() {
            return self.evm_chain_id().is_ok().then_some(Eip681);
        }
        match self.mainnet_counterpart() {
            Self::Bitcoin => Some(Bip21("bitcoin")),
            Self::Litecoin => Some(Bip21("litecoin")),
            Self::Dogecoin => Some(Bip21("dogecoin")),
            Self::BitcoinCash => Some(Bip21("bitcoincash")),
            Self::Dash => Some(Bip21("dash")),
            Self::Zcash => Some(Bip21("zcash")),
            Self::Peercoin => Some(Bip21("peercoin")),
            Self::Solana => Some(SolanaPay),
            Self::Xrp => Some(Xrpl),
            Self::Stellar => Some(Sep7),
            Self::Monero => Some(MoneroUri),
            Self::Ton => Some(TonTransfer),
            _ => None,
        }
    }

    pub fn payment_memo_kinds(self) -> &'static [PaymentMemoKind] {
        match self.mainnet_counterpart() {
            Self::Xrp => &[PaymentMemoKind::DestinationTag],
            Self::Stellar => &[PaymentMemoKind::MemoText, PaymentMemoKind::MemoId],
            _ => &[],
        }
    }

    pub fn stellar_network_passphrase(self) -> Result<&'static str, RegistryError> {
        match self {
            Self::Stellar => Ok("Public Global Stellar Network ; September 2015"),
            Self::StellarTestnet => Ok("Test SDF Network ; September 2015"),
            _ => Err(self.not_in("Stellar")),
        }
    }

    /// Official cluster identities returned by getGenesisHash; see the saved
    /// official-node responses in the chain support audit.
    pub(crate) fn solana_genesis_hash(self) -> Result<&'static str, RegistryError> {
        match self {
            Self::Solana => Ok("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"),
            Self::SolanaDevnet => Ok("EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG"),
            _ => Err(self.not_in("Solana")),
        }
    }

    /// The network magic a Cardano node's genesis states.
    pub(crate) fn cardano_network_magic(self) -> Result<u64, RegistryError> {
        match self {
            Self::Cardano => Ok(764_824_073),
            Self::CardanoPreprod => Ok(1),
            _ => Err(self.not_in("Cardano")),
        }
    }

    pub(crate) fn near_network_name(self) -> Result<&'static str, RegistryError> {
        match self {
            Self::Near => Ok("mainnet"),
            Self::NearTestnet => Ok("testnet"),
            _ => Err(self.not_in("NEAR")),
        }
    }

    /// Aptos network identity bound into each locally constructed transaction.
    pub fn aptos_chain_id(self) -> Option<u8> {
        match self {
            Self::Aptos => Some(1),
            Self::AptosTestnet => Some(2),
            _ => None,
        }
    }

    /// Gas units reserved by Aptos previews and committed into its builders.
    pub fn aptos_max_gas_amount(self) -> Option<u64> {
        matches!(self, Self::Aptos | Self::AptosTestnet).then_some(10_000)
    }

    /// Delegation-pool shares must retain at least 10 APT (framework constant).
    pub fn aptos_delegation_minimum(self) -> Option<u64> {
        (self == Self::Aptos).then_some(1_000_000_000)
    }

    /// Validator-set minimum stake, expressed in MIST.
    pub fn sui_staking_minimum(self) -> Option<u64> {
        (self == Self::Sui).then_some(1_000_000_000)
    }

    /// Maximum native payment reviewed for the staking PTB; dry-run must fit it.
    pub fn sui_staking_gas_budget(self) -> Option<u64> {
        (self == Self::Sui).then_some(10_000_000)
    }

    /// Attached gas budget for the standard staking-pool entry points.
    pub fn near_staking_gas_limit(self) -> Option<u64> {
        (self == Self::Near).then_some(100_000_000_000_000)
    }

    pub(crate) fn near_token_gas_limit(self) -> Option<u64> {
        (self.mainnet_counterpart() == Self::Near).then_some(30_000_000_000_000)
    }

    /// NEP-448: accounts with no more than 770 storage bytes need no storage stake.
    pub(crate) fn near_zero_balance_storage_limit(self) -> Option<u64> {
        (self.mainnet_counterpart() == Self::Near).then_some(770)
    }

    /// EIP-155 chain id. Refuses chains outside the EVM family.
    pub fn evm_chain_id(self) -> Result<u64, RegistryError> {
        Ok(match self {
            Chain::Ethereum => 1,
            Chain::Arbitrum => 42161,
            Chain::Optimism => 10,
            Chain::Avalanche => 43114,
            Chain::Base => 8453,
            Chain::EthereumClassic => 61,
            Chain::BnbChain => 56,
            Chain::Hyperliquid => 999,
            Chain::Polygon => 137,
            Chain::Linea => 59144,
            Chain::Scroll => 534352,
            Chain::Blast => 81457,
            Chain::Mantle => 5000,
            Chain::Sei => 1329,
            Chain::Celo => 42220,
            Chain::Cronos => 25,
            Chain::OpBnb => 204,
            Chain::ZkSyncEra => 324,
            Chain::Sonic => 146,
            Chain::Berachain => 80094,
            Chain::Unichain => 130,
            Chain::Ink => 57073,
            Chain::XLayer => 196,
            Chain::Plasma => 9745,
            Chain::Monad => 143,
            Chain::WorldChain => 480,
            Chain::EthereumSepolia => 11155111,
            Chain::EthereumHoodi => 560048,
            Chain::ArbitrumSepolia => 421614,
            Chain::OptimismSepolia => 11155420,
            Chain::BaseSepolia => 84532,
            Chain::BnbChainTestnet => 97,
            Chain::AvalancheFuji => 43113,
            Chain::PolygonAmoy => 80002,
            Chain::HyperliquidTestnet => 998,
            Chain::LineaSepolia => 59141,
            Chain::CeloSepolia => 11142220,
            Chain::CronosTestnet => 338,
            Chain::ZkSyncEraSepolia => 300,
            Chain::SonicTestnet => 14601,
            Chain::InkSepolia => 763373,
            Chain::XLayerTestnet => 1952,
            Chain::EthereumClassicMordor => 63,
            _ => return Err(self.not_in("EVM")),
        })
    }

    /// Gas-estimate headroom in basis points. Monad charges the entire gas
    /// limit, so its default buffer is smaller than other EVM networks'.
    pub fn evm_gas_buffer_bps(self) -> u32 {
        if self == Self::Monad { 750 } else { 2000 }
    }

    /// Additional rollup fees use the oracle methods deployed on the
    /// concrete network; a missing method must never become a zero fee.
    pub(crate) fn evm_rollup_fee_model(self) -> Option<OpStackFeeModel> {
        if self == Self::CeloSepolia {
            return Some(OpStackFeeModel::Fjord);
        }
        match self.mainnet_counterpart() {
            Self::Optimism
            | Self::Base
            | Self::Celo
            | Self::Unichain
            | Self::Ink
            | Self::WorldChain => Some(OpStackFeeModel::FjordWithOperator),
            Self::OpBnb => Some(OpStackFeeModel::Fjord),
            Self::Blast => Some(OpStackFeeModel::Bedrock),
            _ => None,
        }
    }

    /// Historical operator-fee activation, pinned to the official chain
    /// configurations in docs/audits/chain-support-2026-10-04. Other deployments
    /// require an authoritative historical oracle answer; absence is unknown.
    pub(crate) fn evm_operator_fee_activation(self) -> Option<u64> {
        match self {
            Self::Optimism | Self::Base | Self::Unichain | Self::Ink => Some(1_746_806_401),
            Self::OptimismSepolia | Self::BaseSepolia | Self::InkSepolia => Some(1_744_905_600),
            Self::Celo => Some(1_752_073_200),
            Self::WorldChain => Some(1_764_072_000),
            _ => None,
        }
    }

    /// Ripple's public Mainnet/Testnet IDs, checked against official RPCs.
    pub(crate) fn xrp_network_id(self) -> Option<u64> {
        match self {
            Self::Xrp => Some(0),
            Self::XrpTestnet => Some(1),
            _ => None,
        }
    }

    /// Cross-checked at block zero on TronGrid, PublicNode and Nile's official
    /// endpoint on 2026-10-04; evidence is in the chain-support audit directory.
    pub(crate) fn tron_genesis_block_id(self) -> Option<&'static str> {
        match self {
            Self::Tron => Some("00000000000000001ebf88508a03865c71d452e25f4d51194196a1d22b6653dc"),
            Self::TronNile => {
                Some("0000000000000000d698d4192c56cb6be724a558448e2684802de4d6cd8690dc")
            }
            _ => None,
        }
    }

    /// Last name-mode block timestamp. Proposal 14 on mainnet and proposal 2
    /// on Nile enable parameter 15 after processing the maintenance block's
    /// transactions. The following block uses canonical token IDs. Official
    /// proposal/header evidence: audits/chain-support-2026-10-04/trc10-name-activation.json.
    pub(crate) fn tron_trc10_name_end_ms(self) -> Option<u64> {
        match self {
            Self::Tron => Some(1_546_668_000_000),
            Self::TronNile => Some(1_572_597_600_000),
            _ => None,
        }
    }

    /// Official checkpoint-zero identifiers, cross-checked with public JSON-RPC
    /// and Mysten's GraphQL deployments on 2026-10-04.
    pub(crate) fn sui_network_identity(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Sui => Some(("35834a8a", "4btiuiMPvEENsttpZC7CZ53DruC3MAgfznDbASZ7DR6S")),
            Self::SuiTestnet => Some(("4c78adac", "69WiPg3DAQiwdxfncX6wYQ2siKwAe6L9BZthQea3JNMD")),
            _ => None,
        }
    }

    /// TON masterchain zero-state root/file hashes from the official global
    /// configurations, cross-checked with TON Center on 2026-10-04.
    pub(crate) fn ton_zero_state(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Ton => Some((
                "F6OpKZKqvqeFp6CQmFomXNMfMj2EnaUSOXN+Mh+wVWk=",
                "XplPz01CXAps5qeSWUtxcyBfdAo5zVb1N979KLSKD24=",
            )),
            Self::TonTestnet => Some((
                "gj+B8wb/AmlPk1z1AhVI484rhrUpgSr2oSFIh56VoSg=",
                "Z+IKwYS54DmmJmesw/nAD5DzWadnOCMzee+kdgSYDOg=",
            )),
            _ => None,
        }
    }

    /// The first indexed masterchain block pins v3 indexers that omit the
    /// zero state. Cross-checked with official main/testnet on 2026-10-04.
    pub(crate) fn ton_first_block(self) -> Option<(i64, &'static str, &'static str)> {
        match self {
            Self::Ton => Some((
                -239,
                "8GYhhrigd8CwZGrRT59iulLDcgiTYuvOAzFJxugc0Ts=",
                "V+XzykEwun4yePZhAEPZk77RbMfMOgS/S4GiJkSKY6s=",
            )),
            Self::TonTestnet => Some((
                -3,
                "HBZqdwFA3MSjq0O8ntk6gX1Sibnw7cbWwEjKZt3JJpQ=",
                "eocxdO1VHjKnalajy5t+bM/X7A+rdMMX3Lj2VRRNtFs=",
            )),
            _ => None,
        }
    }

    /// The SS58 prefix a Substrate network's addresses carry: Polkadot's
    /// 0, and the generic 42 on Westend and Bittensor.
    pub fn ss58_prefix(self) -> Option<u16> {
        match self {
            Self::Polkadot => Some(0),
            Self::PolkadotWestend | Self::Bittensor => Some(42),
            _ => None,
        }
    }

    /// Expected genesis for a Substrate deployment. DOT balances and
    /// transfers live on Asset Hub, rather than the relay chain.
    pub fn substrate_genesis_hash(self) -> Option<&'static str> {
        match self {
            Self::Polkadot => {
                Some("0x68d56f15f85d3136970ec16946040bc1752654e906147f7e43e9d539d7c3de2f")
            }
            Self::PolkadotWestend => {
                Some("0x67f9723393ef76214df0118c34bbbd3dbebc8ed46a10973a8c969d48fe7598c9")
            }
            // Cross-checked on both official Finney RPCs on 2026-10-04.
            Self::Bittensor => {
                Some("0x2f0555cc76fc2840a25a6ea3b9637146806f1f44b090c175ffde2a7e5ab36c03")
            }
            _ => None,
        }
    }

    /// The keyless explorer source for this EVM chain, if configured.
    pub fn evm_history_source(self) -> EvmHistorySource<'static> {
        crate::endpoints::catalog()
            .records
            .iter()
            .find(|record| {
                record.chain_id == self
                    && record.api == crate::EndpointApi::Blockscout
                    && record
                        .capabilities
                        .contains(&crate::EndpointCapability::History)
            })
            .map(|record| EvmHistorySource::Open(record.endpoint.as_str()))
            .unwrap_or(EvmHistorySource::Unavailable)
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
            token_standard: "Native".to_string(),
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

    /// Whether this chain's native send needs nothing beyond a destination, an
    /// amount and the fee its preview already supplied.
    ///
    /// The mainnets that answer no are the EVM family, which needs a nonce and
    /// gas overrides, and five that each need something only they have: a UTXO
    /// selection (Bitcoin, Dogecoin), a resolved source account (Internet
    /// Computer), a resource model (Tron) and a mint account (Solana).
    ///
    /// It is a chain fact and it lived as two lists of names in
    /// `AppState+SendExecution`, next to a comment saying the lists should not
    /// be there. A chain reaching the shared path had to be added to whichever
    /// of the two the author happened to be looking at.
    pub fn uses_generic_send_submit(self) -> bool {
        matches!(
            self.mainnet_counterpart(),
            Chain::Sui
                | Chain::Aptos
                | Chain::Ton
                | Chain::Xrp
                | Chain::Stellar
                | Chain::Cardano
                | Chain::Polkadot
                | Chain::Near
                | Chain::BitcoinCash
                | Chain::BitcoinSV
                | Chain::Litecoin
                | Chain::Zcash
                | Chain::BitcoinGold
                | Chain::Decred
                | Chain::Kaspa
                | Chain::Dash
                | Chain::Peercoin
                | Chain::Bittensor
                // It has a shared-path preview like the rest.
                | Chain::Monero
        )
    }

    /// How this chain's fee enters a send, and the fee to assume without a
    /// preview.
    pub fn send_execution_shape(self) -> SendExecutionShape {
        let chain = self.mainnet_counterpart();
        match chain {
            Chain::Sui => SendExecutionShape {
                fee_field: SendFeeField::GasBudget,
                fee_fallback: None,
            },
            Chain::Cardano | Chain::Aptos | Chain::Polkadot | Chain::Bittensor | Chain::Near => {
                SendExecutionShape {
                    fee_field: SendFeeField::FeeAmount,
                    fee_fallback: None,
                }
            }
            Chain::Bitcoin | Chain::BitcoinCash | Chain::BitcoinSV => SendExecutionShape {
                fee_field: SendFeeField::FeeSats,
                fee_fallback: Some("0.00001"),
            },
            Chain::Peercoin => SendExecutionShape {
                fee_field: SendFeeField::FeeSats,
                fee_fallback: None,
            },
            Chain::Litecoin => SendExecutionShape {
                fee_field: SendFeeField::FeeSats,
                fee_fallback: Some("0.0001"),
            },
            // The five whose send existed but was unroutable. `fee_fallback`
            // is the default `execute_send` already applies when the request
            // carries no `fee_sat`, in the chain's own units — so the fee the
            // sheet shows and validates against is the fee core will use.
            // None of them has a shared-path preview, and without a fallback
            // the generic submit refuses for want of an estimate.
            Chain::Zcash | Chain::BitcoinGold | Chain::Kaspa => SendExecutionShape {
                fee_field: SendFeeField::FeeSats,
                fee_fallback: Some("0.00001"),
            },
            Chain::Decred | Chain::Dash => SendExecutionShape {
                fee_field: SendFeeField::FeeSats,
                fee_fallback: Some("0.00002"),
            },
            _ => SendExecutionShape {
                fee_field: SendFeeField::None,
                fee_fallback: None,
            },
        }
    }

    /// How this chain's pending transactions reach a final status.
    ///
    /// Resolved through the mainnet counterpart so a testnet cannot diverge.
    pub fn pending_status_poll(self) -> PendingStatusPoll {
        let chain = self.mainnet_counterpart();
        match chain {
            // Litecoin tracks receives too: its explorer confirms them on a
            // different cadence than the send path assumes.
            Chain::Litecoin => PendingStatusPoll::Utxo {
                require_send_kind: false,
            },
            Chain::Bitcoin
            | Chain::BitcoinCash
            | Chain::BitcoinSV
            | Chain::Dogecoin
            | Chain::Zcash
            | Chain::BitcoinGold
            | Chain::Decred
            | Chain::Kaspa
            | Chain::Dash
            | Chain::Peercoin => PendingStatusPoll::Utxo {
                require_send_kind: true,
            },
            Chain::Ton => PendingStatusPoll::TransactionStatus(EndpointApi::ToncenterV3),
            Chain::Tron
            | Chain::Solana
            | Chain::Cardano
            | Chain::Xrp
            | Chain::Stellar
            | Chain::Monero
            | Chain::Sui
            | Chain::Aptos
            | Chain::Icp
            | Chain::Near => PendingStatusPoll::TransactionStatus(
                chain.default_api().expect("Account transaction status API"),
            ),
            Chain::Polkadot | Chain::Bittensor => PendingStatusPoll::SubstrateFinality,
            other if other.is_evm() => PendingStatusPoll::EvmReceipt,
            _ => PendingStatusPoll::None,
        }
    }

    /// Whether a tracked token on this chain can be sent. Every chain sends
    /// its own asset.
    ///
    /// Every representable fungible-token standard has a transfer builder.
    pub fn sends_tokens(self) -> bool {
        let chain = self.mainnet_counterpart();
        match chain {
            Chain::Solana
            | Chain::Tron
            | Chain::Near
            | Chain::Sui
            | Chain::Aptos
            | Chain::Ton
            | Chain::Xrp
            | Chain::Stellar
            | Chain::Cardano => true,
            _ => chain.is_evm(),
        }
    }

    /// How incoming history merges with stored history.
    /// Exhaustive so each new chain must specify its merge rule.
    pub fn transaction_merge_strategy(
        self,
    ) -> crate::fetch::transactions::TransactionMergeStrategy {
        use crate::fetch::transactions::TransactionMergeStrategy as S;
        // Resolved through the mainnet counterpart so a testnet can never merge
        // differently from the chain it mirrors — listing them separately is
        // how `zcash-testnet` silently ended up account-based.
        match self.mainnet_counterpart() {
            // Dogecoin's own variant: its explorer reports change outputs in a
            // shape the shared UTXO merge mishandles.
            Chain::Dogecoin => S::Dogecoin,
            Chain::Bitcoin
            | Chain::BitcoinCash
            | Chain::BitcoinSV
            | Chain::Litecoin
            | Chain::BitcoinGold
            | Chain::Dash
            | Chain::Decred
            | Chain::Zcash
            | Chain::Kaspa
            | Chain::Peercoin => S::StandardUtxo,
            other if other.is_evm() => S::Evm,
            _ => S::AccountBased,
        }
    }

    /// Encode a discovered public child using this chain's supported address format.
    pub(crate) fn encode_discovery_address(
        self,
        key: &secp256k1::PublicKey,
        script: crate::derivation::types::BitcoinScriptType,
    ) -> Result<String, crate::derivation::error::DerivationError> {
        use crate::derivation::error::DerivationError;
        use crate::derivation::{
            bitcoin as btc, bitcoin_cash as bch, dogecoin as doge, litecoin as ltc,
        };
        Ok(match self {
            Self::Bitcoin => return btc::encode_address_inner(btc::BTC_MAINNET, script, key),
            Self::BitcoinTestnet | Self::BitcoinTestnet4 | Self::BitcoinSignet => {
                return btc::encode_address_inner(btc::BTC_TESTNET, script, key);
            }
            Self::BitcoinCash => btc::encode_p2pkh(bch::BCH_MAINNET_VERSION, &key.serialize()),
            Self::BitcoinCashTestnet => {
                btc::encode_p2pkh(bch::BCH_TESTNET_VERSION, &key.serialize())
            }
            Self::BitcoinSV | Self::BitcoinSVTestnet => {
                btc::encode_p2pkh(self.fixed_utxo_address_versions()?.0, &key.serialize())
            }
            Self::Dogecoin => btc::encode_p2pkh(doge::DOGE_MAINNET_VERSION, &key.serialize()),
            Self::DogecoinTestnet => {
                btc::encode_p2pkh(doge::DOGE_TESTNET_VERSION, &key.serialize())
            }
            Self::Litecoin | Self::LitecoinTestnet => {
                return ltc::encode_litecoin_address(self, script, key);
            }
            Self::Peercoin | Self::PeercoinTestnet => {
                return crate::derivation::peercoin::encode_peercoin_address(self, script, key);
            }
            Self::BitcoinGold | Self::Dash | Self::DashTestnet => {
                btc::encode_p2pkh(self.fixed_utxo_address_versions()?.0, &key.serialize())
            }
            Self::Zcash | Self::ZcashTestnet => {
                let version = if self.is_testnet() {
                    [0x1d, 0x25]
                } else {
                    [0x1c, 0xb8]
                };
                let mut payload = version.to_vec();
                payload.extend(btc::hash160(&key.serialize()));
                btc::base58check_encode(&payload)
            }
            Self::Decred | Self::DecredTestnet => crate::derivation::decred::encode_decred_p2pkh(
                self,
                &crate::derivation::decred::dcr_hash160(&key.serialize()),
            )?,
            Self::Kaspa | Self::KaspaTestnet => {
                let hrp = if self.is_testnet() {
                    crate::derivation::kaspa::KASPA_TESTNET_HRP
                } else {
                    crate::derivation::kaspa::KASPA_HRP
                };
                return crate::derivation::kaspa::encode_kaspa_address(
                    0,
                    &key.x_only_public_key().0.serialize(),
                    hrp,
                );
            }
            _ => {
                return Err(DerivationError::invalid(
                    "chain does not support UTXO discovery",
                ));
            }
        })
    }

    /// Whether the ledger creates an account only once it holds the
    /// network's reserve, so a first payment below it fails.
    pub fn requires_account_reserve(self) -> bool {
        matches!(self.mainnet_counterpart(), Self::Xrp | Self::Stellar)
    }

    /// Whether a key holds one account per wallet contract version (TON's
    /// W5 and v4R2), so a wallet is a key and the version it signs as.
    pub fn has_wallet_versions(self) -> bool {
        self.mainnet_counterpart() == Self::Ton
    }

    /// Whether a wallet's balance and history come from scanning blocks on
    /// the device through a daemon, rather than from a provider's index.
    pub fn scans_for_balance(self) -> bool {
        self.mainnet_counterpart() == Self::Monero
    }

    /// Whether a phrase wallet's address names a key besides the one it
    /// signs with: Cardano's base address carries its account's stake key,
    /// so the payment key alone imports as another (enterprise) address.
    pub(crate) fn phrase_address_has_stake_key(self) -> bool {
        self.mainnet_counterpart() == Self::Cardano
    }

    /// Does a name typed as a destination resolve to an address on this chain?
    ///
    /// ENS is a registry deployed on Ethereum mainnet, so that is the only
    /// chain a `.eth` name is looked up for. The address it returns is a plain
    /// EVM address that would spend anywhere, but a name pointing at a mainnet
    /// contract need not point at anything on an L2 — a destination reached by
    /// name is accepted only where the registry that named it lives, which is
    /// the stricter of the two readings.
    pub fn resolves_ens_names(self) -> bool {
        matches!(self, Chain::Ethereum)
    }

    /// Canonical address normalization for storage and comparison.
    /// Testnets follow their mainnet; EVM addresses are lowercased.
    pub fn address_normalization(self) -> AddressNormalization {
        if self.is_evm() {
            return AddressNormalization::Lowercase;
        }
        match self.mainnet_counterpart() {
            Chain::Stellar => AddressNormalization::Uppercase,
            Chain::Sui | Chain::Aptos => AddressNormalization::LowercaseHexPrefixed,
            Chain::Icp | Chain::Near => AddressNormalization::Lowercase,
            _ => AddressNormalization::None,
        }
    }

    /// Address-format key used by validation. Exhaustive so each new chain
    /// must declare its format; EVM chains share the network-agnostic format.
    pub fn address_validation_kind(self) -> &'static str {
        match self {
            // EVM: one format, network-agnostic on the wire.
            Chain::Ethereum
            | Chain::Arbitrum
            | Chain::Optimism
            | Chain::Avalanche
            | Chain::Base
            | Chain::EthereumClassic
            | Chain::BnbChain
            | Chain::Hyperliquid
            | Chain::Polygon
            | Chain::Linea
            | Chain::Scroll
            | Chain::Blast
            | Chain::Mantle
            | Chain::Sei
            | Chain::Celo
            | Chain::Cronos
            | Chain::OpBnb
            | Chain::ZkSyncEra
            | Chain::Sonic
            | Chain::Berachain
            | Chain::Unichain
            | Chain::Ink
            | Chain::XLayer
            | Chain::Plasma
            | Chain::Monad
            | Chain::WorldChain => "evm",
            Chain::EthereumSepolia
            | Chain::EthereumHoodi
            | Chain::ArbitrumSepolia
            | Chain::OptimismSepolia
            | Chain::BaseSepolia
            | Chain::BnbChainTestnet
            | Chain::AvalancheFuji
            | Chain::PolygonAmoy
            | Chain::HyperliquidTestnet
            | Chain::LineaSepolia
            | Chain::CeloSepolia
            | Chain::CronosTestnet
            | Chain::ZkSyncEraSepolia
            | Chain::SonicTestnet
            | Chain::InkSepolia
            | Chain::XLayerTestnet
            | Chain::EthereumClassicMordor => "evmTestnet",

            Chain::Bitcoin => "bitcoin",
            Chain::BitcoinTestnet => "bitcoinTestnet",
            Chain::BitcoinTestnet4 => "bitcoinTestnet4",
            Chain::BitcoinSignet => "bitcoinSignet",
            Chain::BitcoinCash => "bitcoinCash",
            Chain::BitcoinCashTestnet => "bitcoinCashTestnet",
            Chain::BitcoinSV => "bitcoinSV",
            Chain::BitcoinSVTestnet => "bitcoinSVTestnet",
            Chain::Litecoin => "litecoin",
            Chain::LitecoinTestnet => "litecoinTestnet",
            Chain::Dogecoin => "dogecoin",
            Chain::DogecoinTestnet => "dogecoinTestnet",
            Chain::Peercoin => "peercoin",
            Chain::PeercoinTestnet => "peercoinTestnet",
            Chain::Tron => "tron",
            Chain::TronNile => "tronTestnet",
            Chain::Solana => "solana",
            Chain::SolanaDevnet => "solanaDevnet",
            Chain::Stellar => "stellar",
            Chain::StellarTestnet => "stellarTestnet",
            Chain::Xrp => "xrp",
            Chain::XrpTestnet => "xrpTestnet",
            Chain::Sui => "sui",
            Chain::SuiTestnet => "suiTestnet",
            Chain::Aptos => "aptos",
            Chain::AptosTestnet => "aptosTestnet",
            Chain::Ton => "ton",
            Chain::TonTestnet => "tonTestnet",
            Chain::Icp => "internetComputer",
            Chain::Near => "near",
            Chain::NearTestnet => "nearTestnet",
            Chain::Polkadot => "polkadot",
            Chain::PolkadotWestend => "polkadotTestnet",
            Chain::Monero => "monero",
            Chain::MoneroStagenet => "moneroStagenet",
            Chain::Cardano => "cardano",
            Chain::CardanoPreprod => "cardanoTestnet",
            Chain::Zcash => "zcash",
            Chain::ZcashTestnet => "zcashTestnet",
            Chain::BitcoinGold => "bitcoinGold",
            Chain::Decred => "decred",
            Chain::DecredTestnet => "decredTestnet",
            Chain::Kaspa => "kaspa",
            Chain::KaspaTestnet => "kaspaTestnet",
            Chain::Dash => "dash",
            Chain::DashTestnet => "dashTestnet",
            Chain::Bittensor => "bittensor",
        }
    }

    /// `true` when an account's extended public key can be watched on this
    /// chain, standing in for the whole account as one wallet rather than
    /// one per address: every account UTXO network, each in the encodings
    /// `account_key_versions` lists.
    pub fn accepts_account_xpub(self) -> bool {
        !self.account_key_versions().is_empty()
    }

    /// The rust-bitcoin network a Bitcoin chain is; `None` off Bitcoin.
    pub(crate) fn bitcoin_network(self) -> Option<bitcoin::Network> {
        match self {
            Chain::Bitcoin => Some(bitcoin::Network::Bitcoin),
            Chain::BitcoinTestnet => Some(bitcoin::Network::Testnet),
            Chain::BitcoinTestnet4 => Some(bitcoin::Network::Testnet4),
            Chain::BitcoinSignet => Some(bitcoin::Network::Signet),
            _ => None,
        }
    }

    /// The form a multisig account's policy takes on this chain, where its
    /// address derives from one: a UTXO network's `sortedmulti` descriptor,
    /// Sui's multisig public key, Aptos's MultiKey, Cardano's native script,
    /// a Substrate multisig's signatories.
    pub fn multisig_policy_format(self) -> Option<crate::derivation::setup::WalletSecretFormat> {
        use crate::derivation::setup::WalletSecretFormat;
        if self.utxo_multisig_script().is_some() {
            return Some(WalletSecretFormat::MultisigDescriptor);
        }
        match self.mainnet_counterpart() {
            Self::Sui => Some(WalletSecretFormat::SuiMultisigPublicKey),
            Self::Aptos => Some(WalletSecretFormat::AptosMultiKey),
            Self::Cardano => Some(WalletSecretFormat::CardanoNativeScript),
            Self::Polkadot | Self::Bittensor => Some(WalletSecretFormat::SubstrateMultisig),
            _ => None,
        }
    }

    /// The script a UTXO multisig account pays its `sortedmulti` through:
    /// P2WSH on Bitcoin and Litecoin, P2SH on Bitcoin Cash and Dogecoin,
    /// which have no SegWit.
    pub(crate) fn utxo_multisig_script(
        self,
    ) -> Option<crate::derivation::multisig::UtxoMultisigScript> {
        use crate::derivation::multisig::UtxoMultisigScript;
        match self.mainnet_counterpart() {
            Self::Bitcoin | Self::Litecoin => Some(UtxoMultisigScript::Wsh),
            Self::BitcoinCash | Self::Dogecoin => Some(UtxoMultisigScript::Sh),
            _ => None,
        }
    }

    /// `true` when a wallet on this chain can be watched from its address
    /// and private view key, scanning what it receives without its spend
    /// key: Monero's view-only wallet.
    pub fn watches_with_view_key(self) -> bool {
        self.mainnet_counterpart() == Chain::Monero
    }

    /// The prefixes an account public key starts with on this network, in
    /// `account_key_versions` order: what a front end names in its prompt.
    pub fn account_key_prefixes(self) -> Vec<&'static str> {
        self.account_key_versions()
            .iter()
            .map(|version| version.prefix)
            .collect()
    }

    /// The encodings an account public key is read in on this network, the
    /// one its own wallets export first. Each names the script the account's
    /// addresses pay. From SLIP-132 (Bitcoin, Litecoin), Trezor's coin
    /// definitions (Litecoin's zpub, Dogecoin's dgub, Dash's drkp, Decred's
    /// dpub and its own tpub, and the xpub of Bitcoin Cash, Bitcoin Gold,
    /// Peercoin and Zcash), the nodes' own chain parameters (Dash Core's
    /// xpub, Dogecoin Core's testnet tpub), ElectrumSV's xpub on Bitcoin SV
    /// and rusty-kaspa's kpub and ktub. A Taproot account has no version of
    /// its own, nor do the SegWit accounts of a network Spectra signs only
    /// P2PKH on.
    pub(crate) fn account_key_versions(self) -> &'static [AccountKeyVersion] {
        use crate::derivation::types::BitcoinScriptType::{P2pkh, P2shP2wpkh, P2wpkh};
        const fn v(
            version: u32,
            prefix: &'static str,
            script: crate::derivation::types::BitcoinScriptType,
        ) -> AccountKeyVersion {
            AccountKeyVersion {
                version: version.to_be_bytes(),
                prefix,
                script,
            }
        }
        const XPUB: AccountKeyVersion = v(0x0488_b21e, "xpub", P2pkh);
        const YPUB: AccountKeyVersion = v(0x049d_7cb2, "ypub", P2shP2wpkh);
        const ZPUB: AccountKeyVersion = v(0x04b2_4746, "zpub", P2wpkh);
        const TPUB: AccountKeyVersion = v(0x0435_87cf, "tpub", P2pkh);
        const UPUB: AccountKeyVersion = v(0x044a_5262, "upub", P2shP2wpkh);
        const VPUB: AccountKeyVersion = v(0x045f_1cf6, "vpub", P2wpkh);
        const LTUB: AccountKeyVersion = v(0x019d_a462, "Ltub", P2pkh);
        const MTUB: AccountKeyVersion = v(0x01b2_6ef6, "Mtub", P2shP2wpkh);
        const TTUB: AccountKeyVersion = v(0x0436_f6e1, "ttub", P2pkh);
        const DRKP: AccountKeyVersion = v(0x02fe_52cc, "drkp", P2pkh);
        const DGUB: AccountKeyVersion = v(0x02fa_cafd, "dgub", P2pkh);
        const DPUB: AccountKeyVersion = v(0x02fd_a926, "dpub", P2pkh);
        const DECRED_TPUB: AccountKeyVersion = v(0x0435_87d1, "tpub", P2pkh);
        const KPUB: AccountKeyVersion = v(0x038f_332e, "kpub", P2pkh);
        const KTUB: AccountKeyVersion = v(0x0390_a241, "ktub", P2pkh);
        match self {
            Self::Bitcoin | Self::Peercoin => &[XPUB, YPUB, ZPUB],
            Self::BitcoinTestnet
            | Self::BitcoinTestnet4
            | Self::BitcoinSignet
            | Self::PeercoinTestnet => &[TPUB, UPUB, VPUB],
            Self::Litecoin => &[LTUB, MTUB, ZPUB],
            Self::LitecoinTestnet => &[TPUB, UPUB, VPUB, TTUB],
            Self::BitcoinCash | Self::BitcoinSV | Self::BitcoinGold | Self::Zcash => &[XPUB],
            Self::Dash => &[XPUB, DRKP],
            Self::Dogecoin => &[DGUB],
            Self::Decred => &[DPUB],
            Self::DecredTestnet => &[DECRED_TPUB],
            Self::Kaspa => &[KPUB],
            Self::KaspaTestnet => &[KTUB],
            Self::BitcoinCashTestnet
            | Self::BitcoinSVTestnet
            | Self::ZcashTestnet
            | Self::DashTestnet
            | Self::DogecoinTestnet => &[TPUB],
            _ => &[],
        }
    }

    /// Whether a message is proven for a SegWit or Taproot address with a
    /// BIP-322 signature: Bitcoin's networks, whose verifiers read it.
    pub(crate) fn proves_with_bip322(self) -> bool {
        self.mainnet_counterpart() == Self::Bitcoin
    }

    /// `true` when a wallet on this chain can be imported watch-only from an
    /// address alone.
    ///
    /// Monero is the notable exclusion: watching a Monero account needs the
    /// private view key, which an address does not carry. Addresses are
    /// validated and stored against their concrete mainnet or testnet.
    pub fn supports_watch_only_import(self) -> bool {
        if self.is_evm() {
            return true;
        }
        matches!(
            self.mainnet_counterpart(),
            Chain::Bitcoin
                | Chain::BitcoinCash
                | Chain::BitcoinSV
                | Chain::Litecoin
                | Chain::Dogecoin
                | Chain::Tron
                | Chain::Solana
                | Chain::Xrp
                | Chain::Stellar
                | Chain::Cardano
                | Chain::Sui
                | Chain::Aptos
                | Chain::Ton
                | Chain::Icp
                | Chain::Near
                | Chain::Polkadot
                | Chain::Zcash
                | Chain::BitcoinGold
                | Chain::Decred
                | Chain::Kaspa
                | Chain::Dash
                | Chain::Peercoin
                | Chain::Bittensor
        )
    }

    pub fn flags_evm_address_as_wrong_chain(self) -> bool {
        matches!(
            self,
            Chain::Bitcoin
                | Chain::BitcoinCash
                | Chain::Litecoin
                | Chain::Dogecoin
                | Chain::BitcoinTestnet
                | Chain::BitcoinTestnet4
                | Chain::BitcoinSignet
                | Chain::BitcoinCashTestnet
                | Chain::LitecoinTestnet
                | Chain::DogecoinTestnet
                | Chain::Peercoin
                | Chain::PeercoinTestnet
        )
    }

    /// Protocol fee estimates in native smallest units. Testnets share the
    /// family's estimate; endpoints still belong to the concrete network.
    pub fn static_fee_units(self) -> Option<u128> {
        match self.mainnet_counterpart() {
            Chain::Solana => Some(5_000),
            Chain::Tron => Some(1_000_000),
            Chain::Cardano => Some(170_000),
            Chain::Bittensor => Some(125_000),
            Chain::Sui => Some(1_000),
            Chain::Ton => Some(7_000_000),
            Chain::Icp => Some(10_000),
            Chain::Zcash => Some(10_000),
            Chain::Monero => Some(500_000_000),
            Chain::Dogecoin => Some(1_000_000),
            Chain::Litecoin | Chain::BitcoinSV | Chain::BitcoinGold | Chain::Kaspa => Some(1_000),
            Chain::BitcoinCash | Chain::Decred | Chain::Dash => Some(2_000),
            _ => None,
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peercoin_networks_use_native_precision_and_protocol_rules() {
        for (chain, id, symbol, path, maturity) in [
            (Chain::Peercoin, "peercoin", "PPC", "m/44'/6'/0'/0/0", 500),
            (
                Chain::PeercoinTestnet,
                "peercoin-testnet",
                "tPPC",
                "m/44'/1'/0'/0/0",
                60,
            ),
        ] {
            assert_eq!(chain.str_id(), id);
            assert_eq!(chain.mainnet_counterpart(), Chain::Peercoin);
            assert_eq!(chain.native_decimals(), 6);
            assert_eq!(chain.coin_symbol(), symbol);
            assert_eq!(
                crate::derivation::path::default_path_from_catalog(chain).unwrap(),
                path
            );
            assert_eq!(chain.entry().artwork_name, "peercoin");
            assert_eq!(
                chain.peercoin_generated_output_maturity().unwrap(),
                maturity
            );
            assert_eq!(chain.peercoin_min_output_units().unwrap(), 10_000);
            assert_eq!(chain.peercoin_min_fee_units().unwrap(), 1_000);
            assert_eq!(chain.peercoin_fee_per_kb_units().unwrap(), 10_000);
            assert_eq!(chain.peercoin_max_money().unwrap(), 21_000_000_000_000);
            assert!(chain.uses_utxo_client());
            assert!(chain.uses_account_utxo());
            assert!(chain.supports_watch_only_import());
            assert!(chain.derives_from_private_key());
            assert!(chain.has_send_preview());
            assert!(chain.send_execution_shape().fee_fallback.is_none());
            assert!(chain.static_fee_units().is_none());
            assert!(!chain.supports_staking());
            assert!(!chain.hosts_tokens());
            assert!(!chain.is_evm());
            assert_eq!(chain.endpoint_apis(), &[EndpointApi::Blockbook]);
        }
        assert_eq!(Chain::Peercoin.coingecko_id(), "peercoin");
        assert_eq!(Chain::PeercoinTestnet.coingecko_id(), "");
        assert!(Chain::Bitcoin.peercoin_min_fee_units().is_err());
    }

    /// A chain with several APIs needs one client answering in all of them,
    /// and only the UTXO family has one.
    #[test]
    fn only_the_utxo_client_speaks_several_apis() {
        for chain in Chain::all() {
            if chain.endpoint_apis().len() > 1 {
                assert!(chain.uses_utxo_client(), "{}", chain.str_id());
            }
        }
    }

    /// Every EVM chain carries its EIP-155 id, and each id is the one its
    /// network signs for.
    #[test]
    fn evm_chains_carry_their_eip155_ids() {
        for chain in Chain::all().filter(|chain| chain.is_evm()) {
            assert!(
                chain.evm_chain_id().unwrap() > 0,
                "{} has no chain id",
                chain.str_id()
            );
        }
        assert_eq!(Chain::Ethereum.evm_chain_id().unwrap(), 1);
        assert_eq!(Chain::EthereumSepolia.evm_chain_id().unwrap(), 11_155_111);
        assert_eq!(Chain::EthereumHoodi.evm_chain_id().unwrap(), 560_048);
        assert_eq!(Chain::Arbitrum.evm_chain_id().unwrap(), 42161);
        assert_eq!(Chain::Optimism.evm_chain_id().unwrap(), 10);
        assert_eq!(Chain::Avalanche.evm_chain_id().unwrap(), 43114);
        assert_eq!(Chain::Base.evm_chain_id().unwrap(), 8453);
        assert_eq!(Chain::EthereumClassic.evm_chain_id().unwrap(), 61);
        assert_eq!(Chain::BnbChain.evm_chain_id().unwrap(), 56);
        assert_eq!(Chain::Hyperliquid.evm_chain_id().unwrap(), 999);
        assert_eq!(Chain::Polygon.evm_chain_id().unwrap(), 137);
        assert_eq!(Chain::Linea.evm_chain_id().unwrap(), 59144);
        assert_eq!(Chain::Scroll.evm_chain_id().unwrap(), 534352);
        assert_eq!(Chain::Blast.evm_chain_id().unwrap(), 81457);
        assert_eq!(Chain::Mantle.evm_chain_id().unwrap(), 5000);
        assert_eq!(Chain::Sei.evm_chain_id().unwrap(), 1329);
        assert_eq!(Chain::Celo.evm_chain_id().unwrap(), 42220);
        assert_eq!(Chain::Cronos.evm_chain_id().unwrap(), 25);
        assert_eq!(Chain::OpBnb.evm_chain_id().unwrap(), 204);
        assert_eq!(Chain::ZkSyncEra.evm_chain_id().unwrap(), 324);
        assert_eq!(Chain::Sonic.evm_chain_id().unwrap(), 146);
        assert_eq!(Chain::Berachain.evm_chain_id().unwrap(), 80094);
        assert_eq!(Chain::Unichain.evm_chain_id().unwrap(), 130);
        assert_eq!(Chain::Ink.evm_chain_id().unwrap(), 57073);
        assert_eq!(Chain::XLayer.evm_chain_id().unwrap(), 196);
        assert_eq!(Chain::Plasma.evm_chain_id().unwrap(), 9745);
        assert_eq!(Chain::Monad.evm_chain_id().unwrap(), 143);
        assert_eq!(Chain::WorldChain.evm_chain_id().unwrap(), 480);
    }

    #[test]
    fn non_evm_chains_have_no_eip155_identity() {
        for chain in Chain::all().filter(|chain| !chain.is_evm()) {
            assert!(chain.evm_chain_id().is_err(), "{}", chain.str_id());
        }
    }

    #[test]
    fn new_evm_mainnets_have_native_assets_and_shared_wallet_rules() {
        for (chain, id, chain_id, symbol, token_id, artwork) in [
            (Chain::Plasma, "plasma", 9745, "XPL", "plasma", "plasma"),
            (Chain::Monad, "monad", 143, "MON", "monad", "monad"),
            (
                Chain::WorldChain,
                "world-chain",
                480,
                "ETH",
                "ethereum",
                "worldcoin",
            ),
        ] {
            assert_eq!(Chain::parse(id).unwrap(), chain);
            assert!(!chain.is_testnet());
            assert_eq!(chain.mainnet_counterpart(), chain);
            assert_eq!(chain.evm_chain_id().unwrap(), chain_id);
            assert_eq!(chain.address_slot(), "ethereum");
            assert_eq!(chain.address_validation_kind(), "evm");
            assert!(chain.derives_from_private_key());
            assert!(chain.supports_watch_only_import());
            assert!(chain.sends_tokens());
            assert!(chain.allows_token_standard("ERC-20"));
            assert_eq!(chain.coin_symbol(), symbol);
            assert_eq!(chain.native_decimals(), 18);
            assert_eq!(chain.entry().artwork_name, artwork);
            assert_eq!(
                chain.entry().derivation_path[0].path,
                "m/44'/60'/{account}'/0/0"
            );
            let deployment = crate::tokens::deployment(&format!("{id}:native")).unwrap();
            assert!(deployment.is_native());
            assert_eq!(deployment.token_id, token_id);
            assert_eq!(deployment.chain_id, chain);
            assert_eq!(deployment.symbol, symbol);
            assert_eq!(deployment.decimals, 18);
            assert!(deployment.contract.is_empty());
            assert!(crate::endpoints::catalog().records.iter().any(|record| {
                record.chain_id == chain
                    && record.api == crate::EndpointApi::EvmJsonRpc
                    && record
                        .capabilities
                        .contains(&crate::EndpointCapability::Balance)
                    && record
                        .capabilities
                        .contains(&crate::EndpointCapability::Fee)
                    && record
                        .capabilities
                        .contains(&crate::EndpointCapability::Broadcast)
            }));
        }
    }

    #[test]
    fn verified_new_chain_tokens_have_exact_deployment_identity() {
        for (chain, token_id, contract, decimals) in [
            (
                Chain::Plasma,
                "usd-coin",
                "0x2d661c89d812261039af9764eceaaee884f5f67f",
                6,
            ),
            (
                Chain::Plasma,
                "euro-coin",
                "0x3ee196e78d4d4248b849b8e1c7f44c5457fafd2c",
                6,
            ),
            (
                Chain::Monad,
                "pancakeswap-token",
                "0xf59d81cd43f620e722e07f9cb3f6e41b031017a3",
                18,
            ),
            (
                Chain::Monad,
                "usd-coin",
                "0x754704bc059f8c67012fed69bc8a327a5aafb603",
                6,
            ),
            (
                Chain::WorldChain,
                "usd-coin",
                "0x79a02482a880bce3f13e09da970dc34db4cd24d1",
                6,
            ),
            (
                Chain::WorldChain,
                "worldcoin-wld",
                "0x2cfc85d8e48f8eab294be644d9e25c3030863003",
                18,
            ),
        ] {
            let id = crate::tokens::deployment_id_for(chain, Some(contract)).unwrap();
            let deployment = crate::tokens::deployment(&id).unwrap();
            assert_eq!(deployment.token_id, token_id);
            assert_eq!(deployment.chain_id, chain);
            assert_eq!(deployment.contract, contract);
            assert_eq!(deployment.decimals, decimals);
            assert_eq!(deployment.token_standard, "ERC-20");
            assert!(!deployment.is_native());
        }
    }

    #[test]
    fn new_mainnet_indexers_declare_their_verified_account_methods() {
        for (chain, endpoint) in [
            (
                Chain::Plasma,
                "https://api.routescan.io/v2/network/mainnet/evm/9745/etherscan",
            ),
            (
                Chain::WorldChain,
                "https://worldchain-mainnet.explorer.alchemy.com",
            ),
        ] {
            assert_eq!(chain.evm_history_source(), EvmHistorySource::Open(endpoint));
            let record = crate::endpoints::catalog()
                .records
                .iter()
                .find(|record| record.chain_id == chain && record.endpoint == endpoint)
                .unwrap();
            assert_eq!(record.api, crate::EndpointApi::Blockscout);
            for capability in [
                crate::EndpointCapability::History,
                crate::EndpointCapability::TokenHistory,
                crate::EndpointCapability::TokenDiscovery,
            ] {
                assert!(record.capabilities.contains(&capability));
            }
        }
    }

    #[test]
    fn rollup_fees_follow_deployed_oracle_methods() {
        for chain in [
            Chain::Optimism,
            Chain::OptimismSepolia,
            Chain::Base,
            Chain::BaseSepolia,
            Chain::Celo,
            Chain::Unichain,
            Chain::Ink,
            Chain::InkSepolia,
            Chain::WorldChain,
        ] {
            assert_eq!(
                chain.evm_rollup_fee_model(),
                Some(OpStackFeeModel::FjordWithOperator),
                "{chain}"
            );
        }
        assert_eq!(
            Chain::CeloSepolia.evm_rollup_fee_model(),
            Some(OpStackFeeModel::Fjord)
        );
        assert_eq!(
            Chain::OpBnb.evm_rollup_fee_model(),
            Some(OpStackFeeModel::Fjord)
        );
        assert_eq!(
            Chain::Blast.evm_rollup_fee_model(),
            Some(OpStackFeeModel::Bedrock)
        );
        assert_eq!(Chain::Plasma.evm_rollup_fee_model(), None);
        assert_eq!(Chain::Monad.evm_rollup_fee_model(), None);
        assert_eq!(Chain::Monad.evm_gas_buffer_bps(), 750);
        assert_eq!(Chain::WorldChain.evm_gas_buffer_bps(), 2000);
    }

    /// Exhaustive address validators independently check registry EVM membership.
    #[test]
    fn evm_validation_kind_agrees_with_is_evm() {
        for chain in Chain::all() {
            let kind = chain.address_validation_kind();
            let kind_says_evm = kind == "evm" || kind == "evmTestnet";
            assert_eq!(
                kind_says_evm,
                chain.is_evm(),
                "{} : is_evm()={} but address_validation_kind()={kind:?}",
                chain.str_id(),
                chain.is_evm(),
            );
            if chain.is_evm() {
                assert_eq!(
                    kind,
                    if chain.is_testnet() {
                        "evmTestnet"
                    } else {
                        "evm"
                    },
                    "{} has the wrong EVM flavour",
                    chain.str_id(),
                );
            }
        }
    }

    /// Every chain's kind must be one `validate_address` actually dispatches
    /// on. A kind it doesn't know falls through to `invalid_result()`, which
    /// silently rejects every address on that chain — the failure mode the
    /// old per-module copies of this table had.
    #[test]
    fn every_chain_has_a_kind_validate_address_recognises() {
        use crate::validation::address::{AddressValidationRequest, validate_address};
        for chain in Chain::all() {
            let kind = chain.address_validation_kind();
            assert!(!kind.is_empty(), "{} has an empty kind", chain.str_id());
            // A syntactically impossible address: a recognised kind still
            // reports `is_valid == false`, so this can't distinguish on its
            // own. What it does catch is a kind that panics or is blank.
            let result = validate_address(AddressValidationRequest {
                kind: kind.to_string(),
                value: "!".to_string(),
            });
            assert!(!result.is_valid, "{kind} accepted a bogus address");
        }
    }

    /// Address slots: every chain has one, and the EVM family shares Ethereum's.
    #[test]
    fn address_slots_are_shared_across_the_evm_family_only() {
        for chain in Chain::all() {
            let slot = chain.address_slot();
            assert!(!slot.is_empty(), "{} has no slot", chain.str_id());
            if chain.is_evm() {
                assert_eq!(
                    slot,
                    Chain::Ethereum.str_id(),
                    "{} should share the ethereum slot",
                    chain.str_id()
                );
            } else {
                assert_eq!(
                    slot,
                    chain.str_id(),
                    "{} should own its slot",
                    chain.str_id()
                );
            }
        }
    }

    /// Both networks can watch their validated addresses. Monero additionally
    /// requires a private view key, so an address is insufficient on either.
    #[test]
    fn watch_only_support_excludes_only_the_monero_family() {
        // Every account network reads account keys, each version spelling
        // its prefix and a test network's no mainnet's.
        for chain in Chain::all() {
            assert_eq!(
                chain.accepts_account_xpub(),
                chain.uses_account_utxo(),
                "{chain}"
            );
            for version in chain.account_key_versions() {
                let encoded = crate::derivation::bitcoin::base58check_encode(
                    &[version.version.as_slice(), &[0u8; 74]].concat(),
                );
                assert!(
                    encoded.starts_with(version.prefix),
                    "{chain} {}",
                    version.prefix
                );
                if chain.is_testnet() {
                    assert!(
                        !chain
                            .mainnet_counterpart()
                            .account_key_versions()
                            .contains(version),
                        "{chain} {}",
                        version.prefix
                    );
                }
            }
        }
        for chain in Chain::all() {
            assert!(
                chain.supports_watch_only_import()
                    == (chain.mainnet_counterpart() != Chain::Monero),
                "{} has inconsistent watch support",
                chain.str_id()
            );
        }
        let excluded: Vec<&str> = Chain::all()
            .filter(|c| !c.is_testnet() && !c.supports_watch_only_import())
            .map(|c| c.str_id())
            .collect();
        assert_eq!(excluded, vec!["monero"]);
    }

    /// Hosting follows each network’s protocol set, with no default protocol.
    #[test]
    fn token_hosting_follows_the_token_protocol_set() {
        let hosting: Vec<&str> = Chain::all()
            .filter(|c| c.hosts_tokens())
            .map(Chain::str_id)
            .collect();
        let with_standard: Vec<&str> = crate::chains::catalog()
            .iter()
            .filter(|c| !c.token_standards.is_empty())
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(hosting, with_standard);
        for chain in Chain::all().filter(|c| c.is_testnet()) {
            assert_eq!(
                chain.token_standards(),
                chain.mainnet_counterpart().token_standards(),
                "{}",
                chain.str_id()
            );
        }
        assert!(!Chain::Bitcoin.hosts_tokens() && !Chain::Monero.hosts_tokens());
        for chain in Chain::all().filter(|c| c.hosts_tokens()) {
            for standard in chain.token_standards() {
                assert!(chain.allows_token_standard(standard), "{}", chain.str_id());
            }
        }
    }

    #[test]
    fn protocol_sets_keep_legacy_and_current_protocols_on_one_network() {
        for chain in [Chain::Tron, Chain::TronNile] {
            assert_eq!(chain.token_standards(), ["TRC-10", "TRC-20"]);
            let entries = &chain.entry().token_standards;
            let prompts: Vec<(&str, &str)> = entries
                .iter()
                .map(|s| (s.standard.as_str(), s.identifier_prompt.as_str()))
                .collect();
            assert_eq!(
                prompts,
                [("TRC-10", "Token ID"), ("TRC-20", "Contract Address")]
            );
        }
        for chain in [Chain::Aptos, Chain::AptosTestnet] {
            assert_eq!(chain.token_standards(), ["Aptos Coin", "AIP-21"]);
            assert_eq!(
                chain.token_standard_for_identifier("0x1::coin::T"),
                "Aptos Coin"
            );
            assert_eq!(chain.token_standard_for_identifier("0x123"), "AIP-21");
        }
    }

    #[test]
    fn testnet_derivation_metadata_names_the_concrete_network() {
        for chain in Chain::all().filter(|c| c.is_testnet()) {
            assert_eq!(
                crate::send::flow::seed_derivation_chain_raw(chain).as_deref(),
                Some(chain.chain_display_name())
            );
        }
    }

    #[test]
    fn str_id_roundtrips() {
        for chain in Chain::all() {
            let id = chain.str_id();
            let back = Chain::from_str_id(id).expect("str_id must round-trip");
            assert_eq!(chain, back, "round-trip failed for {id}");
        }
        assert!(Chain::from_str_id("not-a-chain").is_none());
    }

    #[test]
    fn evm_group_includes_mainnets_and_testnets() {
        let mainnet_ids: Vec<&str> = vec![
            "ethereum",
            "arbitrum",
            "optimism",
            "avalanche",
            "base",
            "ethereum-classic",
            "bnb",
            "hyperliquid",
            "polygon",
            "linea",
            "scroll",
            "blast",
            "mantle",
            "sei",
            "celo",
            "cronos",
            "opbnb",
            "zksync-era",
            "sonic",
            "berachain",
            "unichain",
            "ink",
            "x-layer",
            "plasma",
            "monad",
            "world-chain",
        ];
        let testnet_ids: Vec<&str> = vec![
            "ethereum-sepolia",
            "ethereum-hoodi",
            "arbitrum-sepolia",
            "optimism-sepolia",
            "base-sepolia",
            "bnb-testnet",
            "avalanche-fuji",
            "polygon-amoy",
            "hyperliquid-testnet",
            "linea-sepolia",
            "celo-sepolia",
            "cronos-testnet",
            "zksync-era-sepolia",
            "sonic-testnet",
            "ink-sepolia",
            "x-layer-testnet",
            "ethereum-classic-mordor",
        ];
        let mut expected: Vec<&str> = [mainnet_ids, testnet_ids].concat();
        expected.sort();
        let mut actual: Vec<&str> = Chain::all()
            .filter(|c| c.is_evm())
            .map(|c| c.str_id())
            .collect();
        actual.sort();
        assert_eq!(actual, expected);
    }

    #[test]
    fn testnet_mainnet_counterparts_are_mainnets() {
        for testnet in Chain::all().filter(|c| c.is_testnet()) {
            let counterpart = testnet.mainnet_counterpart();
            assert!(
                !counterpart.is_testnet(),
                "{:?} mainnet_counterpart returned testnet {:?}",
                testnet,
                counterpart
            );
        }
    }
}

// ── FFI surface ──────────────────────────────────────────────────────────

/// What identifies one chain to a front end: the enum value and the three
/// facts every screen needs to go with it.
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

#[derive(Debug, Clone, uniffi::Record)]
pub struct ChainIdentity {
    pub chain: Chain,
    /// The catalog's `id` — what endpoint tables and the FFI boundary key on.
    pub id: String,
    /// The catalog's `name` — the one spelling of this chain.
    pub name: String,
    pub is_testnet: bool,
    pub is_evm: bool,
    /// Which chain's slot this chain's address is stored under. The EVM family
    /// shares Ethereum's.
    pub address_slot: String,
    /// A wallet on this chain is an account of many addresses
    /// (`Chain::uses_account_utxo`), which a rescan can walk.
    pub uses_account_utxo: bool,
    /// The prefixes a watched account public key starts with here
    /// (`Chain::account_key_prefixes`); empty where none is taken.
    pub account_key_prefixes: Vec<String>,
    /// The staking tab can query this chain's live validator directory.
    pub supports_staking: bool,
    /// The send screen has a network card to show for this chain — a fee, a
    /// preview, or both. False only where core routes no send at all.
    pub has_send_preview: bool,
    /// The chain can hold tracked tokens.
    pub hosts_tokens: bool,
    /// The ledger creates an account only once it holds the network's reserve.
    pub requires_account_reserve: bool,
    /// The mainnet this chain belongs to, or itself.
    pub mainnet_counterpart: Chain,
}

/// The whole catalog as identities, in declaration order.
///
/// One call rather than an accessor per column: a front end builds its lookups
/// from this once and then reads them locally, and there is no way to ask for
/// an id without the name that goes with it. `Chain` deliberately has no
/// `CaseIterable` on the Swift side — the order that matters is the catalog's.
#[uniffi::export]
pub fn chain_identities() -> Vec<ChainIdentity> {
    Chain::all()
        .map(|chain| ChainIdentity {
            chain,
            id: chain.str_id().to_string(),
            name: chain.chain_display_name().to_string(),
            is_testnet: chain.is_testnet(),
            is_evm: chain.is_evm(),
            address_slot: chain.address_slot().to_string(),
            uses_account_utxo: chain.uses_account_utxo(),
            account_key_prefixes: chain
                .account_key_prefixes()
                .into_iter()
                .map(str::to_string)
                .collect(),
            supports_staking: chain.supports_staking(),
            has_send_preview: chain.has_send_preview(),
            hosts_tokens: chain.hosts_tokens(),
            requires_account_reserve: chain.requires_account_reserve(),
            mainnet_counterpart: chain.mainnet_counterpart(),
        })
        .collect()
}

#[cfg(test)]
mod catalog_agreement_tests {
    use super::*;

    /// The genesis hashes an indexer is checked against are the networks'
    /// published ones.
    #[test]
    fn genesis_hashes_are_the_networks() {
        for (chain, hash) in [
            (
                Chain::Bitcoin,
                "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f",
            ),
            (
                Chain::BitcoinTestnet,
                "000000000933ea01ad0ee984209779baaec3ced90fa3f408719526f8d77f4943",
            ),
            (
                Chain::BitcoinSignet,
                "00000008819873e925422c1ff0f99f7cc9bbb232af63a077a480a3633bee1ef6",
            ),
            (
                Chain::Litecoin,
                "12a765e31ffd4059bada1e25190f6e98c99d9714d334efa41a195a7e7e04bfe2",
            ),
        ] {
            assert_eq!(chain.genesis_block_hash().unwrap(), hash, "{chain}");
        }
        assert!(Chain::Dogecoin.genesis_block_hash().is_err());
    }

    /// A chain id spelled from the variant's own name, so the check below has
    /// a source independent of the catalog it is checking.
    ///
    /// The six exceptions are the whole list of places where the enum and
    /// `chains.toml` spell a chain differently, which is worth being able to
    /// read in one place.
    fn expected_id(chain: Chain) -> String {
        const EXCEPTIONS: &[(Chain, &str)] = &[
            (Chain::Icp, "internet-computer"),
            (Chain::BnbChain, "bnb"),
            (Chain::OpBnb, "opbnb"),
            (Chain::ZkSyncEra, "zksync-era"),
            (Chain::ZkSyncEraSepolia, "zksync-era-sepolia"),
            (Chain::BitcoinTestnet4, "bitcoin-testnet-4"),
            (Chain::BnbChainTestnet, "bnb-testnet"),
        ];
        if let Some((_, id)) = EXCEPTIONS.iter().find(|(c, _)| *c == chain) {
            return (*id).to_string();
        }
        let name = format!("{chain:?}");
        let mut out = String::with_capacity(name.len() + 4);
        let bytes: Vec<char> = name.chars().collect();
        for (i, ch) in bytes.iter().enumerate() {
            let starts_word = ch.is_uppercase()
                && i > 0
                && (bytes[i - 1].is_lowercase()
                    || bytes[i - 1].is_ascii_digit()
                    || bytes.get(i + 1).is_some_and(|n| n.is_lowercase()));
            if starts_word {
                out.push('-');
            }
            out.extend(ch.to_lowercase());
        }
        out
    }

    /// The enum is an index into `chains.toml`, so the two orders must match
    /// exactly — position by position, not merely as sets.
    ///
    /// Asserting `chain.str_id() == entry.id` would prove nothing: `str_id`
    /// *reads* the catalog, so the two agree by construction. The variant
    /// name is the independent source, which is why [`expected_id`] exists.
    /// If the index is not sound, every chain silently becomes a different
    /// chain, and unlike a rename that is invisible from the outside.
    #[test]
    fn chain_order_matches_the_catalog() {
        let catalog = crate::chains::list_all_chains();
        assert_eq!(
            ALL_CHAINS.len(),
            catalog.len(),
            "{} chains in the enum, {} in chains.toml",
            ALL_CHAINS.len(),
            catalog.len()
        );
        for (index, entry) in catalog.iter().enumerate() {
            let chain = ALL_CHAINS[index];
            assert_eq!(
                chain as usize, index,
                "ALL_CHAINS[{index}] is {chain:?}, whose discriminant is {}",
                chain as usize
            );
            assert_eq!(
                expected_id(chain),
                entry.id,
                "position {index}: the enum has {chain:?}, chains.toml has \"{}\"",
                entry.id
            );
        }
    }

    /// Every catalog entry is reachable from the name it publishes, with no
    /// special case in the resolver.
    #[test]
    fn every_catalog_name_resolves() {
        for entry in crate::chains::list_all_chains() {
            assert_eq!(
                Chain::from_display_name(&entry.name).map(Chain::str_id),
                Some(entry.id.as_str()),
                "{} does not resolve from \"{}\"",
                entry.id,
                entry.name
            );
        }
    }
}

#[cfg(test)]
mod the_post_send_refresh_set_is_the_registrys {
    use super::{Chain, PendingStatusPoll};

    /// After a send, a chain either polls for a pending status or refreshes
    /// history. Which it does is `pending_status_poll`, and a testnet does what
    /// its mainnet does.
    #[test]
    fn every_utxo_testnet_polls_the_way_its_mainnet_does() {
        let utxo = |c: Chain| matches!(c.pending_status_poll(), PendingStatusPoll::Utxo { .. });
        assert!(utxo(Chain::Bitcoin));
        for chain in Chain::all() {
            assert_eq!(
                utxo(chain),
                utxo(chain.mainnet_counterpart()),
                "{chain:?} must poll like its mainnet"
            );
        }
    }
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
impl Chain {
    pub(crate) fn history_refresh_kind(self) -> HistoryRefreshKind {
        if self.is_evm() {
            HistoryRefreshKind::Evm
        } else if self.uses_account_utxo() {
            HistoryRefreshKind::Utxo
        } else {
            HistoryRefreshKind::Normalized
        }
    }
}

/// A test network's faucet page, or `None` on a mainnet and on a test
/// network without a working faucet.
#[uniffi::export]
pub fn chain_faucet_url(chain: Chain) -> Option<String> {
    chain.faucet_url().map(str::to_string)
}

#[cfg(test)]
mod zcash_schedule {
    use super::*;

    /// The branch at each upgrade's first block, NU6.3 (Ironwood) included,
    /// and nothing to build for before NU5.
    #[test]
    fn branches_follow_librustzcash() {
        for (chain, height, branch) in [
            (Chain::Zcash, 1_687_104, 0xc2d6_d0b4),
            (Chain::Zcash, 3_364_600, 0x5437_f330),
            (Chain::Zcash, 3_428_142, 0x5437_f330),
            (Chain::Zcash, 3_428_143, 0x37a5_165b),
            (Chain::ZcashTestnet, 4_134_000, 0x37a5_165b),
        ] {
            assert_eq!(
                chain.zcash_consensus_branch(height),
                Ok(branch),
                "{chain} {height}"
            );
        }
        assert_eq!(
            Chain::Zcash.zcash_consensus_branch(1_687_103),
            Err(RegistryError::ZcashV5Inactive(1_687_103))
        );
        assert!(Chain::Bitcoin.zcash_consensus_branch(1).is_err());
    }
}
