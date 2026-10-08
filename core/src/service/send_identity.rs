//! Resolve a stored wallet into one signing identity before any provider reads.
use super::*;
use crate::store::wallet_domain::SensitiveOverrides;
use crate::store::wallet_secrets::{SigningMaterial, load_signing_material};
use zeroize::Zeroizing;

pub(super) struct ResolvedSendIdentity {
    pub from_address: String,
    pub private_key_hex: Zeroizing<String>,
    pub public_key_hex: Option<String>,
    pub account_utxo_sources: Vec<super::send_utxo_sources::UtxoSigningSource>,
}

fn invalid(message: &str) -> SpectraBridgeError {
    SpectraBridgeError::InvalidInput {
        message: message.into(),
    }
}

impl WalletService {
    /// Offline CLI inspection of the same identity that execute_send resolves.
    /// No signing material is returned. NEAR named-account access is checked by
    /// its protocol client before signing, because it needs an on-chain lookup.
    pub async fn send_identity_address(
        &self,
        wallet_id: String,
        chain: crate::registry::Chain,
        password: Option<String>,
    ) -> Result<String, SpectraBridgeError> {
        let password = password.map(Zeroizing::new);
        Ok(self
            .resolve_send_identity(chain, &wallet_id, password.as_ref().map(|p| p.as_str()))
            .await?
            .from_address)
    }

    /// A wallet's BIP-39 seed, with its passphrase: what Zcash's shielded
    /// account and Litecoin's MWEB keys derive from. A wallet holding a
    /// private key has none, and is refused with `no_seed`.
    pub(super) async fn resolve_bip39_seed(
        &self,
        wallet_id: &str,
        password: Option<&str>,
        no_seed: &str,
    ) -> Result<Zeroizing<Vec<u8>>, SpectraBridgeError> {
        let mut wallet = self
            .wallet_state
            .read()
            .await
            .wallets
            .iter()
            .find(|wallet| wallet.id == wallet_id)
            .cloned()
            .ok_or_else(|| invalid("send wallet does not exist"))?;
        let overrides = SensitiveOverrides::take_from(&mut wallet);
        let secrets = self.secrets()?;
        match load_signing_material(&*secrets, wallet_id, password)
            .map_err(|error| invalid(&error.to_string()))?
        {
            SigningMaterial::Mnemonic(phrase) => {
                let seed = crate::derivation::primitives::derive_bip39_seed(
                    &phrase,
                    overrides.passphrase().unwrap_or(""),
                    0,
                    None,
                    None,
                )?;
                Ok(Zeroizing::new(seed.to_vec()))
            }
            SigningMaterial::PrivateKey(_) => Err(invalid(no_seed)),
        }
    }

    pub(super) async fn resolve_send_identity(
        &self,
        chain: Chain,
        wallet_id: &str,
        password: Option<&str>,
    ) -> Result<ResolvedSendIdentity, SpectraBridgeError> {
        let mut wallet = self
            .wallet_state
            .read()
            .await
            .wallets
            .iter()
            .find(|wallet| wallet.id == wallet_id)
            .cloned()
            .ok_or_else(|| invalid("send wallet does not exist"))?;
        let sensitive_overrides = SensitiveOverrides::take_from(&mut wallet);
        if wallet.is_watch_only() {
            return Err(invalid("a watch-only wallet cannot send"));
        }
        let stored = wallet
            .address_on(chain)
            .ok_or_else(|| invalid("wallet has no address on the requested chain"))?;
        let id = chain;
        if !crate::send::flow::is_valid_send_address(id, stored.into()) {
            return Err(invalid(
                "stored sender address is invalid for the requested chain",
            ));
        }
        let from_address = crate::send::flow::normalize_address(id, stored);
        let secrets = self.secrets()?;
        let material = load_signing_material(&*secrets, wallet_id, password)
            .map_err(|error| invalid(&error.to_string()))?;
        let account_utxo_sources = if chain.uses_account_utxo() {
            sensitive_overrides.0.validate_for_chain(chain)?;
            self.resolve_account_utxo_signing_sources(
                &wallet,
                chain,
                &material,
                &sensitive_overrides,
            )
            .await?
        } else {
            Vec::new()
        };
        let (derived, private_key_hex) = match material {
            SigningMaterial::Mnemonic(seed) => {
                let owner = if wallet.chain_id.is_evm() && chain.is_evm() {
                    wallet.chain_id
                } else {
                    chain
                };
                let path = wallet
                    .address_record_on(chain)
                    .and_then(|a| a.derivation_path.as_deref())
                    .or_else(|| {
                        (wallet.chain_id == owner)
                            .then_some(wallet.derivation_path.as_deref())
                            .flatten()
                    })
                    .unwrap_or_default();
                let path = crate::derivation::path::resolve_derivation_path(id, path.into())?;
                let overrides = &sensitive_overrides.0;
                overrides.validate_for_chain(chain)?;
                let script = crate::derivation::dispatch::script_type_for_path(&path);
                let mut derived = crate::derivation::dispatch::derive_for_chain(
                    id,
                    &seed,
                    &path,
                    sensitive_overrides.passphrase(),
                    overrides.hmac_key.as_deref().filter(|s| !s.is_empty()),
                    Some(script),
                    true,
                    true,
                    true,
                )?;
                let key = Zeroizing::new(
                    derived
                        .private_key_hex
                        .take()
                        .ok_or_else(|| invalid("derivation returned no private key"))?,
                );
                (derived, key)
            }
            SigningMaterial::PrivateKey(key) => {
                if !chain.derives_from_private_key() {
                    return Err(invalid(
                        "requested chain does not support private-key wallets",
                    ));
                }
                let key = Zeroizing::new(
                    key.trim()
                        .strip_prefix("0x")
                        .unwrap_or(key.trim())
                        .to_string(),
                );
                let derived = crate::derivation::dispatch::derive_from_private_key(
                    id,
                    key.to_string(),
                    true,
                    true,
                )?
                .ok_or_else(|| invalid("private-key derivation is unavailable for this chain"))?;
                (derived, key)
            }
        };
        let derived_address = derived
            .address
            .as_deref()
            .ok_or_else(|| invalid("derivation returned no sender address"))?;
        let is_named_account = chain.supports_named_sender_accounts()
            && !(from_address.len() == 64 && from_address.bytes().all(|b| b.is_ascii_hexdigit()));
        let derived_matches = if chain.has_wallet_versions() {
            // A TON key holds one account per wallet version, and the
            // stored address names which; the derived one is the default's.
            let public: [u8; 32] = derived
                .public_key_hex
                .as_deref()
                .and_then(|key| hex::decode(key).ok())
                .and_then(|key| key.try_into().ok())
                .ok_or_else(|| invalid("derivation returned no public key"))?;
            crate::derivation::ton::TonWalletVersion::of_address(&public, &from_address, chain)
                .is_some()
        } else {
            crate::send::flow::normalize_address(id, derived_address) == from_address
        };
        if !is_named_account && !derived_matches {
            return Err(invalid(
                "stored sender address does not match the wallet signing key",
            ));
        }
        Ok(ResolvedSendIdentity {
            from_address,
            private_key_hex,
            public_key_hex: derived.public_key_hex,
            account_utxo_sources,
        })
    }
}

#[cfg(test)]
#[path = "tests/send_identity.rs"]
mod tests;
