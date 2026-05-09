//! Central chain + token registry.
//!
//! The canonical `Chain` enum identifies chains by stable string ids
//! (e.g. `"bitcoin"`, `"ethereum"`). `Chain::str_id()` returns the id;
//! `Chain::from_str_id()` parses one back. The numeric discriminants were
//! removed in favour of string-keyed lookups throughout the codebase.

use crate::send::payload::SendChain;

/// Every chain Spectra knows about.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
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
    EthereumSepolia,
    EthereumHoodi,
    ArbitrumSepolia,
    OptimismSepolia,
    BaseSepolia,
    BnbChainTestnet,
    AvalancheFuji,
    PolygonAmoy,
    HyperliquidTestnet,
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

/// Which endpoint-list slot to fetch for a given chain.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EndpointSlot {
    Primary,
    Secondary,
    Explorer,
}

// All variants in stable order. Used by Chain::all().
const ALL_CHAINS: &[Chain] = &[
    Chain::Bitcoin, Chain::Ethereum, Chain::Solana, Chain::Dogecoin,
    Chain::Xrp, Chain::Litecoin, Chain::BitcoinCash, Chain::Tron,
    Chain::Stellar, Chain::Cardano, Chain::Polkadot, Chain::Arbitrum,
    Chain::Optimism, Chain::Avalanche, Chain::Sui, Chain::Aptos,
    Chain::Ton, Chain::Near, Chain::Icp, Chain::Monero,
    Chain::Base, Chain::EthereumClassic, Chain::BitcoinSV, Chain::BnbChain,
    Chain::Hyperliquid, Chain::Polygon, Chain::Linea, Chain::Scroll,
    Chain::Blast, Chain::Mantle, Chain::Zcash, Chain::BitcoinGold,
    Chain::Decred, Chain::Kaspa, Chain::Sei, Chain::Celo,
    Chain::Cronos, Chain::OpBnb, Chain::ZkSyncEra, Chain::Sonic,
    Chain::Berachain, Chain::Unichain, Chain::Ink, Chain::Dash,
    Chain::XLayer, Chain::Bittensor,
    // Testnets
    Chain::BitcoinTestnet, Chain::BitcoinTestnet4, Chain::BitcoinSignet,
    Chain::LitecoinTestnet, Chain::BitcoinCashTestnet, Chain::BitcoinSVTestnet,
    Chain::DogecoinTestnet, Chain::ZcashTestnet, Chain::DecredTestnet,
    Chain::KaspaTestnet, Chain::DashTestnet, Chain::EthereumSepolia,
    Chain::EthereumHoodi, Chain::ArbitrumSepolia, Chain::OptimismSepolia,
    Chain::BaseSepolia, Chain::BnbChainTestnet, Chain::AvalancheFuji,
    Chain::PolygonAmoy, Chain::HyperliquidTestnet, Chain::EthereumClassicMordor,
    Chain::TronNile, Chain::SolanaDevnet, Chain::XrpTestnet,
    Chain::StellarTestnet, Chain::CardanoPreprod, Chain::SuiTestnet,
    Chain::AptosTestnet, Chain::TonTestnet, Chain::NearTestnet,
    Chain::PolkadotWestend, Chain::MoneroStagenet,
];

