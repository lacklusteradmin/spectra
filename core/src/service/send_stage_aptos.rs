//! Aptos: building, revalidating, signing and expiring its staged
//! transactions.
use super::send_stage_protocols::StagedAmount;
use super::*;
use crate::send::payload::SignedPayload;
use crate::send::stages::*;

fn aptos_reviewed_gas_price(
    chain: Chain,
    fee: &str,
    max_gas: u64,
) -> Result<u64, SpectraBridgeError> {
    let octas = crate::send::payload::fee_units(fee, u32::from(chain.native_decimals()))?;
    if max_gas == 0 || !octas.is_multiple_of(max_gas) || octas / max_gas == 0 {
        return Err(SpectraBridgeError::invalid("Invalid fee"));
    }
    Ok(octas / max_gas)
}

impl WalletService {
    /// Recheck a prepared Aptos transaction against the chain.
    pub(super) async fn validate_aptos_state(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        let now = crate::store::now_unix() as u64;
        let PreparedPayload::Aptos(prepared) = &stored.prepared else {
            return Ok(());
        };
        let client = AptosClient::new(
            self.endpoints_for(
                chain,
                &[
                    EndpointCapability::Verification,
                    EndpointCapability::Balance,
                ],
            )
            .await,
        );
        client.verify_network(chain).await?;
        if prepared.chain_id() != chain.aptos_chain_id() {
            return Err(SpectraBridgeError::failure(
                "Aptos endpoint or prepared transaction is on the wrong network",
            ));
        }
        let sequence = prepared.body["sequence_number"]
            .as_str()
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| SpectraBridgeError::invalid("Missing Aptos sequence"))?;
        let expiry = prepared.body["expiration_timestamp_secs"]
            .as_str()
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| SpectraBridgeError::invalid("Missing Aptos expiration"))?;
        if sequence != client.fetch_account_info(&stored.view.sender).await?.0 || expiry <= now {
            return Err(SpectraBridgeError::failure(
                "Aptos sequence or expiration is stale; build and review again",
            ));
        }
        let max_gas = chain
            .aptos_max_gas_amount()
            .ok_or_else(|| SpectraBridgeError::invalid("Missing Aptos gas limit"))?;
        let fee = stored
            .request
            .fee_amount
            .as_deref()
            .ok_or_else(|| SpectraBridgeError::invalid("Missing Aptos gas budget"))?;
        let price = aptos_reviewed_gas_price(chain, fee, max_gas)?;
        if prepared.body["max_gas_amount"].as_str() != Some(max_gas.to_string().as_str())
            || prepared.body["gas_unit_price"].as_str() != Some(price.to_string().as_str())
        {
            return Err(SpectraBridgeError::invalid(
                "Aptos gas budget changed; build and review again",
            ));
        }
        let decimals = stored
            .request
            .token_decimals
            .unwrap_or(u32::from(chain.native_decimals()));
        let amount = crate::amount::input::parse_raw_amount(&stored.request.amount_str, decimals)?;
        let native_amount = if let Some(contract) = &stored.request.contract_address {
            if self.token_contract_decimals(chain, contract).await? != Some(decimals) {
                return Err(SpectraBridgeError::failure(
                    "Aptos token precision changed; review again",
                ));
            }
            if u128::from(
                client
                    .fetch_token_balance(&stored.view.sender, contract)
                    .await?,
            ) < amount
            {
                return Err(SpectraBridgeError::failure(
                    "Insufficient Aptos token balance",
                ));
            }
            0
        } else {
            amount
        };
        let budget = u128::from(max_gas)
            .checked_mul(u128::from(price))
            .and_then(|fee| fee.checked_add(native_amount))
            .ok_or_else(|| SpectraBridgeError::invalid("Aptos gas budget overflow"))?;
        if u128::from(client.fetch_balance(&stored.view.sender).await?.octas) < budget {
            return Err(SpectraBridgeError::failure(
                "Insufficient APT for the reviewed gas budget",
            ));
        }
        Ok(())
    }

    pub(super) async fn prepare_aptos(
        &self,
        chain: Chain,
        request: &mut crate::send::SendExecutionRequest,
        sender: &str,
        staged: StagedAmount,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        let StagedAmount {
            eps, amount_u64, ..
        } = staged;
        let to = request.to_address.as_str();
        Ok(match chain.mainnet_counterpart() {
            Chain::Aptos => {
                let max_gas = chain
                    .aptos_max_gas_amount()
                    .ok_or_else(|| SpectraBridgeError::invalid("Missing Aptos gas limit"))?;
                let gas_price = match request.fee_amount.as_deref() {
                    Some(fee) => aptos_reviewed_gas_price(chain, fee, max_gas)?,
                    None => {
                        AptosClient::new(
                            self.endpoints_for(chain, &[EndpointCapability::Fee]).await,
                        )
                        .fetch_gas_price()
                        .await?
                    }
                };
                let gas_budget = gas_price
                    .checked_mul(max_gas)
                    .filter(|&budget| budget > 0)
                    .ok_or_else(|| SpectraBridgeError::invalid("Invalid fee"))?;
                let client = AptosClient::new(eps);
                let sequence = client.fetch_account_info(sender).await?.0;
                client.verify_network(chain).await?;
                let network = chain
                    .aptos_chain_id()
                    .ok_or_else(|| SpectraBridgeError::failure("Missing Aptos network identity"))?;
                let balance = AptosClient::new(
                    self.endpoints_for(chain, &[EndpointCapability::Balance])
                        .await,
                )
                .fetch_balance(sender)
                .await?
                .octas;
                let required = request
                    .contract_address
                    .as_ref()
                    .map_or(u128::from(amount_u64), |_| 0)
                    + u128::from(gas_budget);
                if u128::from(balance) < required {
                    let required = crate::amount::decimal::from_units(
                        required,
                        u32::from(chain.native_decimals()),
                    );
                    let symbol = chain.coin_symbol();
                    return Err(SpectraBridgeError::failed(
                        "Insufficient %@ for the amount plus the network fee (requires %@ %@).",
                        [symbol, required.as_str(), symbol],
                    ));
                }
                request.fee_amount = Some(crate::amount::decimal::from_units(
                    u128::from(gas_budget),
                    u32::from(chain.native_decimals()),
                ));
                if let Some(contract) = &request.contract_address
                    && client.fetch_token_balance(sender, contract).await? < amount_u64
                {
                    return Err(SpectraBridgeError::failure(
                        "Insufficient Aptos token balance",
                    ));
                }
                PreparedPayload::Aptos(crate::send::aptos::prepare_token_transfer(
                    sender,
                    to,
                    amount_u64,
                    sequence,
                    gas_price,
                    max_gas,
                    crate::store::now_unix() as u64 + 600,
                    network,
                    request
                        .contract_address
                        .as_deref()
                        .unwrap_or("0x1::aptos_coin::AptosCoin"),
                )?)
            }
            _ => {
                return Err(SpectraBridgeError::failure("Not a Aptos network"));
            }
        })
    }

    pub(super) async fn sign_staged_aptos(
        &self,
        stored: &StoredSend,
        signer: &super::send_identity::ResolvedSendIdentity,
    ) -> Result<(SignedPayload, Option<String>), SpectraBridgeError> {
        let seed = || crate::send::keys::Ed25519Seed::from_hex(&signer.private_key_hex);
        Ok(match &stored.prepared {
            PreparedPayload::Aptos(p) => {
                let (body, hash) = p.clone().sign(&seed()?)?;
                (SignedPayload::Json(body), Some(hash))
            }
            _ => {
                return Err(SpectraBridgeError::failure("Not a Aptos transaction"));
            }
        })
    }

    /// Whether a signed Aptos transaction can no longer land.
    pub(super) async fn aptos_signature_expired(
        &self,
        stored: &StoredSend,
    ) -> Result<bool, SpectraBridgeError> {
        let now = crate::store::now_unix();
        Ok(match &stored.prepared {
            PreparedPayload::Aptos(p) => p.body["expiration_timestamp_secs"]
                .as_str()
                .and_then(|s| s.parse::<u64>().ok())
                .is_none_or(|expiry| now >= expiry as f64),
            _ => false,
        })
    }
}
