use super::*;
use crate::send::{payload::PreparedSubmission, stages::*};
type Input = (String, u32, u64, Vec<u8>);
impl WalletService {
    async fn fixed_inputs(
        &self,
        chain: Chain,
        sender: &str,
    ) -> Result<Vec<Input>, SpectraBridgeError> {
        let hash =
            crate::derivation::utxo_address::parse_utxo_address(chain, sender)?.require_p2pkh()?;
        let script = crate::send::bitcoin_wire::p2pkh_script(&hash);
        Ok(self
            .utxo_client(chain, &[EndpointCapability::Utxo])
            .await
            .fetch_utxos(sender)
            .await?
            .into_iter()
            .map(|u| (u.txid, u.vout, u.value, script.clone()))
            .collect())
    }
    pub(super) async fn prepare_fixed_utxo(
        &self,
        chain: Chain,
        request: &crate::send::SendExecutionRequest,
        sender: &str,
        amount: u64,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        let recipient_script = utxo_recipient_script(chain, &request.to_address)?;
        let inputs = self.fixed_inputs(chain, sender).await?;
        let quoted_fee = if chain.mainnet_counterpart() == Chain::Dogecoin {
            request.fee_sat.or(request
                .fee_rate_svb
                .as_deref()
                .map(crate::send::payload::dogecoin_fee)
                .transpose()?)
        } else {
            request.fee_sat
        };
        let mut fee = fee_or_static(chain, quoted_fee)?;
        if inputs.is_empty() {
            return Err(SpectraBridgeError::failure("No spendable inputs"));
        }
        let change =
            crate::send::accounting::checked_change(inputs.iter().map(|u| u.2), amount, fee)?;
        if change <= chain.legacy_change_dust()? {
            fee = fee
                .checked_add(change)
                .ok_or_else(|| SpectraBridgeError::failure("Fee overflow"))?;
        }
        Ok(PreparedPayload::FixedUtxo {
            inputs,
            amount,
            fee,
            recipient_script,
        })
    }
    pub(super) async fn sign_fixed_utxo(
        &self,
        chain: Chain,
        stored: &StoredSend,
        signer: &super::send_identity::ResolvedSendIdentity,
    ) -> Result<(PreparedSubmission, Vec<String>), SpectraBridgeError> {
        let PreparedPayload::FixedUtxo {
            inputs,
            amount,
            fee,
            recipient_script: _,
        } = &stored.prepared
        else {
            return Err(SpectraBridgeError::failure("Expected UTXO transaction"));
        };
        let current = self.fixed_inputs(chain, &stored.view.sender).await?;
        let mut resources = Vec::new();
        for input in inputs {
            if !current.contains(input) {
                return Err(SpectraBridgeError::failure(
                    "UTXO changed or was spent; build and review again",
                ));
            }
            resources.push(format!("{}:utxo:{}:{}", chain.str_id(), input.0, input.1));
        }
        let key = zeroize::Zeroizing::new(hex::decode(signer.private_key_hex.as_str())?);
        let dust = Some(chain.legacy_change_dust()?);
        let from = stored.view.sender.as_str();
        let to = stored.view.recipient.as_str();
        use crate::send::*;
        let raw = match chain.mainnet_counterpart() {
            Chain::Dogecoin => {
                dogecoin::sign_doge_p2pkh(chain, inputs, to, *amount, *fee, from, &key, dust)?
            }
            Chain::BitcoinSV => {
                bitcoin_sv::sign_bsv_tx(chain, inputs, to, *amount, *fee, from, &key, dust)?
            }
            Chain::BitcoinCash => {
                bitcoin_cash::sign_bch_tx(chain, inputs, to, *amount, *fee, from, &key, dust)?
            }
            Chain::BitcoinGold => {
                bitcoin_gold::sign_btg_tx(chain, inputs, to, *amount, *fee, from, &key, dust)?
            }
            Chain::Dash => {
                dash::sign_dash_p2pkh(chain, inputs, to, *amount, *fee, from, &key, dust)?
            }
            _ => return Err(SpectraBridgeError::failure("Unsupported UTXO signer")),
        };
        let payload = hex::encode(raw);
        let hash = crate::send::payload::bitcoin_transaction_id(&payload);
        Ok((
            PreparedSubmission {
                payload,
                result_field: "txid".into(),
                transaction_hash: hash,
                nonce: None,
            },
            resources,
        ))
    }
}

fn utxo_recipient_script(chain: Chain, address: &str) -> Result<Vec<u8>, SpectraBridgeError> {
    Ok(crate::derivation::utxo_address::parse_utxo_address(chain, address)?.script_pubkey())
}
