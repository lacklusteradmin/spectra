// Wallet value types crossing the FFI. Display color is deliberately absent:
// the platform derives it from the asset symbol.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum,
)]
#[serde(rename_all = "camelCase")]
pub enum CoreBitcoinNetworkMode {
    #[default]
    Mainnet,
    Testnet,
    Testnet4,
    Signet,
}

#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum,
)]
#[serde(rename_all = "camelCase")]
pub enum CoreDogecoinNetworkMode {
    #[default]
    Mainnet,
    Testnet,
}

/// Swift `TransactionKind` — rawValues: `"send"`, `"receive"`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum CoreTransactionKind {
    Send,
    Receive,
}

/// Swift `TransactionStatus` — rawValues: `"pending"`, `"confirmed"`, `"failed"`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum CoreTransactionStatus {
    Pending,
    Confirmed,
    Failed,
}

/// Swift `PriceAlertCondition` — rawValues: `"Above"`, `"Below"` (PascalCase).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum CorePriceAlertCondition {
    #[serde(rename = "Above")]
    Above,
    #[serde(rename = "Below")]
    Below,
}

#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum,
)]
#[serde(rename_all = "camelCase")]
pub enum CoreSeedDerivationPreset {
    #[default]
    Standard,
    Account1,
    Account2,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CoreCoin {
    pub id: String,
    pub name: String,
    pub symbol: String,
    pub coin_gecko_id: String,
    pub chain_name: String,
    pub token_standard: String,
    pub contract_address: Option<String>,
    pub amount: f64,
    pub price_usd: f64,
}

/// Power-user derivation overrides layered on top of the chain defaults in
/// `core/data/chains.toml`. Every field is optional; `None` means
/// "use the catalog default." Persisted per-wallet and propagated to
/// every derivation call (import-time preview + send-time signing) so the
/// imported address and the re-derived signing key stay in sync.
///
/// String values (rather than typed enums) keep the UniFFI record stable
/// against future runtime-side additions; invalid values surface as runtime
/// errors from the derivation pipeline.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CoreWalletDerivationOverrides {
    #[serde(default)]
    pub passphrase: Option<String>,
    #[serde(default)]
    pub mnemonic_wordlist: Option<String>,
    #[serde(default)]
    pub iteration_count: Option<u32>,
    #[serde(default)]
    pub salt_prefix: Option<String>,
    #[serde(default)]
    pub hmac_key: Option<String>,
    #[serde(default)]
    pub curve: Option<String>,
    #[serde(default)]
    pub derivation_algorithm: Option<String>,
    #[serde(default)]
    pub address_algorithm: Option<String>,
    #[serde(default)]
    pub public_key_format: Option<String>,
    #[serde(default)]
    pub script_type: Option<String>,
}

impl CoreWalletDerivationOverrides {
    pub fn is_empty(&self) -> bool {
        self.passphrase.is_none()
            && self.mnemonic_wordlist.is_none()
            && self.iteration_count.is_none()
            && self.salt_prefix.is_none()
            && self.hmac_key.is_none()
            && self.curve.is_none()
            && self.derivation_algorithm.is_none()
            && self.address_algorithm.is_none()
            && self.public_key_format.is_none()
            && self.script_type.is_none()
    }

