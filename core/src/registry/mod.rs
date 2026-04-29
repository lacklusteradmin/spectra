//! Central chain + token registry.
//!
//! The canonical `Chain` enum carries the frozen Spectra chain IDs as explicit
//! discriminants. It replaces:
//!   * ad-hoc `match chain_id { 0 => ..., 1 | 11 | 12 | 13 | 20 | 21 | 23 | 24 => ..., ... }`
//!     ladders in `service.rs`
//!   * the free-standing `evm_chain_id_for` / `send_chain_from_chain_id` helpers
//!   * raw offset arithmetic (`SUBSCAN_OFFSET + chain_id`, `EXPLORER_OFFSET + chain_id`, …)
//!
//! The numeric discriminants are part of the persistence wire format and must
//! not change; new chains append new variants with the next unused integer.

pub mod tokens;

use crate::send::payload::SendChain;

/// Every chain Spectra knows about. Discriminants are the frozen chain IDs
/// used in persistence, the UniFFI boundary, and the endpoint table.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Chain {
    Bitcoin = 0,
    Ethereum = 1,
    Solana = 2,
    Dogecoin = 3,
    Xrp = 4,
    Litecoin = 5,
    BitcoinCash = 6,
    Tron = 7,
    Stellar = 8,
    Cardano = 9,
    Polkadot = 10,
    Arbitrum = 11,
    Optimism = 12,
    Avalanche = 13,
    Sui = 14,
    Aptos = 15,
    Ton = 16,
    Near = 17,
    Icp = 18,
    Monero = 19,
    Base = 20,
    EthereumClassic = 21,
    BitcoinSV = 22,
    BnbChain = 23,
    Hyperliquid = 24,
    Polygon = 25,
    Linea = 26,
    Scroll = 27,
    Blast = 28,
    Mantle = 29,
    Zcash = 30,
    BitcoinGold = 31,
    Decred = 32,
    Kaspa = 33,
    Sei = 34,
    Celo = 35,
    Cronos = 36,
    OpBnb = 37,
    ZkSyncEra = 38,
    Sonic = 39,
    Berachain = 40,
    Unichain = 41,
    Ink = 42,
    Dash = 43,
    XLayer = 44,
    Bittensor = 45,

    // ── Testnets (46-77) ────────────────────────────────────────────────
    // Treated as fully separate chains: own keypair derivation, own
    // address space, own RPC endpoints, own catalog row. Their wire-format
    // chain ids are part of persistence; never renumber.
    BitcoinTestnet = 46,
    BitcoinTestnet4 = 47,
    BitcoinSignet = 48,
    LitecoinTestnet = 49,
    BitcoinCashTestnet = 50,
    BitcoinSVTestnet = 51,
    DogecoinTestnet = 52,
    ZcashTestnet = 53,
    DecredTestnet = 54,
    KaspaTestnet = 55,
    DashTestnet = 56,
    EthereumSepolia = 57,
    EthereumHoodi = 58,
    ArbitrumSepolia = 59,
    OptimismSepolia = 60,
    BaseSepolia = 61,
    BnbChainTestnet = 62,
    AvalancheFuji = 63,
    PolygonAmoy = 64,
    HyperliquidTestnet = 65,
    EthereumClassicMordor = 66,
    TronNile = 67,
    SolanaDevnet = 68,
    XrpTestnet = 69,
    StellarTestnet = 70,
    CardanoPreprod = 71,
    SuiTestnet = 72,
    AptosTestnet = 73,
    TonTestnet = 74,
    NearTestnet = 75,
    PolkadotWestend = 76,
    MoneroStagenet = 77,
}

/// Which endpoint-list slot to fetch for a given chain.
///
/// Conceptually belongs with the HTTP/networking layer, not the registry —
/// the registry should own *what chains exist*, the endpoint slot system
/// is *how we connect to them*. Kept here for now because moving requires
/// updating ~40 call sites and a Swift binding regeneration; new code
/// should prefer importing this via `crate::http::EndpointSlot` once the
/// move lands.
///
/// The primary slot is the chain's own id; secondary and explorer slots
/// are stored at `id + 100` and `id + 200` respectively (this is the
/// persistence contract the Swift side currently fills).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EndpointSlot {
    /// The chain's primary RPC provider(s).
    Primary,
    /// A second-kind provider attached to this chain: Subscan for Polkadot,
    /// a v2 bundle for ICP, TonCenter v3 for TON.
    Secondary,
    /// An explorer-compatible provider: Etherscan-family for EVM chains,
    /// Tronscan for Tron, the NEAR indexer.
    Explorer,
}

