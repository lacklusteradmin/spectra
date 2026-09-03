//! Everything between "the user pressed send" and "the network has it":
//! fee estimation, per-chain signing, broadcast, and the previews that quote a
//! send before it is authorised.
//!
//! Signing material arrives per call and is scrubbed after use — nothing in
//! this module outlives the call that supplied it. New arms take a typed
//! record from [`super::send_params`], never a fresh ad-hoc JSON shape.

use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Typed wrapper around `broadcast_raw`: runs the broadcast then extracts
    /// the named field (typically `"txid"` or `"digest"`) from the result JSON.
    /// Returns the field value as a string, or an empty string when missing.
    pub async fn broadcast_raw_extract(
        &self,
        chain_id: String,
        payload: String,
        result_field: String,
    ) -> Result<String, SpectraBridgeError> {
        let json = self.broadcast_raw(&chain_id, payload).await?;
        Ok(crate::send::preview_decode::extract_json_string_field(
            json,
            result_field,
        ))
    }
    /// Typed EVM send preview: fetches the raw preview JSON then decodes it
    /// into `EvmSendPreview` with the caller-supplied nonce / fee
    /// overrides applied. Returns `None` when the decoder rejects the payload.
    pub async fn fetch_evm_send_preview_typed(
        &self,
        chain_id: String,
        from: String,
        to: String,
        value_wei: String,
        data_hex: String,
        explicit_nonce: Option<i64>,
        custom_fees: Option<crate::ethereum_send::EvmCustomFeeConfiguration>,
    ) -> Result<Option<crate::wallet_core::EvmSendPreview>, SpectraBridgeError> {
        let raw = self
            .fetch_evm_send_preview(&chain_id, from, to, value_wei, data_hex)
            .await?;
        Ok(crate::send::preview_decode::build_evm_send_preview_record(
            crate::ethereum_send::EvmPreviewDecodeInput {
                raw_json: raw,
                explicit_nonce,
                custom_fees,
            },
        ))
    }
    /// Typed Tron send preview wrapper around `fetch_tron_send_preview` +
    /// `build_tron_send_preview_record`.
    pub async fn fetch_tron_send_preview_typed(
        &self,
        address: String,
        symbol: String,
        contract_address: String,
    ) -> Result<Option<crate::wallet_core::TronSendPreview>, SpectraBridgeError> {
        let raw = self
            .fetch_tron_send_preview(address, symbol, contract_address)
            .await?;
        Ok(crate::send::preview_decode::build_tron_send_preview_record(
            raw,
        ))
    }

    /// Typed UTXO fee preview wrapper (BTC / LTC / BCH / BSV single-address
    /// flow). Fuses `fetch_utxo_fee_preview` + `build_utxo_send_preview_record`.
    pub async fn fetch_utxo_fee_preview_typed(
        &self,
        chain_id: String,
        address: String,
        fee_rate_svb: u64,
    ) -> Result<Option<crate::wallet_core::BitcoinSendPreview>, SpectraBridgeError> {
        let raw = self
            .fetch_utxo_fee_preview(&chain_id, address, fee_rate_svb)
            .await?;
        Ok(crate::send::preview_decode::build_utxo_send_preview_record(
            raw,
        ))
    }

    /// Typed Dogecoin send preview: runs the UTXO fee-preview fetch on the
    /// Dogecoin chain then decodes with the requested amount + fee priority.
    pub async fn fetch_dogecoin_send_preview_typed(
        &self,
        address: String,
        requested_amount: f64,
        fee_priority: String,
    ) -> Result<Option<crate::wallet_core::DogecoinSendPreview>, SpectraBridgeError> {
        let raw = self
            .fetch_utxo_fee_preview(Chain::Dogecoin.str_id(), address, 0)
            .await?;
        Ok(
            crate::send::preview_decode::build_dogecoin_send_preview_record(
                raw,
                requested_amount,
                fee_priority,
            ),
        )
    }

    /// Typed Bitcoin HD send preview: concurrently fetches the xpub balance
    /// and the Bitcoin fee estimate then decodes into `BitcoinSendPreview`.
    pub async fn fetch_bitcoin_hd_send_preview_typed(
        &self,
        xpub: String,
        receive_count: u32,
        change_count: u32,
    ) -> Result<Option<crate::wallet_core::BitcoinSendPreview>, SpectraBridgeError> {
        let (balance, rate) = tokio::try_join!(
            self.bitcoin_xpub_balance(xpub, receive_count, change_count),
            self.bitcoin_fee_rate(),
        )?;
        Ok(
            crate::send::preview_decode::build_bitcoin_hd_send_preview_record(
                balance.confirmed_sats,
                rate.sats_per_vbyte,
            ),
        )
    }

    /// Typed simple-chain send preview: fuses `fetch_simple_chain_send_preview`
    /// + `build_simple_chain_preview` so Swift never sees the intermediate JSON.
    /// The `chain: SimpleChain` argument is gone: it was derivable from
    /// `chain_id`, and the only way for a caller to get one was an eleven-entry
    /// table in Swift keyed by display name — a second spelling of the registry,
    /// handed back to the registry's owner.
    pub async fn fetch_simple_chain_send_preview_typed(
        &self,
        chain_id: String,
        address: String,
    ) -> Result<crate::send::preview_decode::SimpleChainPreview, SpectraBridgeError> {
        let chain = crate::registry::Chain::from_str_id(&chain_id)
            .and_then(|chain| chain.simple_preview_chain())
            .ok_or_else(|| SpectraBridgeError::InvalidInput {
                message: format!("{chain_id} has no shared-path send preview"),
            })?;
        let raw = self
            .fetch_simple_chain_send_preview(&chain_id, address)
            .await?;
        Ok(crate::send::preview_decode::build_simple_chain_preview(
            raw, chain,
        ))
    }
}

