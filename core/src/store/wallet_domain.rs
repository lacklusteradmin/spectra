// Wallet value types crossing the FFI. Display color is deliberately absent:
// the platform derives it from the asset symbol.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// The operation represented by a stored transaction.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum TransactionKind {
    Send,
    Receive,
    Stake,
    Unstake,
    Withdraw,
    ClaimRewards,
    /// Setting an ERC-20 allowance the wallet gave back to zero.
    RevokeApproval,
    /// Deleting one of a NEAR account's function-call keys.
    DeleteAccessKey,
    /// Merging a Sui coin type's objects into one.
    MergeCoins,
    /// Closing empty Solana token accounts, returning their rent.
    CloseTokenAccounts,
    /// Unregistering a NEAR account from a token it holds none of,
    /// returning the storage deposit.
    RefundTokenStorage,
    /// Opening a trust line so the account can hold an issued asset.
    TrustAsset,
    /// Removing an empty trust line, freeing its reserve.
    RemoveTrustLine,
    /// Moving a Zcash wallet's transparent funds into its own shielded pool.
    Shield,
}
impl TransactionKind {
    pub fn as_raw(self) -> &'static str {
        match self {
            Self::Send => "send",
            Self::Receive => "receive",
            Self::Stake => "stake",
            Self::Unstake => "unstake",
            Self::Withdraw => "withdraw",
            Self::ClaimRewards => "claimRewards",
            Self::RevokeApproval => "revokeApproval",
            Self::DeleteAccessKey => "deleteAccessKey",
            Self::MergeCoins => "mergeCoins",
            Self::CloseTokenAccounts => "closeTokenAccounts",
            Self::RefundTokenStorage => "refundTokenStorage",
            Self::TrustAsset => "trustAsset",
            Self::RemoveTrustLine => "removeTrustLine",
            Self::Shield => "shield",
        }
    }
    pub fn is_submitted(self) -> bool {
        self != Self::Receive
    }
    pub fn is_staking(self) -> bool {
        matches!(
            self,
            Self::Stake | Self::Unstake | Self::Withdraw | Self::ClaimRewards
        )
    }
    /// A kind only core records, for an operation it built: a provider's
    /// send or receive row of the same transaction confirms the record and
    /// is not another.
    pub fn is_operation(self) -> bool {
        !matches!(self, Self::Send | Self::Receive)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TransactionDirection {
    Outgoing,
    Incoming,
    Neutral,
}
#[uniffi::export]
pub fn transaction_kind_direction(kind: TransactionKind) -> TransactionDirection {
    match kind {
        TransactionKind::Send | TransactionKind::Stake => TransactionDirection::Outgoing,
        TransactionKind::Receive | TransactionKind::Withdraw | TransactionKind::ClaimRewards => {
            TransactionDirection::Incoming
        }
        // A revocation or a key deletion moves nothing but its fee.
        TransactionKind::Unstake
        | TransactionKind::RevokeApproval
        | TransactionKind::DeleteAccessKey
        | TransactionKind::MergeCoins
        | TransactionKind::CloseTokenAccounts
        | TransactionKind::RefundTokenStorage
        | TransactionKind::TrustAsset
        | TransactionKind::RemoveTrustLine
        | TransactionKind::Shield => TransactionDirection::Neutral,
    }
}
#[uniffi::export]
pub fn transaction_kind_is_submitted(kind: TransactionKind) -> bool {
    kind.is_submitted()
}

/// Where a transaction stands; stored as `"pending"`, `"confirmed"` or `"failed"`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum TransactionStatus {
    Pending,
    Confirmed,
    Failed,
}

impl TransactionStatus {
    /// The stored and wire spelling.
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

/// Which side of its target a price alert fires on; stored as `"Above"` or `"Below"`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum PriceAlertCondition {
    #[serde(rename = "Above")]
    Above,
    #[serde(rename = "Below")]
    Below,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
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
    pub chain_id: crate::registry::Chain,
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

