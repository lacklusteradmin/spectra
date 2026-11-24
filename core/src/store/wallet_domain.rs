// Wallet value types crossing the FFI. Display color is deliberately absent:
// the platform derives it from the asset symbol.

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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, uniffi::Enum)]
pub enum TransactionDirection {
    Outgoing,
    Incoming,
    #[default]
    Neutral,
}
/// Which way a transaction moved its amount. A transfer of nothing — a
/// zero-value contract call recorded as a send — moved nothing, whatever its
/// kind, and reads neither as money out nor in.
pub fn transaction_amount_direction(kind: TransactionKind, amount: &str) -> TransactionDirection {
    if crate::amount::decimal::is_zero(amount) {
        return TransactionDirection::Neutral;
    }
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

    /// How a transaction the chain executed ended.
    pub(crate) fn executed(succeeded: bool) -> Self {
        if succeeded {
            Self::Confirmed
        } else {
            Self::Failed
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
    /// The protocol a token is; `None` for the network's own coin.
    pub token_standard: Option<crate::registry::TokenStandard>,
    pub contract_address: Option<String>,
    /// The balance, as an exact decimal in the asset's own units.
    pub amount: String,
}

impl AssetHolding {
    /// Native is an explicit protocol type, never a symbol comparison.
    pub fn is_native(&self) -> bool {
        self.token_standard.is_none()
            && self
                .contract_address
                .as_deref()
                .is_none_or(|c| c.is_empty())
    }

    pub fn deployment_id(&self) -> String {
        let contract = crate::catalog::tokens::normalize_token_identifier(
            self.contract_address.clone(),
            self.chain_id,
        )
        .unwrap_or_default();
        crate::catalog::tokens::deployment_id(self.chain_id, self.token_standard, &contract)
    }

    pub fn catalog_token(&self) -> Option<&'static crate::catalog::tokens::TokenDeploymentEntry> {
        let key = self.deployment_id();
        crate::catalog::tokens::catalog()
            .iter()
            .find(|t| t.deployment_id == key)
    }

    /// This holding with its `id` filled in.
    pub fn identified(mut self) -> Self {
        self.id = self.deployment_id();
        self
    }

    /// Validate identity and derive catalog-owned display facts: a holding a
    /// test builds, put in the form core writes. Core writes holdings it
    /// builds itself, and no front end supplies one.
    #[cfg(test)]
    pub fn canonicalize(&mut self) -> Result<(), crate::error::SpectraBridgeError> {
        use crate::error::SpectraBridgeError as E;
        let network = self.chain_id;
        self.amount = crate::amount::decimal::canonical(&self.amount)
            .ok_or_else(|| E::invalid("invalid holding amount"))?;
        self.contract_address = crate::catalog::tokens::normalize_token_identifier(
            self.contract_address.clone(),
            self.chain_id,
        );
        match self.token_standard {
            None if self.contract_address.is_some() => {
                return Err(E::invalid("native token cannot carry a contract"));
            }
            None => {}
            Some(standard) => {
                let contract = self
                    .contract_address
                    .as_ref()
                    .ok_or_else(|| E::invalid("protocol token requires an identifier"))?;
                self.contract_address = Some(crate::catalog::tokens::validate_protocol_identifier(
                    network, standard, contract,
                )?);
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
///
/// An import commit carries these; a stored wallet does not. They are sealed
/// in the `SecretStore` beside the phrase they derive with, under its
/// password, and the wallet record keeps only [`WalletDerivationFlags`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WalletDerivationOverrides {
    pub passphrase: Option<String>,
    pub hmac_key: Option<String>,
}

/// Which derivation overrides a wallet was imported with, without what they
/// say: what deciding where a wallet can go, or which of its keys it can
/// export, needs to know before the wallet is unlocked.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WalletDerivationFlags {
    /// A BIP-39 passphrase is sealed beside the phrase.
    pub passphrase: bool,
    /// A custom HMAC key is sealed beside the phrase.
    pub hmac_key: bool,
    /// The HMAC override is `uniform`: on Polkadot, the root seed expands in
    /// schnorrkel's Uniform mode, the one meaning its override has.
    pub uniform_expansion: bool,
}

impl WalletDerivationFlags {
    pub fn validate_for_chain(
        &self,
        chain: crate::registry::Chain,
    ) -> Result<(), crate::error::SpectraBridgeError> {
        if self.passphrase && !chain.supports_derivation_passphrase() {
            return Err(crate::error::SpectraBridgeError::invalid(format!(
                "{} does not support a derivation passphrase",
                chain.chain_display_name()
            )));
        }
        if self.hmac_key && !chain.supports_derivation_hmac_override() {
            return Err(crate::error::SpectraBridgeError::invalid(format!(
                "{} does not support a custom HMAC key",
                chain.chain_display_name()
            )));
        }
        Ok(())
    }
}

impl WalletDerivationOverrides {
    /// These overrides with an empty value read as none: a form's blank field
    /// is no override.
    pub fn normalized(mut self) -> Self {
        for value in [&mut self.passphrase, &mut self.hmac_key] {
            if value.as_deref().is_some_and(str::is_empty) {
                if let Some(blank) = value.as_mut() {
                    blank.zeroize();
                }
                *value = None;
            }
        }
        self
    }

    /// Which overrides these are, an empty value counting as none.
    pub fn flags(&self) -> WalletDerivationFlags {
        let set = |value: &Option<String>| value.as_deref().is_some_and(|v| !v.is_empty());
        WalletDerivationFlags {
            passphrase: set(&self.passphrase),
            hmac_key: set(&self.hmac_key),
            uniform_expansion: self.hmac_key.as_deref().map(str::trim) == Some("uniform"),
        }
    }

    pub fn validate_for_chain(
        &self,
        chain: crate::registry::Chain,
    ) -> Result<(), crate::error::SpectraBridgeError> {
        self.flags().validate_for_chain(chain)
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

/// A wallet's derivation overrides as read from the `SecretStore`, wiped
/// when they go out of scope.
pub struct SensitiveOverrides(pub(crate) WalletDerivationOverrides);

impl SensitiveOverrides {
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

    /// The custom HMAC key, when there is a non-empty one.
    pub(crate) fn hmac_key(&self) -> Option<&str> {
        self.0.hmac_key.as_deref().filter(|value| !value.is_empty())
    }
}

impl Drop for SensitiveOverrides {
    fn drop(&mut self) {
        self.0.zeroize_sensitive_fields();
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
    pub token: crate::catalog::tokens::TokenDeploymentEntry,
    /// The catalog ships it; the user cannot edit or delete it.
    pub is_built_in: bool,
}

impl TokenPreferenceEntry {
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
/// `holdings` is **where the user holds it**, largest value first, and never
/// empty: an asset held nowhere has no row, pinned or not.
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
    /// held.
    pub identity: AssetHolding,
    pub holdings: Vec<DashboardAssetHolding>,
    /// Who holds it: one entry per wallet, largest value first.
    pub wallets: Vec<DashboardAssetWallet>,
    pub is_pinned: bool,
    /// Worth less than `DASHBOARD_SMALL_BALANCE_USD` in all: the dust and
    /// airdropped tokens a front end may fold away. Never a pinned row, and
    /// never an unpriced one, whose worth is unknown.
    #[uniffi(default = false)]
    pub is_small: bool,
}

/// One wallet's share of a dashboard row: everything it holds of the asset,
/// on every network it holds it on.
#[derive(Debug, Clone, PartialEq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct DashboardAssetWallet {
    pub wallet_id: String,
    /// An exact decimal.
    pub amount: String,
    /// In the display currency; `None` when any of it is unpriced.
    pub value: Option<f64>,
}

/// Below this many US dollars in all, a dashboard row is `is_small`. One
/// threshold, in USD so it needs no exchange rate.
pub const DASHBOARD_SMALL_BALANCE_USD: f64 = 1.0;

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
mod direction_tests {
    use super::*;

    #[test]
    fn a_transfer_of_nothing_moves_neither_way() {
        let send = |amount: &str| transaction_amount_direction(TransactionKind::Send, amount);
        assert_eq!(send("0.5"), TransactionDirection::Outgoing);
        assert_eq!(send("0"), TransactionDirection::Neutral);
        assert_eq!(send("0.000"), TransactionDirection::Neutral);
        assert_eq!(
            transaction_amount_direction(TransactionKind::Receive, "0"),
            TransactionDirection::Neutral
        );
        assert_eq!(
            transaction_amount_direction(TransactionKind::ClaimRewards, "1"),
            TransactionDirection::Incoming
        );
    }
}

#[cfg(test)]
mod derivation_override_tests {
    use super::*;
    use crate::registry::Chain;

    /// A blank field is no override; anything else is kept exactly, its
    /// whitespace included, and unknown fields are refused.
    #[test]
    fn overrides_keep_their_text_and_a_blank_is_none() {
        let typed = WalletDerivationOverrides {
            passphrase: Some(" secret ".into()),
            hmac_key: Some(String::new()),
        }
        .normalized();
        assert_eq!(typed.passphrase.as_deref(), Some(" secret "));
        assert_eq!(typed.hmac_key, None);
        assert_eq!(
            typed.flags(),
            WalletDerivationFlags {
                passphrase: true,
                hmac_key: false,
                uniform_expansion: false,
            }
        );
        for unknown in [r#"{"iterationCount":2048}"#, r#"{"curve":"ed25519"}"#] {
            assert!(serde_json::from_str::<WalletDerivationOverrides>(unknown).is_err());
        }
    }

    /// Polkadot's HMAC override `uniform` is a mode, and the flags name it so
    /// the wallet can say which keys it exports without being unlocked.
    #[test]
    fn a_uniform_hmac_override_is_flagged_as_the_expansion_mode() {
        let flags = WalletDerivationOverrides {
            passphrase: None,
            hmac_key: Some("uniform".into()),
        }
        .flags();
        assert!(flags.hmac_key && flags.uniform_expansion && !flags.passphrase);
    }

    /// A derivation secret is taken only by a network whose derivation reads
    /// it: a passphrase anywhere but Monero, a custom HMAC key only on the
    /// networks that seed from one. Anything else is refused by name; an
    /// empty value is no value.
    #[test]
    fn each_override_is_taken_only_where_derivation_reads_it() {
        let overrides = |passphrase: &str, hmac_key: &str| WalletDerivationOverrides {
            passphrase: Some(passphrase.into()),
            hmac_key: Some(hmac_key.into()),
        };
        let refusal = |chain, passphrase, hmac_key| {
            overrides(passphrase, hmac_key)
                .validate_for_chain(chain)
                .unwrap_err()
                .to_string()
        };
        assert_eq!(
            refusal(Chain::Ethereum, "", "custom"),
            "Ethereum does not support a custom HMAC key"
        );
        assert_eq!(
            refusal(Chain::Monero, "secret", ""),
            "Monero does not support a derivation passphrase"
        );
        assert!(
            overrides("secret", "")
                .validate_for_chain(Chain::Ethereum)
                .is_ok()
        );
        assert!(
            overrides("secret", "custom")
                .validate_for_chain(Chain::Solana)
                .is_ok()
        );
        assert!(overrides("", "").validate_for_chain(Chain::Monero).is_ok());
    }
}
