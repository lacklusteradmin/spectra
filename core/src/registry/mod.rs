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
            _ => return None,
        })
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

    /// `true` for every EVM-compatible chain.
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
        )
    }

    /// True for chains where receiving an EVM-shaped (`0x…` / `.eth`) address
    /// is unambiguously a wrong-chain mistake. Excludes Bitcoin SV because it
    /// shares an address space with Bitcoin Cash and the wrong-chain check is
    /// already covered there.
    pub const fn flags_evm_address_as_wrong_chain(self) -> bool {
        matches!(
            self,
            Chain::Bitcoin | Chain::BitcoinCash | Chain::Litecoin | Chain::Dogecoin
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
            // Dynamic-fee chains: dispatch site queries RPC.
            Chain::Bitcoin
            | Chain::Xrp
            | Chain::Stellar
            | Chain::Aptos => None,
            // EVM family: dispatch site uses EvmClient::fetch_fee_estimate.
            Chain::Ethereum | Chain::Arbitrum | Chain::Optimism | Chain::Avalanche
            | Chain::Base | Chain::EthereumClassic | Chain::BnbChain | Chain::Hyperliquid
            | Chain::Polygon | Chain::Linea | Chain::Scroll | Chain::Blast | Chain::Mantle
            | Chain::Sei | Chain::Celo | Chain::Cronos | Chain::OpBnb | Chain::ZkSyncEra
            | Chain::Sonic | Chain::Berachain | Chain::Unichain | Chain::Ink | Chain::XLayer => None,
            // u128 overflow: dispatch site uses fee_preview_str.
            Chain::Near => None,
        }
    }

    /// Iterator over every known chain, in frozen-id order. Lets callers
    /// build per-chain tables without restating the variant list.
    pub fn all() -> impl Iterator<Item = Self> {
        (0u32..=45).filter_map(Chain::from_id)
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
        for id in 0u32..=45 {
            let chain = Chain::from_id(id).expect("valid id");
            assert_eq!(chain.id(), id);
        }
        assert!(Chain::from_id(46).is_none());
        assert!(Chain::from_id(99).is_none());
    }

    #[test]
    fn evm_group_matches_legacy_or_pattern() {
        let expected: Vec<u32> = vec![1, 11, 12, 13, 20, 21, 23, 24, 25, 26, 27, 28, 29, 34, 35, 36, 37, 38, 39, 40, 41, 42, 44];
        let actual: Vec<u32> = (0u32..=45)
            .filter_map(Chain::from_id)
            .filter(|c| c.is_evm())
            .map(|c| c.id())
            .collect();
        assert_eq!(actual, expected);
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
