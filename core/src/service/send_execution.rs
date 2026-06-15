use super::*;
use zeroize::Zeroizing;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Derive key material, build the chain-specific payload, sign, and
    /// broadcast in a single call.
    ///
    /// This eliminates the Swift↔Rust trampoline where Swift held a closure
    /// between derivation and signing. Swift now passes the seed phrase (or
    /// raw private key) directly, and Rust handles the entire pipeline.
    pub async fn execute_send(
        &self,
        mut request: crate::send::SendExecutionRequest,
    ) -> Result<crate::send::SendExecutionResult, SpectraBridgeError> {
        let result: Result<crate::send::SendExecutionResult, SpectraBridgeError> = async {
            // 1. Derive key material (or use provided private key).
            let (priv_hex, pub_hex) = if let Some(ref seed_phrase) = request.seed_phrase {
                use crate::derivation::types::BitcoinScriptType;
                let ov = request.derivation_overrides.as_ref();
                let pass = ov
                    .and_then(|o| o.passphrase.as_deref())
                    .filter(|s| !s.is_empty());
                let hmac = ov
                    .and_then(|o| o.hmac_key.as_deref())
                    .filter(|s| !s.is_empty());
                let script = ov
                    .and_then(|o| o.script_type.as_deref())
                    .and_then(|s| match s.to_lowercase().as_str() {
                        "p2pkh" => Some(BitcoinScriptType::P2pkh),
                        "p2shp2wpkh" | "p2sh-p2wpkh" => Some(BitcoinScriptType::P2shP2wpkh),
                        "p2wpkh" => Some(BitcoinScriptType::P2wpkh),
                        "p2tr" => Some(BitcoinScriptType::P2tr),
                        _ => None,
                    })
                    .unwrap_or_else(|| {
                        crate::derivation::dispatch::script_type_for_path(&request.derivation_path)
                    });
                let r = crate::derivation::dispatch::derive_for_chain_name(
                    &request.chain_name,
                    seed_phrase,
                    &request.derivation_path,
                    pass,
                    hmac,
                    Some(script),
                    false,
                    true,
                    true,
                )?;
                let priv_h = r.private_key_hex.ok_or_else(|| {
                    SpectraBridgeError::from("derivation returned no private key")
                })?;
                (priv_h, r.public_key_hex)
            } else if let Some(ref pk) = request.private_key_hex {
                let normalized = pk.strip_prefix("0x").unwrap_or(pk).to_string();
                (normalized, None)
            } else {
                return Err(SpectraBridgeError::from(
                    "execute_send: neither seed_phrase nor private_key_hex provided",
                ));
            };
            let priv_hex = Zeroizing::new(priv_hex);

            // 2. Build payload JSON and route to sign_and_send or sign_and_send_token.
            let is_token = request.contract_address.is_some();
            let params_json =
                self.build_execute_send_payload(&request, priv_hex.as_str(), &pub_hex)?;

            let result_json = if is_token {
                self.sign_and_send_token(&request.chain_id, params_json)
                    .await?
            } else {
                self.sign_and_send(&request.chain_id, params_json).await?
            };

            // 3. Classify broadcast result. `is_token` is intentionally unused —
            // `SendChain` is chain-family granularity, not token/native.
            let _ = is_token;
            let send_chain = Chain::from_str_id(&request.chain_id)
                .map(Chain::send_chain)
                .unwrap_or(crate::send::payload::SendChain::Bitcoin);
            let outcome = crate::send::payload::classify_send_broadcast_result(
                send_chain,
                result_json.clone(),
            );

            // 4. For EVM chains, decode the typed result here so Swift doesn't
            // have to round-trip through `decode_evm_send_result(json:)`.
            let evm = if Chain::from_str_id(&request.chain_id).is_some_and(Chain::is_evm) {
                let fallback_nonce = request
                    .evm_overrides
                    .as_ref()
                    .and_then(|o| o.nonce)
                    .unwrap_or(0);
                Some(crate::send::ethereum::decode_evm_send_result_internal(
                    &result_json,
                    fallback_nonce,
                ))
            } else {
                None
            };

            Ok(crate::send::SendExecutionResult {
                rebroadcast_payload: result_json,
                transaction_hash: outcome.transaction_hash,
                payload_format: outcome.payload_format,
                evm,
            })
        }
        .await;
        request.zeroize_sensitive_fields();
        result
    }
}

