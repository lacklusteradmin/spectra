// Domain types for wallet state, ported from Swift CoreModels.swift.
// Color is intentionally omitted — Swift derives display color from symbol.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum CoreBitcoinNetworkMode {
    Mainnet,
    Testnet,
    Testnet4,
    Signet,
}

impl Default for CoreBitcoinNetworkMode {
    fn default() -> Self { Self::Mainnet }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum CoreDogecoinNetworkMode {
    Mainnet,
    Testnet,
}

impl Default for CoreDogecoinNetworkMode {
    fn default() -> Self { Self::Mainnet }
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum CoreSeedDerivationPreset {
    Standard,
    Account1,
    Account2,
}

impl Default for CoreSeedDerivationPreset {
    fn default() -> Self { Self::Standard }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CoreCoin {
    pub id: String,
    pub name: String,
    pub symbol: String,
    pub market_data_id: String,
    pub coin_gecko_id: String,
    pub chain_name: String,
    pub token_standard: String,
    pub contract_address: Option<String>,
    pub amount: f64,
    pub price_usd: f64,
}

/// Power-user derivation overrides, keyed by the same string names as
/// `core/data/derivation_presets.toml`. Every field is optional; `None` means
/// "use the chain preset default." Persisted per-wallet and propagated to
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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CoreSeedDerivationPaths {
    pub is_custom_enabled: bool,
    pub bitcoin: String,
    pub bitcoin_cash: String,
    pub bitcoin_sv: String,
    pub litecoin: String,
    pub dogecoin: String,
    pub ethereum: String,
    pub ethereum_classic: String,
    pub arbitrum: String,
    pub optimism: String,
    pub avalanche: String,
    pub hyperliquid: String,
    pub tron: String,
    pub solana: String,
    pub stellar: String,
    pub xrp: String,
    pub cardano: String,
    pub sui: String,
    pub aptos: String,
    pub ton: String,
    pub internet_computer: String,
    pub near: String,
    pub polkadot: String,
}

// TODO: Align CoreImportedWallet with the single-chain WalletSummary model.
// Per-chain address fields should be replaced with a single `chain_name` +
// `address` pair once Swift is updated to match.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CoreImportedWallet {
    pub id: String,
    pub name: String,
    pub bitcoin_network_mode: CoreBitcoinNetworkMode,
    pub dogecoin_network_mode: CoreDogecoinNetworkMode,
    pub bitcoin_address: Option<String>,
    pub bitcoin_xpub: Option<String>,
    pub bitcoin_cash_address: Option<String>,
    pub bitcoin_sv_address: Option<String>,
    pub litecoin_address: Option<String>,
    pub dogecoin_address: Option<String>,
    pub ethereum_address: Option<String>,
    pub tron_address: Option<String>,
    pub solana_address: Option<String>,
    pub stellar_address: Option<String>,
    pub xrp_address: Option<String>,
    pub monero_address: Option<String>,
    pub cardano_address: Option<String>,
    pub sui_address: Option<String>,
    pub aptos_address: Option<String>,
    pub ton_address: Option<String>,
    pub icp_address: Option<String>,
    pub near_address: Option<String>,
    pub polkadot_address: Option<String>,
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
    #[serde(rename = "marketDataID")]
    pub market_data_id: String,
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
            market_data_id: "825".to_string(),
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
        assert!(json.contains("\"marketDataID\""));
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