impl Chain {
    /// Stable string id matching `chains.toml` `id` field.
    pub const fn str_id(self) -> &'static str {
        match self {
            Chain::Bitcoin => "bitcoin",
            Chain::Ethereum => "ethereum",
            Chain::Solana => "solana",
            Chain::Dogecoin => "dogecoin",
            Chain::Xrp => "xrp",
            Chain::Litecoin => "litecoin",
            Chain::BitcoinCash => "bitcoin-cash",
            Chain::Tron => "tron",
            Chain::Stellar => "stellar",
            Chain::Cardano => "cardano",
            Chain::Polkadot => "polkadot",
            Chain::Arbitrum => "arbitrum",
            Chain::Optimism => "optimism",
            Chain::Avalanche => "avalanche",
            Chain::Sui => "sui",
            Chain::Aptos => "aptos",
            Chain::Ton => "ton",
            Chain::Near => "near",
            Chain::Icp => "internet-computer",
            Chain::Monero => "monero",
            Chain::Base => "base",
            Chain::EthereumClassic => "ethereum-classic",
            Chain::BitcoinSV => "bitcoin-sv",
            Chain::BnbChain => "bnb",
            Chain::Hyperliquid => "hyperliquid",
            Chain::Polygon => "polygon",
            Chain::Linea => "linea",
            Chain::Scroll => "scroll",
            Chain::Blast => "blast",
            Chain::Mantle => "mantle",
            Chain::Zcash => "zcash",
            Chain::BitcoinGold => "bitcoin-gold",
            Chain::Decred => "decred",
            Chain::Kaspa => "kaspa",
            Chain::Sei => "sei",
            Chain::Celo => "celo",
            Chain::Cronos => "cronos",
            Chain::OpBnb => "opbnb",
            Chain::ZkSyncEra => "zksync-era",
            Chain::Sonic => "sonic",
            Chain::Berachain => "berachain",
            Chain::Unichain => "unichain",
            Chain::Ink => "ink",
            Chain::Dash => "dash",
            Chain::XLayer => "x-layer",
            Chain::Bittensor => "bittensor",
            Chain::BitcoinTestnet => "bitcoin-testnet",
            Chain::BitcoinTestnet4 => "bitcoin-testnet-4",
            Chain::BitcoinSignet => "bitcoin-signet",
            Chain::LitecoinTestnet => "litecoin-testnet",
            Chain::BitcoinCashTestnet => "bitcoin-cash-testnet",
            Chain::BitcoinSVTestnet => "bitcoin-sv-testnet",
            Chain::DogecoinTestnet => "dogecoin-testnet",
            Chain::ZcashTestnet => "zcash-testnet",
            Chain::DecredTestnet => "decred-testnet",
            Chain::KaspaTestnet => "kaspa-testnet",
            Chain::DashTestnet => "dash-testnet",
            Chain::EthereumSepolia => "ethereum-sepolia",
            Chain::EthereumHoodi => "ethereum-hoodi",
            Chain::ArbitrumSepolia => "arbitrum-sepolia",
            Chain::OptimismSepolia => "optimism-sepolia",
            Chain::BaseSepolia => "base-sepolia",
            Chain::BnbChainTestnet => "bnb-testnet",
            Chain::AvalancheFuji => "avalanche-fuji",
            Chain::PolygonAmoy => "polygon-amoy",
            Chain::HyperliquidTestnet => "hyperliquid-testnet",
            Chain::EthereumClassicMordor => "ethereum-classic-mordor",
            Chain::TronNile => "tron-nile",
            Chain::SolanaDevnet => "solana-devnet",
            Chain::XrpTestnet => "xrp-testnet",
            Chain::StellarTestnet => "stellar-testnet",
            Chain::CardanoPreprod => "cardano-preprod",
            Chain::SuiTestnet => "sui-testnet",
            Chain::AptosTestnet => "aptos-testnet",
            Chain::TonTestnet => "ton-testnet",
            Chain::NearTestnet => "near-testnet",
            Chain::PolkadotWestend => "polkadot-westend",
            Chain::MoneroStagenet => "monero-stagenet",
        }
    }

    /// Parse a string id (from `chains.toml` or the FFI boundary) into a `Chain`.
    pub fn from_str_id(id: &str) -> Option<Self> {
        Some(match id {
            "bitcoin" => Chain::Bitcoin,
            "ethereum" => Chain::Ethereum,
            "solana" => Chain::Solana,
            "dogecoin" => Chain::Dogecoin,
            "xrp" => Chain::Xrp,
            "litecoin" => Chain::Litecoin,
            "bitcoin-cash" => Chain::BitcoinCash,
            "tron" => Chain::Tron,
            "stellar" => Chain::Stellar,
            "cardano" => Chain::Cardano,
            "polkadot" => Chain::Polkadot,
            "arbitrum" => Chain::Arbitrum,
            "optimism" => Chain::Optimism,
            "avalanche" => Chain::Avalanche,
            "sui" => Chain::Sui,
            "aptos" => Chain::Aptos,
            "ton" => Chain::Ton,
            "near" => Chain::Near,
            "internet-computer" => Chain::Icp,
            "monero" => Chain::Monero,
            "base" => Chain::Base,
            "ethereum-classic" => Chain::EthereumClassic,
            "bitcoin-sv" => Chain::BitcoinSV,
            "bnb" => Chain::BnbChain,
            "hyperliquid" => Chain::Hyperliquid,
            "polygon" => Chain::Polygon,
            "linea" => Chain::Linea,
            "scroll" => Chain::Scroll,
            "blast" => Chain::Blast,
            "mantle" => Chain::Mantle,
            "zcash" => Chain::Zcash,
            "bitcoin-gold" => Chain::BitcoinGold,
            "decred" => Chain::Decred,
            "kaspa" => Chain::Kaspa,
            "sei" => Chain::Sei,
            "celo" => Chain::Celo,
            "cronos" => Chain::Cronos,
            "opbnb" => Chain::OpBnb,
            "zksync-era" => Chain::ZkSyncEra,
            "sonic" => Chain::Sonic,
            "berachain" => Chain::Berachain,
            "unichain" => Chain::Unichain,
            "ink" => Chain::Ink,
            "dash" => Chain::Dash,
            "x-layer" => Chain::XLayer,
            "bittensor" => Chain::Bittensor,
            "bitcoin-testnet" => Chain::BitcoinTestnet,
            "bitcoin-testnet-4" => Chain::BitcoinTestnet4,
            "bitcoin-signet" => Chain::BitcoinSignet,
            "litecoin-testnet" => Chain::LitecoinTestnet,
            "bitcoin-cash-testnet" => Chain::BitcoinCashTestnet,
            "bitcoin-sv-testnet" => Chain::BitcoinSVTestnet,
            "dogecoin-testnet" => Chain::DogecoinTestnet,
            "zcash-testnet" => Chain::ZcashTestnet,
            "decred-testnet" => Chain::DecredTestnet,
            "kaspa-testnet" => Chain::KaspaTestnet,
            "dash-testnet" => Chain::DashTestnet,
            "ethereum-sepolia" => Chain::EthereumSepolia,
            "ethereum-hoodi" => Chain::EthereumHoodi,
            "arbitrum-sepolia" => Chain::ArbitrumSepolia,
            "optimism-sepolia" => Chain::OptimismSepolia,
            "base-sepolia" => Chain::BaseSepolia,
            "bnb-testnet" => Chain::BnbChainTestnet,
            "avalanche-fuji" => Chain::AvalancheFuji,
            "polygon-amoy" => Chain::PolygonAmoy,
            "hyperliquid-testnet" => Chain::HyperliquidTestnet,
            "ethereum-classic-mordor" => Chain::EthereumClassicMordor,
            "tron-nile" => Chain::TronNile,
            "solana-devnet" => Chain::SolanaDevnet,
            "xrp-testnet" => Chain::XrpTestnet,
            "stellar-testnet" => Chain::StellarTestnet,
            "cardano-preprod" => Chain::CardanoPreprod,
            "sui-testnet" => Chain::SuiTestnet,
            "aptos-testnet" => Chain::AptosTestnet,
            "ton-testnet" => Chain::TonTestnet,
            "near-testnet" => Chain::NearTestnet,
            "polkadot-westend" => Chain::PolkadotWestend,
            "monero-stagenet" => Chain::MoneroStagenet,
            _ => return None,
        })
    }

    /// Returns `true` for chains that are testnets.
    pub const fn is_testnet(self) -> bool {
        matches!(
            self,
            Chain::BitcoinTestnet
                | Chain::BitcoinTestnet4
                | Chain::BitcoinSignet
                | Chain::LitecoinTestnet
                | Chain::BitcoinCashTestnet
                | Chain::BitcoinSVTestnet
                | Chain::DogecoinTestnet
                | Chain::ZcashTestnet
                | Chain::DecredTestnet
                | Chain::KaspaTestnet
                | Chain::DashTestnet
                | Chain::EthereumSepolia
                | Chain::EthereumHoodi
                | Chain::ArbitrumSepolia
                | Chain::OptimismSepolia
                | Chain::BaseSepolia
                | Chain::BnbChainTestnet
                | Chain::AvalancheFuji
                | Chain::PolygonAmoy
                | Chain::HyperliquidTestnet
                | Chain::EthereumClassicMordor
                | Chain::TronNile
                | Chain::SolanaDevnet
                | Chain::XrpTestnet
                | Chain::StellarTestnet
                | Chain::CardanoPreprod
                | Chain::SuiTestnet
                | Chain::AptosTestnet
                | Chain::TonTestnet
                | Chain::NearTestnet
                | Chain::PolkadotWestend
                | Chain::MoneroStagenet
        )
    }

    /// Maps a testnet variant to its mainnet counterpart. Returns `self` for mainnets.
    pub const fn mainnet_counterpart(self) -> Chain {
        match self {
            Chain::BitcoinTestnet | Chain::BitcoinTestnet4 | Chain::BitcoinSignet => Chain::Bitcoin,
            Chain::LitecoinTestnet => Chain::Litecoin,
            Chain::BitcoinCashTestnet => Chain::BitcoinCash,
            Chain::BitcoinSVTestnet => Chain::BitcoinSV,
            Chain::DogecoinTestnet => Chain::Dogecoin,
            Chain::ZcashTestnet => Chain::Zcash,
            Chain::DecredTestnet => Chain::Decred,
            Chain::KaspaTestnet => Chain::Kaspa,
            Chain::DashTestnet => Chain::Dash,
            Chain::EthereumSepolia | Chain::EthereumHoodi => Chain::Ethereum,
            Chain::ArbitrumSepolia => Chain::Arbitrum,
            Chain::OptimismSepolia => Chain::Optimism,
            Chain::BaseSepolia => Chain::Base,
            Chain::BnbChainTestnet => Chain::BnbChain,
            Chain::AvalancheFuji => Chain::Avalanche,
            Chain::PolygonAmoy => Chain::Polygon,
            Chain::HyperliquidTestnet => Chain::Hyperliquid,
            Chain::EthereumClassicMordor => Chain::EthereumClassic,
            Chain::TronNile => Chain::Tron,
            Chain::SolanaDevnet => Chain::Solana,
            Chain::XrpTestnet => Chain::Xrp,
            Chain::StellarTestnet => Chain::Stellar,
            Chain::CardanoPreprod => Chain::Cardano,
            Chain::SuiTestnet => Chain::Sui,
            Chain::AptosTestnet => Chain::Aptos,
            Chain::TonTestnet => Chain::Ton,
            Chain::NearTestnet => Chain::Near,
            Chain::PolkadotWestend => Chain::Polkadot,
            Chain::MoneroStagenet => Chain::Monero,
            _ => self,
        }
    }

    /// View the chain as an `EvmChain` if it's EVM-family.
    pub const fn as_evm(self) -> Option<EvmChain> {
        if self.is_evm() { Some(EvmChain(self)) } else { None }
    }

    /// `true` for every EVM-compatible chain (mainnet or testnet).
    pub const fn is_evm(self) -> bool {
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
                | Chain::EthereumSepolia
                | Chain::EthereumHoodi
                | Chain::ArbitrumSepolia
                | Chain::OptimismSepolia
                | Chain::BaseSepolia
                | Chain::BnbChainTestnet
                | Chain::AvalancheFuji
                | Chain::PolygonAmoy
                | Chain::HyperliquidTestnet
                | Chain::EthereumClassicMordor
        )
    }

    /// EIP-155 chain id. Non-EVM chains return `1` (legacy fallback).
    pub const fn evm_chain_id(self) -> u64 {
        match self {
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
            Chain::EthereumSepolia => 11155111,
            Chain::EthereumHoodi => 560048,
            Chain::ArbitrumSepolia => 421614,
            Chain::OptimismSepolia => 11155420,
            Chain::BaseSepolia => 84532,
            Chain::BnbChainTestnet => 97,
            Chain::AvalancheFuji => 43113,
            Chain::PolygonAmoy => 80002,
            Chain::HyperliquidTestnet => 998,
            Chain::EthereumClassicMordor => 63,
            _ => 1,
        }
    }

    /// Map to the `SendChain` discriminant used by send-payload classification.
    pub const fn send_chain(self) -> SendChain {
        match self {
            Chain::Bitcoin => SendChain::Bitcoin,
            Chain::BitcoinCash => SendChain::BitcoinCash,
            Chain::BitcoinSV => SendChain::BitcoinSV,
            Chain::Litecoin => SendChain::Litecoin,
            Chain::Dogecoin => SendChain::Dogecoin,
            Chain::Zcash => SendChain::Zcash,
            Chain::BitcoinGold => SendChain::BitcoinGold,
            Chain::Decred => SendChain::Decred,
            Chain::Kaspa => SendChain::Kaspa,
            Chain::Dash => SendChain::Dash,
            Chain::Bittensor => SendChain::Bittensor,
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
            | Chain::XLayer => SendChain::Ethereum,
            Chain::Tron => SendChain::Tron,
            Chain::Solana => SendChain::Solana,
            Chain::Xrp => SendChain::Xrp,
            Chain::Stellar => SendChain::Stellar,
            Chain::Monero => SendChain::Monero,
            Chain::Cardano => SendChain::Cardano,
            Chain::Sui => SendChain::Sui,
            Chain::Aptos => SendChain::Aptos,
            Chain::Ton => SendChain::Ton,
            Chain::Icp => SendChain::Icp,
            Chain::Near => SendChain::Near,
            Chain::Polkadot => SendChain::Polkadot,
            Chain::BitcoinTestnet | Chain::BitcoinTestnet4 | Chain::BitcoinSignet => SendChain::Bitcoin,
            Chain::LitecoinTestnet => SendChain::Litecoin,
            Chain::BitcoinCashTestnet => SendChain::BitcoinCash,
            Chain::BitcoinSVTestnet => SendChain::BitcoinSV,
            Chain::DogecoinTestnet => SendChain::Dogecoin,
            Chain::ZcashTestnet => SendChain::Zcash,
            Chain::DecredTestnet => SendChain::Decred,
            Chain::KaspaTestnet => SendChain::Kaspa,
            Chain::DashTestnet => SendChain::Dash,
            Chain::EthereumSepolia
            | Chain::EthereumHoodi
            | Chain::ArbitrumSepolia
            | Chain::OptimismSepolia
            | Chain::BaseSepolia
            | Chain::BnbChainTestnet
            | Chain::AvalancheFuji
            | Chain::PolygonAmoy
            | Chain::HyperliquidTestnet
            | Chain::EthereumClassicMordor => SendChain::Ethereum,
            Chain::TronNile => SendChain::Tron,
            Chain::SolanaDevnet => SendChain::Solana,
            Chain::XrpTestnet => SendChain::Xrp,
            Chain::StellarTestnet => SendChain::Stellar,
            Chain::CardanoPreprod => SendChain::Cardano,
            Chain::SuiTestnet => SendChain::Sui,
            Chain::AptosTestnet => SendChain::Aptos,
            Chain::TonTestnet => SendChain::Ton,
            Chain::NearTestnet => SendChain::Near,
            Chain::PolkadotWestend => SendChain::Polkadot,
            Chain::MoneroStagenet => SendChain::Monero,
        }
    }

    /// Endpoint-table key for a given logical slot.
    /// Primary → chain str_id; Secondary → "id:secondary"; Explorer → "id:explorer".
    pub fn endpoint_str_id(self, slot: EndpointSlot) -> String {
        match slot {
            EndpointSlot::Primary => self.str_id().to_string(),
            EndpointSlot::Secondary => format!("{}:secondary", self.str_id()),
            EndpointSlot::Explorer => format!("{}:explorer", self.str_id()),
        }
    }

    // ----------------------------------------------------------------
    // Native-coin metadata
    // ----------------------------------------------------------------

    pub fn coin_name(self) -> &'static str {
        crate::chains::chain_by_str_id(self.str_id())
            .map(|c| c.native_asset_name.as_str())
            .unwrap_or("")
    }

    pub const fn coin_symbol(self) -> &'static str {
        match self {
            Chain::Bitcoin => "BTC",
            Chain::Ethereum
            | Chain::Arbitrum
            | Chain::Optimism
            | Chain::Base
            | Chain::Linea
            | Chain::Scroll
            | Chain::Blast => "ETH",
            Chain::Solana => "SOL",
            Chain::Dogecoin => "DOGE",
            Chain::Xrp => "XRP",
            Chain::Litecoin => "LTC",
            Chain::BitcoinCash => "BCH",
            Chain::Tron => "TRX",
            Chain::Stellar => "XLM",
            Chain::Cardano => "ADA",
            Chain::Polkadot => "DOT",
            Chain::Avalanche => "AVAX",
            Chain::Sui => "SUI",
            Chain::Aptos => "APT",
            Chain::Ton => "TON",
            Chain::Near => "NEAR",
            Chain::Icp => "ICP",
            Chain::Monero => "XMR",
            Chain::EthereumClassic => "ETC",
            Chain::BitcoinSV => "BSV",
            Chain::BnbChain => "BNB",
            Chain::Hyperliquid => "HYPE",
            Chain::Polygon => "POL",
            Chain::Mantle => "MNT",
            Chain::Zcash => "ZEC",
            Chain::BitcoinGold => "BTG",
            Chain::Decred => "DCR",
            Chain::Kaspa => "KAS",
            Chain::Sei => "SEI",
            Chain::Celo => "CELO",
            Chain::Cronos => "CRO",
            Chain::OpBnb => "BNB",
            Chain::ZkSyncEra | Chain::Unichain | Chain::Ink => "ETH",
            Chain::Sonic => "S",
            Chain::Berachain => "BERA",
            Chain::Dash => "DASH",
            Chain::XLayer => "OKB",
            Chain::Bittensor => "TAO",
            Chain::BitcoinTestnet | Chain::BitcoinTestnet4 | Chain::BitcoinSignet => "BTC",
            Chain::LitecoinTestnet => "LTC",
            Chain::BitcoinCashTestnet => "BCH",
            Chain::BitcoinSVTestnet => "BSV",
            Chain::DogecoinTestnet => "DOGE",
            Chain::ZcashTestnet => "ZEC",
            Chain::DecredTestnet => "DCR",
            Chain::KaspaTestnet => "KAS",
            Chain::DashTestnet => "DASH",
            Chain::EthereumSepolia
            | Chain::EthereumHoodi
            | Chain::ArbitrumSepolia
            | Chain::OptimismSepolia
            | Chain::BaseSepolia => "ETH",
            Chain::BnbChainTestnet => "BNB",
            Chain::AvalancheFuji => "AVAX",
            Chain::PolygonAmoy => "POL",
            Chain::HyperliquidTestnet => "HYPE",
            Chain::EthereumClassicMordor => "ETC",
            Chain::TronNile => "TRX",
            Chain::SolanaDevnet => "SOL",
            Chain::XrpTestnet => "XRP",
            Chain::StellarTestnet => "XLM",
            Chain::CardanoPreprod => "ADA",
            Chain::SuiTestnet => "SUI",
            Chain::AptosTestnet => "APT",
            Chain::TonTestnet => "TON",
            Chain::NearTestnet => "NEAR",
            Chain::PolkadotWestend => "DOT",
            Chain::MoneroStagenet => "XMR",
        }
    }

    pub const fn chain_display_name(self) -> &'static str {
        match self {
            Chain::Bitcoin => "Bitcoin",
            Chain::Ethereum => "Ethereum",
            Chain::Solana => "Solana",
            Chain::Dogecoin => "Dogecoin",
            Chain::Xrp => "XRP Ledger",
            Chain::Litecoin => "Litecoin",
            Chain::BitcoinCash => "Bitcoin Cash",
            Chain::Tron => "Tron",
            Chain::Stellar => "Stellar",
            Chain::Cardano => "Cardano",
            Chain::Polkadot => "Polkadot",
            Chain::Arbitrum => "Arbitrum",
            Chain::Optimism => "Optimism",
            Chain::Avalanche => "Avalanche",
            Chain::Sui => "Sui",
            Chain::Aptos => "Aptos",
            Chain::Ton => "TON",
            Chain::Near => "NEAR",
            Chain::Icp => "ICP",
            Chain::Monero => "Monero",
            Chain::Base => "Base",
            Chain::EthereumClassic => "Ethereum Classic",
            Chain::BitcoinSV => "Bitcoin SV",
            Chain::BnbChain => "BNB Chain",
            Chain::Hyperliquid => "Hyperliquid",
            Chain::Polygon => "Polygon",
            Chain::Linea => "Linea",
            Chain::Scroll => "Scroll",
            Chain::Blast => "Blast",
            Chain::Mantle => "Mantle",
            Chain::Zcash => "Zcash",
            Chain::BitcoinGold => "Bitcoin Gold",
            Chain::Decred => "Decred",
            Chain::Kaspa => "Kaspa",
            Chain::Sei => "Sei",
            Chain::Celo => "Celo",
            Chain::Cronos => "Cronos",
            Chain::OpBnb => "opBNB",
            Chain::ZkSyncEra => "zkSync Era",
            Chain::Sonic => "Sonic",
            Chain::Berachain => "Berachain",
            Chain::Unichain => "Unichain",
            Chain::Ink => "Ink",
            Chain::Dash => "Dash",
            Chain::XLayer => "X Layer",
            Chain::Bittensor => "Bittensor",
            Chain::BitcoinTestnet => "Bitcoin Testnet",
            Chain::BitcoinTestnet4 => "Bitcoin Testnet4",
            Chain::BitcoinSignet => "Bitcoin Signet",
            Chain::LitecoinTestnet => "Litecoin Testnet",
            Chain::BitcoinCashTestnet => "Bitcoin Cash Testnet",
            Chain::BitcoinSVTestnet => "Bitcoin SV Testnet",
            Chain::DogecoinTestnet => "Dogecoin Testnet",
            Chain::ZcashTestnet => "Zcash Testnet",
            Chain::DecredTestnet => "Decred Testnet",
            Chain::KaspaTestnet => "Kaspa Testnet",
            Chain::DashTestnet => "Dash Testnet",
            Chain::EthereumSepolia => "Ethereum Sepolia",
            Chain::EthereumHoodi => "Ethereum Hoodi",
            Chain::ArbitrumSepolia => "Arbitrum Sepolia",
            Chain::OptimismSepolia => "Optimism Sepolia",
            Chain::BaseSepolia => "Base Sepolia",
            Chain::BnbChainTestnet => "BNB Chain Testnet",
            Chain::AvalancheFuji => "Avalanche Fuji",
            Chain::PolygonAmoy => "Polygon Amoy",
            Chain::HyperliquidTestnet => "Hyperliquid Testnet",
            Chain::EthereumClassicMordor => "Ethereum Classic Mordor",
            Chain::TronNile => "Tron Nile",
            Chain::SolanaDevnet => "Solana Devnet",
            Chain::XrpTestnet => "XRP Ledger Testnet",
            Chain::StellarTestnet => "Stellar Testnet",
            Chain::CardanoPreprod => "Cardano Preprod",
            Chain::SuiTestnet => "Sui Testnet",
            Chain::AptosTestnet => "Aptos Testnet",
            Chain::TonTestnet => "TON Testnet",
            Chain::NearTestnet => "NEAR Testnet",
            Chain::PolkadotWestend => "Polkadot Westend",
            Chain::MoneroStagenet => "Monero Stagenet",
        }
    }

    pub fn native_decimals(self) -> u8 {
        crate::chains::chain_by_str_id(self.str_id())
            .map(|c| c.native_decimals as u8)
            .unwrap_or(18)
    }

    pub fn coin_gecko_id(self) -> &'static str {
        crate::chains::chain_by_str_id(self.str_id())
            .map(|c| c.native_coingecko_id.as_str())
            .unwrap_or("")
    }

    pub const fn native_balance_field(self) -> Option<&'static str> {
        Some(match self {
            Chain::Bitcoin => "confirmed_sats",
            Chain::Solana => "lamports",
            Chain::Dogecoin => "balance_koin",
            Chain::Xrp => "drops",
            Chain::Litecoin
            | Chain::BitcoinCash
            | Chain::BitcoinSV
            | Chain::Zcash
            | Chain::BitcoinGold
            | Chain::Dash => "balance_sat",
            Chain::Decred => "balance_atoms",
            Chain::Kaspa => "balance_sompi",
            Chain::Tron => "sun",
            Chain::Stellar => "stroops",
            Chain::Cardano => "lovelace",
            Chain::Polkadot => "planck",
            Chain::Bittensor => "rao",
            Chain::Sui => "mist",
            Chain::Aptos => "octas",
            Chain::Ton => "nanotons",
            Chain::Icp => "e8s",
            Chain::Monero => "piconeros",
            Chain::BitcoinTestnet | Chain::BitcoinTestnet4 | Chain::BitcoinSignet => "confirmed_sats",
            Chain::SolanaDevnet => "lamports",
            Chain::DogecoinTestnet => "balance_koin",
            Chain::XrpTestnet => "drops",
            Chain::LitecoinTestnet
            | Chain::BitcoinCashTestnet
            | Chain::BitcoinSVTestnet
            | Chain::ZcashTestnet
            | Chain::DashTestnet => "balance_sat",
            Chain::DecredTestnet => "balance_atoms",
            Chain::KaspaTestnet => "balance_sompi",
            Chain::TronNile => "sun",
            Chain::StellarTestnet => "stroops",
            Chain::CardanoPreprod => "lovelace",
            Chain::PolkadotWestend => "planck",
            Chain::SuiTestnet => "mist",
            Chain::AptosTestnet => "octas",
            Chain::TonTestnet => "nanotons",
            Chain::MoneroStagenet => "piconeros",
            _ => return None,
        })
    }

    pub const fn supports_deep_utxo_discovery(self) -> bool {
        matches!(
            self,
            Chain::Bitcoin
                | Chain::BitcoinCash
                | Chain::BitcoinSV
                | Chain::Litecoin
                | Chain::Dogecoin
                | Chain::BitcoinTestnet
                | Chain::BitcoinTestnet4
                | Chain::BitcoinSignet
                | Chain::BitcoinCashTestnet
                | Chain::BitcoinSVTestnet
                | Chain::LitecoinTestnet
                | Chain::DogecoinTestnet
        )
    }

    pub const fn flags_evm_address_as_wrong_chain(self) -> bool {
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
        )
    }

    pub const fn static_fee_units(self) -> Option<u128> {
        match self {
            Chain::Solana => Some(5_000),
            Chain::Tron => Some(1_000_000),
            Chain::Cardano => Some(170_000),
            Chain::Polkadot => Some(160_000_000),
            Chain::Bittensor => Some(125_000),
            Chain::Sui => Some(1_000),
            Chain::Ton => Some(7_000_000),
            Chain::Icp => Some(10_000),
            Chain::Monero => Some(500_000_000),
            Chain::Dogecoin => Some(1_000_000),
            Chain::Litecoin | Chain::Zcash | Chain::BitcoinSV | Chain::BitcoinGold | Chain::Kaspa => Some(1_000),
            Chain::BitcoinCash | Chain::Decred | Chain::Dash => Some(2_000),
            Chain::SolanaDevnet => Some(5_000),
            Chain::TronNile => Some(1_000_000),
            Chain::CardanoPreprod => Some(170_000),
            Chain::PolkadotWestend => Some(160_000_000),
            Chain::SuiTestnet => Some(1_000),
            Chain::TonTestnet => Some(7_000_000),
            Chain::MoneroStagenet => Some(500_000_000),
            Chain::DogecoinTestnet => Some(1_000_000),
            Chain::LitecoinTestnet
            | Chain::ZcashTestnet
            | Chain::BitcoinSVTestnet
            | Chain::KaspaTestnet => Some(1_000),
            Chain::BitcoinCashTestnet | Chain::DecredTestnet | Chain::DashTestnet => Some(2_000),
            Chain::Bitcoin
            | Chain::Xrp
            | Chain::Stellar
            | Chain::Aptos => None,
            Chain::BitcoinTestnet
            | Chain::BitcoinTestnet4
            | Chain::BitcoinSignet
            | Chain::XrpTestnet
            | Chain::StellarTestnet
            | Chain::AptosTestnet => None,
            Chain::Ethereum | Chain::Arbitrum | Chain::Optimism | Chain::Avalanche
            | Chain::Base | Chain::EthereumClassic | Chain::BnbChain | Chain::Hyperliquid
            | Chain::Polygon | Chain::Linea | Chain::Scroll | Chain::Blast | Chain::Mantle
            | Chain::Sei | Chain::Celo | Chain::Cronos | Chain::OpBnb | Chain::ZkSyncEra
            | Chain::Sonic | Chain::Berachain | Chain::Unichain | Chain::Ink | Chain::XLayer => None,
            Chain::EthereumSepolia
            | Chain::EthereumHoodi
            | Chain::ArbitrumSepolia
            | Chain::OptimismSepolia
            | Chain::BaseSepolia
            | Chain::BnbChainTestnet
            | Chain::AvalancheFuji
            | Chain::PolygonAmoy
            | Chain::HyperliquidTestnet
            | Chain::EthereumClassicMordor => None,
            Chain::Near => None,
            Chain::NearTestnet => None,
        }
    }

    /// Iterator over every known chain.
    pub fn all() -> impl Iterator<Item = Self> {
        ALL_CHAINS.iter().copied()
    }

    /// Iterator over only mainnet chains.
    pub fn mainnets() -> impl Iterator<Item = Self> {
        Self::all().filter(|c| !c.is_testnet())
    }

    /// Iterator over only testnet chains.
    pub fn testnets() -> impl Iterator<Item = Self> {
        Self::all().filter(|c| c.is_testnet())
    }

    /// Resolve a chain from the display name Swift uses on the boundary.
    pub fn from_display_name(name: &str) -> Option<Self> {
        if name == "Internet Computer" {
            return Some(Chain::Icp);
        }
        Chain::all().find(|c| c.chain_display_name() == name)
    }
}

