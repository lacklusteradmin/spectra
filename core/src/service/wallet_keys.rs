//! A wallet's keys, exported in the forms other wallets import.
//!
//! Revealing the phrase was the only way out; a key-imported wallet, which
//! has no phrase, could not leave at all. Each export here is written the way
//! its network's wallets read it — WIF, a Solana keypair, an `S…` seed, a
//! zpub — and reads back through Spectra's own import as the same wallet.
//! What another wallet needs only to watch is here too: an account's public
//! key, and Monero's view key. Both expose the whole history, so a front end
//! gates every export as it gates the phrase.

use super::*;
use crate::derivation::setup::WalletSecretFormat;
use crate::store::state::{WalletSigning, WalletState};

/// One thing a wallet's keys can be exported as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum WalletKeyKind {
    /// The key the wallet signs with, in its network's own encoding.
    PrivateKey,
    /// Monero's private spend key: with the view key and the address, the
    /// whole wallet.
    MoneroSpendKey,
    /// Monero's private view key: with the address, a view-only wallet.
    MoneroViewKey,
    /// The account's extended public key, which watches every address of
    /// the account.
    AccountPublicKey,
}

/// One exported key, and the encoding it is written in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletKeyExport {
    pub kind: WalletKeyKind,
    pub format: WalletSecretFormat,
    pub value: String,
}

/// What `wallet`'s keys can be exported as.
///
/// A phrase wallet on a network that spreads its funds over many addresses
/// (the account-discovery UTXO chains) has no one key that holds them, so
/// it exports its account's public key where the network takes one, and
/// otherwise only its phrase. Nor does a Cardano phrase wallet's payment
/// key: its base address names the account's stake key too, so it exports
/// only its phrase. Nor does a Substrate phrase wallet whose path has a soft
/// junction, which derives a key no seed gives (`derives_importable_seed`).
/// An account key is exported where its network has an encoding for the
/// script the wallet uses: not for Taproot, which has none, nor for a path
/// no profile names.
pub(crate) fn exportable_keys(wallet: &WalletState) -> Vec<WalletKeyKind> {
    let chain = wallet.chain_id;
    match wallet.signing {
        WalletSigning::WatchOnly => {
            if wallet.xpub.is_some() && chain.accepts_account_xpub() {
                vec![WalletKeyKind::AccountPublicKey]
            } else {
                Vec::new()
            }
        }
        WalletSigning::PrivateKey { .. } => vec![WalletKeyKind::PrivateKey],
        WalletSigning::SeedPhrase { .. } if chain.scans_for_balance() => {
            vec![WalletKeyKind::MoneroSpendKey, WalletKeyKind::MoneroViewKey]
        }
        WalletSigning::SeedPhrase { .. } => [
            (
                crate::derivation::key_formats::export_format(chain).is_some()
                    && !chain.uses_account_utxo()
                    && !chain.phrase_address_has_stake_key()
                    && (!chain.derives_along_junctions()
                        || crate::derivation::substrate_path::derives_importable_seed(
                            wallet.derivation_path.as_deref().unwrap_or_default(),
                            wallet.derivation_overrides.hmac_key.as_deref() == Some("uniform"),
                        )),
                WalletKeyKind::PrivateKey,
            ),
            (
                account_key_version(wallet).is_some(),
                WalletKeyKind::AccountPublicKey,
            ),
        ]
        .into_iter()
        .filter_map(|(offered, kind)| offered.then_some(kind))
        .collect(),
    }
}

