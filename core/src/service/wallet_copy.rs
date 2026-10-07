//! A wallet's secret added to another network, as a wallet of its own.
//!
//! A wallet stays on one network, but its secret often belongs on several —
//! most of all the EVM networks, which share one address. Core reads the
//! sealed secret and runs the target network's ordinary import with it, so
//! the secret never crosses to a front end, and the new wallet seals its own
//! copy and stands alone: deleting either leaves the other. A watched wallet
//! adds its address, or its account key, to the networks that read it.

use super::*;
use crate::derivation::import::{
    WalletImportCommit, WalletImportKind, WalletImportOutcome, WalletImportPreview,
    WalletImportRequest,
};
use crate::store::state::{WalletSigning, WalletState};

/// A request to add a stored wallet's secret, or watched address, to
/// another network.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct WalletCopyCommit {
    /// The wallet whose secret or address is added.
    pub source_wallet_id: String,
    /// The network the new wallet is on.
    pub chain: Chain,
    /// The new wallet's name; empty lets core assign an available one.
    pub wallet_name: String,
    /// The source wallet's password where it has one. The copy is sealed
    /// under it, so the new wallet is protected as the source is.
    pub password: Option<String>,
    /// For a phrase, the path the new wallet derives along, as an import
    /// takes it; `None` takes the network's default profile.
    pub derivation_path: Option<String>,
    /// For a Monero target, where the new wallet's scan starts.
    pub restore_height: Option<u64>,
    /// For a TON target, the wallet contract the key holds its account
    /// under; `None` takes the default.
    pub ton_wallet_version: Option<crate::derivation::ton::TonWalletVersion>,
}

/// The networks `wallet`'s secret, or watched address, can be added to as
/// it is: every other network that reads the same secret. A phrase goes to
/// the networks that restore its format with the wallet's derivation
/// overrides, a raw key to those whose key is of the same signature scheme,
/// and a watched address or account key to the networks where it names the
/// same account and validates.
pub(crate) fn copy_targets(wallet: &WalletState) -> Vec<Chain> {
    let source = wallet.chain_id;
    Chain::all()
        .filter(|chain| *chain != source)
        .filter(|chain| match wallet.signing {
            WalletSigning::SeedPhrase { .. } => {
                chain.created_phrase_format() == source.created_phrase_format()
                    && wallet
                        .derivation_overrides
                        .validate_for_chain(*chain)
                        .is_ok()
            }
            WalletSigning::PrivateKey { .. } => {
                source.key_scheme().is_some() && chain.key_scheme() == source.key_scheme()
            }
            // The same text on another family can be a valid address of an
            // unrelated account (an EVM address reads as a short Aptos one),
            // so a watch goes only where the address means the same account:
            // the EVM networks, or another network of the source's chain.
            WalletSigning::WatchOnly => {
                (chain.address_slot() == source.address_slot()
                    || chain.mainnet_counterpart() == source.mainnet_counterpart())
                    && watch_commit(wallet, *chain)
                        .is_some_and(|commit| wallet_import::plan_import(commit).is_ok())
            }
        })
        .collect()
}

/// The watch import that adds `wallet`'s address, or account key, to
/// `chain`: an account key where the wallet watches one, its address
/// otherwise.
fn watch_commit(wallet: &WalletState, chain: Chain) -> Option<WalletImportCommit> {
    let kind = match &wallet.xpub {
        Some(xpub) => WalletImportKind::WatchAccountXpub { xpub: xpub.clone() },
        None => WalletImportKind::WatchAddresses {
            addresses: vec![wallet.address_on(wallet.chain_id)?.to_string()],
        },
    };
    Some(commit(
        WalletImportRequest {
            wallet_name: String::new(),
            chain,
            kind,
        },
        None,
    ))
}

