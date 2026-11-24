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

/// What a wallet signs with, recorded when its secret is stored.
///
/// Non-secret metadata about the secret store's contents, kept with the wallet
/// so rendering a wallet reads no Keychain item. Only an import writes a
/// wallet's secret, and it records this in the same commit.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum WalletSigning {
    /// Addresses only; nothing is stored to sign with. The default, because
    /// it claims no capability.
    #[default]
    WatchOnly,
    /// A seed phrase, sealed under a password when `password_protected`.
    #[serde(rename_all = "camelCase")]
    SeedPhrase { password_protected: bool },
    /// A raw private key, sealed under a password when `password_protected`.
    #[serde(rename_all = "camelCase")]
    PrivateKey { password_protected: bool },
}

impl WalletSigning {
    pub fn is_watch_only(self) -> bool {
        self == Self::WatchOnly
    }

    /// Secret operations such as signing and revealing need the wallet's password.
    pub fn requires_password(self) -> bool {
        matches!(
            self,
            Self::SeedPhrase {
                password_protected: true
            } | Self::PrivateKey {
                password_protected: true
            }
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WalletState {
    pub id: String,
    pub name: String,
    pub signing: WalletSigning,
    /// The concrete network this wallet is on. Its family is the network's
    /// mainnet counterpart; there is no second spelling of either.
    pub chain_id: crate::registry::Chain,
    pub include_in_portfolio_total: bool,
    /// The wallet's address on its network: the one it receives at and
    /// queries. `None` for a watched account key, which has none of its own.
    pub address: Option<String>,
    /// An account public key, for receiving and recovery without unlocking a
    /// seed.
    pub xpub: Option<String>,
    /// The single path this wallet derives from: a profile's or a custom one,
    /// fixed at import. `None` on a chain that derives without a path.
    pub derivation_path: Option<String>,
    /// The profile and account `derivation_path` is on, or `None` for a
    /// custom path or none. Set when core serves the wallet; never stored.
    #[serde(skip)]
    pub derivation_profile: Option<crate::derivation::path::DerivationProfileChoice>,
    /// Which power-user derivation overrides the wallet was imported with.
    /// The overrides themselves are sealed in the `SecretStore`.
    pub derivation_flags: crate::store::wallet_domain::WalletDerivationFlags,
    pub holdings: Vec<crate::store::wallet_domain::AssetHolding>,
    /// The block height a scanning wallet starts from, fixed when it is
    /// imported: a Monero wallet's restore height, or where a Zcash wallet's
    /// scan for shielded funds starts. `None` on every other chain, which
    /// reads balances from a provider instead of scanning.
    pub restore_height: Option<u64>,
    /// The deployment ids of the holdings the user hid, sorted: left out of
    /// the wallet's total and the portfolio, listed apart, still sendable.
    pub hidden_holdings: Vec<String>,
    /// An ICP wallet's principal, the identity its ledger account derives
    /// from, recorded at import from the key. `None` on every other chain
    /// and on a watched ICP account, whose principal its address cannot give.
    pub icp_principal: Option<String>,
    /// The key a NEAR named account signs with, as `ed25519:…`, recorded at
    /// import once the network confirmed it is a full-access key of the
    /// account. `None` elsewhere, and for an implicit account, whose address
    /// is its key.
    pub near_account_key: Option<String>,
    /// A Starknet wallet's Stark public key, as `0x` and 64 hex digits,
    /// recorded at import from the key: what its account's deployment
    /// names, since the address is a hash of it. `None` on every other
    /// chain and on a watched Starknet account.
    #[uniffi(default = None)]
    pub starknet_public_key: Option<String>,
    /// A multisig account's descriptor (`derivation::multisig`), canonical
    /// with its checksum: the policy every address and signature follows.
    /// `None` for every single-key wallet.
    pub multisig_policy: Option<String>,
    /// When a balance read for this wallet first committed, as Unix seconds.
    /// `None` until then: the holdings an import writes are zero
    /// placeholders, not balances, and are shown as still loading.
    #[uniffi(default = None)]
    pub balances_read_at: Option<f64>,
}

// Plain `impl` — deliberately not `#[uniffi::export]`. These are Rust-side
// domain helpers; the FFI surface stays the record's fields.
impl WalletState {
    /// The holdings the wallet's total and the portfolio count: all but the
    /// hidden ones.
    pub(crate) fn counted_holdings(
        &self,
    ) -> impl Iterator<Item = &crate::store::wallet_domain::AssetHolding> {
        self.holdings
            .iter()
            .filter(|holding| !self.hidden_holdings.contains(&holding.deployment_id()))
    }

    /// The wallet as core serves it to a front end: each holding with its
    /// id, and the profile its derivation path is on.
    pub(crate) fn served(&self) -> Self {
        let mut wallet = self.clone();
        wallet.holdings = wallet
            .holdings
            .into_iter()
            .map(crate::store::wallet_domain::AssetHolding::identified)
            .collect();
        wallet.derivation_profile = wallet.derivation_path.as_deref().and_then(|path| {
            crate::derivation::path::derivation_profile_of_path(wallet.chain_id, path)
        });
        wallet
    }

    /// A wallet with one address on one chain.
    ///
    /// A test constructor: production wallets are built by the import and
    /// derivation paths, which fill in the fields this shortcut defaults.
    #[cfg(test)]
    pub fn single_address(
        id: impl Into<String>,
        name: impl Into<String>,
        chain_id: crate::registry::Chain,
        address: impl Into<String>,
        derivation_path: Option<String>,
        is_watch_only: bool,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            signing: if is_watch_only {
                WalletSigning::WatchOnly
            } else {
                WalletSigning::SeedPhrase {
                    password_protected: false,
                }
            },
            chain_id,
            include_in_portfolio_total: true,
            address: Some(address.into()),
            xpub: None,
            derivation_path,
            derivation_profile: None,
            derivation_flags: Default::default(),
            holdings: Vec::new(),
            restore_height: None,
            hidden_holdings: Vec::new(),
            icp_principal: None,
            near_account_key: None,
            starknet_public_key: None,
            multisig_policy: None,
            balances_read_at: None,
        }
    }

    pub fn is_watch_only(&self) -> bool {
        self.signing.is_watch_only()
    }

    /// The mainnet whose family this wallet belongs to.
    pub fn family(&self) -> crate::registry::Chain {
        self.chain_id.mainnet_counterpart()
    }

    /// The wallet's address where `chain` reads it: its own, on any network
    /// whose addresses share its network's slot (every EVM network shares
    /// one key and address). `None` on another network.
    pub fn address_on(&self, chain: crate::registry::Chain) -> Option<&str> {
        self.shares_address_with(chain)
            .then_some(self.address.as_deref())
            .flatten()
    }

    /// Whether `chain` takes this wallet's address and derivation path.
    pub(crate) fn shares_address_with(&self, chain: crate::registry::Chain) -> bool {
        self.chain_id.address_slot() == chain.address_slot()
    }
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

#[cfg(test)]
impl WalletState {
    /// A seed wallet `id` on `chain`, counted in the portfolio, with no
    /// address, holdings or balance read yet: what a test starts from.
    pub(crate) fn fixture(id: &str, chain: crate::registry::Chain) -> Self {
        Self {
            id: id.to_string(),
            name: id.to_string(),
            signing: WalletSigning::SeedPhrase {
                password_protected: false,
            },
            include_in_portfolio_total: true,
            chain_id: chain,
            xpub: None,
            derivation_path: None,
            derivation_profile: None,
            derivation_flags: Default::default(),
            holdings: Vec::new(),
            address: None,
            restore_height: None,
            hidden_holdings: Vec::new(),
            icp_principal: None,
            near_account_key: None,
            starknet_public_key: None,
            multisig_policy: None,
            balances_read_at: None,
        }
    }
}
