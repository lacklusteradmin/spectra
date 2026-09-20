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
    pub name: String,
    pub symbol: String,
    pub coin_gecko_id: String,
    pub chain_name: String,
    pub token_standard: String,
    pub contract_address: Option<String>,
    pub amount: f64,
    pub price_usd: f64,
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

    pub fn network(&self) -> Option<crate::registry::Chain> {
        crate::registry::Chain::from_display_name(&self.chain_name)
            .or_else(|| crate::registry::Chain::from_str_id(&self.chain_name))
    }

    pub fn deployment_key(&self) -> String {
        let network = self
            .network()
            .map(|c| c.str_id())
            .unwrap_or(&self.chain_name);
        if self.is_native() {
            return format!("{network}:native");
        }
        let contract = crate::tokens::normalize_token_identifier(
            self.contract_address.clone(),
            self.chain_name.clone(),
        )
        .unwrap_or_default();
        format!(
            "{network}:{}:{contract}",
            self.token_standard.to_lowercase()
        )
    }

    pub fn catalog_token(&self) -> Option<&'static crate::tokens::TokenEntry> {
        let key = self.deployment_key();
        crate::tokens::catalog().iter().find(|t| t.id == key)
    }

    /// Validate identity before persistence and derive catalog-owned display facts.
    pub fn canonicalize(&mut self) -> Result<(), String> {
        let network = self.network().ok_or("unknown holding network")?;
        if !self.amount.is_finite() || self.amount < 0.0 {
            return Err("invalid holding amount".into());
        }
        self.chain_name = network.chain_display_name().into();
        self.contract_address = crate::tokens::normalize_token_identifier(
            self.contract_address.clone(),
            self.chain_name.clone(),
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
            let hosting = CoreTokenHostingChain::from_chain_name(&self.chain_name)
                .ok_or("network does not support tracked tokens")?;
            if self.token_standard != hosting.token_standard() {
                return Err("token protocol does not match network".into());
            }
            if !crate::validation::address::validate_address(
                crate::validation::address::AddressValidationRequest {
                    kind: hosting.contract_validation_kind().into(),
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
            self.coin_gecko_id = token.coingecko_id.clone();
        } else {
            // Caller-provided market ids must never price or merge an unverified asset.
            self.coin_gecko_id.clear();
        }
        if network.is_testnet() {
            self.coin_gecko_id.clear();
            self.price_usd = 0.0;
        }
        Ok(())
    }

    pub fn token_identity(&self) -> String {
        self.catalog_token()
            .map(|t| t.token_id.clone())
            .unwrap_or_else(|| format!("custom:{}", self.deployment_key()))
    }
}

/// Exact derivation secrets supported consistently by import and signing.
/// Algorithms and iteration settings come from the chain and derivation path.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CoreWalletDerivationOverrides {
    #[serde(default)]
    pub passphrase: Option<String>,
    #[serde(default)]
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
        self.by_chain.insert(
            chain.mainnet_counterpart().str_id().to_string(),
            path.into(),
        );
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
    pub network_chain_id: String,
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
    pub holdings: Vec<AssetHolding>,
    pub include_in_portfolio_total: bool,
}

impl WalletView {
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

// ── WalletView ↔ WalletState ───────────────────────────────────────
//
// `WalletState` is the model core computes with; `WalletView` is the
// shape the iOS app still uses. The conversion exists so the two can coexist
// while the app migrates, and it is **deliberately asymmetric**:
//
// A `WalletView` carries the whole 45-entry derivation-path table and
// two network-mode fields on *every* wallet, even though a wallet belongs to
// one chain and uses one path on one network. Converting to `WalletState`
// keeps the entry that wallet actually uses and drops the other 44 — they are
// global defaults, not per-wallet data. Converting back therefore cannot
// reconstruct them, and rebuilds the table from the defaults instead.
//
// That asymmetry is the point, not a defect: the round trip losing redundant
// copies is what makes `WalletState` the smaller, correcter model.

impl WalletView {
    /// The chain this wallet is actually on, as a registry id.
    ///
    /// `selected_chain` names the family; this says which network of it. They
    /// used to be a family name plus one of two mode enums, chosen by matching
    /// the family name — so the answer lived in three places at once.
    fn active_network_chain_id(&self) -> Option<String> {
        use crate::registry::Chain;
        let family = Chain::from_display_name(&self.selected_chain)?.mainnet_counterpart();
        let selected = Chain::from_str_id(&self.network_chain_id)?;
        // Scoped to the wallet's own family: a wallet on Solana reports no
        // network even if a Bitcoin one was selected when it was imported.
        (selected.mainnet_counterpart() == family).then(|| selected.str_id().to_string())
    }

