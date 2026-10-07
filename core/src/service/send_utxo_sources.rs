//! Resolve every known source in a Litecoin or Peercoin wallet account.
use super::*;
use crate::send::stages::UtxoSendSource;
use crate::store::state::{WalletSigning, WalletState};
use crate::store::wallet_domain::SensitiveOverrides;
use crate::store::wallet_secrets::SigningMaterial;
use std::collections::BTreeMap;
use zeroize::Zeroizing;

pub(super) struct UtxoSigningSource {
    pub source: UtxoSendSource,
    pub private_key_hex: Zeroizing<String>,
}

fn account_utxo_root_path(
    wallet: &WalletState,
    chain: Chain,
) -> Result<String, SpectraBridgeError> {
    // An empty path resolves to the chain's default.
    let path = wallet
        .addresses
        .iter()
        .find(|a| a.chain_id == chain)
        .and_then(|a| a.derivation_path.as_deref())
        .or_else(|| {
            (wallet.chain_id == chain)
                .then_some(wallet.derivation_path.as_deref())
                .flatten()
        })
        .unwrap_or_default();
    crate::derivation::path::resolve_derivation_path(chain, path.into())
}

fn account_utxo_account_path(chain: Chain, path: &str) -> Result<Vec<u32>, SpectraBridgeError> {
    let indices = crate::derivation::bitcoin::parse_bip32_path(path)?;
    if indices.len() != 5
        || crate::derivation::path::utxo_discovery_index(path, chain, indices[3])
            != Some(indices[4])
    {
        return Err(SpectraBridgeError::invalid(
            "UTXO source must use a catalog-supported account address path on its network",
        ));
    }
    Ok(indices)
}

fn account_utxo_source(
    chain: Chain,
    root: &str,
    root_path: Option<&str>,
    address: String,
    path: Option<String>,
) -> Result<UtxoSendSource, SpectraBridgeError> {
    use crate::derivation::utxo_address::{ParsedUtxoAddress, parse_utxo_address};
    let parsed = parse_utxo_address(chain, &address)?;
    let supported = match &parsed {
        ParsedUtxoAddress::P2pkh(_) | ParsedUtxoAddress::P2sh(_) => true,
        ParsedUtxoAddress::Witness {
            version: 0,
            program,
        } => program.len() == 20,
        ParsedUtxoAddress::Witness {
            version: 1,
            program,
        } => chain.mainnet_counterpart() == Chain::Peercoin && program.len() == 32,
        _ => false,
    };
    if !supported {
        return Err(SpectraBridgeError::invalid(
            "UTXO source script is unsupported by its chain's account signer",
        ));
    }
    let path = match root_path {
        Some(root_path) => {
            let path = path
                .or_else(|| (address == root).then(|| root_path.to_string()))
                .ok_or_else(|| SpectraBridgeError::invalid("UTXO source has no derivation path"))?;
            let account = account_utxo_account_path(chain, root_path)?;
            let candidate = account_utxo_account_path(chain, &path)?;
            if candidate[..3] != account[..3] {
                return Err(SpectraBridgeError::invalid(
                    "UTXO source belongs to a different wallet account",
                ));
            }
            Some(path)
        }
        None if address == root => None,
        None => {
            return Err(SpectraBridgeError::invalid(
                "Private-key wallet has an unrelated UTXO source",
            ));
        }
    };
    Ok(UtxoSendSource {
        address,
        derivation_path: path,
        script_pubkey: parsed.script_pubkey(),
    })
}