impl Chain {
    /// Parse a raw u32 id into a `Chain`. Returns `None` for unknown ids.
    pub const fn from_id(id: u32) -> Option<Self> {
        Some(match id {
            0 => Chain::Bitcoin,
            1 => Chain::Ethereum,
            2 => Chain::Solana,
            3 => Chain::Dogecoin,
            4 => Chain::Xrp,
            5 => Chain::Litecoin,
            6 => Chain::BitcoinCash,
            7 => Chain::Tron,
            8 => Chain::Stellar,
            9 => Chain::Cardano,
            10 => Chain::Polkadot,
            11 => Chain::Arbitrum,
            12 => Chain::Optimism,
            13 => Chain::Avalanche,
            14 => Chain::Sui,
            15 => Chain::Aptos,
            16 => Chain::Ton,
            17 => Chain::Near,
            18 => Chain::Icp,
            19 => Chain::Monero,
            20 => Chain::Base,
            21 => Chain::EthereumClassic,
            22 => Chain::BitcoinSV,
            23 => Chain::BnbChain,
            24 => Chain::Hyperliquid,
            25 => Chain::Polygon,
            26 => Chain::Linea,
            27 => Chain::Scroll,
            28 => Chain::Blast,
            29 => Chain::Mantle,
            30 => Chain::Zcash,
            31 => Chain::BitcoinGold,
            32 => Chain::Decred,
            33 => Chain::Kaspa,
            34 => Chain::Sei,
            35 => Chain::Celo,
            36 => Chain::Cronos,
            37 => Chain::OpBnb,
            38 => Chain::ZkSyncEra,
            39 => Chain::Sonic,
            40 => Chain::Berachain,
            41 => Chain::Unichain,
            42 => Chain::Ink,
            43 => Chain::Dash,
            44 => Chain::XLayer,
            45 => Chain::Bittensor,
            46 => Chain::BitcoinTestnet,
            47 => Chain::BitcoinTestnet4,
            48 => Chain::BitcoinSignet,
            49 => Chain::LitecoinTestnet,
            50 => Chain::BitcoinCashTestnet,
            51 => Chain::BitcoinSVTestnet,
            52 => Chain::DogecoinTestnet,
            53 => Chain::ZcashTestnet,
            54 => Chain::DecredTestnet,
            55 => Chain::KaspaTestnet,
            56 => Chain::DashTestnet,
            57 => Chain::EthereumSepolia,
            58 => Chain::EthereumHoodi,
            59 => Chain::ArbitrumSepolia,
            60 => Chain::OptimismSepolia,
            61 => Chain::BaseSepolia,
            62 => Chain::BnbChainTestnet,
            63 => Chain::AvalancheFuji,
            64 => Chain::PolygonAmoy,
            65 => Chain::HyperliquidTestnet,
            66 => Chain::EthereumClassicMordor,
            67 => Chain::TronNile,
            68 => Chain::SolanaDevnet,
            69 => Chain::XrpTestnet,
            70 => Chain::StellarTestnet,
            71 => Chain::CardanoPreprod,
            72 => Chain::SuiTestnet,
            73 => Chain::AptosTestnet,
            74 => Chain::TonTestnet,
            75 => Chain::NearTestnet,
            76 => Chain::PolkadotWestend,
            77 => Chain::MoneroStagenet,
            _ => return None,
        })
    }

    /// Returns `true` for chains that are testnets (faucet-funded, fake-coin,
    /// not real value). Mainnets return `false`.
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

    /// Maps a testnet variant to its mainnet counterpart for shared logic
    /// (derivation engine, send-payload classification, native-decimals
    /// table, etc.). Returns `self` for mainnets.
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

    /// The frozen numeric id.
    pub const fn id(self) -> u32 { self as u32 }

