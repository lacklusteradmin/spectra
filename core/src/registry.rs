//! Central chain + token registry.
//!
//! The canonical `Chain` enum identifies chains by stable string ids
//! (e.g. `"bitcoin"`, `"ethereum"`). `Chain::str_id()` returns the id;
//! `Chain::from_str_id()` parses one back. The numeric discriminants were
//! removed in favour of string-keyed lookups throughout the codebase.

use crate::send::payload::SendChain;

/// Every chain Spectra knows about.
///
/// This crosses the FFI boundary as the one chain type every front end uses.
/// Before it did, each front end kept its own copy of this list — iOS had four
/// (`SpectraChainID`, `SeedDerivationChain`, `AppChainID`,
/// `StandardDiagnosticsChain`), each a different subset, each drifting.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, uniffi::Enum)]
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
/// What a chain requires before a holding may be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendRule {
    /// Only the chain's native asset.
    NativeOnly,
    /// The native asset, or a token the app supports on this chain.
    NativeOrSupportedToken,
    /// Solana's own list of sendable coins.
    SupportedSolanaCoin,
    /// No extra restriction beyond the chain supporting sends at all.
    Any,
}

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
    Chain::EthereumSepolia,
    Chain::EthereumHoodi,
    Chain::ArbitrumSepolia,
    Chain::OptimismSepolia,
    Chain::BaseSepolia,
    Chain::BnbChainTestnet,
    Chain::AvalancheFuji,
    Chain::PolygonAmoy,
    Chain::HyperliquidTestnet,
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

