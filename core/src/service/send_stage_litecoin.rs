//! Litecoin spends every known source in the wallet's selected account.
use super::*;
use crate::send::payload::PreparedSubmission;
use crate::send::stages::{
    LitecoinPreparedInput, LitecoinSendSource, PreparedLitecoinTransaction, PreparedPayload,
    StoredSend,
};
use crate::store::state::{WalletSigning, WalletState};
use crate::store::wallet_domain::SensitiveOverrides;
use crate::store::wallet_secrets::SigningMaterial;
use std::collections::{BTreeMap, BTreeSet};
use zeroize::Zeroizing;

pub(super) struct LitecoinSigningSource {
    pub source: LitecoinSendSource,
    pub private_key_hex: Zeroizing<String>,
}

fn litecoin_root_path(wallet: &WalletState, chain: Chain) -> Result<String, SpectraBridgeError> {
    let defaults = crate::derivation::path::derivation_paths_for_preset(wallet.derivation_preset)?;
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
        .or_else(|| defaults.path_for(chain))
        .unwrap_or_default();
    crate::derivation::path::resolve_derivation_path(chain, path.into())
}

fn litecoin_account_path(chain: Chain, path: &str) -> Result<Vec<u32>, SpectraBridgeError> {
    let indices = crate::derivation::bitcoin::parse_bip32_path(path)?;
    if indices.len() != 5
        || crate::derivation::path::utxo_discovery_index(path, chain, indices[3])
            != Some(indices[4])
    {
        return Err(SpectraBridgeError::invalid(
            "Litecoin source must use a legacy or SegWit account address path on its network",
        ));
    }
    Ok(indices)
}

fn litecoin_source(
    chain: Chain,
    root: &str,
    root_path: Option<&str>,
    address: String,
    path: Option<String>,
) -> Result<LitecoinSendSource, SpectraBridgeError> {
    use crate::derivation::utxo_address::{ParsedUtxoAddress, parse_utxo_address};
    let parsed = parse_utxo_address(chain, &address)?;
    let supported = match &parsed {
        ParsedUtxoAddress::P2pkh(_) | ParsedUtxoAddress::P2sh(_) => true,
        ParsedUtxoAddress::Witness {
            version: 0,
            program,
        } => program.len() == 20,
        _ => false,
    };
    if !supported {
        return Err(SpectraBridgeError::invalid(
            "Litecoin sources must be P2PKH, P2SH-P2WPKH or P2WPKH",
        ));
    }
    let path = match root_path {
        Some(root_path) => {
            let path = path
                .or_else(|| (address == root).then(|| root_path.to_string()))
                .ok_or_else(|| {
                    SpectraBridgeError::invalid("Litecoin source has no derivation path")
                })?;
            let account = litecoin_account_path(chain, root_path)?;
            let candidate = litecoin_account_path(chain, &path)?;
            if candidate[..3] != account[..3] {
                return Err(SpectraBridgeError::invalid(
                    "Litecoin source belongs to a different wallet account",
                ));
            }
            Some(path)
        }
        None if address == root => None,
        None => {
            return Err(SpectraBridgeError::invalid(
                "Private-key wallet has an unrelated Litecoin source",
            ));
        }
    };
    Ok(LitecoinSendSource {
        address,
        derivation_path: path,
        script_pubkey: parsed.script_pubkey(),
    })
}