impl WalletService {
    /// Bitcoin's fee rate, in sat/vB.
    ///
    /// Split out of a `fetch_fee_estimate` that returned three different JSON
    /// shapes by chain — this one, EVM's `EvmFeeEstimate`, and a flat native
    /// amount for everyone else — as a `String` its callers parsed back. The
    /// EVM arm had no reachable caller at all: EVM previews build their own
    /// `EvmClient` and call `fetch_fee_estimate()` on it directly, so nothing
    /// ever asked this function for an EVM chain. What is left is two shapes
    /// with one caller each, so each caller gets its own typed function and
    /// neither goes through JSON.
    pub(crate) async fn bitcoin_fee_rate(
        &self,
    ) -> Result<crate::fetch::chains::bitcoin::FeeRate, SpectraBridgeError> {
        let endpoints = self.endpoints_for(Chain::Bitcoin.str_id()).await;
        let client = BitcoinClient::new(HttpClient::shared(), endpoints);
        Ok(client.fetch_fee_rate(6).await?)
    }

    /// A chain's fee quoted in its own native unit, live where the chain has
    /// an RPC that answers and static where the catalog carries the number.
    ///
    /// Only the eleven chains `simple_preview_chain` covers reach this, which
    /// is why Bitcoin and EVM have no arm: they have their own preview paths.
    pub(crate) async fn native_fee_estimate(
        &self,
        chain: Chain,
    ) -> Result<NativeFeeEstimate, SpectraBridgeError> {
        let endpoints = self.endpoints_for(chain.str_id()).await;
        let native = |raw: u128, source: &'static str| NativeFeeEstimate {
            raw: raw.to_string(),
            display: format_decimals(raw, chain.native_decimals()),
            source,
        };
        match chain {
            // Chains with live RPC fee fetches.
            Chain::Xrp => {
                let drops = XrpClient::new(endpoints).fetch_fee().await?;
                Ok(native(drops as u128, "rpc"))
            }
            Chain::Stellar => {
                let stroops = StellarClient::new(endpoints).fetch_base_fee().await?;
                Ok(native(stroops as u128, "rpc"))
            }
            Chain::Aptos => {
                let price = AptosClient::new(endpoints).fetch_gas_price().await?;
                Ok(native(price as u128, "rpc"))
            }
            // NEAR's static fee overflows u128 — carry it as the string it is.
            Chain::Near => Ok(NativeFeeEstimate {
                raw: "1000000000000000000000".to_string(),
                display: "0.001".to_string(),
                source: "static",
            }),
            // Every remaining supported chain returns a flat static fee from
            // `Chain::static_fee_units`. One arm replaces 18 near-identical ones.
            other => match other.static_fee_units() {
                Some(units) => Ok(native(units, "static")),
                None => Err(SpectraBridgeError::from(format!(
                    "fee estimation not supported for {}",
                    other.chain_display_name()
                ))),
            },
        }
    }


    /// Sign and broadcast, from an already-typed [`ExecuteSendParams`] —
    /// what `sign_and_send` and `sign_and_send_token` did as two separate
    /// JSON-in functions, merged into the one match their data now carries
    /// its own chain identity for. See `build_send_params` for what used to
    /// sit on the other side of the JSON these arms used to parse.
    pub(crate) async fn sign_and_broadcast_send(
        &self,
        chain: Chain,
        params: crate::service::send_params::ExecuteSendParams,
    ) -> Result<String, SpectraBridgeError> {
        use crate::service::send_params::{ExecuteSendParams, SendParams, SendTokenParams};
        let endpoints = self.endpoints_for(chain.str_id()).await;

        match params {
            ExecuteSendParams::Native(native) => match native {
                SendParams::Bitcoin(p) => {
                    let client = BitcoinClient::new(HttpClient::shared(), endpoints);
                    let send_params = BitcoinSendParams {
                        from_address: p.from,
                        private_key_hex: p.private_key_hex,
                        to_address: p.to,
                        amount_sats: p.amount_sat,
                        fee_rate: crate::fetch::chains::bitcoin::FeeRate {
                            sats_per_vbyte: p.fee_rate_svb.unwrap_or(10.0),
                        },
                        available_utxos: vec![],
                        network_chain_id: crate::registry::Chain::Bitcoin.str_id().to_string(),
                        enable_rbf: true,
                        dust_threshold: p.dust_threshold_sats,
                        pinned_utxos: None,
                        extra_outputs: vec![],
                        coin_selection: crate::send::chains::bitcoin::CoinSelectionStrategy::default(),
                        sign_only: p.sign_only,
                    };
                    let r = bitcoin_sign_and_broadcast(&client, send_params).await?;
                    Ok(serde_json::to_string(&r)?)
                }
                SendParams::Evm(p, overrides) => {
                    let priv_bytes = hex::decode(&p.private_key_hex).map_err(|e| {
                        SpectraBridgeError::from(format!("private_key_hex hex decode: {e}"))
                    })?;
                    let client = EvmClient::new(endpoints, chain.evm_chain_id());
                    let r = client
                        .sign_and_broadcast_with_overrides(
                            &p.from,
                            &p.to,
                            p.value_wei,
                            &priv_bytes,
                            overrides,
                        )
                        .await?;
                    Ok(serde_json::to_string(&r)?)
                }
                SendParams::Solana(p) => {
                    let from_arr: [u8; 32] =
                        decode_hex_array(&p.from_pubkey_hex, "from_pubkey_hex")?;
                    let priv_arr: [u8; 64] =
                        decode_hex_array(&p.private_key_hex, "private_key_hex")?;
                    let client = SolanaClient::new(endpoints);
                    let r = client
                        .sign_and_broadcast(&from_arr, &p.to, p.lamports, &priv_arr)
                        .await?;
                    Ok(serde_json::to_string(&r)?)
                }
                SendParams::Xrp(p) => {
                    let priv_bytes = hex::decode(&p.private_key_hex).map_err(|e| {
                        SpectraBridgeError::from(format!("private_key_hex hex decode: {e}"))
                    })?;
                    // public_key_hex is optional: derive compressed secp256k1 pubkey when absent.
                    let derived_pub: String;
                    let pub_hex: &str = match p.public_key_hex.as_deref().filter(|s| !s.is_empty()) {
                        Some(s) => s,
                        None => {
                            use secp256k1::{PublicKey as SecpPubKey, Secp256k1, SecretKey};
                            let secp = Secp256k1::new();
                            let secret = SecretKey::from_slice(&priv_bytes)
                                .map_err(|e| format!("bad privkey: {e}"))?;
                            derived_pub = hex::encode(
                                SecpPubKey::from_secret_key(&secp, &secret).serialize(),
                            );
                            &derived_pub
                        }
                    };
                    let client = XrpClient::new(endpoints);
                    let r = client
                        .sign_and_submit(&p.from, &p.to, p.drops, &priv_bytes, pub_hex)
                        .await?;
                    Ok(serde_json::to_string(&r)?)
                }
                SendParams::Tron(p) => {
                    let priv_bytes = hex::decode(&p.private_key_hex).map_err(|e| {
                        SpectraBridgeError::from(format!("private_key_hex hex decode: {e}"))
                    })?;
                    let client = TronClient::new(endpoints);
                    let r = client
                        .sign_and_broadcast(&p.from, &p.to, p.amount_sun, &priv_bytes)
                        .await?;
                    Ok(serde_json::to_string(&r)?)
                }
                SendParams::Sui(p) => {
                    let priv_arr: [u8; 64] =
                        decode_hex_array(&p.private_key_hex, "private_key_hex")?;
                    let pub_arr: [u8; 32] =
                        decode_hex_array(&p.public_key_hex, "public_key_hex")?;
                    let client = SuiClient::new(endpoints);
                    let r = client
                        .sign_and_send(
                            &p.from,
                            &p.to,
                            p.mist,
                            p.gas_budget.unwrap_or(10_000_000),
                            &priv_arr,
                            &pub_arr,
                        )
                        .await?;
                    Ok(serde_json::to_string(&r)?)
                }
                SendParams::Aptos(p) => {
                    let priv_arr: [u8; 64] =
                        decode_hex_array(&p.private_key_hex, "private_key_hex")?;
                    let pub_arr: [u8; 32] =
                        decode_hex_array(&p.public_key_hex, "public_key_hex")?;
                    let client = AptosClient::new(endpoints);
                    let r = client
                        .sign_and_submit(&p.from, &p.to, p.octas, &priv_arr, &pub_arr)
                        .await?;
                    Ok(serde_json::to_string(&r)?)
                }
                SendParams::Near(p) => {
                    let priv_arr: [u8; 64] =
                        decode_hex_array(&p.private_key_hex, "private_key_hex")?;
                    let pub_arr: [u8; 32] =
                        decode_hex_array(&p.public_key_hex, "public_key_hex")?;
                    let client = NearClient::new(endpoints);
                    let r = client
                        .sign_and_broadcast(&p.from, &p.to, p.yocto_near, &priv_arr, &pub_arr)
                        .await?;
                    Ok(serde_json::to_string(&r)?)
                }
                SendParams::Utxo(p) => {
                    let priv_bytes = hex::decode(&p.private_key_hex).map_err(|e| {
                        SpectraBridgeError::from(format!("private_key_hex hex decode: {e}"))
                    })?;
                    // The one client this shape covers depends on which chain
                    // built it — `build_send_params` never wraps one chain's
                    // params in a `SendParams::Utxo` meant for another.
                    match chain {
                        Chain::Dogecoin => {
                            let client = DogecoinClient::new(endpoints);
                            let r = client
                                .sign_and_broadcast(
                                    &p.from,
                                    &p.to,
                                    p.amount_sat,
                                    p.fee_sat.unwrap_or(200_000),
                                    &priv_bytes,
                                    p.dust_threshold_sats,
                                )
                                .await?;
                            Ok(serde_json::to_string(&r)?)
                        }
                        Chain::Litecoin => {
                            let client = LitecoinClient::new(endpoints);
                            let r = client
                                .sign_and_broadcast(
                                    &p.from,
                                    &p.to,
                                    p.amount_sat,
                                    p.fee_sat.unwrap_or(10_000),
                                    &priv_bytes,
                                    p.dust_threshold_sats,
                                )
                                .await?;
                            Ok(serde_json::to_string(&r)?)
                        }
                        Chain::BitcoinCash => {
                            let client = BitcoinCashClient::new(endpoints);
                            let r = client
                                .sign_and_broadcast(
                                    &p.from,
                                    &p.to,
                                    p.amount_sat,
                                    p.fee_sat.unwrap_or(1_000),
                                    &priv_bytes,
                                    p.dust_threshold_sats,
                                )
                                .await?;
                            Ok(serde_json::to_string(&r)?)
                        }
                        Chain::BitcoinSV => {
                            let client = BitcoinSvClient::new(endpoints);
                            let r = client
                                .sign_and_broadcast(
                                    &p.from,
                                    &p.to,
                                    p.amount_sat,
                                    p.fee_sat.unwrap_or(1_000),
                                    &priv_bytes,
                                    p.dust_threshold_sats,
                                )
                                .await?;
                            Ok(serde_json::to_string(&r)?)
                        }
                        Chain::BitcoinGold => {
                            let client = BitcoinGoldClient::new(endpoints);
                            let r = client
                                .sign_and_broadcast(
                                    &p.from,
                                    &p.to,
                                    p.amount_sat,
                                    p.fee_sat.unwrap_or(1_000),
                                    &priv_bytes,
                                    p.dust_threshold_sats,
                                )
                                .await?;
                            Ok(serde_json::to_string(&r)?)
                        }
                        Chain::Dash => {
                            let client = DashClient::new(endpoints);
                            let r = client
                                .sign_and_broadcast(
                                    &p.from,
                                    &p.to,
                                    p.amount_sat,
                                    p.fee_sat.unwrap_or(2_000),
                                    &priv_bytes,
                                    p.dust_threshold_sats,
                                )
                                .await?;
                            Ok(serde_json::to_string(&r)?)
                        }
                        c => Err(SpectraBridgeError::from(format!(
                            "sign_and_broadcast_send: {c:?} does not use the shared UTXO shape"
                        ))),
                    }
                }
                SendParams::Zcash(p) => {
                    let priv_bytes = hex::decode(&p.private_key_hex).map_err(|e| {
                        SpectraBridgeError::from(format!("private_key_hex hex decode: {e}"))
                    })?;
                    let client = ZcashClient::new(endpoints);
                    let r = client
                        .sign_and_broadcast(
                            &p.from,
                            &p.to,
                            p.amount_sat,
                            p.fee_sat.unwrap_or(1_000),
                            &priv_bytes,
                            crate::send::chains::zcash::ZcashNetworkUpgrade::NU5,
                            p.dust_threshold_zats.unwrap_or(546),
                        )
                        .await?;
                    Ok(serde_json::to_string(&r)?)
                }
                SendParams::Decred(p) => {
                    let priv_bytes = hex::decode(&p.private_key_hex).map_err(|e| {
                        SpectraBridgeError::from(format!("private_key_hex hex decode: {e}"))
                    })?;
                    let client = DecredClient::new(endpoints);
                    let r = client
                        .sign_and_broadcast(
                            &p.from,
                            &p.to,
                            p.amount_sat,
                            p.fee_sat.unwrap_or(2_000),
                            &priv_bytes,
                            p.dust_threshold_atoms,
                        )
                        .await?;
                    Ok(serde_json::to_string(&r)?)
                }
                SendParams::Kaspa(p) => {
                    let priv_bytes = hex::decode(&p.private_key_hex).map_err(|e| {
                        SpectraBridgeError::from(format!("private_key_hex hex decode: {e}"))
                    })?;
                    let client = KaspaClient::new(endpoints);
                    let r = client
                        .sign_and_broadcast(
                            &p.from,
                            &p.to,
                            p.amount_sat,
                            p.fee_sat.unwrap_or(1_000),
                            &priv_bytes,
                            p.min_fee_sompi,
                            p.dust_threshold_sompi,
                        )
                        .await?;
                    Ok(serde_json::to_string(&r)?)
                }
                SendParams::Stellar(p) => {
                    // Accept 32-byte seed (raw import) or 64-byte expanded key (derived).
                    let priv_raw = hex::decode(&p.private_key_hex).map_err(|e| {
                        SpectraBridgeError::from(format!("private_key_hex hex decode: {e}"))
                    })?;
                    let priv_arr: [u8; 64] = if priv_raw.len() == 32 {
                        let mut expanded = [0u8; 64];
                        expanded[..32].copy_from_slice(&priv_raw);
                        expanded
                    } else {
                        priv_raw
                            .try_into()
                            .map_err(|_| "privkey must be 32 or 64 bytes")?
                    };
                    // public_key_hex is optional: derive ed25519 verifying key when absent.
                    let pub_arr: [u8; 32] =
                        match p.public_key_hex.as_deref().filter(|s| !s.is_empty()) {
                            Some(s) => hex::decode(s)
                                .map_err(|e| format!("pubkey hex: {e}"))?
                                .try_into()
                                .map_err(|_| "pubkey wrong length")?,
                            None => {
                                use ed25519_dalek::SigningKey;
                                let seed: [u8; 32] = priv_arr[..32]
                                    .try_into()
                                    .map_err(|_| "privkey seed too short")?;
                                SigningKey::from_bytes(&seed).verifying_key().to_bytes()
                            }
                        };
                    let network_passphrase = p
                        .network_passphrase
                        .as_deref()
                        .map(|s| s.as_bytes().to_vec());
                    let client = StellarClient::new(endpoints);
                    let r = client
                        .sign_and_submit(
                            &p.from,
                            &p.to,
                            p.stroops,
                            &priv_arr,
                            &pub_arr,
                            network_passphrase,
                        )
                        .await?;
                    Ok(serde_json::to_string(&r)?)
                }
                SendParams::Cardano(p) => {
                    let priv_arr: [u8; 64] =
                        decode_hex_array(&p.private_key_hex, "private_key_hex")?;
                    let pub_arr: [u8; 32] =
                        decode_hex_array(&p.public_key_hex, "public_key_hex")?;
                    let api_key = self.api_key_for(chain.str_id()).await.unwrap_or_default();
                    let client = CardanoClient::new(endpoints, api_key);
                    let r = client
                        .sign_and_broadcast(
                            &p.from,
                            &p.to,
                            p.amount_lovelace,
                            p.fee_lovelace.unwrap_or(170_000),
                            &priv_arr,
                            &pub_arr,
                            p.ttl_slots,
                            p.min_change_lovelace,
                        )
                        .await?;
                    Ok(serde_json::to_string(&r)?)
                }
                SendParams::Polkadot(p) => {
                    let priv_arr: [u8; 32] =
                        decode_hex_array(&p.private_key_hex, "private_key_hex")?;
                    let pub_arr: [u8; 32] =
                        decode_hex_array(&p.public_key_hex, "public_key_hex")?;
                    let subscan = self
                        .endpoints_for(&chain.endpoint_str_id(EndpointSlot::Secondary))
                        .await;
                    let api_key = self.api_key_for(chain.str_id()).await;
                    let client = PolkadotClient::new(endpoints, subscan, api_key);
                    let r = client
                        .sign_and_submit(&p.from, &p.to, p.planck, &priv_arr, &pub_arr, p.era, p.tip)
                        .await?;
                    json_response(&r)
                }
                SendParams::Bittensor(p) => {
                    let priv_arr: [u8; 32] =
                        decode_hex_array(&p.private_key_hex, "private_key_hex")?;
                    let pub_arr: [u8; 32] =
                        decode_hex_array(&p.public_key_hex, "public_key_hex")?;
                    let taostats = self
                        .endpoints_for(&chain.endpoint_str_id(EndpointSlot::Secondary))
                        .await;
                    let api_key = self.api_key_for(chain.str_id()).await;
                    let client = BittensorClient::new(endpoints, taostats, api_key);
                    let r = client
                        .sign_and_submit(&p.from, &p.to, p.rao, &priv_arr, &pub_arr)
                        .await?;
                    json_response(&r)
                }
                SendParams::Ton(p) => {
                    let priv_arr: [u8; 64] =
                        decode_hex_array(&p.private_key_hex, "private_key_hex")?;
                    let pub_arr: [u8; 32] =
                        decode_hex_array(&p.public_key_hex, "public_key_hex")?;
                    let api_key = self.api_key_for(chain.str_id()).await;
                    let client = TonClient::new(endpoints, api_key);
                    let seqno = client.fetch_seqno(&p.from).await?;
                    let r = client
                        .sign_and_send(
                            &p.to,
                            p.nanotons,
                            seqno,
                            p.comment.as_deref(),
                            &priv_arr,
                            &pub_arr,
                            p.subwallet_id.map(|n| n as u32),
                            p.expiry_seconds.map(|n| n as u32),
                            p.send_mode.map(|n| n as u8),
                        )
                        .await?;
                    Ok(serde_json::to_string(&r)?)
                }
                SendParams::Icp(p) => {
                    let priv_bytes = hex::decode(&p.private_key_hex).map_err(|e| {
                        SpectraBridgeError::from(format!("private_key_hex hex decode: {e}"))
                    })?;
                    // public_key_hex is optional: derive compressed secp256k1 pubkey when absent.
                    let derived_pub: Vec<u8>;
                    let pub_bytes: &[u8] =
                        match p.public_key_hex.as_deref().filter(|s| !s.is_empty()) {
                            Some(s) => {
                                derived_pub =
                                    hex::decode(s).map_err(|e| format!("pubkey hex: {e}"))?;
                                &derived_pub
                            }
                            None => {
                                use secp256k1::{PublicKey as SecpPubKey, Secp256k1, SecretKey};
                                let secp = Secp256k1::new();
                                let secret = SecretKey::from_slice(&priv_bytes)
                                    .map_err(|e| format!("bad privkey: {e}"))?;
                                derived_pub = SecpPubKey::from_secret_key(&secp, &secret)
                                    .serialize()
                                    .to_vec();
                                &derived_pub
                            }
                        };
                    let client = IcpClient::new(endpoints);
                    let r = client
                        .sign_and_submit(&p.from, &p.to, p.e8s, &priv_bytes, pub_bytes)
                        .await?;
                    Ok(serde_json::to_string(&r)?)
                }
                SendParams::Monero(p) => {
                    let client = MoneroClient::new(endpoints);
                    let r = client
                        .send(&p.to, p.piconeros, 0, p.priority.unwrap_or(2) as u32)
                        .await?;
                    Ok(serde_json::to_string(&r)?)
                }
            },
            ExecuteSendParams::Token(token) => match token {
                SendTokenParams::Evm(p, overrides) => {
                    let priv_bytes = hex::decode(&p.private_key_hex).map_err(|e| {
                        SpectraBridgeError::from(format!("private_key_hex hex decode: {e}"))
                    })?;
                    let client = EvmClient::new(endpoints, chain.evm_chain_id());
                    let r = client
                        .sign_and_broadcast_erc20_with_overrides(
                            &p.from,
                            &p.contract,
                            &p.to,
                            p.amount_raw,
                            &priv_bytes,
                            overrides,
                        )
                        .await?;
                    Ok(serde_json::to_string(&r)?)
                }
                SendTokenParams::Tron(p) => {
                    // Tron — TRC-20. Addresses are base58, amount is in token
                    // units, `fee_limit_sun` defaults to 100 TRX
                    // (100_000_000 sun), which covers typical USDT transfers
                    // (roughly 13-25 TRX actual cost).
                    let priv_bytes = hex::decode(&p.private_key_hex).map_err(|e| {
                        SpectraBridgeError::from(format!("private_key_hex hex decode: {e}"))
                    })?;
                    let client = TronClient::new(endpoints);
                    let r = client
                        .sign_and_broadcast_trc20(
                            &p.from,
                            &p.contract,
                            &p.to,
                            p.amount_raw,
                            p.fee_limit_sun.unwrap_or(100_000_000),
                            &priv_bytes,
                        )
                        .await?;
                    Ok(serde_json::to_string(&r)?)
                }
                SendTokenParams::Near(p) => {
                    // NEAR — NEP-141 fungible token transfer (ft_transfer).
                    let priv_arr: [u8; 64] =
                        decode_hex_array(&p.private_key_hex, "private_key_hex")?;
                    let pub_arr: [u8; 32] =
                        decode_hex_array(&p.public_key_hex, "public_key_hex")?;
                    let client = NearClient::new(endpoints);
                    let r = client
                        .sign_and_broadcast_ft_transfer(
                            &p.from,
                            &p.contract,
                            &p.to,
                            p.amount_raw,
                            &priv_arr,
                            &pub_arr,
                            p.gas_tgas,
                        )
                        .await?;
                    Ok(serde_json::to_string(&r)?)
                }
                SendTokenParams::Solana(p) => {
                    // Solana — SPL token transfer with idempotent ATA create.
                    let from_arr: [u8; 32] =
                        decode_hex_array(&p.from_pubkey_hex, "from_pubkey_hex")?;
                    let priv_arr: [u8; 64] =
                        decode_hex_array(&p.private_key_hex, "private_key_hex")?;
                    let client = SolanaClient::new(endpoints);
                    let r = client
                        .sign_and_broadcast_spl(
                            &from_arr,
                            &p.to,
                            &p.mint,
                            p.amount_raw,
                            p.decimals,
                            &priv_arr,
                        )
                        .await?;
                    Ok(serde_json::to_string(&r)?)
                }
            },
        }
    }

    pub(crate) async fn fetch_utxo_fee_preview(
        &self,
        chain_id: &str,
        address: String,
        fee_rate_svb: u64,
    ) -> Result<String, SpectraBridgeError> {
        let chain = Chain::from_str_id(chain_id).ok_or_else(|| {
            SpectraBridgeError::from(format!(
                "fetch_utxo_fee_preview: unsupported chain_id: {chain_id}"
            ))
        })?;
        let eps = self.endpoints_for(chain.str_id()).await;
        match chain {
            Chain::Bitcoin => {
                let client = BitcoinClient::new(HttpClient::shared(), eps);
                let utxos = client.fetch_utxos(&address).await?;
                let rate = if fee_rate_svb > 0 {
                    fee_rate_svb
                } else {
                    client
                        .fetch_fee_rate(3)
                        .await
                        .map(|r| r.sats_per_vbyte.ceil() as u64)
                        .unwrap_or(5)
                };
                let values: Vec<u64> = utxos.into_iter().map(|u| u.value).collect();
                Ok(utxo_fee_preview_json(values, rate))
            }
            Chain::Dogecoin => {
                let client = DogecoinClient::new(eps);
                let utxos = client.fetch_utxos(&address).await?;
                let rate = if fee_rate_svb > 0 { fee_rate_svb } else { 1 };
                let values: Vec<u64> = utxos.into_iter().map(|u| u.value_koin).collect();
                Ok(utxo_fee_preview_json(values, rate))
            }
            Chain::Litecoin => {
                let client = LitecoinClient::new(eps);
                let utxos = client.fetch_utxos(&address).await?;
                let rate = if fee_rate_svb > 0 {
                    fee_rate_svb
                } else {
                    client.fetch_fee_rate(3).await
                };
                let values: Vec<u64> = utxos.into_iter().map(|u| u.value_sat).collect();
                Ok(utxo_fee_preview_json(values, rate))
            }
            Chain::BitcoinCash => {
                let client = BitcoinCashClient::new(eps);
                let utxos = client.fetch_utxos(&address).await?;
                let rate = if fee_rate_svb > 0 {
                    fee_rate_svb
                } else {
                    client.fetch_fee_rate(3).await
                };
                let values: Vec<u64> = utxos.into_iter().map(|u| u.value_sat).collect();
                Ok(utxo_fee_preview_json(values, rate))
            }
            Chain::BitcoinSV => {
                let client = BitcoinSvClient::new(eps);
                let utxos = client.fetch_utxos(&address).await?;
                let rate = if fee_rate_svb > 0 { fee_rate_svb } else { 1 };
                let values: Vec<u64> = utxos.into_iter().map(|u| u.value_sat).collect();
                Ok(utxo_fee_preview_json(values, rate))
            }
            c => Err(SpectraBridgeError::from(format!(
                "fetch_utxo_fee_preview: unsupported chain: {c:?}"
            ))),
        }
    }

    pub(crate) async fn broadcast_raw(
        &self,
        chain_id: &str,
        payload: String,
    ) -> Result<String, SpectraBridgeError> {
        let chain = Chain::from_str_id(chain_id).ok_or_else(|| {
            SpectraBridgeError::from(format!("broadcast_raw: chain {chain_id} not supported"))
        })?;
        let eps = self.endpoints_for(chain.str_id()).await;
        match chain {
            Chain::Bitcoin => {
                let client = BitcoinClient::new(HttpClient::shared(), eps);
                let txid = client.broadcast_raw_tx(&payload).await?;
                Ok(json!({ "txid": txid }).to_string())
            }
            Chain::Dogecoin => {
                let client = DogecoinClient::new(eps);
                let res = client.broadcast_raw_tx(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Litecoin => {
                let client = LitecoinClient::new(eps);
                let res = client.broadcast_raw_tx(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::BitcoinCash => {
                let client = BitcoinCashClient::new(eps);
                let res = client.broadcast_raw_tx(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::BitcoinSV => {
                let client = BitcoinSvClient::new(eps);
                let res = client.broadcast_raw_tx(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Solana => {
                let client = SolanaClient::new(eps);
                let res = client.broadcast_raw(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Tron => {
                let client = TronClient::new(eps);
                let res = client.broadcast_raw(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            c if c.is_evm() => {
                let client = EvmClient::new(eps, c.evm_chain_id());
                let res = client.broadcast_raw(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Xrp => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let blob = val["tx_blob_hex"]
                    .as_str()
                    .ok_or("broadcast_raw xrp: missing tx_blob_hex")?
                    .to_string();
                let client = XrpClient::new(eps);
                let res = client.submit_signed_blob(&blob).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Stellar => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let xdr = val["signed_xdr_b64"]
                    .as_str()
                    .ok_or("broadcast_raw stellar: missing signed_xdr_b64")?
                    .to_string();
                let client = StellarClient::new(eps);
                let res = client.submit_envelope_b64(&xdr).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Cardano => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let cbor = val["cbor_hex"]
                    .as_str()
                    .ok_or("broadcast_raw cardano: missing cbor_hex")?
                    .to_string();
                let api_key = self.api_key_for(chain.str_id()).await.unwrap_or_default();
                let client = CardanoClient::new(eps, api_key);
                let res = client.submit_tx(&cbor).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Polkadot => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let ext_hex = val["extrinsic_hex"]
                    .as_str()
                    .ok_or("broadcast_raw polkadot: missing extrinsic_hex")?
                    .to_string();
                let subscan = self
                    .endpoints_for(&chain.endpoint_str_id(EndpointSlot::Secondary))
                    .await;
                let api_key = self.api_key_for(chain.str_id()).await;
                let client = PolkadotClient::new(eps, subscan, api_key);
                let res = client.submit_extrinsic_hex(&ext_hex).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Sui => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let tx_bytes = val["tx_bytes_b64"]
                    .as_str()
                    .ok_or("broadcast_raw sui: missing tx_bytes_b64")?
                    .to_string();
                let sig = val["sig_b64"]
                    .as_str()
                    .ok_or("broadcast_raw sui: missing sig_b64")?
                    .to_string();
                let client = SuiClient::new(eps);
                let res = client.execute_signed_tx(&tx_bytes, &sig).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Aptos => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let body_json = val["signed_body_json"]
                    .as_str()
                    .ok_or("broadcast_raw aptos: missing signed_body_json")?
                    .to_string();
                let client = AptosClient::new(eps);
                let res = client.submit_signed_body(&body_json).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Ton => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let boc = val["boc_b64"]
                    .as_str()
                    .ok_or("broadcast_raw ton: missing boc_b64")?
                    .to_string();
                let api_key = self.api_key_for(chain.str_id()).await;
                let client = TonClient::new(eps, api_key);
                let res = client.send_boc(&boc).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Near => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let tx_b64 = val["signed_tx_b64"]
                    .as_str()
                    .ok_or("broadcast_raw near: missing signed_tx_b64")?
                    .to_string();
                let client = NearClient::new(eps);
                let res = client.broadcast_signed_tx_b64(&tx_b64).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Icp => Err(SpectraBridgeError::from(
                "ICP rebroadcast is not supported".to_string(),
            )),
            c => Err(SpectraBridgeError::from(format!(
                "broadcast_raw: chain {c:?} not supported"
            ))),
        }
    }

    pub(crate) async fn fetch_evm_send_preview(
        &self,
        chain_id: &str,
        from: String,
        to: String,
        value_wei: String,
        data_hex: String,
    ) -> Result<String, SpectraBridgeError> {
        let chain = chain_for_evm_id(chain_id)?;
        let eps = self.endpoints_for(chain.str_id()).await;
        let client = EvmClient::new(eps, chain.evm_chain_id());

        let value_u128: u128 = value_wei.parse().unwrap_or(0);
        let data_opt: Option<&str> = if data_hex == "0x" || data_hex.is_empty() {
            None
        } else {
            Some(&data_hex)
        };

        let (nonce_res, fee_res, gas_res, bal_res) = tokio::join!(
            client.fetch_nonce(&from),
            client.fetch_fee_estimate(),
            client.estimate_gas(&from, &to, value_u128, data_opt),
            client.fetch_balance(&from)
        );

        let nonce = nonce_res.unwrap_or(0);
        let fee = fee_res.unwrap_or(crate::fetch::chains::evm::EvmFeeEstimate {
            base_fee_wei: 0,
            priority_fee_wei: 1_000_000_000,
            max_fee_per_gas_wei: 2_000_000_000,
            estimated_fee_wei: 42_000_000_000,
        });
        let gas_limit = gas_res.unwrap_or(21_000);
        let balance_wei_val: u128 = bal_res
            .map(|b| b.balance_wei.parse::<u128>().unwrap_or(0))
            .unwrap_or(0);

        let estimated_fee_wei: u128 = (gas_limit as u128).saturating_mul(fee.max_fee_per_gas_wei);
        let max_fee_gwei = fee.max_fee_per_gas_wei as f64 / 1_000_000_000.0;
        let priority_fee_gwei = fee.priority_fee_wei as f64 / 1_000_000_000.0;
        let balance_eth = balance_wei_val as f64 / 1e18;
        let estimated_fee_eth = estimated_fee_wei as f64 / 1e18;
        let spendable_eth = (balance_wei_val.saturating_sub(estimated_fee_wei)) as f64 / 1e18;

        Ok(json!({
            "nonce": nonce,
            "gas_limit": gas_limit,
            "max_fee_per_gas_gwei": max_fee_gwei,
            "max_priority_fee_per_gas_gwei": priority_fee_gwei,
            "estimated_fee_eth": estimated_fee_eth,
            "balance_eth": balance_eth,
            "spendable_eth": spendable_eth,
            "fee_rate_description": format!("Max {:.2} gwei / Priority {:.2} gwei",
                max_fee_gwei, priority_fee_gwei),
        })
        .to_string())
    }

    pub(crate) async fn fetch_tron_send_preview(
        &self,
        address: String,
        symbol: String,
        contract_address: String,
    ) -> Result<String, SpectraBridgeError> {
        let eps = self.endpoints_for("tron").await;
        let client = TronClient::new(eps);

        let trx_balance = client
            .fetch_balance(&address)
            .await
            .map(|b| b.sun as f64 / 1_000_000.0)
            .unwrap_or(0.0);

        if symbol == "TRX" || contract_address.is_empty() {
            let fee_trx = 1.0_f64;
            let spendable = (trx_balance - fee_trx).max(0.0);
            return Ok(json!({
                "estimated_fee_trx": fee_trx,
                "fee_limit_sun": 0_i64,
                "spendable_balance": spendable,
                "max_sendable": spendable,
                "fee_rate_description": "Static bandwidth estimate",
            })
            .to_string());
        }

        let token_balance = client
            .fetch_trc20_balance_of(&contract_address, &address)
            .await
            .map(|raw| raw as f64 / 1_000_000.0)
            .unwrap_or(0.0);

        let fee_trx = 15.0_f64;
        let fee_limit_sun: i64 = 15_000_000;
        Ok(json!({
            "estimated_fee_trx": fee_trx,
            "fee_limit_sun": fee_limit_sun,
            "spendable_balance": token_balance,
            "max_sendable": token_balance,
            "fee_rate_description": "Static energy estimate",
        })
        .to_string())
    }

    pub(crate) async fn fetch_simple_chain_send_preview(
        &self,
        chain_id: &str,
        address: String,
    ) -> Result<String, SpectraBridgeError> {
        let chain = Chain::from_str_id(chain_id).ok_or_else(|| {
            SpectraBridgeError::from(format!("unknown chain_id: {chain_id}"))
        })?;
        let (fee, balance) = tokio::try_join!(
            self.native_fee_estimate(chain),
            self.fetch_native_balance_summary(chain_id.to_string(), address),
        )?;

        let fee_display = fee.display.parse::<f64>().unwrap_or(0.0);
        let fee_raw = fee.raw;
        let fee_rate_description = fee.source.to_string();

        let balance_display = summary_display_balance(chain_id, &balance);
        let max_sendable = (balance_display - fee_display).max(0.0);

        Ok(json!({
            "fee_display":          fee_display,
            "fee_raw":              fee_raw,
            "fee_rate_description": fee_rate_description,
            "balance_display":      balance_display,
            "max_sendable":         max_sendable,
        })
        .to_string())
    }
}

#[cfg(test)]
mod fee_estimates_are_typed {
    use crate::registry::Chain;
    use crate::service::WalletService;

    /// The static-fee chains quote the catalog's number, scaled by their own
    /// decimals. This went through a serialized `FeePreview` and a
    /// `serde_json::from_str` in the caller before; the numbers are the same
    /// ones, reached without the round trip. No network: `static_fee_units`
    /// is catalog data, so these arms never build a client.
    #[tokio::test]
    async fn a_static_fee_chain_quotes_the_catalog_scaled_by_its_decimals() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        // (chain, raw units, display)
        for (chain, raw, display) in [
            (Chain::Solana, "5000", "0.000005"),      // 9 decimals
            (Chain::Cardano, "170000", "0.17"),       // 6 decimals
            (Chain::Sui, "1000", "0.000001"),         // 9 decimals
            (Chain::Icp, "10000", "0.0001"),          // 8 decimals
            (Chain::Polkadot, "160000000", "0.016"),  // 10 decimals
        ] {
            let fee = service.native_fee_estimate(chain).await.expect("fee");
            assert_eq!(fee.raw, raw, "{}", chain.chain_display_name());
            assert_eq!(fee.display, display, "{}", chain.chain_display_name());
            assert_eq!(fee.source, "static");
        }
    }

    /// NEAR's fee does not fit the `u128 -> display` path the others take —
    /// it is carried as the string it is, which is why it had its own arm
    /// before and still does.
    #[tokio::test]
    async fn near_carries_its_fee_as_a_string() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let fee = service.native_fee_estimate(Chain::Near).await.expect("fee");
        assert_eq!(fee.raw, "1000000000000000000000");
        assert_eq!(fee.display, "0.001");
        assert_eq!(fee.source, "static");
    }

    /// A chain with no fee to quote is an error naming it, where the JSON
    /// version returned `{"note": "fee estimation not supported…"}` that the
    /// caller then read zeros out of. Nothing routes such a chain here —
    /// `simple_preview_chain` covers eleven, all of which answer — so this is
    /// the guard, not a live path.
    #[tokio::test]
    async fn a_chain_with_no_fee_is_a_named_error() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let err = service
            .native_fee_estimate(Chain::Ethereum)
            .await
            .expect_err("EVM has its own preview path, not this one");
        assert!(format!("{err:?}").contains("Ethereum"));
    }
}