impl Chain {
    /// Stable string id matching `chains.toml` `id` field.
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
        self.entry().id.as_str()
    }

    /// Parse a string id (from `chains.toml` or the FFI boundary) into a `Chain`.
    /// Parse a string id (from `chains.toml` or the FFI boundary) into a `Chain`.
    pub fn from_str_id(id: &str) -> Option<Self> {
        static BY_ID: std::sync::LazyLock<std::collections::HashMap<&'static str, Chain>> =
            std::sync::LazyLock::new(|| Chain::all().map(|c| (c.str_id(), c)).collect());
        BY_ID.get(id).copied()
    }

    /// Key under which this chain's address is stored during wallet import.
    ///
    /// Most chains own their address, so the slot is just [`Chain::str_id`].
    /// The exceptions are the EVM family: one derived secp256k1 address serves
    /// Ethereum, Arbitrum, Base and the rest, so they all share the
    /// `"ethereum"` slot rather than each carrying a copy.
    ///
    /// Ethereum Classic is EVM but keeps a slot of its own, because the import
    /// flow lets the user supply a distinct ETC address. A wallet on that chain
    /// is written into *both* slots — see `derivation::import::addresses_for_chain`.
    ///
    /// Testnets fall through to their own `str_id`. Import only ever populates
    /// mainnet slots, so a testnet lookup misses and yields no address — which
    /// is correct: a Bitcoin testnet address is not a Bitcoin address.
    pub fn address_slot(self) -> &'static str {
        match self {
            Chain::EthereumClassic => Chain::EthereumClassic.str_id(),
            _ if self.is_evm() && !self.is_testnet() => Chain::Ethereum.str_id(),
            _ => self.str_id(),
        }
    }

    /// Whether a raw private key alone yields an address on this chain.
    ///
    /// One fact with two readers: `core_derive_from_private_key` dispatches on
    /// it, and the import flow offers the chain because of it. They used to be
    /// four separate lists and all four disagreed — a 39-name array gating the
    /// picker, a 23-name `matches!` gating the submit, a 30-arm Swift switch
    /// deciding whether an address appeared, and the dispatcher, which is the
    /// only one that could actually produce one. Eleven chains satisfied all
    /// four; the rest were offered and then refused somewhere downstream.
    ///
    /// Testnets answer for their mainnet: the key material is the same, and
    /// which network an address is rendered for is the derivation's business.
    pub const fn derives_from_private_key(self) -> bool {
        let chain = self.mainnet_counterpart();
        chain.is_evm()
            || matches!(
                chain,
                Chain::Bitcoin
                    | Chain::BitcoinCash
                    | Chain::Litecoin
                    | Chain::Dogecoin
                    | Chain::Decred
            )
    }

    /// The decode shape this chain's send preview comes back in, for the
    /// chains core estimates through one entry point.
    ///
    /// `None` means the chain has a preview path of its own — the UTXO family,
    /// Dogecoin, Tron and the EVM family each take different inputs and return
    /// a different record, which is why they are not one call.
    ///
    /// Swift held this as an eleven-entry `[String: SimpleChain]` table keyed
    /// by display name, and passed the result back to core beside the chain id
    /// core could have derived it from.
    ///
    /// Through `mainnet_counterpart` because the shape is what decoding needs
    /// and a testnet decodes like its mainnet; which network is reached is the
    /// chain id's business.
    pub const fn simple_preview_chain(self) -> Option<crate::send::preview_decode::SimpleChain> {
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
            _ => return None,
        })
    }

    /// Whether this chain has protocol-native staking Spectra can drive.
    ///
    /// Exact rather than through `mainnet_counterpart`: the staking clients are
    /// built against mainnet endpoints and mainnet contract addresses, so a
    /// testnet answering "yes" would route to a client that cannot serve it.
    ///
    /// One fact with three readers before it existed here — `fetch_validators`
    /// and `fetch_positions` each matched the same seven chain ids, and
    /// `StakingSupportedChain` in Swift was a seven-case enum with a display
    /// name switch and an id switch over the same seven. Widening staking is
    /// adding a client and a variant here; it was three edits and a chance for
    /// the picker to offer what the service refuses.
    pub const fn supports_staking(self) -> bool {
        matches!(
            self,
            Chain::Solana
                | Chain::Cardano
                | Chain::Sui
                | Chain::Aptos
                | Chain::Near
                | Chain::Polkadot
                | Chain::Icp
        )
    }

    /// Whether this chain's derivation reads a BIP-32 path.
    ///
    /// Read from the catalog rather than restated: `derivation_path = []` is
    /// how a row says its keys do not come from a path. Monero is the only
    /// mainnet that says it — its spend and view keys come from the seed
    /// directly — and the five chains whose derivation ignores the path it is
    /// handed still carry one, so this is not "does the arm use `p`".
    ///
    /// The distinction matters because "no default path" was an error
    /// everywhere it was asked, and Monero is a chain for which it is the
    /// answer. See `default_path_from_catalog`.
    pub fn uses_derivation_path(self) -> bool {
        crate::chains::default_derivation_path_template_by_id(self.mainnet_counterpart().str_id())
            .is_some()
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
        if self.is_evm() {
            Some(EvmChain(self))
        } else {
            None
        }
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

    /// Etherscan V2 base URL for this EVM chain, or `None` if the chain is not
    /// indexed by Etherscan. Etherscan V2 is a unified multichain endpoint
    /// (`/v2/api?chainid=X`) — all Etherscan-family chains share the same host.
    /// Chains using other explorers (Blockscout for ETC, Hyperliquid's own
    /// explorer) return `None` and history falls back to empty.
    pub const fn evm_explorer_api_base(self) -> Option<&'static str> {
        match self {
            Chain::EthereumClassic
            | Chain::EthereumClassicMordor
            | Chain::Hyperliquid
            | Chain::HyperliquidTestnet => None,
            _ if self.is_evm() => Some("https://api.etherscan.io"),
            _ => None,
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
            Chain::BitcoinTestnet | Chain::BitcoinTestnet4 | Chain::BitcoinSignet => {
                SendChain::Bitcoin
            }
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

    // ── Native-coin metadata

    pub fn coin_name(self) -> &'static str {
        self.entry().native_asset_name.as_str()
    }

    /// The asset fees are paid in — the catalog's `gas_token_symbol`.
    ///
    /// Not `symbol`: an L2 usually has a governance token of its own while
    /// still charging gas in ETH, and this is the one a balance is denominated
    /// in.
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

    pub fn coin_gecko_id(self) -> &'static str {
        self.entry().native_coingecko_id.as_str()
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
            Chain::BitcoinTestnet | Chain::BitcoinTestnet4 | Chain::BitcoinSignet => {
                "confirmed_sats"
            }
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

    /// The networks a user can pick between for this chain: the mainnet first,
    /// then its testnets in registry order.
    ///
    /// A "network mode" is not a separate concept — it is which `Chain` of a
    /// family the user selected. Three enums used to model this in parallel
    /// (`CoreBitcoinNetworkMode`, `CoreDogecoinNetworkMode` and a Swift-only
    /// Ethereum one), which is why a Dogecoin testnet was quoted at mainnet
    /// prices: the pricing rule listed two of the three by hand.
    pub fn network_choices(self) -> Vec<Chain> {
        let mainnet = self.mainnet_counterpart();
        std::iter::once(mainnet)
            .chain(
                Chain::all().filter(move |c| c.is_testnet() && c.mainnet_counterpart() == mainnet),
            )
            .collect()
    }

    /// The `bitcoin` crate's network for this chain.
    ///
    /// Replaces `bitcoin_network_for_mode(&str)`, which matched on the mode
    /// strings — one more table saying what the registry already knows.
    /// Non-UTXO chains answer `Bitcoin`, which is what the mode-string version
    /// did for anything it did not recognise.
    pub fn bitcoin_network(self) -> bitcoin::Network {
        match self {
            Chain::BitcoinTestnet
            | Chain::LitecoinTestnet
            | Chain::BitcoinCashTestnet
            | Chain::BitcoinSVTestnet
            | Chain::DogecoinTestnet
            | Chain::ZcashTestnet
            | Chain::DecredTestnet
            | Chain::DashTestnet => bitcoin::Network::Testnet,
            Chain::BitcoinTestnet4 => bitcoin::Network::Testnet4,
            Chain::BitcoinSignet => bitcoin::Network::Signet,
            _ => bitcoin::Network::Bitcoin,
        }
    }

    /// True when this chain's family offers more than one network.
    pub fn has_network_choice(self) -> bool {
        self.network_choices().len() > 1
    }

    /// The JSON-RPC method that answers "is this node alive", or `None` for a
    /// chain whose endpoints are checked over plain HTTP.
    ///
    /// Three of these were spelled in three different Swift functions, and
    /// *which* endpoints were RPC was decided by two hand-written id lists
    /// (`NearBalanceService.rpcEndpointCatalog`,
    /// `PolkadotBalanceService.sidecarEndpointCatalog`) beside a catalog that
    /// already carries an `rpc` role per endpoint. Both agreed when this was
    /// written; adding a provider meant editing the JSON and remembering the
    /// Swift list, and forgetting the second probes a JSON-RPC node with a
    /// GET — which many of them answer 405, reported as unreachable.
    pub fn rpc_health_method(self) -> Option<&'static str> {
        if self.is_evm() {
            return Some("eth_chainId");
        }
        match self.mainnet_counterpart() {
            Chain::Near => Some("status"),
            Chain::Polkadot => Some("chain_getHeader"),
            _ => None,
        }
    }

    /// Whether this chain's native send needs nothing beyond a destination, an
    /// amount and the fee its preview already supplied.
    ///
    /// The eleven that answer yes share one submit path. The rest each need
    /// something only they have — a UTXO selection (Bitcoin, Dogecoin), a
    /// resolved source account (Internet Computer), a resource model (Tron), a
    /// mint account (Solana), a view key and a backend (Monero), or a nonce
    /// and gas overrides (every EVM chain).
    ///
    /// It is a chain fact and it lived as two lists of names in
    /// `AppState+SendExecution`, next to a comment saying the lists should not
    /// be there. A twelfth chain reaching the shared path had to be added to
    /// whichever of the two the author happened to be looking at.
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
        )
    }

    /// What a front end needs to assemble a send for this chain.
    ///
    /// Transcribed from the ten call sites that carried these inline; the
    /// values are theirs, not new decisions. The one thing worth noticing is
    /// that `fee_decimals` is 6 nearly everywhere and 7 for Stellar and 8 for
    /// the UTXO chains — a display choice, unrelated to native decimals, which
    /// is why it could not simply be looked up.
    pub fn send_execution_shape(self) -> SendExecutionShape {
        let chain = self.mainnet_counterpart();
        match chain {
            Chain::Sui => SendExecutionShape {
                fee_decimals: 6,
                supports_private_key: false,
                fee_field: SendFeeField::GasBudget,
                fee_fallback: 0.0,
            },
            Chain::Cardano => SendExecutionShape {
                fee_decimals: 6,
                supports_private_key: false,
                fee_field: SendFeeField::FeeAmount,
                fee_fallback: 0.0,
            },
            Chain::Stellar => SendExecutionShape {
                fee_decimals: 7,
                supports_private_key: true,
                fee_field: SendFeeField::None,
                fee_fallback: 0.0,
            },
            Chain::Xrp => SendExecutionShape {
                fee_decimals: 6,
                supports_private_key: true,
                fee_field: SendFeeField::None,
                fee_fallback: 0.0,
            },
            Chain::BitcoinCash | Chain::BitcoinSV => SendExecutionShape {
                fee_decimals: 8,
                supports_private_key: false,
                fee_field: SendFeeField::FeeSats,
                fee_fallback: 0.00001,
            },
            Chain::Litecoin => SendExecutionShape {
                fee_decimals: 8,
                supports_private_key: false,
                fee_field: SendFeeField::FeeSats,
                fee_fallback: 0.0001,
            },
            _ => SendExecutionShape {
                fee_decimals: 6,
                supports_private_key: false,
                fee_field: SendFeeField::None,
                fee_fallback: 0.0,
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
                tracks_finality: false,
                require_send_kind: false,
            },
            // Dogecoin keeps counting after confirmation — the UI shows a
            // confirmation depth for it.
            Chain::Dogecoin => PendingStatusPoll::Utxo {
                tracks_finality: true,
                require_send_kind: true,
            },
            Chain::Bitcoin | Chain::BitcoinCash | Chain::BitcoinSV => PendingStatusPoll::Utxo {
                tracks_finality: false,
                require_send_kind: true,
            },
            Chain::Tron
            | Chain::Solana
            | Chain::Cardano
            | Chain::Xrp
            | Chain::Stellar
            | Chain::Monero
            | Chain::Sui
            | Chain::Aptos
            | Chain::Ton
            | Chain::Icp
            | Chain::Near
            | Chain::Polkadot => PendingStatusPoll::HistoryTxids,
            other if other.is_evm() => PendingStatusPoll::EvmReceipt,
            _ => PendingStatusPoll::None,
        }
    }

    /// The EVM family gates a non-native asset on it being a supported token.
    ///
    /// It used to gate only Ethereum, BNB Chain and Avalanche, and fall through
    /// to no restriction on the other twenty EVM chains — so the same
    /// unsupported token was refused on Ethereum and offered on Arbitrum, where
    /// it would be accepted by the UI and then fail at submit. That split was
    /// an accident of the chain-name match this replaced, not a decision, and
    /// a test used to pin it in place.
    ///
    /// One rule now, on the side that refuses early: being told no is better
    /// than a signed transaction that cannot land.
    pub const fn send_rule(self) -> SendRule {
        let chain = self.mainnet_counterpart();
        match chain {
            Chain::EthereumClassic | Chain::Hyperliquid => SendRule::NativeOnly,
            Chain::Solana => SendRule::SupportedSolanaCoin,
            _ if chain.is_evm() => SendRule::NativeOrSupportedToken,
            _ => SendRule::Any,
        }
    }

    /// How incoming history for this chain merges with what is already stored.
    ///
    /// Exhaustive on purpose: a new chain will not compile until someone says
    /// how its history merges, rather than silently defaulting to the wrong
    /// rule. This used to live as eighteen near-identical Swift wrappers,
    /// which is how a chain could be added and quietly get the wrong one.
    pub const fn transaction_merge_strategy(
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
            | Chain::Kaspa => S::StandardUtxo,
            other if other.is_evm() => S::Evm,
            _ => S::AccountBased,
        }
    }

    /// Whether the merge identity for this chain includes the asset symbol.
    ///
    /// Tron carries multiple assets on one transaction hash, so hash alone is
    /// not a unique key there.
    pub const fn merge_identity_includes_symbol(self) -> bool {
        matches!(self.mainnet_counterpart(), Chain::Tron)
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

    /// The `kind` string [`crate::validation::address::validate_address`]
    /// dispatches on for this chain's address format.
    ///
    /// Address *format* families are coarser than chains: every EVM chain
    /// validates as `"evm"`, and each testnet has its own flavour because the
    /// version bytes differ. This is the single source for that mapping —
    /// import validation, send validation and diagnostics all read it. Do not
    /// re-tabulate it per module; a stale copy silently rejects every address
    /// on the chains it misses.
    /// The match is exhaustive on purpose: adding a `Chain` variant must not
    /// compile until someone states its address format. The EVM arms duplicate
    /// [`Chain::is_evm`]'s list to keep that property; a test asserts the two
    /// never disagree.
    pub const fn address_validation_kind(self) -> &'static str {
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
            | Chain::XLayer => "evm",
            Chain::EthereumSepolia
            | Chain::EthereumHoodi
            | Chain::ArbitrumSepolia
            | Chain::OptimismSepolia
            | Chain::BaseSepolia
            | Chain::BnbChainTestnet
            | Chain::AvalancheFuji
            | Chain::PolygonAmoy
            | Chain::HyperliquidTestnet
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

    /// `true` when a wallet on this chain can be imported watch-only from an
    /// address alone.
    ///
    /// Monero is the notable exclusion: watching a Monero account needs the
    /// private view key, which an address does not carry. Testnets are excluded
    /// because import only populates mainnet slots — see [`Chain::address_slot`].
    pub const fn supports_watch_only_import(self) -> bool {
        if self.is_testnet() {
            return false;
        }
        if self.is_evm() {
            return true;
        }
        matches!(
            self,
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
                | Chain::Bittensor
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
            Chain::Litecoin
            | Chain::Zcash
            | Chain::BitcoinSV
            | Chain::BitcoinGold
            | Chain::Kaspa => Some(1_000),
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
            Chain::Bitcoin | Chain::Xrp | Chain::Stellar | Chain::Aptos => None,
            Chain::BitcoinTestnet
            | Chain::BitcoinTestnet4
            | Chain::BitcoinSignet
            | Chain::XrpTestnet
            | Chain::StellarTestnet
            | Chain::AptosTestnet => None,
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
            | Chain::XLayer => None,
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

    /// Which diagnostics record shape this chain reports.
    ///
    /// A per-chain fact, so it lives here rather than in a `match` inside the
    /// exporter — and it is what lets one JSON builder replace five.
    pub const fn diagnostics_shape(self) -> DiagnosticsShape {
        match self.mainnet_counterpart() {
            Chain::Bitcoin
            | Chain::BitcoinCash
            | Chain::BitcoinSV
            | Chain::Litecoin
            | Chain::Dogecoin => DiagnosticsShape::Utxo,
            Chain::Tron => DiagnosticsShape::Tron,
            Chain::Solana => DiagnosticsShape::Solana,
            other if other.is_evm() => DiagnosticsShape::Evm,
            _ => DiagnosticsShape::Simple,
        }
    }

    /// Resolve a chain from the display name used on the boundary.
    ///
    /// No special cases: the enum and `chains.toml` agree on every name, and
    /// `every_catalog_name_resolves` fails if they ever stop.
    pub fn from_display_name(name: &str) -> Option<Self> {
        Chain::all().find(|c| c.chain_display_name() == name)
    }
}

/// How a chain's estimated fee enters its signing request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SendFeeField {
    /// Sui: the fee becomes the transaction's gas budget.
    GasBudget,
    /// Cardano: the fee is passed as an explicit amount.
    FeeAmount,
    /// UTXO chains: the fee is converted to satoshis.
    FeeSats,
    /// The chain computes its own fee at signing time.
    None,
}

/// What a front end needs to assemble a send for this chain, beyond the
/// amount and the destination.
///
/// Ten near-identical call sites used to carry these four values inline, which
/// is how `fee_decimals` came to differ between chains for no stated reason
/// and how a new chain's arm got written by copying its neighbour.
#[derive(Debug, Clone, Copy, uniffi::Record)]
pub struct SendExecutionShape {
    /// Decimal places to show when reporting that the balance cannot cover
    /// the fee. A display precision, not the chain's native decimals.
    pub fee_decimals: u8,
    /// Whether a wallet holding only a private key (no seed) can sign here.
    pub supports_private_key: bool,
    pub fee_field: SendFeeField,
    /// Fee to assume when no preview is available, in native units. Zero
    /// where the chain always has a preview by the time a send is submitted.
    pub fee_fallback: f64,
}

/// How a chain's pending transactions are polled for confirmation.
///
/// A per-chain fact, so it lives here rather than as one wrapper function per
/// chain in the shell — there were eighteen of those, each naming a chain, a
/// chain id, an address resolver and up to two flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PendingStatusPoll {
    /// Ask the chain's own status endpoint for a txid.
    Utxo {
        /// Keep polling after confirmation to count confirmations.
        tracks_finality: bool,
        /// Only sends are tracked; receives confirm on their own.
        require_send_kind: bool,
    },
    /// Fetch the address's history and treat any txid in it as confirmed.
    HistoryTxids,
    /// Receipt-based, through the EVM history path.
    EvmReceipt,
    /// Not polled.
    None,
}

