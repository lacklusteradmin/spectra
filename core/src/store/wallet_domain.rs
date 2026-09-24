// Wallet value types crossing the FFI. Display color is deliberately absent:
// the platform derives it from the asset symbol.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

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

impl CoreTransactionStatus {
    /// The stored and wire spelling. Four functions used to spell these three
    /// words, two of them disagreeing about what a missing status meant.
    pub fn as_raw(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Confirmed => "confirmed",
            Self::Failed => "failed",
        }
    }

    /// The inverse, for a status read back from a string. `None` means the
    /// string names no status; what to do about that is the caller's rule.
    pub(crate) fn from_raw(raw: &str) -> Option<Self> {
        match raw {
            "pending" => Some(Self::Pending),
            "confirmed" => Some(Self::Confirmed),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
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

impl CoreSeedDerivationPreset {
    /// The BIP-44 account a preset's default paths use.
    ///
    /// Stated once. The wallet record stored the preset as a string, and three
    /// services each matched `"account1"` and `"account2"` back into an index
    /// while the app kept a fourth copy to call the export with.
    pub fn account_index(self) -> u32 {
        match self {
            Self::Standard => 0,
            Self::Account1 => 1,
            Self::Account2 => 2,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AssetHolding {
    /// The deployment this is a holding of — `deployment_id()`, carried so a
    /// front end can key rows by it without asking. Derived, so never stored:
    /// every projection a front end reads fills it in.
    #[serde(default, skip_serializing)]
    pub id: String,
    pub name: String,
    pub symbol: String,
    pub coingecko_id: String,
    pub chain_id: String,
    pub token_standard: String,
    pub contract_address: Option<String>,
    /// The balance, as an exact decimal in the asset's own units.
    pub amount: String,
}

impl AssetHolding {
    /// Native is an explicit protocol type, never a symbol comparison.
    pub fn is_native(&self) -> bool {
        self.token_standard == "Native"
            && self
                .contract_address
                .as_deref()
                .is_none_or(|c| c.is_empty())
    }

    pub fn chain(&self) -> Option<crate::registry::Chain> {
        crate::registry::Chain::from_str_id(&self.chain_id)
    }

    pub fn deployment_id(&self) -> String {
        let network = &self.chain_id;
        if self.is_native() {
            return format!("{network}:native");
        }
        let contract = crate::tokens::normalize_token_identifier(
            self.contract_address.clone(),
            self.chain_id.clone(),
        )
        .unwrap_or_default();
        format!(
            "{network}:{}:{contract}",
            self.token_standard.to_lowercase()
        )
    }

    pub fn catalog_token(&self) -> Option<&'static crate::tokens::TokenDeploymentEntry> {
        let key = self.deployment_id();
        crate::tokens::catalog()
            .iter()
            .find(|t| t.deployment_id == key)
    }

    /// This holding with its `id` filled in.
    pub fn identified(mut self) -> Self {
        self.id = self.deployment_id();
        self
    }

    /// Validate identity before persistence and derive catalog-owned display facts.
    pub fn canonicalize(&mut self) -> Result<(), String> {
        let network = self.chain().ok_or("unknown holding network")?;
        self.amount = crate::decimal::canonical(&self.amount).ok_or("invalid holding amount")?;
        self.contract_address = crate::tokens::normalize_token_identifier(
            self.contract_address.clone(),
            self.chain_id.clone(),
        );
        if self.token_standard == "Native" {
            if self.contract_address.is_some() {
                return Err("native token cannot carry a contract".into());
            }
        } else {
            let contract = self
                .contract_address
                .as_ref()
                .ok_or("protocol token requires an identifier")?;
            if !network.hosts_tokens() {
                return Err("network does not support tracked tokens".into());
            }
            if self.token_standard != network.token_standard() {
                return Err("token protocol does not match network".into());
            }
            if !crate::validation::address::validate_address(
                crate::validation::address::AddressValidationRequest {
                    kind: network.contract_validation_kind().into(),
                    value: contract.clone(),
                },
            )
            .is_valid
            {
                return Err("invalid token identifier".into());
            }
        }
        if let Some(token) = self.catalog_token() {
            self.name = token.name.clone();
            self.symbol = token.symbol.clone();
            self.coingecko_id = token.coingecko_id.clone();
        } else {
            // Caller-provided market ids must never price or merge an unverified asset.
            self.coingecko_id.clear();
        }
        if network.is_testnet() {
            self.coingecko_id.clear();
        }
        self.id = self.deployment_id();
        Ok(())
    }

    pub fn token_identity(&self) -> String {
        self.catalog_token()
            .map(|t| t.token_id.clone())
            .unwrap_or_else(|| format!("custom:{}", self.deployment_id()))
    }
}

/// Exact derivation secrets supported consistently by import and signing.
/// Algorithms and iteration settings come from the chain and derivation path.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CoreWalletDerivationOverrides {
    pub passphrase: Option<String>,
    pub hmac_key: Option<String>,
}

impl CoreWalletDerivationOverrides {
    pub fn validate_for_chain(
        &self,
        chain: crate::registry::Chain,
    ) -> Result<(), crate::SpectraBridgeError> {
        if self.passphrase.as_ref().is_some_and(|s| !s.is_empty())
            && !chain.supports_derivation_passphrase()
        {
            return Err(format!(
                "{} does not support a derivation passphrase",
                chain.chain_display_name()
            )
            .into());
        }
        if self.hmac_key.as_ref().is_some_and(|s| !s.is_empty())
            && !chain.supports_derivation_hmac_override()
        {
            return Err(format!(
                "{} does not support a custom HMAC key",
                chain.chain_display_name()
            )
            .into());
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.passphrase.is_none() && self.hmac_key.is_none()
    }

    pub(crate) fn zeroize_sensitive_fields(&mut self) {
        if let Some(value) = &mut self.passphrase {
            value.zeroize();
        }
        if let Some(value) = &mut self.hmac_key {
            value.zeroize();
        }
    }
}

/// A wallet's derivation overrides, wiped when they go out of scope.
///
/// The passphrase and HMAC key are derivation secrets, and a
/// cloned `WalletState` carries them in the clear — so whatever takes them
/// out of one owes them a wipe. Two paths derive from a stored wallet, the
/// send identity and Bitcoin's history xpub, and this is how both hold them.
pub(crate) struct SensitiveOverrides(pub(crate) CoreWalletDerivationOverrides);

impl SensitiveOverrides {
    /// Take the overrides out of a wallet record, leaving it with none.
    pub(crate) fn take_from(wallet: &mut crate::store::state::WalletState) -> Self {
        Self(std::mem::take(&mut wallet.derivation_overrides))
    }

    /// The BIP39 passphrase, when there is a non-empty one.
    ///
    /// It belongs to the derivation as much as the phrase does: without it a
    /// seed derives a different wallet's keys entirely.
    pub(crate) fn passphrase(&self) -> Option<&str> {
        self.0
            .passphrase
            .as_deref()
            .filter(|value| !value.is_empty())
    }
}

impl Drop for SensitiveOverrides {
    fn drop(&mut self) {
        self.0.zeroize_sensitive_fields();
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CoreSeedDerivationPaths {
    pub is_custom_enabled: bool,
    /// Concrete network ID → derivation path. Mainnet and testnet overrides
    /// are independent, even when their default paths happen to match.
    pub by_chain: HashMap<String, String>,
}

impl CoreSeedDerivationPaths {
    /// Derivation path configured for this exact network.
    pub fn path_for(&self, chain: crate::registry::Chain) -> Option<&str> {
        self.by_chain.get(chain.str_id()).map(String::as_str)
    }

    /// Set the path for this exact network.
    pub fn set_path_for(&mut self, chain: crate::registry::Chain, path: impl Into<String>) {
        self.by_chain
            .insert(chain.str_id().to_string(), path.into());
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletView {
    pub id: String,
    pub name: String,
    /// The network this wallet is on, as a registry chain id — the chain's
    /// own id on a family with one network. Replaced a
    /// `CoreBitcoinNetworkMode` and a `CoreDogecoinNetworkMode` field, which
    /// were two enums spelling out chains the registry already has.
    ///
    /// Was optional, though every wallet core produces has one, so each
    /// reader supplied its own fallback.
    pub chain_id: String,
    /// `Chain::address_slot()` → address for this wallet.
    ///
    /// A wallet belongs to one chain (`chain_id`), so in practice this
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
    pub derivation_overrides: CoreWalletDerivationOverrides,
    pub holdings: Vec<AssetHolding>,
    pub include_in_portfolio_total: bool,
    /// What the wallet signs with and whether a password guards it — read
    /// from the wallet record, so rendering one touches no secret store.
    pub signing: crate::store::state::WalletSigning,
}

impl WalletView {
    /// This wallet's address for `chain`, if it has one.
    pub fn address_for(&self, chain: crate::registry::Chain) -> Option<&str> {
        self.addresses.get(chain.address_slot()).map(String::as_str)
    }

    /// The address for the wallet's own chain — what the UI shows and what
    /// balance/history calls query.
    pub fn primary_address(&self) -> Option<&str> {
        crate::registry::Chain::from_str_id(&self.chain_id)
            .and_then(|chain| self.address_for(chain))
    }
}

// ── WalletView ↔ WalletState ───────────────────────────────────────
//
// `WalletState` owns persisted wallet facts; `WalletView` projects them for the
// native UI. Both identify the selected chain with `chain_id`. View derivation
// defaults are rebuilt from the catalog; only the wallet's actual path is stored.

impl WalletView {
    /// Convert to the model core computes with.
    ///
    /// The import operation supplies signing capability. Network identity must
    /// be valid before a state can be stored.
    pub fn to_wallet_state(
        &self,
    ) -> Result<crate::store::state::WalletState, crate::SpectraBridgeError> {
        use crate::registry::Chain;
        use crate::store::state::{WalletAddress, WalletState};

        let invalid = |message: String| crate::SpectraBridgeError::InvalidInput { message };
        let chain = Chain::from_str_id(&self.chain_id)
            .ok_or_else(|| invalid(format!("unknown wallet network: {}", self.chain_id)))?;
        let derivation_path = self
            .seed_derivation_paths
            .path_for(chain)
            .map(str::to_string);

        Ok(WalletState {
            id: self.id.clone(),
            name: self.name.clone(),
            signing: self.signing,
            chain_id: chain.str_id().to_string(),
            include_in_portfolio_total: self.include_in_portfolio_total,
            xpub: self.bitcoin_xpub.clone(),
            derivation_preset: self.seed_derivation_preset,
            derivation_path: derivation_path.clone(),
            derivation_overrides: self.derivation_overrides.clone(),
            holdings: self.holdings.clone(),
            // Every slot this wallet holds, not only its own chain's. A wallet
            // on a family that has testnets holds one address per network, and
            // dropping the rest here is what left a network switch re-deriving
            // from the seed on every read — which a sealed wallet cannot do, so
            // it silently showed the mainnet address instead.
            //
            // The wallet's own slot comes first and the rest follow by slot id:
            // `primary_address` takes the first `receive` entry, and a
            // `HashMap`'s order would make that whichever network the iterator
            // happened to yield.
            addresses: {
                let own_slot = chain.address_slot();
                let mut slots: Vec<(&str, &String)> = self
                    .addresses
                    .iter()
                    .map(|(slot, address)| (slot.as_str(), address))
                    .collect();
                slots.sort_by_key(|(slot, _)| (*slot != own_slot, *slot));
                slots
                    .into_iter()
                    .filter_map(|(slot, address)| {
                        let owner =
                            Chain::all().find(|candidate| candidate.address_slot() == slot)?;
                        Some(WalletAddress {
                            chain_id: owner.str_id().to_string(),
                            address: address.clone(),
                            kind: "receive".to_string(),
                            derivation_path: self
                                .seed_derivation_paths
                                .path_for(owner)
                                .map(str::to_string),
                        })
                    })
                    .collect()
            },
        })
    }
}

impl crate::store::state::WalletState {
    /// Convert back into the shape the iOS app renders.
    ///
    /// The reverse of [`WalletView::to_wallet_state`], and lossy in the
    /// direction that does not matter: network defaults are overlaid with
    /// the explicit paths stored on addresses and the active wallet path.
    ///
    /// `WalletState` remains the authority. This produces a view model.
    pub fn to_wallet_view(&self, defaults: &CoreSeedDerivationPaths) -> WalletView {
        use crate::registry::Chain;

        let chain = Chain::from_str_id(&self.chain_id);
        let mut seed_derivation_paths = defaults.clone();
        for address in &self.addresses {
            if let (Some(network), Some(path)) = (
                Chain::from_str_id(&address.chain_id),
                address.derivation_path.as_deref(),
            ) {
                seed_derivation_paths.set_path_for(network, path);
            }
        }
        if let (Some(chain), Some(path)) = (chain, self.derivation_path.as_deref()) {
            seed_derivation_paths.set_path_for(chain, path);
        }

        WalletView {
            id: self.id.clone(),
            name: self.name.clone(),
            chain_id: self.chain_id.clone(),
            addresses: self
                .addresses
                .iter()
                .filter_map(|entry| {
                    Chain::from_str_id(&entry.chain_id)
                        .map(|chain| (chain.address_slot().to_string(), entry.address.clone()))
                })
                .collect(),
            bitcoin_xpub: self.xpub.clone(),
            seed_derivation_preset: self.derivation_preset,
            seed_derivation_paths,
            derivation_overrides: self.derivation_overrides.clone(),
            holdings: self
                .holdings
                .iter()
                .cloned()
                .map(AssetHolding::identified)
                .collect(),
            include_in_portfolio_total: self.include_in_portfolio_total,
            signing: self.signing,
        }
    }
}

/// Token preference categories for built-in and user-added tokens.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
pub enum CoreTokenPreferenceCategory {
    Stablecoin,
    Meme,
    Custom,
}

/// A token the app knows about, and what the user has done to it.
///
/// Held seven copies of the catalog's fields under different names —
/// `contract_address` for `contract`, `coingecko_id` for `coingecko_id`,
/// `decimals: i32` for `decimals: u32` — so a token had four spellings of its
/// contract across the catalog, the state, the Swift mirror and the fetch
/// descriptor. It embeds the token now: there is one spelling because there is
/// one record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CoreTokenPreferenceEntry {
    pub token: crate::tokens::TokenDeploymentEntry,
    pub category: CoreTokenPreferenceCategory,
    /// The catalog ships it; the user cannot edit or delete it.
    pub is_built_in: bool,
    pub is_enabled: bool,
}

impl CoreTokenPreferenceEntry {
    /// Identity: a token *is* its contract on its chain. The id used to be a
    /// stored `builtin:{chain}:{contract}` string, regenerated on every launch.
    pub fn id(&self) -> String {
        format!("{}|{}", self.token.chain_id, self.token.contract)
    }

    /// The category the catalog's tags imply. It was stored beside the tags it
    /// is computed from, which is a second encoding of one fact.
    pub fn category_from_tags(tags: &[String]) -> CoreTokenPreferenceCategory {
        tags.iter()
            .find_map(|tag| match tag.as_str() {
                "stablecoin" => Some(CoreTokenPreferenceCategory::Stablecoin),
                "meme" => Some(CoreTokenPreferenceCategory::Meme),
                _ => None,
            })
            .unwrap_or(CoreTokenPreferenceCategory::Custom)
    }

    /// The chain hosting this token.
    pub fn hosting_chain(&self) -> Option<crate::registry::Chain> {
        crate::registry::Chain::from_str_id(&self.token.chain_id).filter(|c| c.hosts_tokens())
    }
}

/// One place an asset is held: a chain, a token standard, a contract.
#[derive(Debug, Clone, PartialEq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CoreDashboardAssetHolding {
    pub coin: AssetHolding,
    /// In the display currency; `None` when unpriced.
    pub value: Option<f64>,
}

/// One dashboard row: an asset, and everywhere it is held.
///
/// A row is per **asset**, not per (chain, asset) — ETH on Ethereum and ETH on
/// Arbitrum are one row.
///
/// `holdings` is **where the user holds it**, largest value first, and it is
/// empty for a pinned asset they hold nowhere. It used to carry a synthesized
/// holding in that case so the row had something to name itself with, which
/// said the user held zero of the asset on whichever chain the catalog listed
/// first — a place they had never been shown and could not have chosen. The
/// naming job is `identity`'s now, and `holdings` states only what is true.
///
/// Valuation is derived with the holdings in the same snapshot; it is not
/// persisted beside them. None means at least one place is unpriced.
#[derive(Debug, Clone, PartialEq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CoreDashboardAssetGroup {
    /// Everything held across `holdings`, as an exact decimal.
    pub total_amount: String,
    /// In the display currency; `None` when any place is unpriced.
    pub total_value: Option<f64>,
    /// One unit of the asset in the display currency, from `identity`.
    pub price: Option<f64>,
    pub id: String,
    /// What the row calls itself and prices itself by: the largest place it is
    /// held, or the catalog's entry for it when it is held nowhere. An identity,
    /// not a place — read `holdings` for those.
    pub identity: AssetHolding,
    pub holdings: Vec<CoreDashboardAssetHolding>,
    pub is_pinned: bool,
}

/// An asset the dashboard can pin. `deployment_id` is the place it is drawn
/// from — the colour and artwork follow the deployment, never the ticker.
#[derive(Debug, Clone, PartialEq, Serialize, uniffi::Record)]
pub struct CoreDashboardPinOption {
    pub token_id: String,
    pub deployment_id: String,
    pub symbol: String,
    pub name: String,
    pub subtitle: String,
    pub artwork_name: Option<String>,
    /// Whether this asset is in the saved dashboard pin selection.
    pub is_pinned: bool,
}

#[cfg(test)]
mod roundtrip_tests {
    use super::*;

    #[test]
    fn token_preference_entry_roundtrip_matches_swift_keys() {
        let entry = CoreTokenPreferenceEntry {
            category: CoreTokenPreferenceCategory::Stablecoin,
            is_built_in: true,
            is_enabled: true,
            token: crate::tokens::TokenDeploymentEntry {
                deployment_id: "fixture:token".into(),
                token_id: "fixture:token".into(),
                kind: crate::tokens::TokenKind::Protocol {
                    standard: "fixture".into(),
                    identifier: "fixture".into(),
                },
                chain_id: "bnb-chain".to_string(),
                name: "Tether USD".to_string(),
                symbol: "USDT".to_string(),
                token_standard: "BEP-20".to_string(),
                contract: "0x55d39897".to_string(),
                coingecko_id: "tether".to_string(),
                coinpaprika_id: String::new(),
                decimals: 18,
                tags: vec!["stablecoin".to_string()],
                color: Some(crate::chains::CatalogColor::Green),
                artwork_name: "usdt".to_string(),
                enabled: true,
            },
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"chainId\":\"bnb-chain\""));
        assert!(json.contains("\"category\":\"stablecoin\""));
        assert!(json.contains("\"coingeckoId\""));
        assert!(json.contains("\"isBuiltIn\":true"));
        let decoded: CoreTokenPreferenceEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, entry);

        // Identity is the token's, not a stored string. It used to be a
        // `builtin:{chain}:{contract}` id regenerated on every launch.
        assert_eq!(entry.id(), "bnb-chain|0x55d39897");
        // And the category the tags imply, rather than a second copy of it.
        assert_eq!(
            CoreTokenPreferenceEntry::category_from_tags(&entry.token.tags),
            entry.category
        );
    }
}