    pub(crate) fn zeroize_sensitive_fields(&mut self) {
        if let Some(value) = &mut self.passphrase {
            value.zeroize();
        }
        if let Some(value) = &mut self.hmac_key {
            value.zeroize();
        }
        if let Some(value) = &mut self.salt_prefix {
            value.zeroize();
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CoreSeedDerivationPaths {
    pub is_custom_enabled: bool,
    /// `Chain::str_id()` → derivation path, for mainnet chains only.
    ///
    /// Testnets resolve through `Chain::mainnet_counterpart()` rather than
    /// carrying their own entry — a testnet wallet derives from the same path
    /// as its mainnet, only the address encoding differs. Use
    /// [`CoreSeedDerivationPaths::path_for`] rather than indexing directly so
    /// that stays true at every call site.
    pub by_chain: HashMap<String, String>,
}

impl CoreSeedDerivationPaths {
    /// Derivation path configured for `chain`, resolving testnets to their
    /// mainnet counterpart's entry.
    pub fn path_for(&self, chain: crate::registry::Chain) -> Option<&str> {
        self.by_chain
            .get(chain.mainnet_counterpart().str_id())
            .map(String::as_str)
    }

    /// Set the path for `chain`, writing through to the mainnet slot.
    pub fn set_path_for(&mut self, chain: crate::registry::Chain, path: impl Into<String>) {
        self.by_chain
            .insert(chain.mainnet_counterpart().str_id().to_string(), path.into());
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CoreImportedWallet {
    pub id: String,
    pub name: String,
    pub bitcoin_network_mode: CoreBitcoinNetworkMode,
    pub dogecoin_network_mode: CoreDogecoinNetworkMode,
    /// `Chain::address_slot()` → address for this wallet.
    ///
    /// A wallet belongs to one chain (`selected_chain`), so in practice this
    /// holds a single entry — two for Ethereum Classic, which occupies both the
    /// shared EVM slot and its own. It is a map rather than one `Option<String>`
    /// per chain so that adding a chain is a registry edit and not a schema
    /// change here, in the Swift record, in its `Codable`, and at every
    /// construction site.
    pub addresses: HashMap<String, String>,
    /// Bitcoin account xpub/ypub/zpub. Not an address, so it keeps its own field.
    pub bitcoin_xpub: Option<String>,
    pub seed_derivation_preset: CoreSeedDerivationPreset,
    pub seed_derivation_paths: CoreSeedDerivationPaths,
    #[serde(default)]
    pub derivation_overrides: CoreWalletDerivationOverrides,
    pub selected_chain: String,
    pub holdings: Vec<CoreCoin>,
    pub include_in_portfolio_total: bool,
}

impl CoreImportedWallet {
    pub fn total_balance(&self) -> f64 {
        self.holdings.iter().map(|c| c.amount * c.price_usd).sum()
    }

    /// This wallet's address for `chain`, if it has one.
    pub fn address_for(&self, chain: crate::registry::Chain) -> Option<&str> {
        self.addresses.get(chain.address_slot()).map(String::as_str)
    }

    /// The address for the wallet's own chain — what the UI shows and what
    /// balance/history calls query.
    pub fn primary_address(&self) -> Option<&str> {
        crate::registry::Chain::from_display_name(&self.selected_chain)
            .and_then(|chain| self.address_for(chain))
    }
}

// ── CoreImportedWallet ↔ WalletSummary ───────────────────────────────────────
//
// `WalletSummary` is the model core computes with; `CoreImportedWallet` is the
// shape the iOS app still uses. The conversion exists so the two can coexist
// while the app migrates, and it is **deliberately asymmetric**:
//
// A `CoreImportedWallet` carries the whole 45-entry derivation-path table and
// two network-mode fields on *every* wallet, even though a wallet belongs to
// one chain and uses one path on one network. Converting to `WalletSummary`
// keeps the entry that wallet actually uses and drops the other 44 — they are
// global defaults, not per-wallet data. Converting back therefore cannot
// reconstruct them, and rebuilds the table from the defaults instead.
//
// That asymmetry is the point, not a defect: the round trip losing redundant
// copies is what makes `WalletSummary` the smaller, correcter model.

impl CoreImportedWallet {
    /// The network mode this wallet is actually on, as a raw string.
    ///
    /// Only one of the two stored modes applies — the one matching the wallet's
    /// chain — and every other chain is always mainnet.
    fn active_network_mode(&self) -> Option<String> {
        match self.selected_chain.as_str() {
            "Bitcoin" => Some(match self.bitcoin_network_mode {
                CoreBitcoinNetworkMode::Mainnet => "mainnet",
                CoreBitcoinNetworkMode::Testnet => "testnet",
                CoreBitcoinNetworkMode::Testnet4 => "testnet4",
                CoreBitcoinNetworkMode::Signet => "signet",
            }),
            "Dogecoin" => Some(match self.dogecoin_network_mode {
                CoreDogecoinNetworkMode::Mainnet => "mainnet",
                CoreDogecoinNetworkMode::Testnet => "testnet",
            }),
            _ => None,
        }
        .map(str::to_string)
    }

    /// Convert to the model core computes with.
    ///
    /// `is_watch_only` cannot be read off this record — the app derives it from
    /// whether the Keychain holds signing material — so the caller supplies it.
    pub fn to_summary(&self, is_watch_only: bool) -> crate::store::state::WalletSummary {
        use crate::registry::Chain;
        use crate::store::state::{AssetHolding, WalletAddress, WalletSummary};

        let chain = Chain::from_display_name(&self.selected_chain);
        let derivation_path = chain.and_then(|chain| {
            self.seed_derivation_paths
                .path_for(chain)
                .map(str::to_string)
        });

        WalletSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            is_watch_only,
            chain_name: self.selected_chain.clone(),
            include_in_portfolio_total: self.include_in_portfolio_total,
            network_mode: self.active_network_mode(),
            xpub: self.bitcoin_xpub.clone(),
            derivation_preset: match self.seed_derivation_preset {
                CoreSeedDerivationPreset::Standard => "standard",
                CoreSeedDerivationPreset::Account1 => "account1",
                CoreSeedDerivationPreset::Account2 => "account2",
            }
            .to_string(),
            derivation_path: derivation_path.clone(),
            derivation_overrides: self.derivation_overrides.clone(),
            holdings: self
                .holdings
                .iter()
                .map(|coin| AssetHolding {
                    // `CoreCoin::id` is a SwiftUI `Identifiable` key, not
                    // domain data, so it does not survive into the summary.
                    name: coin.name.clone(),
                    symbol: coin.symbol.clone(),
                    coin_gecko_id: coin.coin_gecko_id.clone(),
                    chain_name: coin.chain_name.clone(),
                    token_standard: coin.token_standard.clone(),
                    contract_address: coin.contract_address.clone(),
                    amount: coin.amount,
                    price_usd: coin.price_usd,
                })
                .collect(),
            addresses: chain
                .and_then(|chain| {
                    self.addresses
                        .get(chain.address_slot())
                        .map(|address| WalletAddress {
                            chain_name: self.selected_chain.clone(),
                            address: address.clone(),
                            kind: "receive".to_string(),
                            derivation_path,
                        })
                })
                .into_iter()
                .collect(),
        }
    }
}

/// Render an app wallet record back into the authoritative model.
///
/// Exported so the shell can hand core a `WalletSummary` without reimplementing
/// the mapping. `is_watch_only` is a platform fact the record cannot carry.
#[uniffi::export]
pub fn core_wallet_summary(
    wallet: CoreImportedWallet,
    is_watch_only: bool,
) -> crate::store::state::WalletSummary {
    wallet.to_summary(is_watch_only)
}

impl crate::store::state::WalletSummary {
    /// Convert back into the shape the iOS app renders.
    ///
    /// The reverse of [`CoreImportedWallet::to_summary`], and lossy in the
    /// direction that does not matter: the 45-entry derivation-path table is
    /// rebuilt from `defaults` with this wallet's own path written over its
    /// chain's slot. Those defaults were never per-wallet data.
    ///
    /// `WalletSummary` remains the authority. This produces a view model.
    pub fn to_imported_wallet(&self, defaults: &CoreSeedDerivationPaths) -> CoreImportedWallet {
        use crate::registry::Chain;

        let chain = Chain::from_display_name(&self.chain_name);
        let mut seed_derivation_paths = defaults.clone();
        if let (Some(chain), Some(path)) = (chain, self.derivation_path.as_deref()) {
            seed_derivation_paths.set_path_for(chain, path);
        }

        let network_mode = self.network_mode.as_deref();
        CoreImportedWallet {
            id: self.id.clone(),
            name: self.name.clone(),
            bitcoin_network_mode: match network_mode {
                Some("testnet") => CoreBitcoinNetworkMode::Testnet,
                Some("testnet4") => CoreBitcoinNetworkMode::Testnet4,
                Some("signet") => CoreBitcoinNetworkMode::Signet,
                _ => CoreBitcoinNetworkMode::Mainnet,
            },
            dogecoin_network_mode: match network_mode {
                Some("testnet") => CoreDogecoinNetworkMode::Testnet,
                _ => CoreDogecoinNetworkMode::Mainnet,
            },
            addresses: self
                .addresses
                .iter()
                .filter_map(|entry| {
                    Chain::from_display_name(&entry.chain_name)
                        .map(|chain| (chain.address_slot().to_string(), entry.address.clone()))
                })
                .collect(),
            bitcoin_xpub: self.xpub.clone(),
            seed_derivation_preset: match self.derivation_preset.as_str() {
                "account1" => CoreSeedDerivationPreset::Account1,
                "account2" => CoreSeedDerivationPreset::Account2,
                _ => CoreSeedDerivationPreset::Standard,
            },
            seed_derivation_paths,
            derivation_overrides: self.derivation_overrides.clone(),
            selected_chain: self.chain_name.clone(),
            holdings: self
                .holdings
                .iter()
                .map(|holding| CoreCoin {
                    // Derived from what identifies the holding, not random.
                    // A view model is rebuilt on every projection refresh, and
                    // a fresh id each time would make SwiftUI treat every row
                    // as new — losing selection and animating the whole list.
                    id: holding_identity(holding),
                    name: holding.name.clone(),
                    symbol: holding.symbol.clone(),
                    coin_gecko_id: holding.coin_gecko_id.clone(),
                    chain_name: holding.chain_name.clone(),
                    token_standard: holding.token_standard.clone(),
                    contract_address: holding.contract_address.clone(),
                    amount: holding.amount,
                    price_usd: holding.price_usd,
                })
                .collect(),
            include_in_portfolio_total: self.include_in_portfolio_total,
        }
    }
}

/// Stable identity for a holding: chain, symbol and contract are what make two
/// holdings the same asset.
fn holding_identity(holding: &crate::store::state::AssetHolding) -> String {
    format!(
        "{}|{}|{}",
        holding.chain_name,
        holding.symbol,
        holding.contract_address.as_deref().unwrap_or("")
    )
}

/// Swift `TokenTrackingChain` — rawValues are chain display names.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum CoreTokenTrackingChain {
    #[serde(rename = "Ethereum")]
    Ethereum,
    #[serde(rename = "Arbitrum")]
    Arbitrum,
    #[serde(rename = "Optimism")]
    Optimism,
    #[serde(rename = "BNB Chain")]
    Bnb,
    #[serde(rename = "Avalanche")]
    Avalanche,
    #[serde(rename = "Hyperliquid")]
    Hyperliquid,
    #[serde(rename = "Polygon")]
    Polygon,
    #[serde(rename = "Base")]
    Base,
    #[serde(rename = "Linea")]
    Linea,
    #[serde(rename = "Scroll")]
    Scroll,
    #[serde(rename = "Blast")]
    Blast,
    #[serde(rename = "Mantle")]
    Mantle,
    #[serde(rename = "Solana")]
    Solana,
    #[serde(rename = "Sui")]
    Sui,
    #[serde(rename = "Aptos")]
    Aptos,
    #[serde(rename = "TON")]
    Ton,
    #[serde(rename = "NEAR")]
    Near,
    #[serde(rename = "Tron")]
    Tron,
}

impl CoreTokenTrackingChain {
    /// Every variant, in declaration order.
    pub const ALL: &'static [Self] = &[
        Self::Ethereum,
        Self::Arbitrum,
        Self::Optimism,
        Self::Bnb,
        Self::Avalanche,
        Self::Hyperliquid,
        Self::Polygon,
        Self::Base,
        Self::Linea,
        Self::Scroll,
        Self::Blast,
        Self::Mantle,
        Self::Solana,
        Self::Sui,
        Self::Aptos,
        Self::Ton,
        Self::Near,
        Self::Tron,
    ];

    /// The chain a tracked token belongs to, from its display name.
    ///
    /// Matches case-insensitively and accepts `tokens.toml`'s `"bnb"` for BNB
    /// Chain. Derived from [`chain_name`] rather than tabulated again: this
    /// mapping had four copies — here, its inverse below, a `chain_label`
    /// helper in the merge planner, and `tokenTrackingChainFor` in Swift.
    ///
    /// Not every chain can host tracked tokens, so this returns `None` rather
    /// than guessing.
    pub fn from_chain_name(name: &str) -> Option<Self> {
        let needle = name.trim();
        if needle.eq_ignore_ascii_case("bnb") {
            return Some(Self::Bnb);
        }
        Self::ALL
            .iter()
            .copied()
            .find(|chain| chain.chain_name().eq_ignore_ascii_case(needle))
    }

    /// The display name this variant stands for.
    pub const fn chain_name(self) -> &'static str {
        match self {
            Self::Ethereum => "Ethereum",
            Self::Arbitrum => "Arbitrum",
            Self::Optimism => "Optimism",
            Self::Bnb => "BNB Chain",
            Self::Avalanche => "Avalanche",
            Self::Hyperliquid => "Hyperliquid",
            Self::Polygon => "Polygon",
            Self::Base => "Base",
            Self::Linea => "Linea",
            Self::Scroll => "Scroll",
            Self::Blast => "Blast",
            Self::Mantle => "Mantle",
            Self::Solana => "Solana",
            Self::Sui => "Sui",
            Self::Aptos => "Aptos",
            Self::Ton => "TON",
            Self::Near => "NEAR",
            Self::Tron => "Tron",
        }
    }
}


/// Swift `TokenPreferenceCategory` — rawValues: "stablecoin", "meme", "custom".
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
pub enum CoreTokenPreferenceCategory {
    Stablecoin,
    Meme,
    Custom,
}

/// Swift `TokenPreferenceEntry`. UUID id is encoded as its standard string form.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CoreTokenPreferenceEntry {
    pub id: String,
    pub chain: CoreTokenTrackingChain,
    pub name: String,
    pub symbol: String,
    pub token_standard: String,
    pub contract_address: String,
    #[serde(rename = "coinGeckoID")]
    pub coin_gecko_id: String,
    pub decimals: i32,
    pub display_decimals: Option<i32>,
    pub category: CoreTokenPreferenceCategory,
    pub is_built_in: bool,
    pub is_enabled: bool,
}