impl WalletService {
    /// Secret-free candidate resolution also used by the owned send preview.
    /// Signing later derives every selected address; persisted rows never
    /// authorize deriving another account's key.
    pub(super) async fn litecoin_send_sources(
        &self,
        wallet_id: &str,
        chain: Chain,
    ) -> Result<Vec<LitecoinSendSource>, SpectraBridgeError> {
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
            .ok_or_else(|| SpectraBridgeError::invalid("wallet has no Litecoin address"))?;
        let root_path = match wallet.signing {
            WalletSigning::SeedPhrase { .. } => Some(litecoin_root_path(&wallet, chain)?),
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
        let mut sources = BTreeMap::<String, LitecoinSendSource>::new();
        for (address, path) in candidates {
            let source = litecoin_source(chain, root, root_path.as_deref(), address, path)?;
            if let Some(previous) = sources.get(&source.address)
                && previous != &source
            {
                return Err(SpectraBridgeError::invalid(
                    "Conflicting Litecoin source derivation paths",
                ));
            }
            sources.insert(source.address.clone(), source);
        }
        Ok(sources.into_values().collect())
    }

    pub(super) async fn resolve_litecoin_signing_sources(
        &self,
        wallet: &WalletState,
        chain: Chain,
        material: &SigningMaterial,
        overrides: &SensitiveOverrides,
    ) -> Result<Vec<LitecoinSigningSource>, SpectraBridgeError> {
        let sources = self.litecoin_send_sources(&wallet.id, chain).await?;
        sources
            .into_iter()
            .map(|source| {
                let private_key_hex = match material {
                    SigningMaterial::Mnemonic(seed) => {
                        let path = source.derivation_path.as_deref().ok_or_else(|| {
                            SpectraBridgeError::invalid("Litecoin source has no derivation path")
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
                                "Litecoin source address does not match its wallet derivation path",
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
                crate::send::litecoin::validate_ltc_sender(chain, &source.address, &bytes)?;
                Ok(LitecoinSigningSource {
                    source,
                    private_key_hex,
                })
            })
            .collect()
    }

    pub(super) async fn collect_litecoin_inputs(
        &self,
        chain: Chain,
        wallet_id: &str,
    ) -> Result<Vec<LitecoinPreparedInput>, SpectraBridgeError> {
        let sources = self.litecoin_send_sources(wallet_id, chain).await?;
        let client = self.utxo_client(chain, &[EndpointCapability::Utxo]).await;
        let mut inputs = Vec::new();
        let mut outpoints = BTreeSet::new();
        for source in sources {
            for utxo in client.fetch_utxos(&source.address).await? {
                let outpoint = bitcoin::OutPoint {
                    txid: utxo
                        .txid
                        .parse()
                        .map_err(crate::send::error::SendError::invalid)?,
                    vout: utxo.vout,
                };
                if outpoint.is_null() {
                    return Err(SpectraBridgeError::invalid(
                        "Invalid Litecoin input outpoint",
                    ));
                }
                if !outpoints.insert(outpoint) {
                    return Err(SpectraBridgeError::invalid("Duplicate Litecoin outpoint"));
                }
                if !utxo.status.confirmed {
                    continue;
                }
                let script = source.script_pubkey.clone();
                inputs.push(LitecoinPreparedInput {
                    source: source.clone(),
                    utxo: (outpoint.txid.to_string(), outpoint.vout, utxo.value, script),
                });
            }
        }
        crate::send::litecoin::validate_ltc_values(chain, inputs.iter().map(|input| input.utxo.2))?;
        Ok(inputs)
    }

    pub(super) async fn prepare_litecoin(
        &self,
        chain: Chain,
        request: &crate::send::SendExecutionRequest,
        sender: &str,
        amount: u64,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        // Parse before provider reads, including refusing unsupported MWEB addresses.
        let recipient_script =
            crate::derivation::utxo_address::parse_utxo_address(chain, &request.to_address)?
                .script_pubkey();
        if amount < crate::send::litecoin::litecoin_dust_threshold(chain, &recipient_script)? {
            return Err(SpectraBridgeError::invalid(
                "Litecoin recipient amount is below the dust threshold",
            ));
        }
        let inputs = self
            .collect_litecoin_inputs(chain, &request.wallet_id)
            .await?;
        if inputs.is_empty() {
            return Err(SpectraBridgeError::failure("No spendable inputs"));
        }
        let change_script =
            crate::derivation::utxo_address::parse_utxo_address(chain, sender)?.script_pubkey();
        let vsize = crate::send::litecoin::estimate_ltc_vsize(
            inputs.iter().map(|i| i.utxo.3.as_slice()),
            recipient_script.len(),
            Some(&change_script),
        )?;
        let rate_fee = request
            .fee_rate_svb
            .as_deref()
            .map(|rate| litecoin_fee_for_vsize(rate, vsize))
            .transpose()?;
        let mut fee = match request.fee_sat {
            Some(0) => return Err(SpectraBridgeError::invalid("Invalid fee")),
            Some(fee) if rate_fee.is_some_and(|minimum| fee < minimum) => {
                return Err(SpectraBridgeError::invalid(
                    "Litecoin fee changed; build and review again",
                ));
            }
            Some(fee) => fee,
            None => match rate_fee {
                Some(fee) => fee,
                None => fee_or_static(chain, None)?.max(vsize),
            },
        };
        let change =
            crate::send::accounting::checked_change(inputs.iter().map(|i| i.utxo.2), amount, fee)?;
        if change < crate::send::litecoin::litecoin_dust_threshold(chain, &change_script)? {
            fee = fee
                .checked_add(change)
                .ok_or_else(|| SpectraBridgeError::invalid("Fee overflow"))?;
        }
        Ok(PreparedPayload::Litecoin(PreparedLitecoinTransaction {
            inputs,
            amount,
            fee,
            recipient_script,
        }))
    }

    pub(super) async fn sign_litecoin(
        &self,
        chain: Chain,
        stored: &StoredSend,
        signer: &super::send_identity::ResolvedSendIdentity,
    ) -> Result<(PreparedSubmission, Vec<String>), SpectraBridgeError> {
        let PreparedPayload::Litecoin(prepared) = &stored.prepared else {
            return Err(SpectraBridgeError::invalid("Expected Litecoin transaction"));
        };
        let mut by_address = BTreeMap::new();
        for key in &signer.litecoin_sources {
            by_address.insert(key.source.address.as_str(), key);
        }
        let client = self.utxo_client(chain, &[EndpointCapability::Utxo]).await;
        let mut current = BTreeMap::new();
        let mut keys = Vec::new();
        let mut resources = Vec::new();
        let mut outpoints = BTreeSet::new();
        for input in &prepared.inputs {
            let key = by_address
                .get(input.source.address.as_str())
                .filter(|key| key.source == input.source)
                .ok_or_else(|| {
                    SpectraBridgeError::invalid("Litecoin source changed; build and review again")
                })?;
            if input.utxo.3 != input.source.script_pubkey
                || !outpoints.insert((&input.utxo.0, input.utxo.1))
            {
                return Err(SpectraBridgeError::invalid(
                    "Invalid prepared Litecoin input",
                ));
            }
            if let std::collections::btree_map::Entry::Vacant(entry) =
                current.entry(input.source.address.clone())
            {
                let utxos = client.fetch_utxos(&input.source.address).await?;
                let mut seen = BTreeSet::new();
                for utxo in &utxos {
                    if !seen.insert((utxo.txid.to_ascii_lowercase(), utxo.vout)) {
                        return Err(SpectraBridgeError::invalid("Duplicate Litecoin outpoint"));
                    }
                }
                entry.insert(utxos);
            }
            if !current[&input.source.address].iter().any(|utxo| {
                utxo.txid.eq_ignore_ascii_case(&input.utxo.0)
                    && utxo.vout == input.utxo.1
                    && utxo.value == input.utxo.2
                    && utxo.status.confirmed
            }) {
                return Err(SpectraBridgeError::failure(
                    "Litecoin input changed or was spent; build and review again",
                ));
            }
            keys.push(Zeroizing::new(hex::decode(key.private_key_hex.as_str())?));
            resources.push(format!(
                "{}:utxo:{}:{}",
                chain.str_id(),
                input.utxo.0,
                input.utxo.1
            ));
        }
        let signing_inputs: Vec<_> = prepared
            .inputs
            .iter()
            .zip(&keys)
            .map(|(input, key)| crate::send::litecoin::LtcSigningInput {
                utxo: &input.utxo,
                private_key: key,
            })
            .collect();
        let change_key = Zeroizing::new(hex::decode(signer.private_key_hex.as_str())?);
        let raw = crate::send::litecoin::sign_ltc_inputs_with_output_script(
            chain,
            &signing_inputs,
            &prepared.recipient_script,
            prepared.amount,
            prepared.fee,
            &stored.view.sender,
            &change_key,
        )?;
        let hash = crate::send::payload::bitcoin_transaction_id(&hex::encode(&raw));
        Ok((
            PreparedSubmission {
                payload: hex::encode(raw),
                result_field: "txid".into(),
                transaction_hash: hash,
                nonce: None,
            },
            resources,
        ))
    }
}

/// Exact decimal sat/vB multiplication, rounded up once to whole satoshis.
pub(super) fn litecoin_fee_for_vsize(rate: &str, vsize: u64) -> Result<u64, SpectraBridgeError> {
    let invalid = || SpectraBridgeError::invalid("Invalid Litecoin fee rate");
    let rate = crate::decimal::canonical(rate)
        .filter(|rate| rate != "0")
        .ok_or_else(invalid)?;
    let (whole, fraction) = rate.split_once('.').unwrap_or((&rate, ""));
    let numerator: u128 = format!("{whole}{fraction}")
        .parse()
        .map_err(|_| invalid())?;
    let denominator = 10u128
        .checked_pow(fraction.len().try_into().map_err(|_| invalid())?)
        .ok_or_else(invalid)?;
    let product = numerator
        .checked_mul(u128::from(vsize))
        .ok_or_else(invalid)?;
    let rounded = (product / denominator)
        .checked_add(u128::from(product % denominator != 0))
        .ok_or_else(invalid)?;
    u64::try_from(rounded).map_err(|_| invalid())
}

#[cfg(test)]
#[path = "tests/send_litecoin.rs"]
mod tests;