    pub fn deployment_id(&self) -> String {
        let network = &self.chain_id;
        if self.is_native() {
            return format!("{network}:native");
        }
        let contract =
            crate::tokens::normalize_token_identifier(self.contract_address.clone(), self.chain_id)
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
    pub fn canonicalize(&mut self) -> Result<(), crate::SpectraBridgeError> {
        use crate::SpectraBridgeError as E;
        let network = self.chain_id;
        self.amount = crate::decimal::canonical(&self.amount)
            .ok_or_else(|| E::invalid("invalid holding amount"))?;
        self.contract_address =
            crate::tokens::normalize_token_identifier(self.contract_address.clone(), self.chain_id);
        if self.token_standard == "Native" {
            if self.contract_address.is_some() {
                return Err(E::invalid("native token cannot carry a contract"));
            }
        } else {
            let contract = self
                .contract_address
                .as_ref()
                .ok_or_else(|| E::invalid("protocol token requires an identifier"))?;
            self.contract_address = Some(crate::tokens::validate_protocol_identifier(
                network,
                &self.token_standard,
                contract,
            )?);
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
pub struct WalletDerivationOverrides {
    pub passphrase: Option<String>,
    pub hmac_key: Option<String>,
}

impl WalletDerivationOverrides {
    pub fn validate_for_chain(
        &self,
        chain: crate::registry::Chain,
    ) -> Result<(), crate::SpectraBridgeError> {
        if self.passphrase.as_ref().is_some_and(|s| !s.is_empty())
            && !chain.supports_derivation_passphrase()
        {
            return Err(crate::SpectraBridgeError::invalid(format!(
                "{} does not support a derivation passphrase",
                chain.chain_display_name()
            )));
        }
        if self.hmac_key.as_ref().is_some_and(|s| !s.is_empty())
            && !chain.supports_derivation_hmac_override()
        {
            return Err(crate::SpectraBridgeError::invalid(format!(
                "{} does not support a custom HMAC key",
                chain.chain_display_name()
            )));
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
pub(crate) struct SensitiveOverrides(pub(crate) WalletDerivationOverrides);

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletView {
    pub id: String,
    pub name: String,
    /// The network this wallet is on, as a registry chain id — the chain's
    /// own id on a family with one network.
    pub chain_id: crate::registry::Chain,
    /// `Chain::address_slot()` → address for this wallet.
    ///
    /// A wallet belongs to one chain (`chain_id`), so in practice this
    /// holds a single entry. It is a map rather than one `Option<String>`
    /// per chain so that adding a chain is a registry edit and not a schema
    /// change here, in the Swift record, in its `Codable`, and at every
    /// construction site.
    pub addresses: HashMap<String, String>,
    /// Account public key for receiving and recovery without unlocking a seed.
    pub account_xpub: Option<String>,
    /// The one path this wallet derives from: a profile's, or a custom one.
    /// `None` on a chain that derives without a path.
    pub derivation_path: Option<String>,
    pub derivation_overrides: WalletDerivationOverrides,
    pub holdings: Vec<AssetHolding>,
    pub include_in_portfolio_total: bool,
    /// What the wallet signs with and whether a password guards it — read
    /// from the wallet record, so rendering one touches no secret store.
    pub signing: crate::store::state::WalletSigning,
    /// A Monero wallet's restore height; `None` on other chains.
    pub restore_height: Option<u64>,
    /// The deployment ids of the holdings the user hid from the wallet's
    /// total and the portfolio.
    pub hidden_holdings: Vec<String>,
    /// An ICP wallet's principal; `None` elsewhere.
    pub icp_principal: Option<String>,
    /// The key a NEAR named account signs with, as `ed25519:…`; `None`
    /// elsewhere, and for an implicit account, whose address is its key.
    pub near_account_key: Option<String>,
    /// A multisig account's descriptor; `None` for a single-key wallet.
    pub multisig_descriptor: Option<String>,
}

impl WalletView {
    /// This wallet's address for `chain`, if it has one.
    pub fn address_for(&self, chain: crate::registry::Chain) -> Option<&str> {
        self.addresses.get(chain.address_slot()).map(String::as_str)
    }

    /// The address for the wallet's own chain — what the UI shows and what
    /// balance/history calls query.
    pub fn primary_address(&self) -> Option<&str> {
        self.address_for(self.chain_id)
    }
}

// ── WalletView ↔ WalletState ───────────────────────────────────────
//
// `WalletState` owns persisted wallet facts; `WalletView` projects them for the
// native UI. Both identify the selected chain with `chain_id` and carry the
// wallet's one derivation path.

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

        let chain = self.chain_id;
        Ok(WalletState {
            id: self.id.clone(),
            name: self.name.clone(),
            signing: self.signing,
            chain_id: chain,
            include_in_portfolio_total: self.include_in_portfolio_total,
            xpub: self.account_xpub.clone(),
            derivation_path: self.derivation_path.clone(),
            derivation_overrides: self.derivation_overrides.clone(),
            holdings: self.holdings.clone(),
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
                        let owner = if slot == own_slot {
                            chain
                        } else {
                            Chain::all().find(|candidate| candidate.address_slot() == slot)?
                        };
                        Some(WalletAddress {
                            chain_id: owner,
                            address: address.clone(),
                            kind: "receive".to_string(),
                            derivation_path: (owner == chain)
                                .then(|| self.derivation_path.clone())
                                .flatten(),
                        })
                    })
                    .collect()
            },
            restore_height: self.restore_height,
            hidden_holdings: self.hidden_holdings.clone(),
            icp_principal: self.icp_principal.clone(),
            near_account_key: self.near_account_key.clone(),
            multisig_descriptor: self.multisig_descriptor.clone(),
        })
    }
}

impl crate::store::state::WalletState {
    /// Convert back into the shape the iOS app renders.
    ///
    /// The reverse of [`WalletView::to_wallet_state`]. `WalletState`
    /// remains the authority; this produces a view model.
    pub fn to_wallet_view(&self) -> WalletView {
        WalletView {
            id: self.id.clone(),
            name: self.name.clone(),
            chain_id: self.chain_id,
            addresses: self
                .addresses
                .iter()
                .map(|entry| {
                    (
                        entry.chain_id.address_slot().to_string(),
                        entry.address.clone(),
                    )
                })
                .collect(),
            account_xpub: self.xpub.clone(),
            derivation_path: self.derivation_path.clone(),
            derivation_overrides: self.derivation_overrides.clone(),
            holdings: self
                .holdings
                .iter()
                .cloned()
                .map(AssetHolding::identified)
                .collect(),
            include_in_portfolio_total: self.include_in_portfolio_total,
            signing: self.signing,
            restore_height: self.restore_height,
            hidden_holdings: self.hidden_holdings.clone(),
            icp_principal: self.icp_principal.clone(),
            near_account_key: self.near_account_key.clone(),
            multisig_descriptor: self.multisig_descriptor.clone(),
        }
    }
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
pub struct TokenPreferenceEntry {
    pub token: crate::tokens::TokenDeploymentEntry,
    /// The catalog ships it; the user cannot edit or delete it.
    pub is_built_in: bool,
}

impl TokenPreferenceEntry {
    /// Identity: a token *is* its contract on its chain.
    pub fn id(&self) -> String {
        self.token.deployment_id.clone()
    }

    /// The chain hosting this token.
    pub fn hosting_chain(&self) -> Option<crate::registry::Chain> {
        Some(self.token.chain_id).filter(|c| c.hosts_tokens())
    }
}

/// One place an asset is held: a chain, a token standard, a contract.
#[derive(Debug, Clone, PartialEq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct DashboardAssetHolding {
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
/// empty for a pinned asset they hold nowhere. Naming the row is `identity`'s
/// job, and `holdings` states only what is true.
///
/// Valuation is derived with the holdings in the same snapshot; it is not
/// persisted beside them. None means at least one place is unpriced.
#[derive(Debug, Clone, PartialEq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct DashboardAssetGroup {
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
    pub holdings: Vec<DashboardAssetHolding>,
    pub is_pinned: bool,
}

/// An asset the dashboard can pin. `deployment_id` is the place it is drawn
/// from — the colour and artwork follow the deployment, never the ticker.
#[derive(Debug, Clone, PartialEq, Serialize, uniffi::Record)]
pub struct DashboardPinOption {
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
mod token_preference_tests {
    use super::*;

    #[test]
    fn an_entry_is_identified_by_its_deployment() {
        let entry = TokenPreferenceEntry {
            is_built_in: true,
            token: crate::tokens::TokenDeploymentEntry {
                deployment_id: "bnb:bep-20:0x1111111111111111111111111111111111111111".into(),
                token_id: "fixture:token".into(),
                kind: crate::tokens::TokenKind::Protocol {
                    standard: "BEP-20".into(),
                    identifier: "0x1111111111111111111111111111111111111111".into(),
                },
                chain_id: crate::registry::Chain::BnbChain,
                name: "Tether USD".to_string(),
                symbol: "USDT".to_string(),
                token_standard: "BEP-20".to_string(),
                contract: "0x1111111111111111111111111111111111111111".to_string(),
                coingecko_id: "tether".to_string(),
                coinpaprika_id: String::new(),
                decimals: 18,
                tags: vec![crate::tokens::TokenTag::Stablecoin],
                color: Some(crate::chains::CatalogColor::Green),
                artwork_name: "usdt".to_string(),
            },
        };
        // Identity is the token's, not a stored string.
        assert_eq!(
            entry.id(),
            "bnb:bep-20:0x1111111111111111111111111111111111111111"
        );
    }
}
