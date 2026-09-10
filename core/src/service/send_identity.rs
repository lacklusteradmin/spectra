//! Resolve a stored wallet into one signing identity before any provider reads.
use super::*;
use crate::derivation::types::BitcoinScriptType;
use crate::store::wallet_domain::SensitiveOverrides;
use crate::store::wallet_secrets::{load_signing_material, SigningMaterial};
use zeroize::Zeroizing;

pub(super) struct ResolvedSendIdentity {
    pub from_address: String,
    pub private_key_hex: Zeroizing<String>,
    pub public_key_hex: Option<String>,
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
        chain_id: String,
        password: Option<String>,
    ) -> Result<String, SpectraBridgeError> {
        let password = password.map(Zeroizing::new);
        let chain = chain_for_id(&chain_id)?;
        Ok(self
            .resolve_send_identity(chain, &wallet_id, password.as_ref().map(|p| p.as_str()))
            .await?
            .from_address)
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
        if wallet.is_watch_only {
            return Err(invalid("a watch-only wallet cannot send"));
        }
        let stored = wallet
            .address_on(chain)
            .ok_or_else(|| invalid("wallet has no address on the requested chain"))?;
        let name = chain.chain_display_name();
        if !crate::send::flow::is_valid_send_address(name.into(), stored.into()) {
            return Err(invalid(
                "stored sender address is invalid for the requested chain",
            ));
        }
        let from_address = crate::send::flow::normalize_address(name, stored);
        let secrets = self.secrets()?;
        let material = load_signing_material(&*secrets, wallet_id, password)
            .map_err(|error| invalid(&error.to_string()))?;
        let (derived, private_key_hex) = match material {
            SigningMaterial::Mnemonic(seed) => {
                let account = match wallet.derivation_preset.as_str() {
                    "account1" => 1,
                    "account2" => 2,
                    _ => 0,
                };
                let defaults = crate::app_core_derivation_paths_for_preset(account)?;
                let path = wallet
                    .derivation_path
                    .as_deref()
                    .or_else(|| defaults.path_for(chain))
                    .unwrap_or_default();
                let path = crate::app_core_resolve_derivation_path(name.into(), path.into())?
                    .normalized_path;
                let overrides = &sensitive_overrides.0;
                let script = match overrides
                    .script_type
                    .as_deref()
                    .map(str::to_ascii_lowercase)
                    .as_deref()
                {
                    None => crate::derivation::dispatch::script_type_for_path(&path),
                    Some("p2pkh") => BitcoinScriptType::P2pkh,
                    Some("p2shp2wpkh" | "p2sh-p2wpkh") => BitcoinScriptType::P2shP2wpkh,
                    Some("p2wpkh") => BitcoinScriptType::P2wpkh,
                    Some("p2tr") => BitcoinScriptType::P2tr,
                    Some(_) => return Err(invalid("unsupported stored script type")),
                };
                let mut derived = crate::derivation::dispatch::derive_for_chain_name(
                    name,
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
                let derived = crate::derivation::dispatch::core_derive_from_private_key(
                    name.into(),
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
        if !is_named_account
            && crate::send::flow::normalize_address(name, derived_address) != from_address
        {
            return Err(invalid(
                "stored sender address does not match the wallet signing key",
            ));
        }
        Ok(ResolvedSendIdentity {
            from_address,
            private_key_hex,
            public_key_hex: derived.public_key_hex,
        })
    }
}

#[cfg(test)]
mod tests;
