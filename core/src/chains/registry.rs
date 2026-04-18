//! Central chain registry.
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
}

/// Which endpoint-list slot to fetch for a given chain. The primary slot is
/// the chain's own id; secondary and explorer slots are stored at
/// `id + 100` and `id + 200` respectively (this is the persistence contract
/// the Swift side currently fills).
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
            _ => return None,
        })
    }

    /// The frozen numeric id.
    pub const fn id(self) -> u32 { self as u32 }

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
        )
    }

    /// `true` for Bitcoin-family UTXO chains.
    pub const fn is_bitcoin_utxo(self) -> bool {
        matches!(
            self,
            Chain::Bitcoin
                | Chain::Litecoin
                | Chain::BitcoinCash
                | Chain::BitcoinSV
                | Chain::Dogecoin
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
            Chain::Ethereum
            | Chain::Arbitrum
            | Chain::Optimism
            | Chain::Avalanche
            | Chain::Base
            | Chain::EthereumClassic
            | Chain::BnbChain
            | Chain::Hyperliquid => SendChain::Ethereum,
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
            Chain::Ethereum | Chain::Arbitrum | Chain::Optimism | Chain::Base => "Ethereum",
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
        }
    }

    /// Ticker of the native coin (BTC, ETH, ...).
    pub const fn coin_symbol(self) -> &'static str {
        match self {
            Chain::Bitcoin => "BTC",
            Chain::Ethereum | Chain::Arbitrum | Chain::Optimism | Chain::Base => "ETH",
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
        }
    }

    /// Decimals of the native coin (BTC=8, ETH=18, ...).
    pub const fn native_decimals(self) -> u8 {
        match self {
            Chain::Bitcoin
            | Chain::Dogecoin
            | Chain::Litecoin
            | Chain::BitcoinCash
            | Chain::BitcoinSV => 8,
            Chain::Ethereum
            | Chain::Arbitrum
            | Chain::Optimism
            | Chain::Avalanche
            | Chain::Base
            | Chain::EthereumClassic
            | Chain::BnbChain
            | Chain::Hyperliquid => 18,
            Chain::Solana | Chain::Sui | Chain::Ton => 9,
            Chain::Xrp | Chain::Tron | Chain::Cardano => 6,
            Chain::Stellar => 7,
            Chain::Aptos | Chain::Icp => 8,
            Chain::Polkadot => 10,
            Chain::Near => 24,
            Chain::Monero => 12,
        }
    }

    /// CoinMarketCap id — stringified integer, used as `market_data_id`.
    pub const fn market_data_id(self) -> &'static str {
        match self {
            Chain::Bitcoin => "1",
            Chain::Ethereum | Chain::Arbitrum | Chain::Optimism | Chain::Base => "1027",
            Chain::Solana => "5426",
            Chain::Dogecoin => "74",
            Chain::Xrp => "52",
            Chain::Litecoin => "2",
            Chain::BitcoinCash => "1831",
            Chain::Tron => "1958",
            Chain::Stellar => "512",
            Chain::Cardano => "2010",
            Chain::Polkadot => "6636",
            Chain::Avalanche => "5805",
            Chain::Sui => "20947",
            Chain::Aptos => "21794",
            Chain::Ton => "11419",
            Chain::Near => "6535",
            Chain::Icp => "8916",
            Chain::Monero => "328",
            Chain::EthereumClassic => "1321",
            Chain::BitcoinSV => "3602",
            Chain::BnbChain => "1839",
            Chain::Hyperliquid => "32196",
        }
    }

    /// CoinGecko id (lowercase slug).
    pub const fn coin_gecko_id(self) -> &'static str {
        match self {
            Chain::Bitcoin => "bitcoin",
            Chain::Ethereum | Chain::Arbitrum | Chain::Optimism | Chain::Base => "ethereum",
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
            Chain::Litecoin | Chain::BitcoinCash | Chain::BitcoinSV => "balance_sat",
            Chain::Tron => "sun",
            Chain::Stellar => "stroops",
            Chain::Cardano => "lovelace",
            Chain::Polkadot => "planck",
            Chain::Sui => "mist",
            Chain::Aptos => "octas",
            Chain::Ton => "nanotons",
            Chain::Icp => "e8s",
            Chain::Monero => "piconeros",
            // EVM + NEAR have multi-field balance shapes handled at the call site.
            _ => return None,
        })
    }

    /// Iterator over every known chain, in frozen-id order. Lets callers
    /// build per-chain tables without restating the variant list.
    pub fn all() -> impl Iterator<Item = Self> {
        (0u32..=24).filter_map(Chain::from_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_roundtrips() {
        for id in 0u32..=24 {
            let chain = Chain::from_id(id).expect("valid id");
            assert_eq!(chain.id(), id);
        }
        assert!(Chain::from_id(25).is_none());
        assert!(Chain::from_id(99).is_none());
    }

    #[test]
    fn evm_group_matches_legacy_or_pattern() {
        let expected: Vec<u32> = vec![1, 11, 12, 13, 20, 21, 23, 24];
        let actual: Vec<u32> = (0u32..=24)
            .filter_map(Chain::from_id)
            .filter(|c| c.is_evm())
            .map(|c| c.id())
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn bitcoin_utxo_group_matches_legacy() {
        let expected: Vec<u32> = vec![0, 3, 5, 6, 22];
        let mut actual: Vec<u32> = (0u32..=24)
            .filter_map(Chain::from_id)
            .filter(|c| c.is_bitcoin_utxo())
            .map(|c| c.id())
            .collect();
        actual.sort();
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