/// Newtype wrapper that proves the inner `Chain` is EVM-family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvmChain(Chain);

impl EvmChain {
    pub const fn chain(self) -> Chain { self.0 }
    pub const fn chain_id(self) -> u64 { self.0.evm_chain_id() }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn all_chain_count() {
        assert_eq!(Chain::all().count(), 78);
    }

    #[test]
    fn evm_group_includes_mainnets_and_testnets() {
        let mainnet_ids: Vec<&str> = vec![
            "ethereum", "arbitrum", "optimism", "avalanche", "base",
            "ethereum-classic", "bnb", "hyperliquid", "polygon", "linea",
            "scroll", "blast", "mantle", "sei", "celo", "cronos", "opbnb",
            "zksync-era", "sonic", "berachain", "unichain", "ink", "x-layer",
        ];
        let testnet_ids: Vec<&str> = vec![
            "ethereum-sepolia", "ethereum-hoodi", "arbitrum-sepolia",
            "optimism-sepolia", "base-sepolia", "bnb-testnet", "avalanche-fuji",
            "polygon-amoy", "hyperliquid-testnet", "ethereum-classic-mordor",
        ];
        let mut expected: Vec<&str> = [mainnet_ids, testnet_ids].concat();
        expected.sort();
        let mut actual: Vec<&str> = Chain::all().filter(|c| c.is_evm()).map(|c| c.str_id()).collect();
        actual.sort();
        assert_eq!(actual, expected);
    }