/// The networks available for a chain's family, mainnet first.
#[derive(Debug, Clone, uniffi::Record)]
pub struct NetworkChoice {
    /// Registry id — what `SelectNetworkChain` takes.
    pub chain_id: String,
    /// What to show in a picker: "Bitcoin", "Bitcoin Testnet4", …
    pub title: String,
    pub is_testnet: bool,
}

/// The `kind` string `validate_address` judges this chain's addresses by.
///
/// Exported so a front end asks the registry instead of keeping its own
/// mode-to-kind table — Bitcoin had one, mapping four network modes onto four
/// kind strings the registry already assigns per chain.
/// Not exported: a column of `core_chain_identities` now.
pub fn core_address_validation_kind(chain_id: String) -> String {
    Chain::from_str_id(&chain_id)
        .map(|c| c.address_validation_kind().to_string())
        .unwrap_or_default()
}

/// What a front end needs to assemble a send for this chain.
/// Not exported: a column of `core_chain_identities` now.
pub fn core_send_execution_shape(chain_name: String) -> SendExecutionShape {
    Chain::from_display_name(&chain_name)
        .map(Chain::send_execution_shape)
        .unwrap_or(SendExecutionShape {
            fee_decimals: 6,
            supports_private_key: false,
            fee_field: SendFeeField::None,
            fee_fallback: 0.0,
        })
}