    /// View the chain as an `EvmChain` if it's EVM-family. Lets generic code
    /// take an `EvmChain` argument instead of accepting any `Chain` and
    /// asserting `is_evm()` at the call site — the family check moves into
    /// the type system.
    ///
    /// Why not restructure to `Chain::Evm(EvmChain) | Chain::Bitcoin | …`?
    /// The flat `Chain` enum is referenced by hundreds of match arms across
    /// derivation, fetch, send, service dispatch, and persistence. A nested
    /// representation would break every arm and force a flag-day migration,
    /// for the benefit of one structural property. The `EvmChain` newtype
    /// gives that benefit at the boundary where it matters (callers that
    /// genuinely need to enforce "EVM only" can take `EvmChain` directly)
    /// while leaving the broader codebase untouched.
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

    /// EIP-155 chain id. Non-EVM chains return `1` (legacy fallback that
    /// callers rely on when they know they're already on the EVM branch).
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
            // EVM testnets — chain ids per chainlist.org / official docs.
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
            // Testnets reuse their mainnet counterpart's send-payload classification —
            // the on-the-wire envelope shape is identical; only the network parameter
            // (handled inside the per-chain client) differs.
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

    /// Endpoint-table key for a given logical slot. The offsets (`+100`,
    /// `+200`) are part of the persistence contract filled by the Swift
    /// `EndpointStore` — changing them requires coordinating both sides.
    pub const fn endpoint_id(self, slot: EndpointSlot) -> u32 {
        match slot {
            EndpointSlot::Primary => self.id(),
            EndpointSlot::Secondary => self.id() + 100,
            EndpointSlot::Explorer => self.id() + 200,
        }
    }

    // ----------------------------------------------------------------
    // Native-coin metadata
    // ----------------------------------------------------------------
    //
    // These replace the freestanding table in `service.rs::native_coin_template`
    // and the inline per-chain field/decimals pairs in `native_amount_from_balance_json`,
    // `simple_chain_balance_display`, and `fetch_fee_estimate`. Keep them in sync:
    // every variant must return a value.