/// Swift `DashboardAssetChainEntry` — Color omitted (derived in Swift).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CoreDashboardAssetChainEntry {
    pub coin: CoreCoin,
    pub value_usd: Option<f64>,
}

/// Swift `DashboardAssetGroup` — Color omitted (derived from representative coin in Swift).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CoreDashboardAssetGroup {
    pub id: String,
    pub representative_coin: CoreCoin,
    pub total_amount: f64,
    pub total_value_usd: Option<f64>,
    pub chain_entries: Vec<CoreDashboardAssetChainEntry>,
    pub is_pinned: bool,
}

/// Swift `DashboardPinOption` — Color omitted (derived from symbol in Swift).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CoreDashboardPinOption {
    pub symbol: String,
    pub name: String,
    pub subtitle: String,
    pub asset_identifier: Option<String>,
}

/// Swift `WalletRustSecretMaterialDescriptor`. JSON keys preserved for decode compat.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CoreWalletRustSecretMaterialDescriptor {
    #[serde(rename = "walletID")]
    pub wallet_id: String,
    pub secret_kind: String,
    pub has_seed_phrase: bool,
    pub has_private_key: bool,
    pub has_password: bool,
    pub has_signing_material: bool,
    pub seed_phrase_store_key: String,
    pub password_store_key: String,
    pub private_key_store_key: String,
}