impl WalletService {
    /// Secret-free candidate resolution also used by the owned send preview.
    /// Signing later derives every selected address; persisted rows never
    /// authorize deriving another account's key.
    pub(super) async fn account_utxo_send_sources(
        &self,
        wallet_id: &str,
        chain: Chain,
    ) -> Result<Vec<UtxoSendSource>, SpectraBridgeError> {
        if !chain.uses_account_utxo() {
            return Err(SpectraBridgeError::invalid(
                "Unsupported account UTXO network",
            ));
        }
        let mut wallet = self
            .wallet_state
            .read()
            .await
            .wallets
            .iter()
            .find(|w| w.id == wallet_id)
            .cloned()
            .ok_or_else(|| SpectraBridgeError::invalid("send wallet does not exist"))?;
        let _overrides = SensitiveOverrides::take_from(&mut wallet);
        let root = wallet
            .address_on(chain)
            .ok_or_else(|| SpectraBridgeError::invalid("wallet has no UTXO address"))?;
        let root_path = match wallet.signing {
            WalletSigning::SeedPhrase { .. } => Some(account_utxo_root_path(&wallet, chain)?),
            WalletSigning::PrivateKey { .. } => None,
            WalletSigning::WatchOnly => {
                return Err(SpectraBridgeError::invalid(
                    "a watch-only wallet cannot send",
                ));
            }
        };
        let mut candidates: Vec<_> = wallet
            .addresses
            .iter()
            .filter(|a| a.chain_id == chain)
            .map(|a| (a.address.clone(), a.derivation_path.clone()))
            .collect();
        candidates.extend(
            self.keypool
                .read()
                .await
                .owned_on(chain)
                .iter()
                .filter(|row| row.wallet_id == wallet_id)
                .map(|row| (row.address.clone(), row.derivation_path.clone())),
        );
        for tx in self
            .transactions_for_wallet(wallet_id.into())
            .await?
            .into_iter()
            .filter(|tx| tx.chain_id == chain)
        {
            if let Some(address) = tx.source_address {
                candidates.push((address, tx.source_derivation_path));
            }
            if let Some(address) = tx.change_address {
                candidates.push((address, tx.change_derivation_path));
            }
        }
        let mut sources = BTreeMap::<String, UtxoSendSource>::new();
        for (address, path) in candidates {
            let source = account_utxo_source(chain, root, root_path.as_deref(), address, path)?;
            if let Some(previous) = sources.get(&source.address)
                && previous != &source
            {
                return Err(SpectraBridgeError::invalid(
                    "Conflicting UTXO source derivation paths",
                ));
            }
            sources.insert(source.address.clone(), source);
        }
        Ok(sources.into_values().collect())
    }

    pub(super) async fn resolve_account_utxo_signing_sources(
        &self,
        wallet: &WalletState,
        chain: Chain,
        material: &SigningMaterial,
        overrides: &SensitiveOverrides,
    ) -> Result<Vec<UtxoSigningSource>, SpectraBridgeError> {
        let sources = self.account_utxo_send_sources(&wallet.id, chain).await?;
        sources
            .into_iter()
            .map(|source| {
                let private_key_hex = match material {
                    SigningMaterial::Mnemonic(seed) => {
                        let path = source.derivation_path.as_deref().ok_or_else(|| {
                            SpectraBridgeError::invalid("UTXO source has no derivation path")
                        })?;
                        let mut derived = crate::derivation::dispatch::derive_for_chain(
                            chain,
                            seed,
                            path,
                            overrides.passphrase(),
                            overrides.0.hmac_key.as_deref().filter(|s| !s.is_empty()),
                            Some(crate::derivation::dispatch::script_type_for_path(path)),
                            true,
                            false,
                            true,
                        )?;
                        if derived.address.as_deref() != Some(source.address.as_str()) {
                            return Err(SpectraBridgeError::invalid(
                                "UTXO source address does not match its wallet derivation path",
                            ));
                        }
                        Zeroizing::new(derived.private_key_hex.take().ok_or_else(|| {
                            SpectraBridgeError::invalid("derivation returned no private key")
                        })?)
                    }
                    SigningMaterial::PrivateKey(key) => Zeroizing::new(
                        key.trim()
                            .strip_prefix("0x")
                            .unwrap_or(key.trim())
                            .to_string(),
                    ),
                };
                let bytes = Zeroizing::new(hex::decode(private_key_hex.as_str())?);
                match chain.mainnet_counterpart() {
                    Chain::Litecoin => {
                        crate::send::litecoin::validate_ltc_sender(chain, &source.address, &bytes)?;
                    }
                    Chain::Peercoin => {
                        crate::send::peercoin::validate_peercoin_sender(
                            chain,
                            &source.address,
                            &bytes,
                        )?;
                    }
                    _ => {
                        return Err(SpectraBridgeError::invalid(
                            "Unsupported account UTXO network",
                        ));
                    }
                }
                Ok(UtxoSigningSource {
                    source,
                    private_key_hex,
                })
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "tests/send_utxo_sources.rs"]
mod tests;
