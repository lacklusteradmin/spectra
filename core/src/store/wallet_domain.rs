// Domain types for wallet state, ported from Swift CoreModels.swift.
// Color is intentionally omitted — Swift derives display color from symbol.

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