/// How a chain's pending transactions reach a final status.
/// Not exported: a column of `core_chain_identities` now.
pub fn core_pending_status_poll(chain_name: String) -> PendingStatusPoll {
    Chain::from_display_name(&chain_name)
        .map(Chain::pending_status_poll)
        .unwrap_or(PendingStatusPoll::None)
}

/// A chain's display name, from its registry id.
/// Not exported: a column of `core_chain_identities` now.
pub fn core_chain_display_name(chain_id: String) -> String {
    Chain::from_str_id(&chain_id)
        .map(|c| c.chain_display_name().to_string())
        .unwrap_or(chain_id)
}

/// The networks available for a chain's family, mainnet first.
///
/// A front end enumerating these itself is how the mode enums came to exist.
/// Not exported: a column of `core_chain_identities` now.
pub fn core_network_choices(chain_id: String) -> Vec<NetworkChoice> {
    let Some(chain) = Chain::from_str_id(&chain_id) else {
        return Vec::new();
    };
    chain
        .network_choices()
        .into_iter()
        .map(|c| NetworkChoice {
            chain_id: c.str_id().to_string(),
            title: c.chain_display_name().to_string(),
            is_testnet: c.is_testnet(),
        })
        .collect()
}

/// The record shape a chain's history diagnostics use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DiagnosticsShape {
    Utxo,
    Evm,
    Simple,
    Tron,
    Solana,
}