    /// Display name of the native coin (what shows in holding lists).
    /// For EVM sidechains this is the coin they pay gas in, not the chain
    /// (e.g. Arbitrum pays in ETH so `coin_name()` returns `"Ethereum"`).
    pub const fn coin_name(self) -> &'static str {
        match self {
            Chain::Bitcoin => "Bitcoin",
            Chain::Ethereum
            | Chain::Arbitrum
            | Chain::Optimism
            | Chain::Base
            | Chain::Linea
            | Chain::Scroll
            | Chain::Blast => "Ethereum",
            Chain::Solana => "Solana",
            Chain::Dogecoin => "Dogecoin",
            Chain::Xrp => "XRP",
            Chain::Litecoin => "Litecoin",
            Chain::BitcoinCash => "Bitcoin Cash",
            Chain::Tron => "Tron",
            Chain::Stellar => "Stellar",
            Chain::Cardano => "Cardano",
            Chain::Polkadot => "Polkadot",
            Chain::Avalanche => "Avalanche",
            Chain::Sui => "Sui",
            Chain::Aptos => "Aptos",
            Chain::Ton => "TON",
            Chain::Near => "NEAR",
            Chain::Icp => "Internet Computer",
            Chain::Monero => "Monero",
            Chain::EthereumClassic => "Ethereum Classic",
            Chain::BitcoinSV => "Bitcoin SV",
            Chain::BnbChain => "BNB",
            Chain::Hyperliquid => "Hyperliquid",
            Chain::Polygon => "Polygon",
            Chain::Mantle => "Mantle",
            Chain::Zcash => "Zcash",
            Chain::BitcoinGold => "Bitcoin Gold",
            Chain::Decred => "Decred",
            Chain::Kaspa => "Kaspa",
            Chain::Sei => "Sei",
            Chain::Celo => "Celo",
            Chain::Cronos => "Cronos",
            Chain::OpBnb => "BNB",
            Chain::ZkSyncEra | Chain::Unichain | Chain::Ink => "Ethereum",
            Chain::Sonic => "Sonic",
            Chain::Berachain => "Berachain",
            Chain::Dash => "Dash",
            Chain::XLayer => "OKB",
            Chain::Bittensor => "Bittensor",
            // Testnets — same coin name as mainnet (the asset is logically the
            // same; the chain row just makes clear it's not real money).
            Chain::BitcoinTestnet | Chain::BitcoinTestnet4 | Chain::BitcoinSignet => "Bitcoin",
            Chain::LitecoinTestnet => "Litecoin",
            Chain::BitcoinCashTestnet => "Bitcoin Cash",
            Chain::BitcoinSVTestnet => "Bitcoin SV",
            Chain::DogecoinTestnet => "Dogecoin",
            Chain::ZcashTestnet => "Zcash",
            Chain::DecredTestnet => "Decred",
            Chain::KaspaTestnet => "Kaspa",
            Chain::DashTestnet => "Dash",
            Chain::EthereumSepolia
            | Chain::EthereumHoodi
            | Chain::ArbitrumSepolia
            | Chain::OptimismSepolia
            | Chain::BaseSepolia => "Ethereum",
            Chain::BnbChainTestnet => "BNB",
            Chain::AvalancheFuji => "Avalanche",
            Chain::PolygonAmoy => "Polygon",
            Chain::HyperliquidTestnet => "Hyperliquid",
            Chain::EthereumClassicMordor => "Ethereum Classic",
            Chain::TronNile => "Tron",
            Chain::SolanaDevnet => "Solana",
            Chain::XrpTestnet => "XRP",
            Chain::StellarTestnet => "Stellar",
            Chain::CardanoPreprod => "Cardano",
            Chain::SuiTestnet => "Sui",
            Chain::AptosTestnet => "Aptos",
            Chain::TonTestnet => "TON",
            Chain::NearTestnet => "NEAR",
            Chain::PolkadotWestend => "Polkadot",
            Chain::MoneroStagenet => "Monero",
        }
    }

    /// Ticker of the native coin (BTC, ETH, ...).
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
            // Testnets — same ticker as mainnet (e.g. testnet BTC is still BTC,
            // Sepolia ETH is still ETH; the chain row, not the symbol, signals
            // that it isn't real value).
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

    /// Chain/network display name used in `AssetHolding.chain_name`. Differs
    /// from `coin_name()` for EVM sidechains (e.g. Arbitrum chain, ETH coin).
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
            // Testnet display names — these are the *chain* names users see in the catalog.
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

    /// Decimals of the native coin (BTC=8, ETH=18, ...).
    pub const fn native_decimals(self) -> u8 {
        match self {
            Chain::Bitcoin
            | Chain::Dogecoin
            | Chain::Litecoin
            | Chain::BitcoinCash
            | Chain::BitcoinSV
            | Chain::Zcash
            | Chain::BitcoinGold
            | Chain::Decred
            | Chain::Kaspa
            | Chain::Dash => 8,
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
            | Chain::XLayer => 18,
            Chain::Solana | Chain::Sui | Chain::Ton | Chain::Bittensor => 9,
            Chain::Xrp | Chain::Tron | Chain::Cardano => 6,
            Chain::Stellar => 7,
            Chain::Aptos | Chain::Icp => 8,
            Chain::Polkadot => 10,
            Chain::Near => 24,
            Chain::Monero => 12,
            // Testnets — same decimals as mainnet counterpart.
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
            | Chain::DashTestnet => 8,
            Chain::EthereumSepolia
            | Chain::EthereumHoodi
            | Chain::ArbitrumSepolia
            | Chain::OptimismSepolia
            | Chain::BaseSepolia
            | Chain::BnbChainTestnet
            | Chain::AvalancheFuji
            | Chain::PolygonAmoy
            | Chain::HyperliquidTestnet
            | Chain::EthereumClassicMordor => 18,
            Chain::TronNile => 6,
            Chain::SolanaDevnet | Chain::SuiTestnet | Chain::TonTestnet => 9,
            Chain::XrpTestnet | Chain::CardanoPreprod => 6,
            Chain::StellarTestnet => 7,
            Chain::AptosTestnet => 8,
            Chain::PolkadotWestend => 10,
            Chain::NearTestnet => 24,
            Chain::MoneroStagenet => 12,
        }
    }

    /// CoinGecko id (lowercase slug).
    pub const fn coin_gecko_id(self) -> &'static str {
        match self {
            Chain::Bitcoin => "bitcoin",
            Chain::Ethereum
            | Chain::Arbitrum
            | Chain::Optimism
            | Chain::Base
            | Chain::Linea
            | Chain::Scroll
            | Chain::Blast => "ethereum",
            Chain::Solana => "solana",
            Chain::Dogecoin => "dogecoin",
            Chain::Xrp => "ripple",
            Chain::Litecoin => "litecoin",
            Chain::BitcoinCash => "bitcoin-cash",
            Chain::Tron => "tron",
            Chain::Stellar => "stellar",
            Chain::Cardano => "cardano",
            Chain::Polkadot => "polkadot",
            Chain::Avalanche => "avalanche-2",
            Chain::Sui => "sui",
            Chain::Aptos => "aptos",
            Chain::Ton => "the-open-network",
            Chain::Near => "near",
            Chain::Icp => "internet-computer",
            Chain::Monero => "monero",
            Chain::EthereumClassic => "ethereum-classic",
            Chain::BitcoinSV => "bitcoin-cash-sv",
            Chain::BnbChain => "binancecoin",
            Chain::Hyperliquid => "hyperliquid",
            Chain::Polygon => "matic-network",
            Chain::Mantle => "mantle",
            Chain::Zcash => "zcash",
            Chain::BitcoinGold => "bitcoin-gold",
            Chain::Decred => "decred",
            Chain::Kaspa => "kaspa",
            Chain::Sei => "sei-network",
            Chain::Celo => "celo",
            Chain::Cronos => "crypto-com-chain",
            Chain::OpBnb => "binancecoin",
            Chain::ZkSyncEra | Chain::Unichain | Chain::Ink => "ethereum",
            Chain::Sonic => "sonic-3",
            Chain::Berachain => "berachain-bera",
            Chain::Dash => "dash",
            Chain::XLayer => "okb",
            Chain::Bittensor => "bittensor",
            // Testnets — reuse the mainnet CoinGecko id so the price feed
            // shows a non-zero number, but the UI is expected to flag the
            // chain as testnet so users don't confuse it with real money.
            Chain::BitcoinTestnet | Chain::BitcoinTestnet4 | Chain::BitcoinSignet => "bitcoin",
            Chain::LitecoinTestnet => "litecoin",
            Chain::BitcoinCashTestnet => "bitcoin-cash",
            Chain::BitcoinSVTestnet => "bitcoin-cash-sv",
            Chain::DogecoinTestnet => "dogecoin",
            Chain::ZcashTestnet => "zcash",
            Chain::DecredTestnet => "decred",
            Chain::KaspaTestnet => "kaspa",
            Chain::DashTestnet => "dash",
            Chain::EthereumSepolia
            | Chain::EthereumHoodi
            | Chain::ArbitrumSepolia
            | Chain::OptimismSepolia
            | Chain::BaseSepolia => "ethereum",
            Chain::BnbChainTestnet => "binancecoin",
            Chain::AvalancheFuji => "avalanche-2",
            Chain::PolygonAmoy => "matic-network",
            Chain::HyperliquidTestnet => "hyperliquid",
            Chain::EthereumClassicMordor => "ethereum-classic",
            Chain::TronNile => "tron",
            Chain::SolanaDevnet => "solana",
            Chain::XrpTestnet => "ripple",
            Chain::StellarTestnet => "stellar",
            Chain::CardanoPreprod => "cardano",
            Chain::SuiTestnet => "sui",
            Chain::AptosTestnet => "aptos",
            Chain::TonTestnet => "the-open-network",
            Chain::NearTestnet => "near",
            Chain::PolkadotWestend => "polkadot",
            Chain::MoneroStagenet => "monero",
        }
    }

    /// Primary JSON field name for the raw native balance in the Rust
    /// balance-response shape. Pair with `native_decimals()` to produce a
    /// human display amount. `None` for chains whose balance is encoded in a
    /// non-numeric form (EVM uses `balance_wei` as a decimal string *plus*
    /// an already-formatted `balance_display`; NEAR uses `yocto_near` as a
    /// string plus `near_display`).
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
            // Testnets share their mainnet's balance-field name (the wire
            // shape returned by the per-chain client is identical).
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
            // EVM + NEAR have multi-field balance shapes handled at the call site.
            _ => return None,
        })
    }

    /// True when this chain is a Bitcoin-derived UTXO chain that supports
    /// deep address discovery via xpub-derived gap-limit scans (BIP-32 family).
    /// Replaces `matches!(chain_name.as_str(), "Bitcoin" | "Bitcoin Cash" | …)`
    /// at the call sites — keeps the family membership in one place where
    /// adding a new BTC fork only needs one edit instead of N.
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

    /// True for chains where receiving an EVM-shaped (`0x…` / `.eth`) address
    /// is unambiguously a wrong-chain mistake. Excludes Bitcoin SV because it
    /// shares an address space with Bitcoin Cash and the wrong-chain check is
    /// already covered there.
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

    /// Static fee preview value for chains that use a flat fee, expressed in
    /// the chain's smallest unit (sats / lamports / drops / planck …).
    /// `None` for chains where the dispatch site fetches a live value over
    /// RPC or where the value would overflow `u128`.
    ///
    /// Exhaustively matches every `Chain` variant — adding a new chain
    /// forces the compiler to make the static-vs-dynamic-fee decision
    /// explicit. Do not reintroduce a `_ =>` catch-all arm.
    pub const fn static_fee_units(self) -> Option<u128> {
        match self {
            // Static flat fees (smallest unit).
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
            // Testnets reuse their mainnet counterpart's static-fee policy.
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
            // Dynamic-fee chains: dispatch site queries RPC.
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
            // EVM family: dispatch site uses EvmClient::fetch_fee_estimate.
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
            // u128 overflow: dispatch site uses fee_preview_str.
            Chain::Near => None,
            Chain::NearTestnet => None,
        }
    }

    /// Iterator over every known chain, in frozen-id order. Lets callers
    /// build per-chain tables without restating the variant list.
    pub fn all() -> impl Iterator<Item = Self> {
        (0u32..=77).filter_map(Chain::from_id)
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
    /// Accepts both `chain_display_name()` and the legacy "Internet Computer"
    /// alias for `Chain::Icp`.
    pub fn from_display_name(name: &str) -> Option<Self> {
        if name == "Internet Computer" {
            return Some(Chain::Icp);
        }
        Chain::all().find(|c| c.chain_display_name() == name)
    }
}

