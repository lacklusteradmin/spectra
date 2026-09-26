use super::*;
use crate::send::payload::PreparedSubmission;
use crate::send::stages::*;
use base64::{Engine, engine::general_purpose::STANDARD};

impl WalletService {
    pub(super) async fn prepare_staged_protocol(
        &self,
        chain: Chain,
        request: &mut crate::send::SendExecutionRequest,
        sender: &str,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        let eps = self.endpoints_for(chain.str_id(), &["verification"]).await;
        let decimals = if let Some(contract) = &request.contract_address {
            let decimals = match chain.mainnet_counterpart() {
                Chain::Solana => u32::from(
                    SolanaClient::new(self.endpoints_for(chain.str_id(), &["token-balance"]).await)
                        .fetch_transfer_mint(contract)
                        .await?
                        .1,
                ),
                Chain::Near => self
                    .token_contract_decimals(chain, contract)
                    .await?
                    .or(request.token_decimals)
                    .ok_or("NEAR token precision unavailable")?,
                Chain::Tron => u32::from(
                    TronClient::new(self.endpoints_for(chain.str_id(), &["token-balance"]).await)
                        .fetch_trc20_metadata(contract)
                        .await?
                        .decimals,
                ),
                _ => return Err("Token transfer unavailable on this protocol".into()),
            };
            if request.token_decimals.is_some_and(|d| d != decimals) {
                return Err("Token decimals changed; review again".into());
            }
            request.token_decimals = Some(decimals);
            decimals
        } else {
            u32::from(chain.native_decimals())
        };
        let amount = crate::send::amount_input::parse_raw_amount(&request.amount_str, decimals)?;
        let amount_u64 = if chain.mainnet_counterpart() == Chain::Near {
            0
        } else {
            u64::try_from(amount).map_err(|_| "Amount exceeds protocol range")?
        };
        let to = request.to_address.as_str();
        Ok(match chain.mainnet_counterpart() {
            Chain::Dogecoin
            | Chain::BitcoinSV
            | Chain::BitcoinCash
            | Chain::BitcoinGold
            | Chain::Litecoin
            | Chain::Dash => {
                self.prepare_fixed_utxo(chain, request, sender, amount_u64)
                    .await?
            }
            Chain::Monero => {
                PreparedPayload::Monero(self.prepare_monero(request, amount_u64).await?)
            }
            Chain::Icp => PreparedPayload::Icp(
                IcpClient::new(
                    self.endpoints_for(chain.str_id(), &["verification", "fee"])
                        .await,
                )
                .prepare_transfer(sender, to, amount_u64)
                .await?,
            ),
            Chain::Zcash => PreparedPayload::Zcash(
                BlockbookClient::new(
                    self.endpoints_for(chain.str_id(), &["verification", "utxo"])
                        .await,
                    chain,
                )
                .prepare_zcash(sender, to, amount_u64, request.fee_sat)
                .await?,
            ),
            Chain::Near => {
                let client = NearClient::new(eps);
                let public_key: [u8; 32] = if sender.len() == 64
                    && sender.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    hex::decode(sender)
                        .map_err(|e| e.to_string())?
                        .try_into()
                        .map_err(|_| "Invalid NEAR public key")?
                } else {
                    let response = client.call("query", json!({"request_type":"view_access_key_list","finality":"final","account_id":sender})).await?;
                    let keys: Vec<_> = response["keys"]
                        .as_array()
                        .ok_or("Missing NEAR keys")?
                        .iter()
                        .filter(|k| k["access_key"]["permission"] == "FullAccess")
                        .collect();
                    if keys.len() != 1 {
                        return Err(
                            "Named NEAR sender requires one unambiguous full-access signing key"
                                .into(),
                        );
                    }
                    let key = keys[0]["public_key"]
                        .as_str()
                        .and_then(|k| k.strip_prefix("ed25519:"))
                        .ok_or("Unsupported NEAR key")?;
                    crate::derivation::solana::decode_b58_32(key)?
                };
                let nonce = client
                    .fetch_access_key_nonce(sender, &bs58::encode(public_key).into_string())
                    .await?
                    .checked_add(1)
                    .ok_or("Nonce exhausted")?;
                let block_hash = crate::derivation::solana::decode_b58_32(
                    &client.fetch_latest_block_hash().await?,
                )?;
                PreparedPayload::Near {
                    public_key,
                    nonce,
                    block_hash,
                    amount,
                    token_contract: request.contract_address.clone(),
                }
            }
            Chain::Decred => PreparedPayload::Decred(
                DecredClient::new(self.endpoints_for(chain.str_id(), &["utxo"]).await)
                    .prepare_transfer(
                        sender,
                        to,
                        amount_u64,
                        request.fee_sat.unwrap_or(
                            u64::try_from(chain.static_fee_units().ok_or("Missing chain fee")?)
                                .map_err(|_| "Invalid chain fee")?,
                        ),
                        None,
                    )
                    .await?,
            ),
            Chain::Kaspa => PreparedPayload::Kaspa(
                KaspaClient::new(self.endpoints_for(chain.str_id(), &["utxo"]).await)
                    .prepare_transfer(
                        sender,
                        to,
                        amount_u64,
                        request.fee_sat.unwrap_or(
                            u64::try_from(chain.static_fee_units().ok_or("Missing chain fee")?)
                                .map_err(|_| "Invalid chain fee")?,
                        ),
                        None,
                        None,
                    )
                    .await?,
            ),
            Chain::Ton => PreparedPayload::Ton {
                seqno: TonClient::new(eps).fetch_seqno(sender).await?,
                amount: amount_u64,
                valid_until: u32::try_from(crate::store::now_unix() as u64 + 60)
                    .map_err(|_| "TON expiry overflow")?,
            },
            Chain::Xrp => {
                let client = XrpClient::new(eps);
                PreparedPayload::Xrp {
                    sequence: client.fetch_sequence(sender).await?,
                    fee_drops: XrpClient::new(self.endpoints_for(chain.str_id(), &["fee"]).await)
                        .fetch_fee()
                        .await?,
                    amount_drops: amount_u64,
                }
            }
            Chain::Stellar => {
                let client = StellarClient::new(eps);
                PreparedPayload::Stellar {
                    sequence: client
                        .fetch_sequence(sender)
                        .await?
                        .checked_add(1)
                        .ok_or("Sequence exhausted")?,
                    fee_stroops: StellarClient::new(
                        self.endpoints_for(chain.str_id(), &["fee"]).await,
                    )
                    .fetch_base_fee()
                    .await?,
                    amount_stroops: i64::try_from(amount).map_err(|_| "Amount too large")?,
                }
            }
            Chain::Polkadot | Chain::Bittensor => {
                let (nonce, version, genesis_hash, block_hash) =
                    if chain.mainnet_counterpart() == Chain::Polkadot {
                        let client = PolkadotClient::new(eps);
                        (
                            client.fetch_nonce(sender).await?,
                            client.fetch_runtime_version().await?,
                            client.fetch_genesis_hash().await?,
                            client.fetch_block_hash_latest().await?,
                        )
                    } else {
                        let client = BittensorClient::new(eps);
                        (
                            client.fetch_nonce(sender).await?,
                            client.fetch_runtime_version().await?,
                            client.fetch_genesis_hash().await?,
                            client.fetch_block_hash_latest().await?,
                        )
                    };
                PreparedPayload::Substrate {
                    nonce,
                    spec_version: version.0,
                    transaction_version: version.1,
                    genesis_hash,
                    block_hash,
                    amount,
                }
            }
            Chain::Cardano => {
                let client = CardanoClient::new(eps);
                let fee = request
                    .fee_amount
                    .map(|v| crate::send::payload::fee_units(v, 6))
                    .transpose()?
                    .unwrap_or(
                        u64::try_from(chain.static_fee_units().ok_or("No Cardano fee")?)
                            .map_err(|_| "Invalid fee")?,
                    );
                let inputs: Vec<_> =
                    CardanoClient::new(self.endpoints_for(chain.str_id(), &["utxo"]).await)
                        .fetch_utxos(sender)
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
                        .ok_or("Slot overflow")?,
                }
            }
            Chain::Bitcoin => {
                let client = BitcoinClient::new(
                    HttpClient::shared(),
                    self.endpoints_for(chain.str_id(), &["utxo"]).await,
                );
                let rate = match request.fee_rate_svb {
                    Some(rate) => rate,
                    None => {
                        BitcoinClient::new(
                            HttpClient::shared(),
                            self.endpoints_for(chain.str_id(), &["fee"]).await,
                        )
                        .fetch_fee_rate(6)
                        .await?
                        .sats_per_vbyte
                    }
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
                SolanaClient::new(
                    self.endpoints_for(
                        chain.str_id(),
                        if request.contract_address.is_some() {
                            &["verification", "token-balance"]
                        } else {
                            &["verification"]
                        },
                    )
                    .await,
                )
                .prepare_transfer(
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
                let transfer = match request.contract_address.as_deref() {
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
                PreparedPayload::Tron(prepare_transfer(
                    sender,
                    transfer,
                    TronClient::new(eps).transfer_reference().await?,
                )?)
            }
            Chain::Aptos => {
                let client = AptosClient::new(eps);
                let sequence = client.fetch_account_info(sender).await?.0;
                let network = chain
                    .aptos_chain_id()
                    .ok_or("Missing Aptos network identity")?;
                if client.fetch_ledger_info().await?.0 != u64::from(network) {
                    return Err("Aptos endpoint is on the wrong network".into());
                }
                PreparedPayload::Aptos(crate::send::aptos::prepare_transfer(
                    sender,
                    to,
                    amount_u64,
                    sequence,
                    AptosClient::new(self.endpoints_for(chain.str_id(), &["fee"]).await)
                        .fetch_gas_price()
                        .await?,
                    10_000,
                    crate::store::now_unix() as u64 + 600,
                    network,
                )?)
            }
            Chain::Sui => {
                let gas = request
                    .gas_budget
                    .map(|v| crate::send::payload::fee_units(v, 9))
                    .transpose()?
                    .unwrap_or(10_000_000);
                PreparedPayload::Sui(
                    SuiClient::new(
                        self.endpoints_for(chain.str_id(), &["verification", "balance", "fee"])
                            .await,
                    )
                    .prepare_native_transfer(sender, to, amount_u64, gas)
                    .await?,
                )
            }
            _ => return Err("Transparent preparation unavailable for this protocol".into()),
        })
    }

    pub(super) async fn sign_staged_protocol(
        &self,
        chain: Chain,
        stored: &StoredSend,
        signer: &super::send_identity::ResolvedSendIdentity,
    ) -> Result<(PreparedSubmission, Vec<String>), SpectraBridgeError> {
        let eps = self.endpoints_for(chain.str_id(), &["verification"]).await;
        let seed = || crate::send::keys::Ed25519Seed::from_hex(&signer.private_key_hex);
        let mut resources = Vec::new();
        let (payload, field, hash) = match &stored.prepared {
            PreparedPayload::Near {
                public_key,
                nonce,
                block_hash,
                amount,
                token_contract,
            } => {
                let client = NearClient::new(eps);
                if seed()?.public_key() != *public_key
                    || client
                        .fetch_access_key_nonce(
                            &stored.view.sender,
                            &bs58::encode(public_key).into_string(),
                        )
                        .await?
                        .checked_add(1)
                        != Some(*nonce)
                    || crate::store::now_unix() - stored.view.created_at > 120.0
                {
                    return Err(
                        "NEAR signer, nonce or block reference is stale; build and review again"
                            .into(),
                    );
                }
                let bytes = zeroize::Zeroizing::new(
                    hex::decode(signer.private_key_hex.as_str()).map_err(|e| e.to_string())?,
                );
                let key: &[u8; 32] = bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| "Invalid NEAR seed")?;
                let raw = if let Some(contract) = token_contract {
                    let args = serde_json::to_vec(
                        &json!({"receiver_id":stored.view.recipient,"amount":amount.to_string()}),
                    )?;
                    crate::send::near::build_near_function_call_tx(
                        &stored.view.sender,
                        public_key,
                        *nonce,
                        contract,
                        "ft_transfer",
                        &args,
                        30_000_000_000_000,
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
                (
                    json!({"signed_tx_b64":STANDARD.encode(raw)}).to_string(),
                    "txid",
                    None,
                )
            }
            PreparedPayload::Decred(p) => {
                let request = &stored.request;
                let refreshed =
                    DecredClient::new(self.endpoints_for(chain.str_id(), &["utxo"]).await)
                        .prepare_transfer(
                            &stored.view.sender,
                            &stored.view.recipient,
                            u64::try_from(crate::send::amount_input::parse_raw_amount(
                                &request.amount_str,
                                8,
                            )?)
                            .map_err(|_| "Amount too large")?,
                            request.fee_sat.unwrap_or(
                                u64::try_from(chain.static_fee_units().ok_or("Missing chain fee")?)
                                    .map_err(|_| "Invalid chain fee")?,
                            ),
                            None,
                        )
                        .await?;
                if serde_json::to_vec(p)? != serde_json::to_vec(&refreshed)? {
                    return Err("Decred inputs changed; build and review again".into());
                }
                resources = p.resources();
                let key = decode_private_key(&signer.private_key_hex)?;
                (p.sign(&key)?, "txid", None)
            }
            PreparedPayload::Kaspa(p) => {
                let request = &stored.request;
                let refreshed =
                    KaspaClient::new(self.endpoints_for(chain.str_id(), &["utxo"]).await)
                        .prepare_transfer(
                            &stored.view.sender,
                            &stored.view.recipient,
                            u64::try_from(crate::send::amount_input::parse_raw_amount(
                                &request.amount_str,
                                8,
                            )?)
                            .map_err(|_| "Amount too large")?,
                            request.fee_sat.unwrap_or(
                                u64::try_from(chain.static_fee_units().ok_or("Missing chain fee")?)
                                    .map_err(|_| "Invalid chain fee")?,
                            ),
                            None,
                            None,
                        )
                        .await?;
                if serde_json::to_vec(p)? != serde_json::to_vec(&refreshed)? {
                    return Err("Kaspa inputs changed; build and review again".into());
                }
                resources = p.resources();
                let key = decode_private_key(&signer.private_key_hex)?;
                (p.sign(&key)?.to_string(), "txid", None)
            }
            PreparedPayload::Ton {
                seqno,
                amount,
                valid_until,
            } => {
                if *valid_until <= crate::store::now_unix() as u32
                    || TonClient::new(eps).fetch_seqno(&stored.view.sender).await? != *seqno
                {
                    return Err("TON sequence or expiration changed; build and review again".into());
                }
                let key = decode_secret_array::<32>(&signer.private_key_hex)?;
                let public = seed()?.public_key();
                let raw = crate::send::ton::build_transfer_for_address(
                    crate::derivation::ton::parse_ton_address(&stored.view.recipient)?
                        .for_network(chain.is_testnet())?,
                    *amount,
                    *seqno,
                    None,
                    &key,
                    &public,
                    698_983_191,
                    *valid_until,
                    3,
                )?;
                resources.push(format!(
                    "{}:{}:sequence:{seqno}",
                    chain.str_id(),
                    stored.view.sender
                ));
                (
                    json!({"boc_b64":STANDARD.encode(raw)}).to_string(),
                    "message_hash",
                    None,
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
                    return Err("ICP transaction expired; build and review again".into());
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
            PreparedPayload::Zcash(p) => {
                let client = BlockbookClient::new(
                    self.endpoints_for(chain.str_id(), &["verification", "utxo"])
                        .await,
                    chain,
                );
                client.validate_zcash_prepared(p).await?;
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
            PreparedPayload::Xrp {
                sequence,
                fee_drops,
                amount_drops,
            } => {
                if XrpClient::new(eps)
                    .fetch_sequence(&stored.view.sender)
                    .await?
                    != *sequence
                {
                    return Err("XRP sequence changed; build and review again".into());
                }
                let key = decode_private_key(&signer.private_key_hex)?;
                let public = signer
                    .public_key_hex
                    .as_deref()
                    .ok_or("Missing XRP public key")?;
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
                if StellarClient::new(eps)
                    .fetch_sequence(&stored.view.sender)
                    .await?
                    .checked_add(1)
                    != Some(*sequence)
                {
                    return Err("Stellar sequence changed; build and review again".into());
                }
                let bytes = zeroize::Zeroizing::new(
                    hex::decode(signer.private_key_hex.as_str()).map_err(|e| e.to_string())?,
                );
                if !matches!(bytes.len(), 32 | 64) {
                    return Err("Invalid Stellar signing key".into());
                }
                let seed: &[u8; 32] = bytes[..32].try_into().map_err(|_| "Invalid Stellar seed")?;
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
            PreparedPayload::Substrate {
                nonce,
                spec_version,
                transaction_version,
                genesis_hash,
                block_hash,
                amount,
            } => {
                let (current_nonce, version, genesis) =
                    if chain.mainnet_counterpart() == Chain::Polkadot {
                        let client = PolkadotClient::new(eps);
                        (
                            client.fetch_nonce(&stored.view.sender).await?,
                            client.fetch_runtime_version().await?,
                            client.fetch_genesis_hash().await?,
                        )
                    } else {
                        let client = BittensorClient::new(eps);
                        (
                            client.fetch_nonce(&stored.view.sender).await?,
                            client.fetch_runtime_version().await?,
                            client.fetch_genesis_hash().await?,
                        )
                    };
                if current_nonce != *nonce
                    || version != (*spec_version, *transaction_version)
                    || genesis != *genesis_hash
                {
                    return Err(
                        "Substrate runtime, network or nonce changed; build and review again"
                            .into(),
                    );
                }
                let bytes = zeroize::Zeroizing::new(
                    hex::decode(signer.private_key_hex.as_str()).map_err(|e| e.to_string())?,
                );
                let key: &[u8; 32] = bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| "Invalid Substrate seed")?;
                let public = hex::decode(
                    signer
                        .public_key_hex
                        .as_deref()
                        .ok_or("Missing Substrate public key")?,
                )
                .map_err(|e| e.to_string())?;
                let public: &[u8; 32] = public
                    .as_slice()
                    .try_into()
                    .map_err(|_| "Invalid Substrate public key")?;
                let raw = if chain.mainnet_counterpart() == Chain::Polkadot {
                    crate::send::polkadot::build_signed_transfer(
                        &stored.view.recipient,
                        *amount,
                        *nonce,
                        *spec_version,
                        *transaction_version,
                        genesis_hash,
                        block_hash,
                        key,
                        public,
                        None,
                        None,
                    )?
                } else {
                    crate::send::bittensor::build_signed_transfer(
                        &stored.view.recipient,
                        *amount,
                        *nonce,
                        *spec_version,
                        *transaction_version,
                        genesis_hash,
                        block_hash,
                        key,
                        public,
                    )?
                };
                resources.push(format!(
                    "{}:{}:nonce:{nonce}",
                    chain.str_id(),
                    stored.view.sender
                ));
                (
                    json!({"extrinsic_hex":format!("0x{}",hex::encode(raw))}).to_string(),
                    "txid",
                    None,
                )
            }
            PreparedPayload::Cardano {
                inputs,
                amount,
                fee,
                ttl,
            } => {
                let client = CardanoClient::new(eps);
                if client.fetch_latest_slot().await? >= *ttl {
                    return Err("Cardano transaction expired; build and review again".into());
                }
                let current =
                    CardanoClient::new(self.endpoints_for(chain.str_id(), &["utxo"]).await)
                        .fetch_utxos(&stored.view.sender)
                        .await?;
                for (hash, index, value) in inputs {
                    if !current
                        .iter()
                        .any(|u| &u.tx_hash == hash && u.tx_index == *index && u.lovelace == *value)
                    {
                        return Err("Cardano input changed; build and review again".into());
                    }
                    resources.push(format!("{}:utxo:{hash}:{index}", chain.str_id()));
                }
                let bytes = zeroize::Zeroizing::new(
                    hex::decode(signer.private_key_hex.as_str()).map_err(|e| e.to_string())?,
                );
                let key: &[u8; 64] = bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| "Invalid Cardano key")?;
                let public = hex::decode(
                    signer
                        .public_key_hex
                        .as_deref()
                        .ok_or("Missing Cardano public key")?,
                )
                .map_err(|e| e.to_string())?;
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
                let current = BitcoinClient::new(
                    HttpClient::shared(),
                    self.endpoints_for(chain.str_id(), &["utxo"]).await,
                )
                .fetch_utxos(&stored.view.sender)
                .await?;
                for input in &p.inputs {
                    if !current.iter().any(|u| {
                        u.txid == input.txid && u.vout == input.vout && u.value == input.value
                    }) {
                        return Err(
                            "Bitcoin input changed or was spent; build and review again".into()
                        );
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
                let client = SolanaClient::new(eps);
                let valid = client
                    .call(
                        "isBlockhashValid",
                        json!([p.blockhash, {"commitment":"confirmed"}]),
                    )
                    .await?;
                if valid["value"].as_bool() != Some(true) {
                    return Err("Solana blockhash expired; build and review again".into());
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
                    .ok_or("Invalid Tron expiration")?;
                if expiry <= (crate::store::now_unix() * 1000.0) as u64 {
                    return Err("Tron transaction expired; build and review again".into());
                }
                let key = decode_private_key(&signer.private_key_hex)?;
                (
                    p.clone().sign(&key)?,
                    "txid",
                    p.body["txID"].as_str().map(str::to_string),
                )
            }
            PreparedPayload::Aptos(p) => {
                let seq: u64 = p.body["sequence_number"]
                    .as_str()
                    .ok_or("Missing sequence")?
                    .parse()
                    .map_err(|_| "Invalid sequence")?;
                let expiry: u64 = p.body["expiration_timestamp_secs"]
                    .as_str()
                    .ok_or("Missing expiration")?
                    .parse()
                    .map_err(|_| "Invalid expiration")?;
                if AptosClient::new(eps)
                    .fetch_account_info(&stored.view.sender)
                    .await?
                    .0
                    != seq
                    || expiry <= crate::store::now_unix() as u64
                {
                    return Err(
                        "Aptos sequence or expiration is stale; build and review again".into(),
                    );
                }
                resources.push(format!(
                    "{}:{}:sequence:{seq}",
                    chain.str_id(),
                    stored.view.sender
                ));
                (
                    json!({"signed_body_json":p.clone().sign(&seed()?)?}).to_string(),
                    "txid",
                    None,
                )
            }
            PreparedPayload::Sui(p) => {
                // Re-read object versions before signing; compare exact bytes, never substitute them.
                let mut request = stored.request.clone();
                let refreshed = self
                    .prepare_staged_protocol(chain, &mut request, &stored.view.sender)
                    .await?;
                if serde_json::to_vec(&refreshed)? != serde_json::to_vec(&stored.prepared)? {
                    return Err("Sui objects or gas changed; build and review again".into());
                }
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
                    None,
                )
            }
            PreparedPayload::Evm(_) => return Err("EVM signing uses its typed signer".into()),
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
        let allowed = self.endpoints_for(chain.str_id(), &["broadcast"]).await;
        if !allowed
            .iter()
            .any(|url| url.trim_end_matches('/') == endpoint.trim_end_matches('/'))
        {
            return Err("Endpoint does not support broadcasts on the selected network".into());
        }
        self.validate_endpoint_network(chain, endpoint).await
    }

    pub(super) async fn validate_endpoint_network(
        &self,
        chain: Chain,
        endpoint: &str,
    ) -> Result<(), SpectraBridgeError> {
        let catalog = crate::app_core::endpoint_catalog()?;
        let known = catalog.endpoint_records.iter().any(|r| {
            r.chain_id == chain.str_id()
                && r.api == chain.endpoint_api(EndpointSlot::Primary)
                && r.endpoint.trim_end_matches('/') == endpoint.trim_end_matches('/')
        });
        let eps = Arc::new(vec![endpoint.to_string()]);
        if chain.is_evm() {
            let actual = EvmClient::new(eps, chain.evm_chain_id()?)
                .call("eth_chainId", json!([]))
                .await?;
            if crate::fetch::evm::parse_hex_u64(
                actual.as_str().ok_or("Missing endpoint network identity")?,
            )? != chain.evm_chain_id()?
            {
                return Err("Endpoint is on the wrong network".into());
            }
        } else if chain.mainnet_counterpart() == Chain::Aptos {
            if AptosClient::new(eps).fetch_ledger_info().await?.0
                != u64::from(
                    chain
                        .aptos_chain_id()
                        .ok_or("Missing Aptos network identity")?,
                )
            {
                return Err("Endpoint is on the wrong network".into());
            }
        } else if chain.mainnet_counterpart() == Chain::Zcash {
            BlockbookClient::new(eps, chain).zcash_context().await?;
        } else if chain == Chain::Icp {
            IcpClient::new(eps).verify_network().await?;
        } else if chain.mainnet_counterpart() == Chain::Monero {
            crate::send::monero_local::daemon(endpoint, chain).await?;
        } else if !known {
            return Err("Endpoint network and broadcast capability cannot be verified".into());
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
        let endpoints = self.endpoints_for(chain.str_id(), &["verification"]).await;
        let expired = match &stored.prepared {
            PreparedPayload::Zcash(p) => {
                let (height, branch) = BlockbookClient::new(endpoints, chain)
                    .zcash_context()
                    .await?;
                height >= p.expiry_height || branch != p.upgrade.consensus_branch_id
            }
            PreparedPayload::Icp(p) => (now * 1_000_000_000.0) as u64 >= p.ingress_expiry_ns,
            PreparedPayload::Ton { valid_until, .. } => now >= f64::from(*valid_until),
            PreparedPayload::Near { .. } => now - stored.view.created_at >= 120.0,
            PreparedPayload::Tron(p) => p.body["raw_data"]["expiration"]
                .as_u64()
                .is_none_or(|expiry| now * 1000.0 >= expiry as f64),
            PreparedPayload::Aptos(p) => p.body["expiration_timestamp_secs"]
                .as_str()
                .and_then(|s| s.parse::<u64>().ok())
                .is_none_or(|expiry| now >= expiry as f64),
            PreparedPayload::Cardano { ttl, .. } => {
                CardanoClient::new(endpoints).fetch_latest_slot().await? >= *ttl
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
            return Err("Signed transaction expired; inspect its on-chain status before building a new transaction".into());
        }
        Ok(())
    }
}