    #[test]
    fn testnet_counts_match_total() {
        let total = Chain::all().count();
        let testnets = Chain::testnets().count();
        let mainnets = Chain::mainnets().count();
        assert_eq!(testnets + mainnets, total);
        assert_eq!(testnets, 32);
    }

    #[test]
    fn testnet_mainnet_counterparts_are_mainnets() {
        for testnet in Chain::testnets() {
            let counterpart = testnet.mainnet_counterpart();
            assert!(!counterpart.is_testnet(), "{:?} mainnet_counterpart returned testnet {:?}", testnet, counterpart);
        }
    }

    #[test]
    fn evm_chain_ids_match_legacy_table() {
        assert_eq!(Chain::Ethereum.evm_chain_id(), 1);
        assert_eq!(Chain::Arbitrum.evm_chain_id(), 42161);
        assert_eq!(Chain::Optimism.evm_chain_id(), 10);
        assert_eq!(Chain::Avalanche.evm_chain_id(), 43114);
        assert_eq!(Chain::Base.evm_chain_id(), 8453);
        assert_eq!(Chain::EthereumClassic.evm_chain_id(), 61);
        assert_eq!(Chain::BnbChain.evm_chain_id(), 56);
        assert_eq!(Chain::Hyperliquid.evm_chain_id(), 999);
        assert_eq!(Chain::Polygon.evm_chain_id(), 137);
        assert_eq!(Chain::Linea.evm_chain_id(), 59144);
        assert_eq!(Chain::Scroll.evm_chain_id(), 534352);
        assert_eq!(Chain::Blast.evm_chain_id(), 81457);
        assert_eq!(Chain::Mantle.evm_chain_id(), 5000);
        assert_eq!(Chain::Sei.evm_chain_id(), 1329);
        assert_eq!(Chain::Celo.evm_chain_id(), 42220);
        assert_eq!(Chain::Cronos.evm_chain_id(), 25);
        assert_eq!(Chain::OpBnb.evm_chain_id(), 204);
        assert_eq!(Chain::ZkSyncEra.evm_chain_id(), 324);
        assert_eq!(Chain::Sonic.evm_chain_id(), 146);
        assert_eq!(Chain::Berachain.evm_chain_id(), 80094);
        assert_eq!(Chain::Unichain.evm_chain_id(), 130);
        assert_eq!(Chain::Ink.evm_chain_id(), 57073);
        assert_eq!(Chain::XLayer.evm_chain_id(), 196);
    }