fn commit(request: WalletImportRequest, password: Option<String>) -> WalletImportCommit {
    WalletImportCommit {
        password,
        request,
        derivation_path: None,
        derivation_overrides: Default::default(),
        seed_phrase: None,
        private_key: None,
        restore_height: None,
        named_account: None,
        ton_wallet_version: None,
        upgrade_wallet_id: None,
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// The networks the wallet's secret, or watched address, can be added to.
    /// Contacts nothing and reads no secret.
    pub async fn wallet_copy_targets(
        &self,
        wallet_id: String,
    ) -> Result<Vec<Chain>, SpectraBridgeError> {
        Ok(copy_targets(&self.stored_wallet(&wallet_id).await?))
    }

    /// The address the copy would store, without storing or sealing
    /// anything: the preview of the import it runs.
    pub async fn preview_wallet_copy(
        &self,
        commit: WalletCopyCommit,
    ) -> Result<WalletImportPreview, SpectraBridgeError> {
        let import = self.copy_import(commit).await?;
        self.preview_wallet_import(import).await
    }

    /// Add the wallet's secret, or watched address, to another network as a
    /// wallet of its own, sealed under the source's password.
    pub async fn copy_wallet_to_network(
        &self,
        commit: WalletCopyCommit,
    ) -> Result<WalletImportOutcome, SpectraBridgeError> {
        let import = self.copy_import(commit).await?;
        self.import_wallets(import).await
    }
}

impl WalletService {
    pub(super) async fn stored_wallet(
        &self,
        wallet_id: &str,
    ) -> Result<WalletState, SpectraBridgeError> {
        self.wallet_state
            .read()
            .await
            .wallets
            .iter()
            .find(|wallet| wallet.id == wallet_id)
            .cloned()
            .ok_or_else(|| SpectraBridgeError::failure("Wallet removed"))
    }

    /// The import a copy runs: the target's, carrying the source's secret,
    /// read from its seal with the source's password. Refuses a target the
    /// source's secret does not belong on before reading the secret.
    async fn copy_import(
        &self,
        copy: WalletCopyCommit,
    ) -> Result<WalletImportCommit, SpectraBridgeError> {
        let source = self.stored_wallet(&copy.source_wallet_id).await?;
        if !copy_targets(&source).contains(&copy.chain) {
            return Err(crate::derivation::error::DerivationError::refused(
                "This wallet cannot be added to %@.",
                [copy.chain.chain_display_name()],
            )
            .into());
        }
        let password = copy.password.map(zeroize::Zeroizing::new);
        let request = |kind| WalletImportRequest {
            wallet_name: copy.wallet_name.clone(),
            chain: copy.chain,
            kind,
        };
        let mut import = match source.signing {
            WalletSigning::WatchOnly => {
                let mut import = watch_commit(&source, copy.chain)
                    .ok_or_else(|| SpectraBridgeError::failure("Wallet has no address"))?;
                import.request.wallet_name = copy.wallet_name.clone();
                import
            }
            WalletSigning::SeedPhrase { .. } => {
                let phrase = crate::store::wallet_secrets::load_seed_phrase(
                    &*self.secrets()?,
                    &source.id,
                    password.as_deref().map(String::as_str),
                )?;
                let mut import = commit(
                    request(WalletImportKind::Phrase),
                    password.as_deref().cloned(),
                );
                import.seed_phrase = Some(phrase.to_string());
                import.derivation_overrides = source.derivation_overrides.clone();
                import
            }
            WalletSigning::PrivateKey { .. } => {
                let key = crate::store::wallet_secrets::load_private_key(
                    &*self.secrets()?,
                    &source.id,
                    password.as_deref().map(String::as_str),
                )?;
                let mut import = commit(
                    request(WalletImportKind::PrivateKey),
                    password.as_deref().cloned(),
                );
                import.private_key = Some(key.to_string());
                import
            }
        };
        import.derivation_path = copy.derivation_path;
        import.restore_height = copy.restore_height;
        import.ton_wallet_version = copy.ton_wallet_version;
        Ok(import)
    }
}

#[cfg(test)]
#[path = "tests/wallet_copy.rs"]
mod tests;