/// Newtype wrapper that proves the inner `Chain` is EVM-family. Construct
/// only via `Chain::as_evm`. Callers that need a "definitely EVM" argument
/// take `EvmChain` directly; callers that hold any `Chain` can dispatch on
/// `chain.as_evm()` without the runtime `is_evm()` check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvmChain(Chain);

impl EvmChain {
    /// The underlying `Chain` variant.
    pub const fn chain(self) -> Chain { self.0 }

    /// EIP-155 chain id — guaranteed non-`1`-fallback because `EvmChain` can
    /// only be constructed from a chain that returns `true` for `is_evm()`.
    pub const fn chain_id(self) -> u64 { self.0.evm_chain_id() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_roundtrips() {
        for id in 0u32..=77 {
            let chain = Chain::from_id(id).expect("valid id");
            assert_eq!(chain.id(), id);
        }
        assert!(Chain::from_id(78).is_none());
        assert!(Chain::from_id(99).is_none());
    }

    #[test]
    fn evm_group_includes_mainnets_and_testnets() {
        let mainnet_ids: Vec<u32> = vec![1, 11, 12, 13, 20, 21, 23, 24, 25, 26, 27, 28, 29, 34, 35, 36, 37, 38, 39, 40, 41, 42, 44];
        let testnet_ids: Vec<u32> = vec![57, 58, 59, 60, 61, 62, 63, 64, 65, 66];
        let mut expected: Vec<u32> = [mainnet_ids, testnet_ids].concat();
        expected.sort();
        let actual: Vec<u32> = Chain::all().filter(|c| c.is_evm()).map(|c| c.id()).collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn testnet_counts_match_total() {
        let total: usize = Chain::all().count();
        let testnets: usize = Chain::testnets().count();
        let mainnets: usize = Chain::mainnets().count();
        assert_eq!(testnets + mainnets, total);
        assert_eq!(testnets, 32, "32 testnet chains are expected");
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
    fn endpoint_slots_match_legacy_offsets() {
        assert_eq!(Chain::Polkadot.endpoint_id(EndpointSlot::Secondary), 110);
        assert_eq!(Chain::Icp.endpoint_id(EndpointSlot::Secondary), 118);
        assert_eq!(Chain::Ton.endpoint_id(EndpointSlot::Secondary), 116);
        assert_eq!(Chain::Ethereum.endpoint_id(EndpointSlot::Explorer), 201);
        assert_eq!(Chain::Tron.endpoint_id(EndpointSlot::Explorer), 207);
        assert_eq!(Chain::Near.endpoint_id(EndpointSlot::Explorer), 217);
    }
}