    /// Convert to the model core computes with.
    ///
    /// `is_watch_only` cannot be read off this record — the app derives it from
    /// whether the Keychain holds signing material — so the caller supplies it.
    pub fn to_wallet_state(&self, is_watch_only: bool) -> crate::store::state::WalletState {
        use crate::registry::Chain;
        use crate::store::state::{WalletAddress, WalletState};

        let chain = Chain::from_display_name(&self.selected_chain);
        let derivation_path = chain.and_then(|chain| {
            self.seed_derivation_paths
                .path_for(chain)
                .map(str::to_string)
        });

        WalletState {
            id: self.id.clone(),
            name: self.name.clone(),
            is_watch_only,
            chain_name: self.selected_chain.clone(),
            include_in_portfolio_total: self.include_in_portfolio_total,
            network_id: self
                .active_network_chain_id()
                .or_else(|| chain.map(|c| c.str_id().into()))
                .unwrap_or_default(),
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
                let own_slot = chain.map(|chain| chain.address_slot());
                let mut slots: Vec<(&str, &String)> = self
                    .addresses
                    .iter()
                    .map(|(slot, address)| (slot.as_str(), address))
                    .collect();
                slots.sort_by_key(|(slot, _)| (Some(*slot) != own_slot, *slot));
                slots
                    .into_iter()
                    .filter_map(|(slot, address)| {
                        let owner =
                            Chain::all().find(|candidate| candidate.address_slot() == slot)?;
                        Some(WalletAddress {
                            chain_name: owner.chain_display_name().to_string(),
                            address: address.clone(),
                            kind: "receive".to_string(),
                            derivation_path: derivation_path.clone(),
                        })
                    })
                    .collect()
            },
        }
    }
}