/// The version an account key on `wallet`'s path is written with: its
/// network's first encoding for the script the path's profile pays
/// (`Chain::account_key_versions`). `None` where the network takes no
/// account key, the path is no profile's, or the script has no encoding, as
/// for Taproot.
fn account_key_version(wallet: &WalletState) -> Option<[u8; 4]> {
    let chain = wallet.chain_id;
    let path = wallet.derivation_path.clone()?;
    crate::derivation::path::derivation_profile_of_path(chain, path.clone())?;
    let script = crate::derivation::dispatch::script_type_for_path(&path);
    chain
        .account_key_versions()
        .iter()
        .find(|version| version.script == script)
        .map(|version| version.version)
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// What the wallet's keys can be exported as. Reads no secret.
    pub async fn wallet_key_exports(
        &self,
        wallet_id: String,
    ) -> Result<Vec<WalletKeyKind>, SpectraBridgeError> {
        Ok(exportable_keys(&self.stored_wallet(&wallet_id).await?))
    }

    /// One of the wallet's keys, written as its network's wallets import it.
    /// Opens the wallet's seal with its password where the key is derived
    /// from the secret; a watched account key needs none.
    pub async fn export_wallet_key(
        &self,
        wallet_id: String,
        kind: WalletKeyKind,
        password: Option<String>,
    ) -> Result<WalletKeyExport, SpectraBridgeError> {
        let wallet = self.stored_wallet(&wallet_id).await?;
        if !exportable_keys(&wallet).contains(&kind) {
            return Err(SpectraBridgeError::invalid(
                "This wallet's keys cannot be exported that way.",
            ));
        }
        let chain = wallet.chain_id;
        let password = password.map(zeroize::Zeroizing::new);
        let password = password.as_deref().map(String::as_str);
        let store = || self.secrets();
        let (format, value) = match (wallet.signing, kind) {
            (WalletSigning::WatchOnly, _) => (
                WalletSecretFormat::AccountXpub,
                zeroize::Zeroizing::new(wallet.xpub.clone().unwrap_or_default()),
            ),
            (WalletSigning::PrivateKey { .. }, _) => {
                let key = crate::store::wallet_secrets::load_private_key(
                    &*store()?,
                    &wallet.id,
                    password,
                )?;
                export_private_key(chain, &key)?
            }
            (WalletSigning::SeedPhrase { .. }, kind) => {
                let phrase = crate::store::wallet_secrets::load_seed_phrase(
                    &*store()?,
                    &wallet.id,
                    password,
                )?;
                let overrides = &wallet.derivation_overrides;
                match kind {
                    WalletKeyKind::MoneroSpendKey | WalletKeyKind::MoneroViewKey => {
                        let (_, _, keys) = crate::derivation::monero::derive_from_seed_phrase(
                            !chain.is_testnet(),
                            &phrase,
                            false,
                            false,
                            true,
                        )?;
                        let keys = zeroize::Zeroizing::new(keys.unwrap_or_default());
                        let half = if kind == WalletKeyKind::MoneroSpendKey {
                            &keys[..64]
                        } else {
                            &keys[64..]
                        };
                        (
                            WalletSecretFormat::HexSecret32,
                            zeroize::Zeroizing::new(half.to_string()),
                        )
                    }
                    WalletKeyKind::AccountPublicKey => {
                        let version = account_key_version(&wallet).ok_or_else(|| {
                            SpectraBridgeError::failure("This wallet has no account key")
                        })?;
                        let account =
                            super::address_discovery::UtxoDerivation::account_private_key(
                                chain,
                                &phrase,
                                wallet.derivation_path.as_deref().unwrap_or_default(),
                                overrides,
                            )?;
                        (
                            WalletSecretFormat::AccountXpub,
                            zeroize::Zeroizing::new(
                                account
                                    .to_neutered(&secp256k1::Secp256k1::new())
                                    .to_xpub_string(version),
                            ),
                        )
                    }
                    WalletKeyKind::PrivateKey => {
                        let key = if chain.has_wallet_versions() {
                            let (secret, _) = crate::derivation::ton::ton_key_pair(
                                &phrase,
                                overrides.passphrase.as_deref(),
                            )?;
                            zeroize::Zeroizing::new(hex::encode(*secret))
                        } else {
                            let derived = crate::derivation::dispatch::derive_for_chain(
                                chain,
                                &phrase,
                                wallet.derivation_path.as_deref().unwrap_or_default(),
                                overrides.passphrase.as_deref(),
                                overrides.hmac_key.as_deref(),
                                None,
                                false,
                                false,
                                true,
                            )?;
                            zeroize::Zeroizing::new(derived.private_key_hex.unwrap_or_default())
                        };
                        export_private_key(chain, &key)?
                    }
                }
            }
        };
        Ok(WalletKeyExport {
            kind,
            format,
            value: value.to_string(),
        })
    }
}

/// A key in hex, written in its network's export encoding.
fn export_private_key(
    chain: Chain,
    key_hex: &str,
) -> Result<(WalletSecretFormat, zeroize::Zeroizing<String>), SpectraBridgeError> {
    let format = crate::derivation::key_formats::export_format(chain)
        .ok_or_else(|| SpectraBridgeError::failure("This network takes no private key"))?;
    let value = crate::derivation::key_formats::encode_private_key(chain, format, key_hex)?;
    Ok((format, value))
}

#[cfg(test)]
#[path = "tests/wallet_keys.rs"]
mod tests;
