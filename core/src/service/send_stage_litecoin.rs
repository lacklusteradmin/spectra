//! Litecoin spends every known source in the wallet's selected account.
use super::*;
use crate::send::payload::PreparedSubmission;
use crate::send::stages::{
    PreparedAccountUtxoTransaction, PreparedPayload, StoredSend, UtxoPreparedInput,
};
use std::collections::{BTreeMap, BTreeSet};
use zeroize::Zeroizing;

impl WalletService {
    pub(super) async fn collect_litecoin_inputs(
        &self,
        chain: Chain,
        wallet_id: &str,
    ) -> Result<Vec<UtxoPreparedInput>, SpectraBridgeError> {
        let sources = self.account_utxo_send_sources(wallet_id, chain).await?;
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
                inputs.push(UtxoPreparedInput {
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
        Ok(PreparedPayload::Litecoin(PreparedAccountUtxoTransaction {
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
        for key in &signer.account_utxo_sources {
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