impl crate::store::state::WalletState {
    /// Convert back into the shape the iOS app renders.
    ///
    /// The reverse of [`WalletView::to_wallet_state`], and lossy in the
    /// direction that does not matter: the 45-entry derivation-path table is
    /// rebuilt from `defaults` with this wallet's own path written over its
    /// chain's slot. Those defaults were never per-wallet data.
    ///
    /// `WalletState` remains the authority. This produces a view model.
    pub fn to_wallet_view(&self, defaults: &CoreSeedDerivationPaths) -> WalletView {
        use crate::registry::Chain;

        let chain = Chain::from_display_name(&self.chain_name);
        let mut seed_derivation_paths = defaults.clone();
        if let (Some(chain), Some(path)) = (chain, self.derivation_path.as_deref()) {
            seed_derivation_paths.set_path_for(chain, path);
        }

        WalletView {
            id: self.id.clone(),
            name: self.name.clone(),
            network_chain_id: self.network_id.clone(),
            addresses: self
                .addresses
                .iter()
                .filter_map(|entry| {
                    Chain::from_display_name(&entry.chain_name)
                        .map(|chain| (chain.address_slot().to_string(), entry.address.clone()))
                })
                .collect(),
            bitcoin_xpub: self.xpub.clone(),
            seed_derivation_preset: self.derivation_preset,
            seed_derivation_paths,
            derivation_overrides: self.derivation_overrides.clone(),
            selected_chain: self.chain_name.clone(),
            holdings: self
                .holdings
                .iter()
                .map(|holding| AssetHolding {
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

/// Stable deployment identity: network, explicit native/protocol type and identifier.
///
/// The front end's list key. It used to be an `id` field on the record, filled
/// by whoever built it — five different formats across the callers, one of them
/// a fresh `UUID` per build, which makes SwiftUI treat every row as new and
/// re-animate the whole list. Derived from the holding, it cannot drift.
#[uniffi::export]
pub fn holding_identity(holding: &crate::store::wallet_domain::AssetHolding) -> String {
    holding.deployment_key()
}

/// Swift `TokenHostingChain` — rawValues are chain display names.
///
/// Exactly the chains `chains.toml` gives a `token_standard`, which is the
/// fact this used to disagree with: eighteen of the twenty-eight were listed,
/// so a catalog row on Berachain or Ink was dropped from
/// `built_in_token_preferences` and a custom token could not be added there.
/// `the_hosting_chains_are_the_chains_with_a_token_standard` holds the two
/// together.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum CoreTokenHostingChain {
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
    #[serde(rename = "Sei")]
    Sei,
    #[serde(rename = "Celo")]
    Celo,
    #[serde(rename = "Cronos")]
    Cronos,
    #[serde(rename = "opBNB")]
    OpBnb,
    #[serde(rename = "zkSync Era")]
    ZkSyncEra,
    #[serde(rename = "Sonic")]
    Sonic,
    #[serde(rename = "Berachain")]
    Berachain,
    #[serde(rename = "Unichain")]
    Unichain,
    #[serde(rename = "Ink")]
    Ink,
    #[serde(rename = "X Layer")]
    XLayer,
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

impl CoreTokenHostingChain {
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
        Self::Sei,
        Self::Celo,
        Self::Cronos,
        Self::OpBnb,
        Self::ZkSyncEra,
        Self::Sonic,
        Self::Berachain,
        Self::Unichain,
        Self::Ink,
        Self::XLayer,
        Self::Solana,
        Self::Sui,
        Self::Aptos,
        Self::Ton,
        Self::Near,
        Self::Tron,
    ];

    /// The chain a known token belongs to, from its display name.
    ///
    /// Matches case-insensitively and accepts `tokens.toml`'s `"bnb"` for BNB
    /// Chain. Derived from [`chain_name`] rather than tabulated again: this
    /// mapping had four copies — here, its inverse below, a `chain_label`
    /// helper in the merge planner, and `tokenTrackingChainFor` in Swift.
    ///
    /// Not every chain can host known tokens, so this returns `None` rather
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
            Self::Sei => "Sei",
            Self::Celo => "Celo",
            Self::Cronos => "Cronos",
            Self::OpBnb => "opBNB",
            Self::ZkSyncEra => "zkSync Era",
            Self::Sonic => "Sonic",
            Self::Berachain => "Berachain",
            Self::Unichain => "Unichain",
            Self::Ink => "Ink",
            Self::XLayer => "X Layer",
            Self::Solana => "Solana",
            Self::Sui => "Sui",
            Self::Aptos => "Aptos",
            Self::Ton => "TON",
            Self::Near => "NEAR",
            Self::Tron => "Tron",
        }
    }

    /// The catalog's token standard for this chain, e.g. `SPL Token` for
    /// Solana. Read from the registry entry rather than tabulated again.
    pub fn token_standard(self) -> String {
        crate::registry::Chain::from_display_name(self.chain_name())
            .map(|chain| chain.entry().token_standard.clone())
            .unwrap_or_default()
    }

    /// Which validator a *token contract* on this chain is judged by.
    ///
    /// Not the same question as [`crate::registry::Chain::address_validation_kind`]
    /// for two of them: a Sui or Aptos token is named by a coin *type*
    /// (`0xADDR::module::NAME`), not by an address, and a package address is
    /// only the degenerate case of one. Everywhere else the contract is an
    /// address in the chain's own format.
    ///
    /// Swift wrote this as a seven-arm switch with a `default` that assumed
    /// EVM, and the CLI did not check the contract at all. Stating it here
    /// means a chain joining the hosting list arrives with its validator
    /// rather than falling into whichever arm was written last.
    pub fn contract_validation_kind(self) -> &'static str {
        match self {
            Self::Solana => "solana",
            Self::Sui => "suiCoinType",
            Self::Aptos => "aptosTokenType",
            Self::Ton => "ton",
            Self::Near => "near",
            Self::Tron => "tron",
            // Every remaining variant is an EVM chain, which the registry is
            // the authority on — asking it keeps the two from drifting.
            other => crate::registry::Chain::from_display_name(other.chain_name())
                .map(crate::registry::Chain::address_validation_kind)
                .unwrap_or("evm"),
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
/// `contract_address` for `contract`, `coin_gecko_id` for `coingecko_id`,
/// `decimals: i32` for `decimals: u32` — so a token had four spellings of its
/// contract across the catalog, the state, the Swift mirror and the fetch
/// descriptor. It embeds the token now: there is one spelling because there is
/// one record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CoreTokenPreferenceEntry {
    pub token: crate::tokens::TokenEntry,
    pub category: CoreTokenPreferenceCategory,
    /// The catalog ships it; the user cannot edit or delete it.
    pub is_built_in: bool,
    pub is_enabled: bool,
}

impl CoreTokenPreferenceEntry {
    /// Identity: a token *is* its contract on its chain. The id used to be a
    /// stored `builtin:{chain}:{contract}` string, regenerated on every launch.
    pub fn id(&self) -> String {
        format!("{}|{}", self.token.chain, self.token.contract)
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

    pub fn hosting_chain(&self) -> Option<CoreTokenHostingChain> {
        CoreTokenHostingChain::from_chain_name(&self.token.chain)
    }
}

/// One place an asset is held: a chain, a token standard, a contract.
#[derive(Debug, Clone, PartialEq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CoreDashboardAssetHolding {
    pub coin: AssetHolding,
    pub value_usd: Option<f64>,
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
    pub total_value_usd: Option<f64>,
    pub id: String,
    /// What the row calls itself and prices itself by: the largest place it is
    /// held, or the catalog's entry for it when it is held nowhere. An identity,
    /// not a place — read `holdings` for those.
    pub identity: AssetHolding,
    pub holdings: Vec<CoreDashboardAssetHolding>,
    pub is_pinned: bool,
}

/// Swift `DashboardPinOption` — Color omitted (derived from symbol in Swift).
#[derive(Debug, Clone, PartialEq, Serialize, uniffi::Record)]
pub struct CoreDashboardPinOption {
    pub token_id: String,
    pub symbol: String,
    pub name: String,
    pub subtitle: String,
    pub artwork_name: Option<String>,
    /// Whether this asset is in the saved dashboard pin selection.
    pub is_pinned: bool,
}

/// What signing material a wallet has, and whether a password guards it.
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
            token: crate::tokens::TokenEntry {
                id: "fixture:token".into(),
                token_id: "fixture:token".into(),
                kind: crate::tokens::TokenKind::Protocol {
                    standard: "fixture".into(),
                    identifier: "fixture".into(),
                },
                chain: "BNB Chain".to_string(),
                name: "Tether USD".to_string(),
                symbol: "USDT".to_string(),
                token_standard: "BEP-20".to_string(),
                contract: "0x55d39897".to_string(),
                coingecko_id: "tether".to_string(),
                decimals: 18,
                tags: vec!["stablecoin".to_string()],
                color: Some(crate::chains::CatalogColor::Green),
                artwork_name: "usdt".to_string(),
                enabled: true,
            },
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"chain\":\"BNB Chain\""));
        assert!(json.contains("\"category\":\"stablecoin\""));
        assert!(json.contains("\"coingeckoId\""));
        assert!(json.contains("\"isBuiltIn\":true"));
        let decoded: CoreTokenPreferenceEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, entry);

        // Identity is the token's, not a stored string. It used to be a
        // `builtin:{chain}:{contract}` id regenerated on every launch.
        assert_eq!(entry.id(), "BNB Chain|0x55d39897");
        // And the category the tags imply, rather than a second copy of it.
        assert_eq!(
            CoreTokenPreferenceEntry::category_from_tags(&entry.token.tags),
            entry.category
        );
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
mod token_hosting_chain_tests {
    use super::CoreTokenHostingChain;
    use crate::registry::Chain;