#[cfg(test)]
mod roundtrip_tests {
    use super::*;

    #[test]
    fn token_preference_entry_roundtrip_matches_swift_keys() {
        let entry = CoreTokenPreferenceEntry {
            id: "11111111-1111-1111-1111-111111111111".to_string(),
            chain: CoreTokenTrackingChain::Bnb,
            name: "Tether USD".to_string(),
            symbol: "USDT".to_string(),
            token_standard: "BEP-20".to_string(),
            contract_address: "0x55d39897".to_string(),
            coin_gecko_id: "tether".to_string(),
            decimals: 18,
            display_decimals: Some(6),
            category: CoreTokenPreferenceCategory::Stablecoin,
            is_built_in: true,
            is_enabled: true,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"chain\":\"BNB Chain\""));
        assert!(json.contains("\"category\":\"stablecoin\""));
        assert!(json.contains("\"coinGeckoID\""));
        assert!(json.contains("\"isBuiltIn\":true"));
        let decoded: CoreTokenPreferenceEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, entry);
    }

    #[test]
    fn secret_descriptor_decodes_swift_camelcase() {
        let json = r#"{
            "walletID": "w1",
            "secretKind": "seedPhrase",
            "hasSeedPhrase": true,
            "hasPrivateKey": false,
            "hasPassword": true,
            "hasSigningMaterial": true,
            "seedPhraseStoreKey": "wallet.seed.w1",
            "passwordStoreKey": "wallet.seed.password.w1",
            "privateKeyStoreKey": "wallet.privatekey.w1"
        }"#;
        let d: CoreWalletRustSecretMaterialDescriptor = serde_json::from_str(json).unwrap();
        assert_eq!(d.wallet_id, "w1");
        assert!(d.has_password);
    }
}