    #[test]
    fn endpoint_slots_use_string_suffixes() {
        assert_eq!(Chain::Polkadot.endpoint_str_id(EndpointSlot::Primary), "polkadot");
        assert_eq!(Chain::Polkadot.endpoint_str_id(EndpointSlot::Secondary), "polkadot:secondary");
        assert_eq!(Chain::Ethereum.endpoint_str_id(EndpointSlot::Explorer), "ethereum:explorer");
        assert_eq!(Chain::Tron.endpoint_str_id(EndpointSlot::Explorer), "tron:explorer");
        assert_eq!(Chain::Near.endpoint_str_id(EndpointSlot::Explorer), "near:explorer");
    }
}

// ── FFI surface ──────────────────────────────────────────────────────────

/// Resolve a display name to the chain's string id. Returns `None` for unknown names.
#[uniffi::export]
pub fn core_chain_str_id_for_name(name: String) -> Option<String> {
    Chain::from_display_name(&name).map(|c| c.str_id().to_string())
}

/// Endpoint-table key for a given chain + slot combination.
#[uniffi::export]
pub fn core_endpoint_str_id(chain_id: String, slot: crate::app_core::AppCoreEndpointSlot) -> Option<String> {
    let chain = Chain::from_str_id(&chain_id)?;
    let mapped = match slot {
        crate::app_core::AppCoreEndpointSlot::Primary => EndpointSlot::Primary,
        crate::app_core::AppCoreEndpointSlot::Secondary => EndpointSlot::Secondary,
        crate::app_core::AppCoreEndpointSlot::Explorer => EndpointSlot::Explorer,
    };
    Some(chain.endpoint_str_id(mapped))
}

/// Resolve any chain name, display name, or ticker symbol to its canonical
/// string id as stored in the `chains.toml` catalog.
#[uniffi::export]
pub fn core_resolve_chain_id(input: String) -> String {
    let normalized = input.trim().to_lowercase();
    if normalized.is_empty() {
        return input;
    }
    for entry in crate::chains::catalog() {
        if entry.id.to_lowercase() == normalized
            || entry.name.trim().to_lowercase() == normalized
            || entry.symbol.trim().to_lowercase() == normalized
        {
            return entry.id.clone();
        }
    }
    let kebab: String = normalized
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    kebab
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[uniffi::export]
pub fn core_evm_seed_derivation_chain_name(chain_name: String) -> Option<String> {
    Some(
        match Chain::from_display_name(&chain_name)? {
            Chain::Ethereum => "Ethereum",
            Chain::EthereumClassic => "Ethereum Classic",
            Chain::Arbitrum => "Arbitrum",
            Chain::BnbChain => "Ethereum",
            Chain::Avalanche => "Avalanche",
            Chain::Hyperliquid => "Hyperliquid",
            _ => return None,
        }
        .to_string(),
    )
}
