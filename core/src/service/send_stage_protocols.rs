use super::*;
use crate::api::error::{ApiError, OrDecode};
use crate::send::payload::PreparedSubmission;
use crate::send::stages::*;
use base64::{Engine, engine::general_purpose::STANDARD};

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
    /// Verify the persisted transaction against current chain state without
    /// substituting new fields. Used before signing and before its first send.
    pub(super) async fn validate_prepared_protocol_state(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        let now = crate::store::now_unix() as u64;
        match &stored.prepared {
            PreparedPayload::Near { .. } => self.validate_near_transfer_state(stored).await?,
            PreparedPayload::Evm(prepared) => {
                let client = EvmClient::new(
                    self.endpoints_for(chain, &[EndpointCapability::Verification])
                        .await,
                    chain.evm_chain_id()?,
                );
                let nonce =
                    if stored
                        .request
                        .evm_overrides
                        .as_ref()
                        .and_then(|overrides| overrides.nonce)
                        .is_some()
                    {
                        let response = client
                            .call(
                                "eth_getTransactionCount",
                                json!([stored.view.sender, "latest"]),
                            )
                            .await?;
                        crate::api::evm_json_rpc::parse_hex_u64(response.as_str().ok_or_else(
                            || SpectraBridgeError::failure("Missing confirmed nonce"),
                        )?)?
                    } else {
                        client.fetch_nonce(&stored.view.sender).await?
                    };
                if prepared.chain_id != chain.evm_chain_id()? || nonce > prepared.nonce {
                    return Err(SpectraBridgeError::failure(
                        "Prepared nonce or network is stale; build and review again",
                    ));
                }
                self.validate_evm_fee_budget(chain, prepared).await?;
                self.validate_evm_funds(chain, &stored.view.sender, prepared)
                    .await?;
            }
            PreparedPayload::Sui(_) => {
                let mut request = stored.request.clone();
                let refreshed = self
                    .prepare_staged_protocol(chain, &mut request, &stored.view.sender)
                    .await?;
                if serde_json::to_vec(&refreshed)? != serde_json::to_vec(&stored.prepared)? {
                    return Err(SpectraBridgeError::failure(
                        "Sui objects or gas changed; build and review again",
                    ));
                }
            }
            PreparedPayload::Aptos(prepared) => {
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
                let expected = chain
                    .aptos_chain_id()
                    .ok_or_else(|| SpectraBridgeError::failure("Missing Aptos network identity"))?;
                if client.fetch_ledger_info().await?.0 != u64::from(expected)
                    || prepared.chain_id() != Some(expected)
                {
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
                if sequence != client.fetch_account_info(&stored.view.sender).await?.0
                    || expiry <= now
                {
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
                let amount = crate::send::amount_input::parse_raw_amount(
                    &stored.request.amount_str,
                    decimals,
                )?;
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
            }
            PreparedPayload::Ton {
                seqno,
                amount,
                valid_until,
                jetton,
            } => {
                let client = ToncenterV2Client::new(
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
                if u64::from(*valid_until) <= now
                    || client.fetch_seqno(&stored.view.sender).await? != *seqno
                {
                    return Err(SpectraBridgeError::failure(
                        "TON sequence or expiration changed; build and review again",
                    ));
                }
                let value = if let Some(plan) = jetton {
                    if stored.request.contract_address.as_deref() != Some(plan.master.as_str()) {
                        return Err(SpectraBridgeError::invalid(
                            "Jetton master differs from the reviewed asset",
                        ));
                    }
                    if self.token_contract_decimals(chain, &plan.master).await?
                        != stored.request.token_decimals
                    {
                        return Err(SpectraBridgeError::failure(
                            "Jetton precision changed; review again",
                        ));
                    }
                    if self
                        .ton_transfer_wallet(chain, &stored.view.sender, &plan.master, *amount)
                        .await?
                        != plan.source_wallet
                    {
                        return Err(SpectraBridgeError::failure(
                            "Jetton source wallet changed; review again",
                        ));
                    }
                    u128::from(plan.attached_nanotons)
                } else {
                    *amount
                };
                let fee = chain
                    .static_fee_units()
                    .ok_or_else(|| SpectraBridgeError::failure("TON fee unavailable"))?;
                if u128::from(client.fetch_balance(&stored.view.sender).await?.nanotons)
                    < value + fee
                {
                    return Err(SpectraBridgeError::failure(
                        "Insufficient TON for the reviewed transfer and network fee",
                    ));
                }
            }
            PreparedPayload::Tron(prepared)
                if stored
                    .request
                    .contract_address
                    .as_deref()
                    .is_some_and(|id| chain.token_standard_for_identifier(id) == "TRC-10") =>
            {
                let fee = self
                    .validate_trc10_funds(chain, prepared, &stored.request, &stored.view.sender)
                    .await?;
                let reviewed = stored
                    .request
                    .fee_amount
                    .as_deref()
                    .ok_or_else(|| SpectraBridgeError::invalid("Missing TRC-10 fee budget"))?;
                if crate::send::payload::fee_units(reviewed, u32::from(chain.native_decimals()))?
                    < fee
                {
                    return Err(SpectraBridgeError::failure(
                        "TRC-10 fee budget changed; review again",
                    ));
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn validate_trc10_funds(
        &self,
        chain: Chain,
        prepared: &crate::send::tron::PreparedTronTransfer,
        request: &crate::send::SendExecutionRequest,
        sender: &str,
    ) -> Result<u64, SpectraBridgeError> {
        let id = request
            .contract_address
            .as_deref()
            .ok_or_else(|| SpectraBridgeError::invalid("Missing TRC-10 ID"))?;
        let decimals = request
            .token_decimals
            .ok_or_else(|| SpectraBridgeError::invalid("Missing TRC-10 precision"))?;
        let amount = crate::send::amount_input::parse_raw_amount(&request.amount_str, decimals)?;
        let amount = u64::try_from(amount)
            .ok()
            .filter(|n| *n > 0 && *n <= i64::MAX as u64)
            .ok_or_else(|| {
                SpectraBridgeError::invalid("TRC-10 amount exceeds positive int64 protocol range")
            })?;
        let contract = &prepared.body["raw_data"]["contract"][0];
        let value = &contract["parameter"]["value"];
        let owner = format!(
            "41{}",
            crate::derivation::tron::tron_base58_to_evm_hex(sender)?
        );
        let receiver = format!(
            "41{}",
            crate::derivation::tron::tron_base58_to_evm_hex(&request.to_address)?
        );
        if contract["type"].as_str() != Some("TransferAssetContract")
            || value["asset_name"].as_str() != Some(hex::encode(id).as_str())
            || value["owner_address"].as_str() != Some(owner.as_str())
            || value["to_address"].as_str() != Some(receiver.as_str())
            || value["amount"].as_u64() != Some(amount)
        {
            return Err(SpectraBridgeError::invalid(
                "TRC-10 transaction differs from the reviewed transfer",
            ));
        }
        let expiry = prepared.body["raw_data"]["expiration"]
            .as_u64()
            .ok_or_else(|| SpectraBridgeError::invalid("Missing Tron expiration"))?;
        if expiry <= (crate::store::now_unix() * 1000.0) as u64 {
            return Err(SpectraBridgeError::failure(
                "Tron transaction expired; build and review again",
            ));
        }
        let state = TronHttpClient::new(
            self.endpoints_for(
                chain,
                &[
                    EndpointCapability::Verification,
                    EndpointCapability::TokenBalance,
                    EndpointCapability::Fee,
                ],
            )
            .await,
        )
        .fetch_trc10_transfer_state(
            chain,
            id,
            sender,
            Some(&request.to_address),
            prepared.bandwidth_bytes()?,
        )
        .await?;
        if u32::from(state.token.metadata.decimals) != decimals {
            return Err(SpectraBridgeError::failure(
                "TRC-10 precision changed; review again",
            ));
        }
        if state.token.balance_raw < amount {
            return Err(SpectraBridgeError::failure(
                "Insufficient TRC-10 token balance",
            ));
        }
        if state
            .recipient_balance
            .checked_add(amount)
            .is_none_or(|sum| sum > i64::MAX as u64)
        {
            return Err(SpectraBridgeError::failure(
                "TRC-10 recipient balance would exceed the protocol range",
            ));
        }
        if state.native_balance < state.fee_budget_sun {
            return Err(SpectraBridgeError::failure(
                "Insufficient TRX for the TRC-10 bandwidth and account activation budget",
            ));
        }
        Ok(state.fee_budget_sun)
    }

    async fn ton_transfer_wallet(
        &self,
        chain: Chain,
        owner: &str,
        master: &str,
        amount: u128,
    ) -> Result<String, SpectraBridgeError> {
        let canonical = |value: &str| {
            crate::tokens::normalize_token_identifier(Some(value.to_string()), chain)
                .ok_or_else(|| SpectraBridgeError::invalid("Invalid TON account identity"))
        };
        let master = canonical(master)?;
        let owner = canonical(owner)?;
        let client = crate::api::toncenter_v3::ToncenterV3Client::new(Arc::new(
            self.api_endpoints(
                chain,
                crate::EndpointApi::ToncenterV3,
                &[
                    EndpointCapability::Verification,
                    EndpointCapability::TokenBalance,
                ],
            )
            .await?,
        ));
        let wallet = client.fetch_transfer_wallet(chain, &owner, &master).await?;
        if canonical(&wallet.owner)? != owner || canonical(&wallet.jetton)? != master {
            return Err(SpectraBridgeError::invalid(
                "Jetton source wallet owner or master mismatch",
            ));
        }
        let balance: u128 = wallet
            .balance
            .parse()
            .map_err(|_| SpectraBridgeError::invalid("Invalid jetton balance"))?;
        if balance < amount {
            return Err(SpectraBridgeError::failure("Insufficient jetton balance"));
        }
        canonical(&wallet.address)
    }

    pub(super) async fn prepare_staged_protocol(
        &self,
        chain: Chain,
        request: &mut crate::send::SendExecutionRequest,
        sender: &str,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        let eps = self
            .endpoints_for(chain, &[EndpointCapability::Verification])
            .await;
        let decimals = if let Some(contract) = &request.contract_address {
            let decimals = match chain.mainnet_counterpart() {
                Chain::Solana => u32::from(
                    SolanaClient::new(
                        self.endpoints_for(chain, &[EndpointCapability::TokenBalance])
                            .await,
                    )
                    .fetch_transfer_mint(contract)
                    .await?
                    .decimals,
                ),
                Chain::Near => self
                    .token_contract_decimals(chain, contract)
                    .await?
                    .or(request.token_decimals)
                    .ok_or_else(|| {
                        SpectraBridgeError::failure("NEAR token precision unavailable")
                    })?,
                Chain::Tron => self
                    .token_contract_decimals(chain, contract)
                    .await?
                    .ok_or_else(|| {
                        SpectraBridgeError::failure("TRON token precision unavailable")
                    })?,
                Chain::Sui | Chain::Aptos | Chain::Ton => self
                    .token_contract_decimals(chain, contract)
                    .await?
                    .ok_or_else(|| SpectraBridgeError::failure("Token precision unavailable"))?,
                _ => {
                    return Err(SpectraBridgeError::failure(
                        "Token transfer unavailable on this protocol",
                    ));
                }
            };
            if request.token_decimals.is_some_and(|d| d != decimals) {
                return Err(SpectraBridgeError::failure(
                    "Token decimals changed; review again",
                ));
            }
            request.token_decimals = Some(decimals);
            decimals
        } else {
            u32::from(chain.native_decimals())
        };
        let amount = crate::send::amount_input::parse_raw_amount(&request.amount_str, decimals)?;
        let amount_u64 = if matches!(chain.mainnet_counterpart(), Chain::Near | Chain::Polkadot)
            || (chain.mainnet_counterpart() == Chain::Ton && request.contract_address.is_some())
        {
            0
        } else {
            u64::try_from(amount)
                .map_err(|_| SpectraBridgeError::failure("Amount exceeds protocol range"))?
        };
        let to = request.to_address.as_str();
        Ok(match chain.mainnet_counterpart() {
            Chain::Dogecoin
            | Chain::BitcoinSV
            | Chain::BitcoinCash
            | Chain::BitcoinGold
            | Chain::Dash => {
                self.prepare_fixed_utxo(chain, request, sender, amount_u64)
                    .await?
            }
            Chain::Litecoin => {
                self.prepare_litecoin(chain, request, sender, amount_u64)
                    .await?
            }
            Chain::Peercoin => {
                self.prepare_peercoin(chain, request, sender, amount_u64)
                    .await?
            }
            Chain::Monero => {
                PreparedPayload::Monero(self.prepare_monero(request, amount_u64).await?)
            }
            Chain::Icp => PreparedPayload::Icp(
                crate::send::icp_stages::prepare_transfer(
                    &IcpClient::new(
                        self.endpoints_for(
                            chain,
                            &[EndpointCapability::Verification, EndpointCapability::Fee],
                        )
                        .await,
                    ),
                    sender,
                    to,
                    amount_u64,
                )
                .await?,
            ),
            Chain::Zcash => PreparedPayload::Zcash(
                crate::send::zcash_stages::prepare_zcash(
                    &BlockbookClient::new(
                        self.endpoints_for(
                            chain,
                            &[EndpointCapability::Verification, EndpointCapability::Utxo],
                        )
                        .await,
                        chain,
                    ),
                    sender,
                    to,
                    amount_u64,
                    request.fee_sat,
                )
                .await?,
            ),
            Chain::Near => {
                let client = NearClient::new(eps);
                client.verify_network(chain).await?;
                let quote = self
                    .near_send_quote(
                        chain,
                        sender,
                        to,
                        amount,
                        request.contract_address.as_deref(),
                    )
                    .await?;
                if let Some(reviewed) = &request.fee_amount
                    && crate::send::amount_input::parse_raw_amount(
                        reviewed,
                        u32::from(chain.native_decimals()),
                    )? < quote.budget
                {
                    return Err(SpectraBridgeError::invalid(
                        "NEAR fee exceeds reviewed budget; review again",
                    ));
                }
                let required = quote
                    .budget
                    .checked_add(if request.contract_address.is_none() {
                        amount
                    } else {
                        0
                    })
                    .ok_or_else(|| SpectraBridgeError::invalid("NEAR amount and fee overflow"))?;
                if quote.spendable < required {
                    return Err(SpectraBridgeError::invalid(
                        "Insufficient spendable NEAR for amount, protocol fee and storage stake",
                    ));
                }
                if quote.token_balance.is_some_and(|balance| balance < amount) {
                    return Err(SpectraBridgeError::invalid("Insufficient NEP-141 balance"));
                }
                request.fee_amount = Some(crate::decimal::from_units(
                    quote.budget,
                    u32::from(chain.native_decimals()),
                ));
                let public_key: [u8; 32] = if sender.len() == 64
                    && sender.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    hex::decode(sender)?
                        .try_into()
                        .map_err(|_| SpectraBridgeError::failure("Invalid NEAR public key"))?
                } else {
                    let response = client.call("query", json!({"request_type":"view_access_key_list","finality":"final","account_id":sender})).await?;
                    let keys: Vec<_> = response["keys"]
                        .as_array()
                        .ok_or_else(|| SpectraBridgeError::failure("Missing NEAR keys"))?
                        .iter()
                        .filter(|k| k["access_key"]["permission"] == "FullAccess")
                        .collect();
                    if keys.len() != 1 {
                        return Err(crate::SpectraBridgeError::failure(
                            "Named NEAR sender requires one unambiguous full-access signing key",
                        ));
                    }
                    let key = keys[0]["public_key"]
                        .as_str()
                        .and_then(|k| k.strip_prefix("ed25519:"))
                        .ok_or_else(|| SpectraBridgeError::failure("Unsupported NEAR key"))?;
                    crate::derivation::solana::decode_b58_32(key)?
                };
                let nonce = client
                    .fetch_full_access_key_nonce(sender, &bs58::encode(public_key).into_string())
                    .await?
                    .checked_add(1)
                    .ok_or_else(|| SpectraBridgeError::failure("Nonce exhausted"))?;
                let block_hash = crate::derivation::solana::decode_b58_32(
                    &client.fetch_latest_block_hash().await?,
                )?;
                PreparedPayload::Near {
                    public_key,
                    nonce,
                    block_hash,
                    amount,
                    token_contract: request.contract_address.clone(),
                    fee_budget: quote.budget.to_string(),
                }
            }
            Chain::Decred => PreparedPayload::Decred(
                crate::send::decred::prepare_transfer(
                    &InsightClient::new(
                        self.endpoints_for(chain, &[EndpointCapability::Utxo]).await,
                    ),
                    sender,
                    to,
                    amount_u64,
                    request.fee_sat.unwrap_or(
                        u64::try_from(
                            chain
                                .static_fee_units()
                                .ok_or_else(|| SpectraBridgeError::failure("Missing chain fee"))?,
                        )
                        .map_err(|_| SpectraBridgeError::failure("Invalid chain fee"))?,
                    ),
                    None,
                )
                .await?,
            ),
            Chain::Kaspa => PreparedPayload::Kaspa(
                crate::send::kaspa::prepare_transfer(
                    &KaspaClient::new(self.endpoints_for(chain, &[EndpointCapability::Utxo]).await),
                    sender,
                    to,
                    amount_u64,
                    request.fee_sat.unwrap_or(
                        u64::try_from(
                            chain
                                .static_fee_units()
                                .ok_or_else(|| SpectraBridgeError::failure("Missing chain fee"))?,
                        )
                        .map_err(|_| SpectraBridgeError::failure("Invalid chain fee"))?,
                    ),
                    None,
                    None,
                )
                .await?,
            ),
            Chain::Ton => {
                if amount >= 1u128 << 120 {
                    return Err(SpectraBridgeError::invalid(
                        "Jetton amount exceeds 120-bit protocol range",
                    ));
                }
                let client = ToncenterV2Client::new(eps);
                client.verify_network(chain).await?;
                let jetton = if let Some(master) = &request.contract_address {
                    let source_wallet = self
                        .ton_transfer_wallet(chain, sender, master, amount)
                        .await?;
                    Some(crate::send::ton::PreparedJettonTransfer {
                        master: master.clone(),
                        source_wallet,
                        attached_nanotons: 100_000_000,
                    })
                } else {
                    None
                };
                let value = jetton.as_ref().map_or(amount_u64, |j| j.attached_nanotons);
                let network_fee = chain
                    .static_fee_units()
                    .ok_or_else(|| SpectraBridgeError::failure("TON fee unavailable"))?;
                let required = u128::from(value) + network_fee;
                if u128::from(client.fetch_balance(sender).await?.nanotons) < required {
                    return Err(SpectraBridgeError::failure(
                        "Insufficient TON for transfer and network fee",
                    ));
                }
                request.fee_amount = Some(crate::decimal::from_units(
                    network_fee
                        + jetton
                            .as_ref()
                            .map_or(0, |j| u128::from(j.attached_nanotons)),
                    9,
                ));
                let seqno = client.fetch_seqno(sender).await?;
                PreparedPayload::Ton {
                    seqno,
                    amount,
                    // Deployment messages sign the all-ones expiry value.
                    valid_until: if seqno == 0 {
                        u32::MAX
                    } else {
                        u32::try_from(crate::store::now_unix() as u64 + 60)
                            .map_err(|_| SpectraBridgeError::failure("TON expiry overflow"))?
                    },
                    jetton,
                }
            }
            Chain::Xrp => {
                let client = XrplClient::new(eps);
                let fee_drops =
                    XrplClient::new(self.endpoints_for(chain, &[EndpointCapability::Fee]).await)
                        .fetch_fee()
                        .await?;
                crate::send::xrp::validate_drops(u128::from(fee_drops))?;
                PreparedPayload::Xrp {
                    sequence: client.fetch_sequence(sender).await?,
                    fee_drops,
                    amount_drops: amount_u64,
                }
            }
            Chain::Stellar => {
                let client = HorizonClient::new(eps);
                PreparedPayload::Stellar {
                    sequence: client
                        .fetch_sequence(sender)
                        .await?
                        .checked_add(1)
                        .ok_or_else(|| SpectraBridgeError::failure("Sequence exhausted"))?,
                    fee_stroops: HorizonClient::new(
                        self.endpoints_for(chain, &[EndpointCapability::Fee]).await,
                    )
                    .fetch_base_fee()
                    .await?,
                    amount_stroops: i64::try_from(amount)
                        .map_err(|_| SpectraBridgeError::failure("Amount too large"))?,
                }
            }
            Chain::Polkadot | Chain::Bittensor => {
                let prepared = crate::api::http::race(&eps, |endpoint| async move {
                    crate::send::polkadot::prepare_transfer(
                        &SubstrateClient::new(Arc::new(vec![endpoint])),
                        chain,
                        sender,
                        to,
                        amount,
                    )
                    .await
                })
                .await?;
                if let Some(reviewed) = &request.fee_amount {
                    let budget = crate::send::amount_input::parse_raw_amount(
                        reviewed,
                        u32::from(chain.native_decimals()),
                    )?;
                    if prepared.fee > budget {
                        return Err(SpectraBridgeError::failure(
                            "Substrate fee increased; refresh the preview and review again",
                        ));
                    }
                }
                request.fee_amount = Some(crate::decimal::from_units(
                    prepared.fee,
                    u32::from(chain.native_decimals()),
                ));
                PreparedPayload::Substrate(prepared)
            }
            Chain::Cardano => {
                let client = KoiosClient::new(eps);
                let fee = request
                    .fee_amount
                    .as_deref()
                    .map(|v| crate::send::payload::fee_units(v, 6))
                    .transpose()?
                    .unwrap_or(
                        u64::try_from(
                            chain
                                .static_fee_units()
                                .ok_or_else(|| SpectraBridgeError::failure("No Cardano fee"))?,
                        )
                        .map_err(|_| SpectraBridgeError::failure("Invalid fee"))?,
                    );
                let inputs: Vec<_> =
                    KoiosClient::new(self.endpoints_for(chain, &[EndpointCapability::Utxo]).await)
                        .fetch_ada_utxos(sender)
                        .await?
                        .into_iter()
                        .map(|u| (u.tx_hash, u.tx_index, u.lovelace))
                        .collect();
                crate::send::accounting::checked_change(
                    inputs.iter().map(|u| u.2),
                    amount_u64,
                    fee,
                )?;
                PreparedPayload::Cardano {
                    inputs,
                    amount: amount_u64,
                    fee,
                    ttl: client
                        .fetch_latest_slot()
                        .await?
                        .checked_add(7200)
                        .ok_or_else(|| SpectraBridgeError::failure("Slot overflow"))?,
                }
            }
            Chain::Bitcoin => {
                let client = self.utxo_client(chain, &[EndpointCapability::Utxo]).await;
                // A rate, not an amount: the builder rounds the fee it implies
                // up to whole satoshis.
                let rate = match request.fee_rate_svb.as_deref() {
                    Some(rate) => crate::decimal::canonical(rate)
                        .map(|rate| crate::decimal::to_f64(&rate))
                        .filter(|rate| *rate > 0.0)
                        .ok_or_else(|| SpectraBridgeError::failure("Invalid fee rate"))?,
                    None => self.bitcoin_fee_rate(chain).await?.sats_per_vbyte,
                };
                PreparedPayload::Bitcoin(crate::send::bitcoin::PreparedBitcoinTransaction::prepare(
                    chain,
                    sender,
                    to,
                    amount_u64,
                    rate,
                    client.fetch_utxos(sender).await?,
                )?)
            }
            Chain::Solana => PreparedPayload::Solana(
                crate::send::solana::prepare_transfer(
                    &SolanaClient::new(
                        self.endpoints_for(
                            chain,
                            if request.contract_address.is_some() {
                                &[
                                    EndpointCapability::Verification,
                                    EndpointCapability::TokenBalance,
                                ]
                            } else {
                                &[EndpointCapability::Verification]
                            },
                        )
                        .await,
                    ),
                    sender,
                    to,
                    amount_u64,
                    request
                        .contract_address
                        .as_deref()
                        .map(|mint| (mint, decimals as u8)),
                )
                .await?,
            ),
            Chain::Tron => {
                use crate::send::tron::{Transfer, prepare_transfer};
                let trc10 = request
                    .contract_address
                    .as_deref()
                    .is_some_and(|id| chain.token_standard_for_identifier(id) == "TRC-10");
                let transfer = match request.contract_address.as_deref() {
                    Some(asset_id) if chain.token_standard_for_identifier(asset_id) == "TRC-10" => {
                        Transfer::Trc10 {
                            asset_id,
                            to,
                            amount: amount_u64,
                        }
                    }
                    Some(contract) => Transfer::Token {
                        contract,
                        to,
                        amount,
                        fee_limit: 100_000_000,
                    },
                    None => Transfer::Native {
                        to,
                        amount: amount_u64,
                    },
                };
                let client = TronHttpClient::new(eps);
                let reference = if trc10 {
                    client.transfer_reference_for(chain).await?
                } else {
                    client.transfer_reference().await?
                };
                let prepared = prepare_transfer(sender, transfer, reference)?;
                if trc10 {
                    let fee = self
                        .validate_trc10_funds(chain, &prepared, request, sender)
                        .await?;
                    request.fee_amount = Some(crate::decimal::from_units(
                        u128::from(fee),
                        u32::from(chain.native_decimals()),
                    ));
                }
                PreparedPayload::Tron(prepared)
            }
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
                let network = chain
                    .aptos_chain_id()
                    .ok_or_else(|| SpectraBridgeError::failure("Missing Aptos network identity"))?;
                if client.fetch_ledger_info().await?.0 != u64::from(network) {
                    return Err(SpectraBridgeError::failure(
                        "Aptos endpoint is on the wrong network",
                    ));
                }
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
                    let required =
                        crate::decimal::from_units(required, u32::from(chain.native_decimals()));
                    let symbol = chain.coin_symbol();
                    return Err(SpectraBridgeError::failed(
                        "Insufficient %@ for the amount plus the network fee (requires %@ %@).",
                        [symbol, required.as_str(), symbol],
                    ));
                }
                request.fee_amount = Some(crate::decimal::from_units(
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
            Chain::Sui => {
                let gas = request
                    .gas_budget
                    .as_deref()
                    .map(|v| crate::send::payload::fee_units(v, 9))
                    .transpose()?
                    .unwrap_or(10_000_000);
                let client = SuiClient::new(
                    self.endpoints_for(
                        chain,
                        &[
                            EndpointCapability::Verification,
                            EndpointCapability::Balance,
                            EndpointCapability::Fee,
                        ],
                    )
                    .await,
                );
                client.verify_network(chain).await?;
                PreparedPayload::Sui(if let Some(coin_type) = &request.contract_address {
                    crate::send::sui::prepare_token_transfer(
                        &client, sender, to, amount_u64, gas, coin_type,
                    )
                    .await?
                } else {
                    crate::send::sui::prepare_native_transfer(&client, sender, to, amount_u64, gas)
                        .await?
                })
            }
            _ => {
                return Err(SpectraBridgeError::failure(
                    "Transparent preparation unavailable for this protocol",
                ));
            }
        })
    }

    pub(super) async fn sign_staged_protocol(
        &self,
        chain: Chain,
        stored: &StoredSend,
        signer: &super::send_identity::ResolvedSendIdentity,
    ) -> Result<(PreparedSubmission, Vec<String>), SpectraBridgeError> {
        let eps = self
            .endpoints_for(chain, &[EndpointCapability::Verification])
            .await;
        let seed = || crate::send::keys::Ed25519Seed::from_hex(&signer.private_key_hex);
        let mut resources = Vec::new();
        let (payload, field, hash) = match &stored.prepared {
            PreparedPayload::NearFunctionCall(p) => {
                if !NearClient::new(eps)
                    .transaction_block_is_valid(chain, &p.block_hash)
                    .await?
                {
                    return Err(SpectraBridgeError::invalid(
                        "NEAR block reference is stale; review again",
                    ));
                }
                let (raw, hash) = p.sign(&seed()?)?;
                resources.push(format!(
                    "{}:{}:{}:nonce:{}",
                    chain.str_id(),
                    stored.view.sender,
                    hex::encode(p.public_key),
                    p.nonce
                ));
                (
                    json!({"signed_tx_b64":STANDARD.encode(raw)}).to_string(),
                    "txid",
                    Some(hash),
                )
            }
            PreparedPayload::Near {
                public_key,
                nonce,
                block_hash,
                amount,
                token_contract,
                ..
            } => {
                let client = NearClient::new(eps);
                if seed()?.public_key() != *public_key
                    || client
                        .fetch_full_access_key_nonce(
                            &stored.view.sender,
                            &bs58::encode(public_key).into_string(),
                        )
                        .await?
                        .checked_add(1)
                        != Some(*nonce)
                    || !client.transaction_block_is_valid(chain, block_hash).await?
                {
                    return Err(crate::SpectraBridgeError::failure(
                        "NEAR signer, nonce or block reference is stale; build and review again",
                    ));
                }
                let bytes = zeroize::Zeroizing::new(hex::decode(signer.private_key_hex.as_str())?);
                let key: &[u8; 32] = bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| SpectraBridgeError::failure("Invalid NEAR seed"))?;
                let raw = if let Some(contract) = token_contract {
                    let args =
                        crate::send::near::nep141_transfer_args(&stored.view.recipient, *amount)?;
                    crate::send::near::build_near_function_call_tx(
                        &stored.view.sender,
                        public_key,
                        *nonce,
                        contract,
                        "ft_transfer",
                        &args,
                        chain.near_token_gas_limit().unwrap(),
                        1,
                        block_hash,
                        key,
                    )?
                } else {
                    crate::send::near::build_near_transfer_tx(
                        &stored.view.sender,
                        public_key,
                        *nonce,
                        &stored.view.recipient,
                        *amount,
                        block_hash,
                        key,
                    )?
                };
                resources.push(format!(
                    "{}:{}:{}:nonce:{nonce}",
                    chain.str_id(),
                    stored.view.sender,
                    hex::encode(public_key)
                ));
                let hash = crate::send::near::signed_transaction_hash(&raw)?;
                (
                    json!({"signed_tx_b64":STANDARD.encode(raw)}).to_string(),
                    "txid",
                    Some(hash),
                )
            }
            PreparedPayload::Decred(p) => {
                let request = &stored.request;
                let refreshed = crate::send::decred::prepare_transfer(
                    &InsightClient::new(
                        self.endpoints_for(chain, &[EndpointCapability::Utxo]).await,
                    ),
                    &stored.view.sender,
                    &stored.view.recipient,
                    u64::try_from(crate::send::amount_input::parse_raw_amount(
                        &request.amount_str,
                        8,
                    )?)
                    .map_err(|_| SpectraBridgeError::failure("Amount too large"))?,
                    request.fee_sat.unwrap_or(
                        u64::try_from(
                            chain
                                .static_fee_units()
                                .ok_or_else(|| SpectraBridgeError::failure("Missing chain fee"))?,
                        )
                        .map_err(|_| SpectraBridgeError::failure("Invalid chain fee"))?,
                    ),
                    None,
                )
                .await?;
                if serde_json::to_vec(p)? != serde_json::to_vec(&refreshed)? {
                    return Err(SpectraBridgeError::failure(
                        "Decred inputs changed; build and review again",
                    ));
                }
                resources = p.resources();
                let key = decode_private_key(&signer.private_key_hex)?;
                (p.sign(&key)?, "txid", None)
            }
            PreparedPayload::Kaspa(p) => {
                let request = &stored.request;
                let refreshed = crate::send::kaspa::prepare_transfer(
                    &KaspaClient::new(self.endpoints_for(chain, &[EndpointCapability::Utxo]).await),
                    &stored.view.sender,
                    &stored.view.recipient,
                    u64::try_from(crate::send::amount_input::parse_raw_amount(
                        &request.amount_str,
                        8,
                    )?)
                    .map_err(|_| SpectraBridgeError::failure("Amount too large"))?,
                    request.fee_sat.unwrap_or(
                        u64::try_from(
                            chain
                                .static_fee_units()
                                .ok_or_else(|| SpectraBridgeError::failure("Missing chain fee"))?,
                        )
                        .map_err(|_| SpectraBridgeError::failure("Invalid chain fee"))?,
                    ),
                    None,
                    None,
                )
                .await?;
                if serde_json::to_vec(p)? != serde_json::to_vec(&refreshed)? {
                    return Err(SpectraBridgeError::failure(
                        "Kaspa inputs changed; build and review again",
                    ));
                }
                resources = p.resources();
                let key = decode_private_key(&signer.private_key_hex)?;
                (p.sign(&key)?.to_string(), "txid", None)
            }
            PreparedPayload::Ton {
                seqno,
                amount,
                valid_until,
                jetton,
            } => {
                let key = decode_secret_array::<32>(&signer.private_key_hex)?;
                let public = seed()?.public_key();
                // The stored sender names the wallet version this key signs as.
                let wallet = crate::send::ton::TonSigner::for_sender(
                    chain,
                    &stored.view.sender,
                    &key,
                    &public,
                )?;
                let recipient = crate::derivation::ton::parse_ton_address(&stored.view.recipient)?
                    .for_network(chain.is_testnet())?;
                let raw = if let Some(plan) = jetton {
                    crate::send::ton::build_jetton_transfer(
                        &wallet,
                        plan,
                        recipient,
                        crate::derivation::ton::parse_ton_address(&stored.view.sender)?
                            .for_network(chain.is_testnet())?,
                        *amount,
                        *seqno,
                        *valid_until,
                    )?
                } else {
                    crate::send::ton::build_transfer_for_address(
                        &wallet,
                        recipient,
                        u64::try_from(*amount).map_err(|_| {
                            SpectraBridgeError::invalid("TON native amount exceeds protocol range")
                        })?,
                        *seqno,
                        None,
                        *valid_until,
                        3,
                    )?
                };
                resources.push(format!(
                    "{}:{}:sequence:{seqno}",
                    chain.str_id(),
                    stored.view.sender
                ));
                let message_hash = STANDARD.encode(crate::derivation::ton::boc_root_hash(&raw)?);
                (
                    json!({"boc_b64":STANDARD.encode(raw)}).to_string(),
                    "message_hash",
                    Some(message_hash),
                )
            }
            PreparedPayload::Monero(p) => {
                let (raw, hash) = self
                    .sign_monero(p, &stored.view.wallet_id, &signer.private_key_hex)
                    .await?;
                resources = p
                    .input_key_images
                    .iter()
                    .map(|image| format!("{}:key-image:{image}", chain.str_id()))
                    .collect();
                (raw, "txid", Some(hash))
            }
            PreparedPayload::Icp(p) => {
                if (crate::store::now_unix() * 1_000_000_000.0) as u64 >= p.ingress_expiry_ns {
                    return Err(SpectraBridgeError::failure(
                        "ICP transaction expired; build and review again",
                    ));
                }
                resources.push(format!(
                    "{}:{}:transfer:{}:{}",
                    chain.str_id(),
                    stored.view.sender,
                    p.created_at_time_ns,
                    p.memo
                ));
                (p.sign(&seed()?)?, "txid", Some(p.transaction_hash()?))
            }
            PreparedPayload::IcpStaking(p) => {
                if (crate::store::now_unix() * 1_000_000_000.0) as u64 >= p.ingress_expiry_ns {
                    return Err(SpectraBridgeError::failure(
                        "ICP staking expired; build and review again",
                    ));
                }
                let calls = p.sign(&seed()?)?;
                resources.push(format!(
                    "{}:neuron:{}:{}",
                    chain.str_id(),
                    p.controller_hex,
                    p.subaccount_hex
                ));
                let hash = calls.last().map(|c| c.request_id.clone()).ok_or_else(|| {
                    SpectraBridgeError::invalid("Missing ICP staking ingress IDs")
                })?;
                (serde_json::to_string(&calls)?, "ingress_id", Some(hash))
            }
            PreparedPayload::Zcash(p) => {
                let client = BlockbookClient::new(
                    self.endpoints_for(
                        chain,
                        &[EndpointCapability::Verification, EndpointCapability::Utxo],
                    )
                    .await,
                    chain,
                );
                crate::send::zcash_stages::validate_zcash_prepared(&client, p).await?;
                resources = p
                    .inputs
                    .iter()
                    .map(|u| format!("{}:utxo:{}:{}", chain.str_id(), u.0, u.1))
                    .collect();
                let key = decode_private_key(&signer.private_key_hex)?;
                let (raw, hash) = p.sign(&key)?;
                (hex::encode(raw), "txid", Some(hash))
            }
            PreparedPayload::FixedUtxo { .. } => {
                return self.sign_fixed_utxo(chain, stored, signer).await;
            }
            PreparedPayload::Litecoin(_) => {
                return self.sign_litecoin(chain, stored, signer).await;
            }
            PreparedPayload::Peercoin(_) => return self.sign_peercoin(chain, stored, signer).await,
            PreparedPayload::Xrp {
                sequence,
                fee_drops,
                amount_drops,
            } => {
                if XrplClient::new(eps)
                    .fetch_sequence(&stored.view.sender)
                    .await?
                    != *sequence
                {
                    return Err(SpectraBridgeError::failure(
                        "XRP sequence changed; build and review again",
                    ));
                }
                let key = decode_private_key(&signer.private_key_hex)?;
                let public = signer
                    .public_key_hex
                    .as_deref()
                    .ok_or_else(|| SpectraBridgeError::failure("Missing XRP public key"))?;
                let blob = crate::send::xrp::build_signed_payment(
                    &stored.view.sender,
                    &stored.view.recipient,
                    *amount_drops,
                    *fee_drops,
                    *sequence,
                    &key,
                    public,
                )?;
                resources.push(format!(
                    "{}:{}:sequence:{sequence}",
                    chain.str_id(),
                    stored.view.sender
                ));
                (json!({"tx_blob_hex":blob}).to_string(), "txid", None)
            }
            PreparedPayload::Stellar {
                sequence,
                fee_stroops,
                amount_stroops,
            } => {
                if HorizonClient::new(eps)
                    .fetch_sequence(&stored.view.sender)
                    .await?
                    .checked_add(1)
                    != Some(*sequence)
                {
                    return Err(SpectraBridgeError::failure(
                        "Stellar sequence changed; build and review again",
                    ));
                }
                let bytes = zeroize::Zeroizing::new(hex::decode(signer.private_key_hex.as_str())?);
                if !matches!(bytes.len(), 32 | 64) {
                    return Err(SpectraBridgeError::failure("Invalid Stellar signing key"));
                }
                let seed: &[u8; 32] = bytes[..32]
                    .try_into()
                    .map_err(|_| SpectraBridgeError::failure("Invalid Stellar seed"))?;
                let public = ed25519_dalek::SigningKey::from_bytes(seed)
                    .verifying_key()
                    .to_bytes();
                let mut key = zeroize::Zeroizing::new([0u8; 64]);
                key[..32].copy_from_slice(seed);
                key[32..].copy_from_slice(&public);
                let raw = crate::send::stellar::build_signed_payment_xdr(
                    &stored.view.sender,
                    &stored.view.recipient,
                    *amount_stroops,
                    *fee_stroops,
                    *sequence,
                    chain.stellar_network_passphrase()?.as_bytes(),
                    &key,
                    &public,
                )?;
                resources.push(format!(
                    "{}:{}:sequence:{sequence}",
                    chain.str_id(),
                    stored.view.sender
                ));
                (
                    json!({"signed_xdr_b64":STANDARD.encode(raw)}).to_string(),
                    "txid",
                    None,
                )
            }
            PreparedPayload::Substrate(prepared) => {
                crate::api::http::race(&eps, |endpoint| async move {
                    prepared
                        .validate_for_signing(
                            &SubstrateClient::new(Arc::new(vec![endpoint])),
                            chain,
                            &stored.view.sender,
                        )
                        .await
                })
                .await?;
                let bytes = zeroize::Zeroizing::new(hex::decode(signer.private_key_hex.as_str())?);
                let key: &[u8; 32] = bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| SpectraBridgeError::failure("Invalid sr25519 seed"))?;
                let public =
                    hex::decode(signer.public_key_hex.as_deref().ok_or_else(|| {
                        SpectraBridgeError::failure("Missing sr25519 public key")
                    })?)?;
                let public: &[u8; 32] = public
                    .as_slice()
                    .try_into()
                    .map_err(|_| SpectraBridgeError::failure("Invalid sr25519 public key"))?;
                let raw = prepared.sign(key, public)?;
                let hash = format!(
                    "0x{}",
                    hex::encode(crate::send::substrate::blake2b_256(&raw))
                );
                resources.push(format!(
                    "{}:{}:nonce:{}",
                    chain.str_id(),
                    stored.view.sender,
                    prepared.nonce
                ));
                (
                    json!({"extrinsic_hex":format!("0x{}",hex::encode(raw))}).to_string(),
                    "txid",
                    Some(hash),
                )
            }
            PreparedPayload::Cardano {
                inputs,
                amount,
                fee,
                ttl,
            } => {
                let client = KoiosClient::new(eps);
                if client.fetch_latest_slot().await? >= *ttl {
                    return Err(SpectraBridgeError::failure(
                        "Cardano transaction expired; build and review again",
                    ));
                }
                let current =
                    KoiosClient::new(self.endpoints_for(chain, &[EndpointCapability::Utxo]).await)
                        .fetch_ada_utxos(&stored.view.sender)
                        .await?;
                for (hash, index, value) in inputs {
                    if !current
                        .iter()
                        .any(|u| &u.tx_hash == hash && u.tx_index == *index && u.lovelace == *value)
                    {
                        return Err(SpectraBridgeError::failure(
                            "Cardano input changed; build and review again",
                        ));
                    }
                    resources.push(format!("{}:utxo:{hash}:{index}", chain.str_id()));
                }
                let bytes = zeroize::Zeroizing::new(hex::decode(signer.private_key_hex.as_str())?);
                let key: &[u8; 64] = bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| SpectraBridgeError::failure("Invalid Cardano key"))?;
                let public =
                    hex::decode(signer.public_key_hex.as_deref().ok_or_else(|| {
                        SpectraBridgeError::failure("Missing Cardano public key")
                    })?)?;
                let public = decode_hex_array::<32>(&hex::encode(public), "Cardano public key")?;
                let raw = crate::send::cardano::build_signed_ada_tx(
                    inputs,
                    &crate::derivation::cardano::decode_cardano_addr_bytes(&stored.view.recipient)?,
                    *amount,
                    *fee,
                    &crate::derivation::cardano::decode_cardano_addr_bytes(&stored.view.sender)?,
                    key,
                    &public,
                    *ttl,
                    None,
                )?;
                (json!({"cbor_hex":raw}).to_string(), "txid", None)
            }
            PreparedPayload::Bitcoin(p) => {
                let current = self
                    .utxo_client(chain, &[EndpointCapability::Utxo])
                    .await
                    .fetch_utxos(&stored.view.sender)
                    .await?;
                for input in &p.inputs {
                    if !current.iter().any(|u| {
                        u.txid == input.txid && u.vout == input.vout && u.value == input.value
                    }) {
                        return Err(crate::SpectraBridgeError::failure(
                            "Bitcoin input changed or was spent; build and review again",
                        ));
                    }
                    resources.push(format!(
                        "{}:utxo:{}:{}",
                        chain.str_id(),
                        input.txid,
                        input.vout
                    ));
                }
                let raw = p.sign(signer.private_key_hex.to_string())?;
                let hash = crate::send::payload::bitcoin_transaction_id(&raw);
                (raw, "txid", hash)
            }
            PreparedPayload::Solana(p) => {
                if let Some(staking) = &stored.view.staking {
                    let account = if let Some(seed) = &p.account_seed {
                        crate::send::solana::stake_account_address(&stored.view.sender, seed)?
                    } else {
                        staking.position_id.clone().ok_or_else(|| {
                            SpectraBridgeError::invalid("Missing Solana stake account")
                        })?
                    };
                    resources.push(format!("{}:stake:{}", chain.str_id(), account));
                }
                let client = SolanaClient::new(eps);
                let valid = client
                    .call(
                        "isBlockhashValid",
                        json!([p.blockhash, {"commitment":"confirmed"}]),
                    )
                    .await?;
                if valid["value"].as_bool() != Some(true) {
                    return Err(SpectraBridgeError::failure(
                        "Solana blockhash expired; build and review again",
                    ));
                }
                let raw = p.sign(&seed()?)?;
                (
                    STANDARD.encode(&raw),
                    "signature",
                    Some(bs58::encode(&raw[1..65]).into_string()),
                )
            }
            PreparedPayload::Tron(p) => {
                let expiry = p.body["raw_data"]["expiration"]
                    .as_u64()
                    .ok_or_else(|| SpectraBridgeError::failure("Invalid Tron expiration"))?;
                if expiry <= (crate::store::now_unix() * 1000.0) as u64 {
                    return Err(SpectraBridgeError::failure(
                        "Tron transaction expired; build and review again",
                    ));
                }
                let key = decode_private_key(&signer.private_key_hex)?;
                (
                    p.clone().sign(&key)?,
                    "txid",
                    p.body["txID"].as_str().map(str::to_string),
                )
            }
            PreparedPayload::Aptos(p) => {
                let seq = p.body["sequence_number"]
                    .as_str()
                    .and_then(|value| value.parse::<u64>().ok())
                    .ok_or_else(|| SpectraBridgeError::invalid("Missing Aptos sequence"))?;
                resources.push(format!(
                    "{}:{}:sequence:{seq}",
                    chain.str_id(),
                    stored.view.sender
                ));
                let (body, hash) = p.clone().sign(&seed()?)?;
                (
                    json!({"signed_body_json":body}).to_string(),
                    "txid",
                    Some(hash),
                )
            }
            PreparedPayload::Sui(p) => {
                resources = p
                    .objects
                    .iter()
                    .map(|c| {
                        format!(
                            "{}:object:{}:{}",
                            chain.str_id(),
                            hex::encode(c.id),
                            c.version
                        )
                    })
                    .collect();
                let (bytes, signature) = p.clone().sign(&seed()?)?;
                (
                    json!({"tx_bytes_b64":bytes,"sig_b64":signature}).to_string(),
                    "digest",
                    Some(p.transaction_digest()),
                )
            }
            PreparedPayload::Evm(_) => {
                return Err(SpectraBridgeError::failure(
                    "EVM signing uses its typed signer",
                ));
            }
        };
        Ok((
            PreparedSubmission {
                payload,
                result_field: field.into(),
                transaction_hash: hash,
                nonce: None,
            },
            resources,
        ))
    }
}

impl WalletService {
    pub(super) async fn validate_broadcast_endpoint(
        &self,
        chain: Chain,
        endpoint: &str,
    ) -> Result<(), SpectraBridgeError> {
        let allowed = self
            .endpoints_for(chain, &[EndpointCapability::Broadcast])
            .await;
        if !allowed
            .iter()
            .any(|url| url.trim_end_matches('/') == endpoint.trim_end_matches('/'))
        {
            return Err(SpectraBridgeError::failure(
                "Endpoint does not support broadcasts on the selected network",
            ));
        }
        Ok(self.validate_endpoint_network(chain, endpoint).await?)
    }

    /// Refuse an endpoint that is not on `chain`'s network, asking the node
    /// itself where the protocol lets it say.
    pub(super) async fn validate_endpoint_network(
        &self,
        chain: Chain,
        endpoint: &str,
    ) -> Result<(), ApiError> {
        let wrong_network = || ApiError::invalid("Endpoint is on the wrong network");
        let known = crate::endpoints::catalog().records.iter().any(|r| {
            r.chain_id == chain
                && chain.endpoint_apis().contains(&r.api)
                && r.endpoint.trim_end_matches('/') == endpoint.trim_end_matches('/')
        });
        let eps = Arc::new(vec![endpoint.to_string()]);
        if chain.is_evm() {
            let actual = EvmClient::new(eps, chain.evm_chain_id()?)
                .call("eth_chainId", json!([]))
                .await?;
            if crate::api::evm_json_rpc::parse_hex_u64(
                actual
                    .as_str()
                    .or_decode("Missing endpoint network identity")?,
            )? != chain.evm_chain_id()?
            {
                return Err(wrong_network());
            }
        } else if matches!(
            chain.mainnet_counterpart(),
            Chain::Polkadot | Chain::Bittensor
        ) {
            SubstrateClient::new(eps).polkadot_context(chain).await?;
        } else if chain.mainnet_counterpart() == Chain::Sui {
            SuiClient::new(eps).verify_network(chain).await?;
        } else if chain.mainnet_counterpart() == Chain::Solana {
            SolanaClient::new(eps).verify_network(chain).await?;
        } else if chain.mainnet_counterpart() == Chain::Near {
            NearClient::new(eps).verify_network(chain).await?;
        } else if chain.mainnet_counterpart() == Chain::Stellar {
            HorizonClient::new(eps).verify_network(chain).await?;
        } else if chain.mainnet_counterpart() == Chain::Xrp {
            XrplClient::new(eps).verify_network(chain).await?;
        } else if chain.mainnet_counterpart() == Chain::Tron {
            TronHttpClient::new(eps).verify_network(chain).await?;
        } else if chain.mainnet_counterpart() == Chain::Ton {
            ToncenterV2Client::new(eps).verify_network(chain).await?;
        } else if chain.mainnet_counterpart() == Chain::Aptos {
            let expected = chain
                .aptos_chain_id()
                .or_decode("Missing Aptos network identity")?;
            if AptosClient::new(eps).fetch_ledger_info().await?.0 != u64::from(expected) {
                return Err(wrong_network());
            }
        } else if chain.mainnet_counterpart() == Chain::Zcash {
            BlockbookClient::new(eps, chain).zcash_context().await?;
        } else if chain.mainnet_counterpart() == Chain::Peercoin {
            BlockbookClient::new(eps, chain)
                .verify_peercoin_network()
                .await?;
        } else if chain == Chain::Icp {
            IcpClient::new(eps).verify_network().await?;
        } else if chain.mainnet_counterpart() == Chain::Monero {
            crate::api::monero_daemon_rpc::daemon(endpoint, chain).await?;
        } else if !known {
            return Err(ApiError::invalid(
                "Endpoint network and broadcast capability cannot be verified",
            ));
        }
        Ok(())
    }
}

impl WalletService {
    /// Recheck expiry at submission time: a saved signature may outlive its block reference.
    /// Never rebuild or resign here, including after a lost response.
    pub(super) async fn validate_signed_expiry(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        let now = crate::store::now_unix();
        let endpoints = self
            .endpoints_for(chain, &[EndpointCapability::Verification])
            .await;
        let expired = match &stored.prepared {
            PreparedPayload::Substrate(prepared) => {
                crate::api::http::race(&endpoints, |endpoint| async move {
                    prepared
                        .validate_for_submission(
                            &SubstrateClient::new(Arc::new(vec![endpoint])),
                            chain,
                        )
                        .await
                })
                .await?;
                false
            }
            PreparedPayload::Zcash(p) => {
                let (height, branch) = BlockbookClient::new(endpoints, chain)
                    .zcash_context()
                    .await?;
                height >= p.expiry_height || branch != p.upgrade.consensus_branch_id
            }
            PreparedPayload::Icp(p) => (now * 1_000_000_000.0) as u64 >= p.ingress_expiry_ns,
            PreparedPayload::IcpStaking(p) => (now * 1_000_000_000.0) as u64 >= p.ingress_expiry_ns,
            PreparedPayload::Ton { valid_until, .. } => now >= f64::from(*valid_until),
            PreparedPayload::Near { block_hash, .. } => {
                !NearClient::new(endpoints)
                    .transaction_block_is_valid(chain, block_hash)
                    .await?
            }
            PreparedPayload::NearFunctionCall(p) => {
                !NearClient::new(endpoints)
                    .transaction_block_is_valid(chain, &p.block_hash)
                    .await?
            }
            PreparedPayload::Tron(p) => p.body["raw_data"]["expiration"]
                .as_u64()
                .is_none_or(|expiry| now * 1000.0 >= expiry as f64),
            PreparedPayload::Aptos(p) => p.body["expiration_timestamp_secs"]
                .as_str()
                .and_then(|s| s.parse::<u64>().ok())
                .is_none_or(|expiry| now >= expiry as f64),
            PreparedPayload::Cardano { ttl, .. } => {
                KoiosClient::new(endpoints).fetch_latest_slot().await? >= *ttl
            }
            PreparedPayload::Solana(p) => {
                SolanaClient::new(endpoints)
                    .call(
                        "isBlockhashValid",
                        json!([p.blockhash, {"commitment":"confirmed"}]),
                    )
                    .await?["value"]
                    .as_bool()
                    != Some(true)
            }
            _ => false,
        };
        if expired {
            return Err(SpectraBridgeError::failure(
                "Signed transaction expired; inspect its on-chain status before building a new transaction",
            ));
        }
        Ok(())
    }
}