#[cfg(test)]
mod token_tracking_chain_tests {
    use super::CoreTokenTrackingChain;
    use crate::registry::Chain;

    /// Every chain that can host tracked tokens resolves both ways, and the
    /// name it round-trips through is one the registry recognises.
    #[test]
    fn every_tracking_chain_round_trips_through_the_registry() {
        for variant in [
            CoreTokenTrackingChain::Ethereum,
            CoreTokenTrackingChain::Arbitrum,
            CoreTokenTrackingChain::Optimism,
            CoreTokenTrackingChain::Bnb,
            CoreTokenTrackingChain::Avalanche,
            CoreTokenTrackingChain::Hyperliquid,
            CoreTokenTrackingChain::Polygon,
            CoreTokenTrackingChain::Base,
            CoreTokenTrackingChain::Linea,
            CoreTokenTrackingChain::Scroll,
            CoreTokenTrackingChain::Blast,
            CoreTokenTrackingChain::Mantle,
            CoreTokenTrackingChain::Solana,
            CoreTokenTrackingChain::Sui,
            CoreTokenTrackingChain::Aptos,
            CoreTokenTrackingChain::Ton,
            CoreTokenTrackingChain::Near,
            CoreTokenTrackingChain::Tron,
        ] {
            let name = variant.chain_name();
            assert_eq!(CoreTokenTrackingChain::from_chain_name(name), Some(variant));
            assert!(
                Chain::from_display_name(name).is_some(),
                "{name} is not a chain the registry knows"
            );
        }
    }

    #[test]
    fn a_chain_without_tracked_tokens_has_no_variant() {
        assert_eq!(CoreTokenTrackingChain::from_chain_name("Bitcoin"), None);
        assert_eq!(CoreTokenTrackingChain::from_chain_name("Monero"), None);
    }
}
