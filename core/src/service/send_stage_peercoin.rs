//! Peercoin spends every owned account source, including mature P2PK rewards.
use super::*;
use crate::send::payload::PreparedSubmission;
use crate::send::stages::{
    PreparedAccountUtxoTransaction, PreparedPayload, StoredSend, UtxoPreparedInput,
};
use std::collections::{BTreeMap, BTreeSet};
use zeroize::Zeroizing;

impl WalletService {
    pub(super) async fn collect_peercoin_inputs(
        &self,
        chain: Chain,
        wallet_id: &str,
    ) -> Result<Vec<UtxoPreparedInput>, SpectraBridgeError> {
        let sources = self.account_utxo_send_sources(wallet_id, chain).await?;
        let client = self.utxo_client(chain, &[EndpointCapability::Utxo]).await;
        let mut inputs = Vec::new();
        let mut seen = BTreeSet::new();
        for source in sources {
            for utxo in client.fetch_peercoin_inputs(&source.address).await? {
                if !seen.insert((utxo.0.clone(), utxo.1)) {
                    return Err(SpectraBridgeError::invalid("Duplicate Peercoin outpoint"));
                }
                if !crate::send::peercoin::peercoin_input_matches_sender(
                    &utxo.3,
                    &source.script_pubkey,
                ) {
                    return Err(SpectraBridgeError::invalid(
                        "Peercoin input does not belong to its wallet source",
                    ));
                }
                inputs.push(UtxoPreparedInput {
                    source: source.clone(),
                    utxo,
                });
            }
        }
        Ok(inputs)
    }

    pub(super) async fn prepare_peercoin(
        &self,
        chain: Chain,
        request: &crate::send::SendExecutionRequest,
        sender: &str,
        amount: u64,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        let recipient_script =
            crate::derivation::utxo_address::parse_utxo_address(chain, &request.to_address)?
                .script_pubkey();
        let sender_script =
            crate::derivation::utxo_address::parse_utxo_address(chain, sender)?.script_pubkey();
        let inputs = self
            .collect_peercoin_inputs(chain, &request.wallet_id)
            .await?;
        let utxos: Vec<_> = inputs.iter().map(|input| input.utxo.clone()).collect();
        let selection = crate::send::peercoin::select_peercoin_inputs(
            chain,
            &utxos,
            amount,
            &recipient_script,
            &sender_script,
            request.fee_sat,
        )?;
        let inputs = selection
            .indices
            .iter()
            .map(|index| inputs[*index].clone())
            .collect();
        Ok(PreparedPayload::Peercoin(PreparedAccountUtxoTransaction {
            inputs,
            amount,
            fee: selection.quote.fee,
            recipient_script,
        }))
    }