impl WalletService {
    fn build_execute_send_payload(
        &self,
        req: &crate::send::SendExecutionRequest,
        priv_hex: &str,
        pub_hex: &Option<String>,
    ) -> Result<serde_json::Value, SpectraBridgeError> {
        use crate::send::payload::*;
        use crate::send::preview_decode::{
            amount_to_raw_units_string, build_utxo_sat_send_payload, decimal_str_to_raw_units,
        };

        let from = &req.from_address;
        let to = &req.to_address;
        let amount = req.amount;
        let priv_str = Zeroizing::new(priv_hex.to_string());
        let priv_owned = || priv_str.as_str().to_string();

        let chain = Chain::from_str_id(&req.chain_id).ok_or_else(|| {
            SpectraBridgeError::from(format!(
                "execute_send: unsupported chain_id: {}",
                req.chain_id
            ))
        })?;

        // When the caller supplies the original decimal string (from
        // `SendSubmitPreflightPlan.amount_str`), raw-unit conversion uses pure
        // string arithmetic to avoid f64 precision loss. Falls back to the f64
        // path for legacy callers that don't set `amount_str`.
        let to_raw = |dec: u32| -> String {
            match req.amount_str.as_deref() {
                Some(s) => decimal_str_to_raw_units(s, dec),
                None => amount_to_raw_units_string(amount, dec),
            }
        };

        // Each chain's `build_*_send_payload` still produces a JSON String
        // internally; parse to `Value` on exit so `sign_and_send` receives the
        // same typed representation as the legacy path.
        let json_string: String = if let Some(ref contract) = req.contract_address {
            let decimals = req.token_decimals.unwrap_or(6);
            match chain {
                c if c.is_evm() => {
                    let amount_raw = to_raw(decimals);
                    let overrides = crate::send::ethereum::render_evm_overrides_fragment(
                        req.evm_overrides.as_ref(),
                    );
                    crate::send::ethereum::build_evm_token_send_payload(
                        from.clone(),
                        contract.clone(),
                        to.clone(),
                        amount_raw,
                        priv_owned(),
                        overrides,
                    )
                }
                Chain::Tron => build_tron_token_send_payload(
                    from.clone(),
                    contract.clone(),
                    to.clone(),
                    amount,
                    decimals,
                    priv_owned(),
                ),
                Chain::Solana => build_solana_token_send_payload(
                    pub_hex.clone().unwrap_or_default(),
                    contract.clone(),
                    to.clone(),
                    amount,
                    decimals,
                    priv_owned(),
                ),
                Chain::Near => build_near_token_send_payload(
                    from.clone(),
                    contract.clone(),
                    to.clone(),
                    to_raw(decimals),
                    priv_owned(),
                    pub_hex.clone().unwrap_or_default(),
                ),
                c => {
                    return Err(SpectraBridgeError::from(format!(
                        "execute_send: unsupported token chain: {c:?}"
                    )))
                }
            }
        } else {
            match chain {
                Chain::Bitcoin => build_btc_send_payload(
                    from.clone(),
                    to.clone(),
                    amount,
                    req.fee_rate_svb.unwrap_or(10.0),
                    priv_owned(),
                ),
                c if c.is_evm() => {
                    let value_wei = to_raw(18);
                    let overrides = crate::send::ethereum::render_evm_overrides_fragment(
                        req.evm_overrides.as_ref(),
                    );
                    crate::send::ethereum::build_evm_native_send_payload(
                        from.clone(),
                        to.clone(),
                        value_wei,
                        priv_owned(),
                        overrides,
                    )
                }
                Chain::Solana => build_solana_native_send_payload(
                    pub_hex.clone().unwrap_or_default(),
                    to.clone(),
                    amount,
                    priv_owned(),
                ),
                Chain::Dogecoin => build_doge_send_payload(
                    from.clone(),
                    to.clone(),
                    amount,
                    req.fee_rate_svb.unwrap_or(0.01),
                    priv_owned(),
                ),
                Chain::Xrp => build_xrp_send_payload(
                    from.clone(),
                    to.clone(),
                    amount,
                    priv_owned(),
                    pub_hex.clone(),
                ),
                Chain::Litecoin | Chain::BitcoinCash => {
                    let amount_sat =
                        (amount * 10f64.powi(chain.native_decimals() as i32)).round() as u64;
                    let fee_sat = req.fee_sat.unwrap_or(if chain == Chain::Litecoin {
                        10_000
                    } else {
                        1_000
                    });
                    build_utxo_sat_send_payload(
                        from.clone(),
                        to.clone(),
                        amount_sat,
                        fee_sat,
                        priv_owned(),
                    )
                }
                Chain::Tron => {
                    build_tron_native_send_payload(from.clone(), to.clone(), amount, priv_owned())
                }
                Chain::Stellar => build_stellar_send_payload(
                    from.clone(),
                    to.clone(),
                    amount,
                    priv_owned(),
                    pub_hex.clone(),
                ),
                Chain::Cardano => build_cardano_send_payload(
                    from.clone(),
                    to.clone(),
                    amount,
                    req.fee_amount.unwrap_or(0.17),
                    priv_owned(),
                    pub_hex.clone().unwrap_or_default(),
                ),
                Chain::Polkadot => build_polkadot_send_payload(
                    from.clone(),
                    to.clone(),
                    to_raw(10),
                    priv_owned(),
                    pub_hex.clone().unwrap_or_default(),
                ),
                Chain::Bittensor => crate::send::payload::build_bittensor_send_payload(
                    from.clone(),
                    to.clone(),
                    to_raw(9),
                    priv_owned(),
                    pub_hex.clone().unwrap_or_default(),
                ),
                Chain::Sui => build_sui_send_payload(
                    from.clone(),
                    to.clone(),
                    amount,
                    req.gas_budget.unwrap_or(0.01),
                    priv_owned(),
                    pub_hex.clone().unwrap_or_default(),
                ),
                Chain::Aptos => build_aptos_send_payload(
                    from.clone(),
                    to.clone(),
                    amount,
                    priv_owned(),
                    pub_hex.clone().unwrap_or_default(),
                ),
                Chain::Ton => build_ton_send_payload(
                    from.clone(),
                    to.clone(),
                    amount,
                    priv_owned(),
                    pub_hex.clone().unwrap_or_default(),
                ),
                Chain::Near => build_near_send_payload(
                    from.clone(),
                    to.clone(),
                    to_raw(24),
                    priv_owned(),
                    pub_hex.clone().unwrap_or_default(),
                ),
                Chain::Icp => build_icp_send_payload(
                    from.clone(),
                    to.clone(),
                    amount,
                    priv_owned(),
                    pub_hex.clone(),
                ),
                Chain::Monero => {
                    build_monero_send_payload(to.clone(), amount, req.monero_priority.unwrap_or(2))
                }
                Chain::BitcoinSV => {
                    let amount_sat =
                        (amount * 10f64.powi(chain.native_decimals() as i32)).round() as u64;
                    let fee_sat = req.fee_sat.unwrap_or(1_000);
                    build_utxo_sat_send_payload(
                        from.clone(),
                        to.clone(),
                        amount_sat,
                        fee_sat,
                        priv_owned(),
                    )
                }
                Chain::Zcash => {
                    let amount_sat =
                        (amount * 10f64.powi(chain.native_decimals() as i32)).round() as u64;
                    let fee_sat = req.fee_sat.unwrap_or(1_000);
                    build_utxo_sat_send_payload(
                        from.clone(),
                        to.clone(),
                        amount_sat,
                        fee_sat,
                        priv_owned(),
                    )
                }
                Chain::BitcoinGold => {
                    let amount_sat =
                        (amount * 10f64.powi(chain.native_decimals() as i32)).round() as u64;
                    let fee_sat = req.fee_sat.unwrap_or(1_000);
                    build_utxo_sat_send_payload(
                        from.clone(),
                        to.clone(),
                        amount_sat,
                        fee_sat,
                        priv_owned(),
                    )
                }
                Chain::Decred => {
                    let amount_atoms =
                        (amount * 10f64.powi(chain.native_decimals() as i32)).round() as u64;
                    let fee_atoms = req.fee_sat.unwrap_or(2_000);
                    build_utxo_sat_send_payload(
                        from.clone(),
                        to.clone(),
                        amount_atoms,
                        fee_atoms,
                        priv_owned(),
                    )
                }
                Chain::Kaspa => {
                    let amount_sompi =
                        (amount * 10f64.powi(chain.native_decimals() as i32)).round() as u64;
                    let fee_sompi = req.fee_sat.unwrap_or(1_000);
                    build_utxo_sat_send_payload(
                        from.clone(),
                        to.clone(),
                        amount_sompi,
                        fee_sompi,
                        priv_owned(),
                    )
                }
                Chain::Dash => {
                    let amount_sat =
                        (amount * 10f64.powi(chain.native_decimals() as i32)).round() as u64;
                    let fee_sat = req.fee_sat.unwrap_or(2_000);
                    build_utxo_sat_send_payload(
                        from.clone(),
                        to.clone(),
                        amount_sat,
                        fee_sat,
                        priv_owned(),
                    )
                }
                c => {
                    return Err(SpectraBridgeError::from(format!(
                        "execute_send: unsupported chain: {c:?}"
                    )))
                }
            }
        };
        serde_json::from_str(&json_string).map_err(Into::into)
    }
}