    /// Every chain that can host known tokens resolves both ways, and the
    /// name it round-trips through is one the registry recognises. The list
    /// this walks is `ALL` rather than a copy of it: the copy was the reason
    /// adding a variant left a test still asserting eighteen.
    #[test]
    fn every_tracking_chain_round_trips_through_the_registry() {
        for variant in CoreTokenHostingChain::ALL {
            let name = variant.chain_name();
            assert_eq!(CoreTokenHostingChain::from_chain_name(name), Some(*variant));
            assert!(
                Chain::from_display_name(name).is_some(),
                "{name} is not a chain the registry knows"
            );
        }
    }

    /// The variants are exactly the chains `chains.toml` gives a token
    /// standard to. Hosting is not a second opinion about which chains carry
    /// tokens; it is that column, spelled as an enum for the FFI.
    #[test]
    fn the_hosting_chains_are_the_chains_with_a_token_standard() {
        let listed: std::collections::BTreeSet<&str> = CoreTokenHostingChain::ALL
            .iter()
            .map(|c| c.chain_name())
            .collect();
        let with_standard: std::collections::BTreeSet<&str> = crate::chains::catalog()
            .iter()
            .filter(|c| !c.token_standard.is_empty())
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(listed, with_standard);
    }

    #[test]
    fn a_chain_without_tracked_tokens_has_no_variant() {
        assert_eq!(CoreTokenHostingChain::from_chain_name("Bitcoin"), None);
        assert_eq!(CoreTokenHostingChain::from_chain_name("Monero"), None);
    }
}