    pub(super) async fn sign_peercoin(
        &self,
        chain: Chain,
        stored: &StoredSend,
        signer: &super::send_identity::ResolvedSendIdentity,
    ) -> Result<(PreparedSubmission, Vec<String>), SpectraBridgeError> {
        let PreparedPayload::Peercoin(prepared) = &stored.prepared else {
            return Err(SpectraBridgeError::invalid("Expected Peercoin transaction"));
        };
        let current = self
            .collect_peercoin_inputs(chain, &stored.view.wallet_id)
            .await?;
        let by_address: BTreeMap<_, _> = signer
            .account_utxo_sources
            .iter()
            .map(|key| (key.source.address.as_str(), key))
            .collect();
        let mut keys = Vec::new();
        let mut resources = Vec::new();
        let mut seen = BTreeSet::new();
        for input in &prepared.inputs {
            let key = by_address
                .get(input.source.address.as_str())
                .filter(|key| key.source == input.source)
                .ok_or_else(|| {
                    SpectraBridgeError::invalid("Peercoin source changed; build and review again")
                })?;
            if !seen.insert((&input.utxo.0, input.utxo.1))
                || !current.iter().any(|candidate| {
                    candidate.source == input.source && candidate.utxo == input.utxo
                })
            {
                return Err(SpectraBridgeError::invalid(
                    "Peercoin input changed or was spent; build and review again",
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
            .map(|(input, key)| crate::send::peercoin::PeercoinSigningInput {
                utxo: &input.utxo,
                private_key: key,
            })
            .collect();
        let change_key = Zeroizing::new(hex::decode(signer.private_key_hex.as_str())?);
        let raw = crate::send::peercoin::sign_peercoin_inputs_with_output_script(
            chain,
            &signing_inputs,
            &prepared.recipient_script,
            prepared.amount,
            prepared.fee,
            &stored.view.sender,
            &change_key,
        )?;
        let payload = hex::encode(raw);
        Ok((
            PreparedSubmission {
                transaction_hash: crate::send::payload::bitcoin_transaction_id(&payload),
                payload,
                result_field: "txid".into(),
                nonce: None,
            },
            resources,
        ))
    }

    pub(super) async fn preview_peercoin_owned_send(
        &self,
        chain: Chain,
        wallet_id: &str,
        amount: &str,
        destination: &str,
    ) -> Result<crate::send::preview_types::BitcoinSendPreview, SpectraBridgeError> {
        let amount = u64::try_from(crate::send::amount_input::parse_raw_amount(
            amount,
            u32::from(chain.native_decimals()),
        )?)
        .map_err(|_| SpectraBridgeError::invalid("Amount exceeds protocol range"))?;
        if amount > 0
            && (amount < chain.peercoin_min_output_units()?
                || amount > chain.peercoin_max_money()?)
        {
            return Err(SpectraBridgeError::invalid(
                "Peercoin amount is outside the supported range",
            ));
        }
        let sources = self.account_utxo_send_sources(wallet_id, chain).await?;
        let sender = self
            .wallet_state
            .read()
            .await
            .wallets
            .iter()
            .find(|wallet| wallet.id == wallet_id)
            .and_then(|wallet| wallet.address_on(chain))
            .map(str::to_string)
            .ok_or_else(|| SpectraBridgeError::invalid("wallet has no Peercoin address"))?;
        let change = sources
            .iter()
            .find(|source| source.address == sender)
            .ok_or_else(|| SpectraBridgeError::invalid("Peercoin change address is not owned"))?;
        let recipient = if destination.trim().is_empty() {
            sender.as_str()
        } else {
            destination
        };
        let recipient_script =
            crate::derivation::utxo_address::parse_utxo_address(chain, recipient)?.script_pubkey();
        let inputs = self.collect_peercoin_inputs(chain, wallet_id).await?;
        let total = inputs
            .iter()
            .try_fold(0u64, |total, input| total.checked_add(input.utxo.2))
            .ok_or_else(|| SpectraBridgeError::invalid("Peercoin input total overflow"))?;
        let units = |raw: u64| {
            crate::decimal::from_units(u128::from(raw), u32::from(chain.native_decimals()))
        };
        let mut preview = crate::send::preview_types::BitcoinSendPreview {
            estimatedFeeRateSatVb: chain.peercoin_fee_per_kb_units()? / 1_000,
            estimatedNetworkFee: "0".into(),
            feeRateDescription: Some("0.01 PPC/kB".into()),
            spendableBalance: Some(units(total)),
            estimatedTransactionBytes: Some(0),
            selectedInputCount: Some(inputs.len() as i64),
            usesChangeOutput: Some(false),
            maxSendable: Some("0".into()),
        };
        if inputs.is_empty() && amount == 0 {
            return Ok(preview);
        }
        let utxos: Vec<_> = inputs.iter().map(|input| input.utxo.clone()).collect();
        let quote = crate::send::peercoin::select_peercoin_inputs(
            chain,
            &utxos,
            amount.max(chain.peercoin_min_output_units()?),
            &recipient_script,
            &change.script_pubkey,
            None,
        );
        let selection = match quote {
            Ok(selection) => selection,
            Err(crate::send::error::SendError::InsufficientFunds(_)) if amount == 0 => {
                return Ok(preview);
            }
            Err(error) => return Err(error.into()),
        };
        let quote = selection.quote;
        preview.spendableBalance = Some(units(selection.spendable_balance));
        preview.selectedInputCount = Some(selection.indices.len() as i64);
        preview.estimatedNetworkFee = units(quote.fee);
        preview.estimatedTransactionBytes = Some(quote.estimated_bytes as i64);
        preview.usesChangeOutput = Some(quote.change > 0);
        preview.maxSendable = Some(units(quote.max_sendable));
        Ok(preview)
    }
}

#[cfg(test)]
#[path = "tests/send_peercoin.rs"]
mod tests;