/// Newtype wrapper that proves the inner `Chain` is EVM-family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvmChain(Chain);

impl EvmChain {
    pub const fn chain(self) -> Chain {
        self.0
    }
    pub const fn chain_id(self) -> u64 {
        self.0.evm_chain_id()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `address_validation_kind`'s EVM arms duplicate `is_evm`'s list so the
    /// match can stay exhaustive. This is what stops the two from drifting.
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
        use crate::validation::address::{validate_address, AddressValidationRequest};
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
            if chain.is_evm() && !chain.is_testnet() && chain != Chain::EthereumClassic {
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

    /// Watch-only support must never be claimed for a chain with no slot to
    /// read, and Monero must stay excluded.
    #[test]
    fn watch_only_support_excludes_monero_and_testnets() {
        assert!(!Chain::Monero.supports_watch_only_import());
        assert!(!Chain::BitcoinTestnet.supports_watch_only_import());
        assert!(Chain::Bitcoin.supports_watch_only_import());
        assert!(Chain::Polygon.supports_watch_only_import());
        assert!(Chain::EthereumClassic.supports_watch_only_import());
        for chain in Chain::all() {
            if chain.supports_watch_only_import() {
                assert!(!chain.is_testnet(), "{} is a testnet", chain.str_id());
            }
        }
    }

    /// Monero is the only mainnet the flag excludes, and one piece of iOS copy
    /// depends on that: the watch-only footer note names Monero while its
    /// condition reads the flag. A second excluded chain means generalising the
    /// string, which is a localisation edit rather than something to discover
    /// from a screenshot.
    #[test]
    fn token_tracking_chains_map_one_to_one() {
        let mut seen = std::collections::HashMap::new();
        for identity in core_chain_identities() {
            if let Some(t) = identity.token_tracking_chain {
                if let Some(prev) = seen.insert(format!("{t:?}"), identity.name.clone()) {
                    panic!("{t:?} claimed by both {prev} and {}", identity.name);
                }
            }
        }
        assert_eq!(seen.len(), 18, "expected eighteen token-tracking chains");
    }

    #[test]
    fn only_monero_is_excluded_from_watch_only_import() {
        let excluded: Vec<&str> = Chain::all()
            .filter(|c| !c.is_testnet() && !c.supports_watch_only_import())
            .map(|c| c.chain_display_name())
            .collect();
        assert_eq!(excluded, vec!["Monero"]);
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
    fn testnet_counts_match_total() {
        let total = Chain::all().count();
        let testnets = Chain::testnets().count();
        let mainnets = Chain::mainnets().count();
        assert_eq!(testnets + mainnets, total);
        assert!(testnets > 0 && mainnets > 0);
    }

    #[test]
    fn testnet_mainnet_counterparts_are_mainnets() {
        for testnet in Chain::testnets() {
            let counterpart = testnet.mainnet_counterpart();
            assert!(
                !counterpart.is_testnet(),
                "{:?} mainnet_counterpart returned testnet {:?}",
                testnet,
                counterpart
            );
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
        assert_eq!(
            Chain::Polkadot.endpoint_str_id(EndpointSlot::Primary),
            "polkadot"
        );
        assert_eq!(
            Chain::Polkadot.endpoint_str_id(EndpointSlot::Secondary),
            "polkadot:secondary"
        );
        assert_eq!(
            Chain::Ethereum.endpoint_str_id(EndpointSlot::Explorer),
            "ethereum:explorer"
        );
        assert_eq!(
            Chain::Tron.endpoint_str_id(EndpointSlot::Explorer),
            "tron:explorer"
        );
        assert_eq!(
            Chain::Near.endpoint_str_id(EndpointSlot::Explorer),
            "near:explorer"
        );
    }
}

// ── FFI surface ──────────────────────────────────────────────────────────

/// What identifies one chain to a front end: the enum value and the three
/// facts every screen needs to go with it.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ChainIdentity {
    pub chain: Chain,
    /// The catalog's `id` — what endpoint tables and the FFI boundary key on.
    pub id: String,
    /// The catalog's `name` — the one spelling of this chain.
    pub name: String,
    pub is_testnet: bool,
    pub is_evm: bool,
    pub diagnostics_shape: DiagnosticsShape,
    /// Which chain's slot this chain's address is stored under. The EVM family
    /// shares Ethereum's.
    pub address_slot: String,
    /// The address format family `validate_address` dispatches on.
    pub address_validation_kind: String,
    /// HD discovery walks this chain's addresses past the last used one.
    pub supports_deep_utxo_discovery: bool,
    /// A watch-only import can carry addresses for this chain.
    pub supports_watch_only_import: bool,
    /// The chain has protocol-native staking the staking tab can drive.
    pub supports_staking: bool,
    /// Which `CoreTokenTrackingChain` this chain is, if it can host tracked
    /// tokens. `None` for the chains that cannot.
    ///
    /// `CoreTokenTrackingChain::chain_name` and its inverse already collapsed
    /// four copies of this mapping inside Rust; publishing it here removes the
    /// three that were left in Swift, which hand-wrote `rawValue`,
    /// `init?(rawValue:)` and `allCases` for an enum core owns.
    pub token_tracking_chain: Option<crate::store::wallet_domain::CoreTokenTrackingChain>,
    pub send_execution_shape: SendExecutionShape,
    /// The JSON-RPC method that answers "is this node alive", or `None` for a
    /// chain whose endpoints are checked over plain HTTP.
    pub rpc_health_method: Option<String>,
    pub pending_status_poll: PendingStatusPoll,
    /// Which chain's derivation path this chain reuses, as a display name.
    /// `None` for a chain with no BIP-32 path.
    pub seed_derivation_chain: Option<String>,
    /// The EVM chain whose derivation this chain reuses, or `None` off the
    /// EVM family.
    pub evm_seed_derivation_chain: Option<String>,
    /// The mainnet this chain belongs to, or itself.
    ///
    /// Was published as `seed_derivation_path_key` — the same value under a
    /// name that described one of its callers. A testnet shares its mainnet's
    /// derivation-path slot *because* it is that mainnet's testnet, and other
    /// call sites want the same fact for other reasons.
    pub mainnet_counterpart: Chain,
    /// The networks this chain's family offers, mainnet first.
    pub network_choices: Vec<NetworkChoice>,
}

/// The whole catalog as identities, in declaration order.
///
/// One call rather than an accessor per column: a front end builds its lookups
/// from this once and then reads them locally, and there is no way to ask for
/// an id without the name that goes with it. `Chain` deliberately has no
/// `CaseIterable` on the Swift side — the order that matters is the catalog's.
#[uniffi::export]
pub fn core_chain_identities() -> Vec<ChainIdentity> {
    Chain::all()
        .map(|chain| ChainIdentity {
            chain,
            id: chain.str_id().to_string(),
            name: chain.chain_display_name().to_string(),
            is_testnet: chain.is_testnet(),
            is_evm: chain.is_evm(),
            diagnostics_shape: chain.diagnostics_shape(),
            address_slot: chain.address_slot().to_string(),
            address_validation_kind: chain.address_validation_kind().to_string(),
            supports_deep_utxo_discovery: chain.supports_deep_utxo_discovery(),
            supports_watch_only_import: chain.supports_watch_only_import(),
            supports_staking: chain.supports_staking(),
            token_tracking_chain: crate::store::wallet_domain::CoreTokenTrackingChain::from_chain_name(
                chain.chain_display_name(),
            ),
            send_execution_shape: chain.send_execution_shape(),
            rpc_health_method: chain.rpc_health_method().map(str::to_string),
            pending_status_poll: chain.pending_status_poll(),
            seed_derivation_chain: crate::send::flow::seed_derivation_chain_raw(chain),
            evm_seed_derivation_chain: chain
                .is_evm()
                .then(|| evm_seed_derivation_chain(chain))
                .flatten(),
            mainnet_counterpart: chain.mainnet_counterpart(),
            network_choices: chain
                .network_choices()
                .into_iter()
                .map(|c| NetworkChoice {
                    chain_id: c.str_id().to_string(),
                    title: c.chain_display_name().to_string(),
                    is_testnet: c.is_testnet(),
                })
                .collect(),
        })
        .collect()
}

/// Resolve a display name to the chain's string id. Returns `None` for unknown names.
/// Not exported: a column of `core_chain_identities` now.
pub fn core_chain_str_id_for_name(name: String) -> Option<String> {
    Chain::from_display_name(&name).map(|c| c.str_id().to_string())
}

/// Endpoint-table key for a given chain + slot combination.
#[uniffi::export]
pub fn core_endpoint_str_id(
    chain_id: String,
    slot: crate::app_core::AppCoreEndpointSlot,
) -> Option<String> {
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

/// Not exported: it is a column of `core_chain_identities` now.
pub fn evm_seed_derivation_chain(chain: Chain) -> Option<String> {
    Some(
        match chain {
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

#[cfg(test)]
mod catalog_agreement_tests {
    use super::*;

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
    /// *reads* the catalog now, so the two agree by construction. The variant
    /// name is the independent source, which is why [`expected_id`] exists.
    ///
    /// This replaces `display_names_match_the_catalog`, which asked whether two
    /// tables agreed on a name. There is one table now. The question worth
    /// asking is whether the index is sound — if it is not, every chain
    /// silently becomes a different chain, and unlike a rename that is
    /// invisible from the outside.
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
