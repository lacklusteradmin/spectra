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

            // 2. Resolve the chain once — `build_send_params` and
            // `sign_and_broadcast_send` both take it as a parameter rather
            // than each re-deriving it from `chain_id`, so the two can never
            // disagree about which chain this is.
            let chain = Chain::from_str_id(&request.chain_id).ok_or_else(|| {
                SpectraBridgeError::from(format!(
                    "execute_send: unsupported chain_id: {}",
                    request.chain_id
                ))
            })?;
            let params = self
                .build_send_params(chain, &request, priv_hex.as_str(), &pub_hex)
                .await?;
            let result_json = self.sign_and_broadcast_send(chain, params).await?;

            // 3. Classify broadcast result.
            let send_chain = chain.send_chain();
            let outcome = crate::send::payload::classify_send_broadcast_result(
                send_chain,
                result_json.clone(),
            );

            // 4. For EVM chains, decode the typed result here so Swift doesn't
            // have to round-trip through `decode_evm_send_result(json:)`.
            let evm = if chain.is_evm() {
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
    /// What the contract says its token is denominated in, or `None` when the
    /// family does not expose it or the node will not answer.
    async fn token_contract_decimals(&self, chain: Chain, contract: &str) -> Option<u32> {
        let endpoints = self.endpoints_for(chain.str_id()).await;
        if chain.is_evm() {
            let client = crate::fetch::chains::evm::EvmClient::new(endpoints, chain.evm_chain_id());
            return client
                .fetch_erc20_metadata(contract)
                .await
                .ok()
                .map(|m| u32::from(m.decimals));
        }
        if chain == Chain::Tron {
            let client = crate::fetch::chains::tron::TronClient::new(endpoints);
            return client
                .fetch_trc20_metadata(contract)
                .await
                .ok()
                .map(|m| u32::from(m.decimals));
        }
        None
    }

    /// Build what this send needs to sign, directly as a typed [`SendParams`]
    /// or [`SendTokenParams`] — no JSON in the middle.
    ///
    /// This is what `build_execute_send_payload` used to do by formatting a
    /// JSON string per chain, which `sign_and_send` / `sign_and_send_token`
    /// then re-parsed one match away. Two matches over the same 24-chain enum,
    /// hand-kept in sync, for a value that never left the process: building it
    /// and reading it back happened in the same `execute_send` call, on the
    /// same thread. This match is now the only one; `sign_and_broadcast_send`
    /// reads the variant it produced instead of re-deriving `Chain` from a
    /// string and re-parsing a blob it was just handed.
    ///
    /// Every per-chain amount conversion below is unchanged from what the old
    /// JSON builders computed — same function, same scale, same rounding —
    /// only the destination changed, from a formatted string to a struct
    /// field. Where two chains used to convert the same kind of amount two
    /// different ways (the string-exact `to_raw` path for EVM/NEAR token
    /// amounts, the f64-scaled `amount_u64` path for Tron/Solana token
    /// amounts), that difference is preserved exactly, not unified — this is
    /// a structural cleanup, not a precision change.
    async fn build_send_params(
        &self,
        chain: Chain,
        req: &crate::send::SendExecutionRequest,
        priv_hex: &str,
        pub_hex: &Option<String>,
    ) -> Result<crate::service::send_params::ExecuteSendParams, SpectraBridgeError> {
        use crate::send::payload::{amount_i64, amount_u64};
        use crate::send::preview_decode::{amount_to_raw_units_string, decimal_str_to_raw_units};
        use crate::service::send_params::*;

        let from = req.from_address.clone();
        let to = req.to_address.clone();
        let amount = req.amount;
        let private_key_hex = priv_hex.to_string();
        let public_key_hex = pub_hex.clone();

        // Same string-exact conversion `build_execute_send_payload` used: the
        // caller's original decimal string when it supplied one, otherwise
        // `amount` formatted to `dec` places first so the f64 path also goes
        // through pure string arithmetic rather than binary-float scaling.
        let to_raw = |dec: u32| -> String {
            match req.amount_str.as_deref() {
                Some(s) => decimal_str_to_raw_units(s, dec),
                None => amount_to_raw_units_string(amount, dec),
            }
        };
        // What `deserialize_u128_from_string_or_number` did to a JSON string
        // produced by `to_raw`: parse it, and turn a parse failure into the
        // same kind of error `parse_params` would have raised.
        let raw_u128 = |dec: u32, field: &'static str| -> Result<u128, SpectraBridgeError> {
            to_raw(dec)
                .parse::<u128>()
                .map_err(|e| SpectraBridgeError::from(format!("{field}: {e}")))
        };

        if let Some(ref contract) = req.contract_address {
            // The contract's own `decimals`, read before signing.
            //
            // This was `req.token_decimals.unwrap_or(6)` — a caller that did
            // not supply the count got a transfer denominated at six places
            // whatever the contract says, and a caller that supplied a stale
            // one was believed. Both are the same mistake the Tron send arm
            // made one layer up, and the cost of not making it is one constant
            // call on a path that is about to move funds.
            //
            // The caller's value is the fallback for a family that does not
            // expose the count, and for a node that will not answer.
            let decimals = self
                .token_contract_decimals(chain, contract)
                .await
                .or(req.token_decimals)
                .ok_or_else(|| {
                    SpectraBridgeError::from(format!(
                        "execute_send: {contract} did not report its decimals and none were supplied"
                    ))
                })?;
            let params = match chain {
                c if c.is_evm() => SendTokenParams::Evm(
                    TokenAmountSendParams {
                        from,
                        contract: contract.clone(),
                        to,
                        amount_raw: raw_u128(decimals, "amount_raw")?,
                        private_key_hex,
                    },
                    evm_send_overrides(req.evm_overrides.as_ref()),
                ),
                Chain::Tron => SendTokenParams::Tron(TronTokenSendParams {
                    from,
                    contract: contract.clone(),
                    to,
                    // Tron's token send always took the f64-scaled path, not
                    // `to_raw`'s string-exact one — preserved as-is.
                    amount_raw: u128::from(amount_u64(amount, 10f64.powi(decimals as i32))),
                    fee_limit_sun: None,
                    private_key_hex,
                }),
                Chain::Solana => SendTokenParams::Solana(SolanaTokenSendParams {
                    from_pubkey_hex: public_key_hex.unwrap_or_default(),
                    to,
                    mint: contract.clone(),
                    // Also f64-scaled, like Tron — never went through `to_raw`.
                    amount_raw: amount_u64(amount, 10f64.powi(decimals as i32)),
                    decimals: decimals as u8,
                    private_key_hex,
                }),
                Chain::Near => SendTokenParams::Near(NearTokenSendParams {
                    from,
                    contract: contract.clone(),
                    to,
                    amount_raw: raw_u128(decimals, "amount_raw")?,
                    private_key_hex,
                    public_key_hex: public_key_hex.unwrap_or_default(),
                    gas_tgas: None,
                }),
                c => {
                    return Err(SpectraBridgeError::from(format!(
                        "execute_send: unsupported token chain: {c:?}"
                    )))
                }
            };
            return Ok(ExecuteSendParams::Token(params));
        }

        // Takes `from`/`to`/`private_key_hex` as parameters rather than
        // capturing the outer bindings: several arms below move those same
        // bindings by value (building a different struct), and a closure
        // that borrowed them for `.clone()` would hold that borrow live
        // across the whole match, conflicting with the moves.
        let utxo = |from: String, to: String, private_key_hex: String, amount_sat: u64, fee_sat: Option<u64>| {
            UtxoFixedFeeSendParams {
                from,
                to,
                amount_sat,
                fee_sat,
                private_key_hex,
                dust_threshold_sats: None,
            }
        };
        // The `Chain::X => amount_sat = (amount * 10^native_decimals).round()`
        // shape six chains below share — same rounding `build_utxo_sat_send_payload`
        // received, computed once here. `amount` and `chain` are `Copy`, so
        // this closure captures them freely with no borrow/move conflict.
        let scaled_amount = |fee_default: u64| -> (u64, u64) {
            let amount_units =
                (amount * 10f64.powi(chain.native_decimals() as i32)).round() as u64;
            (amount_units, req.fee_sat.unwrap_or(fee_default))
        };

        let params = match chain {
            Chain::Bitcoin => SendParams::Bitcoin(BitcoinNativeSendParams {
                from,
                to,
                amount_sat: amount_u64(amount, 1e8),
                fee_rate_svb: Some(req.fee_rate_svb.unwrap_or(10.0)),
                private_key_hex,
                dust_threshold_sats: None,
                sign_only: false,
            }),
            c if c.is_evm() => SendParams::Evm(
                EvmNativeSendParams {
                    from,
                    to,
                    value_wei: raw_u128(18, "value_wei")?,
                    private_key_hex,
                },
                evm_send_overrides(req.evm_overrides.as_ref()),
            ),
            Chain::Solana => SendParams::Solana(SolanaNativeSendParams {
                from_pubkey_hex: public_key_hex.unwrap_or_default(),
                to,
                lamports: amount_u64(amount, 1e9),
                private_key_hex,
            }),
            Chain::Dogecoin => {
                // Doge's own builder used a literal 1e8 scale (not
                // `chain.native_decimals()`, which every other UTXO arm
                // below uses) and a kb-rate → tx-size-estimate fee, not a
                // flat `req.fee_sat` — both preserved exactly, not folded
                // into the shared `scaled_amount`/`utxo` shapes below.
                let fee_rate_doge_per_kb = req.fee_rate_svb.unwrap_or(0.01);
                SendParams::Utxo(UtxoFixedFeeSendParams {
                    from,
                    to,
                    amount_sat: amount_u64(amount, 1e8),
                    fee_sat: Some(amount_u64(fee_rate_doge_per_kb * 350.0 / 1000.0, 1e8)),
                    private_key_hex,
                    dust_threshold_sats: None,
                })
            }
            Chain::Xrp => SendParams::Xrp(XrpSendParams {
                from,
                to,
                drops: amount_u64(amount, 1e6),
                private_key_hex,
                public_key_hex,
            }),
            Chain::Litecoin => {
                let (amount_sat, fee_sat) = scaled_amount(10_000);
                SendParams::Utxo(utxo(from, to, private_key_hex, amount_sat, Some(fee_sat)))
            }
            Chain::BitcoinCash => {
                let (amount_sat, fee_sat) = scaled_amount(1_000);
                SendParams::Utxo(utxo(from, to, private_key_hex, amount_sat, Some(fee_sat)))
            }
            Chain::Tron => SendParams::Tron(TronNativeSendParams {
                from,
                to,
                amount_sun: amount_u64(amount, 1e6),
                private_key_hex,
            }),
            Chain::Stellar => SendParams::Stellar(StellarSendParams {
                from,
                to,
                stroops: amount_i64(amount, 1e7),
                private_key_hex,
                public_key_hex,
                network_passphrase: None,
            }),
            Chain::Cardano => SendParams::Cardano(CardanoSendParams {
                from,
                to,
                amount_lovelace: amount_u64(amount, 1e6),
                fee_lovelace: Some(amount_u64(req.fee_amount.unwrap_or(0.17), 1e6)),
                private_key_hex,
                public_key_hex: public_key_hex.unwrap_or_default(),
                ttl_slots: None,
                min_change_lovelace: None,
            }),
            Chain::Polkadot => SendParams::Polkadot(PolkadotSendParams {
                from,
                to,
                planck: raw_u128(10, "planck")?,
                private_key_hex,
                public_key_hex: public_key_hex.unwrap_or_default(),
                era: None,
                tip: None,
            }),
            Chain::Bittensor => SendParams::Bittensor(BittensorSendParams {
                from,
                to,
                rao: raw_u128(9, "rao")?,
                private_key_hex,
                public_key_hex: public_key_hex.unwrap_or_default(),
            }),
            Chain::Sui => SendParams::Sui(SuiSendParams {
                from,
                to,
                mist: amount_u64(amount, 1e9),
                gas_budget: Some(amount_u64(req.gas_budget.unwrap_or(0.01), 1e9)),
                private_key_hex,
                public_key_hex: public_key_hex.unwrap_or_default(),
            }),
            Chain::Aptos => SendParams::Aptos(AptosSendParams {
                from,
                to,
                octas: amount_u64(amount, 1e8),
                private_key_hex,
                public_key_hex: public_key_hex.unwrap_or_default(),
            }),
            Chain::Ton => SendParams::Ton(TonSendParams {
                from,
                to,
                nanotons: amount_u64(amount, 1e9),
                comment: None,
                private_key_hex,
                public_key_hex: public_key_hex.unwrap_or_default(),
                subwallet_id: None,
                expiry_seconds: None,
                send_mode: None,
            }),
            Chain::Near => SendParams::Near(NearNativeSendParams {
                from,
                to,
                yocto_near: raw_u128(24, "yocto_near")?,
                private_key_hex,
                public_key_hex: public_key_hex.unwrap_or_default(),
            }),
            Chain::Icp => SendParams::Icp(IcpSendParams {
                from,
                to,
                e8s: amount_u64(amount, 1e8),
                private_key_hex,
                public_key_hex,
            }),
            Chain::Monero => SendParams::Monero(MoneroSendParams {
                to,
                piconeros: amount_u64(amount, 1e12),
                priority: Some(u64::from(req.monero_priority.unwrap_or(2))),
            }),
            Chain::BitcoinSV => {
                let (amount_sat, fee_sat) = scaled_amount(1_000);
                SendParams::Utxo(utxo(from, to, private_key_hex, amount_sat, Some(fee_sat)))
            }
            Chain::Zcash => {
                let (amount_sat, fee_sat) = scaled_amount(1_000);
                SendParams::Zcash(ZcashSendParams {
                    from,
                    to,
                    amount_sat,
                    fee_sat: Some(fee_sat),
                    private_key_hex,
                    dust_threshold_zats: None,
                })
            }
            Chain::BitcoinGold => {
                let (amount_sat, fee_sat) = scaled_amount(1_000);
                SendParams::Utxo(utxo(from, to, private_key_hex, amount_sat, Some(fee_sat)))
            }
            Chain::Decred => {
                let (amount_sat, fee_sat) = scaled_amount(2_000);
                SendParams::Decred(DecredSendParams {
                    from,
                    to,
                    amount_sat,
                    fee_sat: Some(fee_sat),
                    private_key_hex,
                    dust_threshold_atoms: None,
                })
            }
            Chain::Kaspa => {
                let (amount_sat, fee_sat) = scaled_amount(1_000);
                SendParams::Kaspa(KaspaSendParams {
                    from,
                    to,
                    amount_sat,
                    fee_sat: Some(fee_sat),
                    private_key_hex,
                    min_fee_sompi: None,
                    dust_threshold_sompi: None,
                })
            }
            Chain::Dash => {
                let (amount_sat, fee_sat) = scaled_amount(2_000);
                SendParams::Utxo(utxo(from, to, private_key_hex, amount_sat, Some(fee_sat)))
            }
            c => {
                return Err(SpectraBridgeError::from(format!(
                    "execute_send: unsupported chain: {c:?}"
                )))
            }
        };
        Ok(ExecuteSendParams::Native(params))
    }
}

/// Direct equivalent of `render_evm_overrides_fragment` (send/ethereum.rs)
/// composed with `read_evm_overrides` (service/helpers.rs) — what those two
/// functions did together by writing a JSON fragment and reading it back,
/// with no JSON in between.
///
/// `access_list` is always empty here, matching what the pair actually
/// produced: `render_evm_overrides_fragment` wrote `access_list_json` as a
/// raw, unquoted JSON array, but `read_evm_overrides` read it back with
/// `.as_str()`, which returns `None` for anything that is not a JSON
/// string — so a caller-supplied access list was silently dropped on every
/// real send. No Swift call site sets it today (`EvmSendOverridesInput` is
/// always constructed with `accessListJson: nil`), so this has never had a
/// visible effect, but it is a pre-existing bug, not something this refactor
/// should silently fix — preserved as-is.
///
/// `gas_buffer_pct` is likewise always `None`: `EvmSendOverridesInput` has no
/// such field, so `read_evm_overrides`'s `params["gas_buffer_pct"]` lookup
/// never found one on this path either.
fn evm_send_overrides(
    input: Option<&crate::send::ethereum::EvmSendOverridesInput>,
) -> crate::send::chains::evm::EvmSendOverrides {
    use crate::send::chains::evm::EvmSendOverrides;
    let Some(o) = input else {
        return EvmSendOverrides::default();
    };
    let max_fee_per_gas_wei = o
        .custom_fees
        .as_ref()
        .map(|cf| (cf.max_fee_per_gas_gwei * 1e9).round() as u64 as u128);
    let max_priority_fee_per_gas_wei = o
        .custom_fees
        .as_ref()
        .map(|cf| (cf.max_priority_fee_per_gas_gwei * 1e9).round() as u64 as u128);
    let calldata = o
        .calldata_hex
        .as_deref()
        .and_then(|s| hex::decode(s.trim_start_matches("0x")).ok());
    EvmSendOverrides {
        nonce: o.nonce.map(|n| n as u64),
        max_fee_per_gas_wei,
        max_priority_fee_per_gas_wei,
        gas_limit: o.gas_limit.map(|n| n as u64),
        calldata,
        access_list: Vec::new(),
        sign_only: o.sign_only.unwrap_or(false),
        gas_buffer_pct: None,
    }
}


#[cfg(test)]
mod token_decimals_come_from_the_contract {
    use crate::registry::Chain;
    use crate::service::WalletService;

    /// Which families core can ask, and which still take the caller's word.
    ///
    /// `build_execute_send_payload` used `req.token_decimals.unwrap_or(6)`, so
    /// a caller that supplied nothing denominated its transfer at six places
    /// whatever the contract said. It reads `decimals()` off the token now,
    /// and the caller's value is only a fallback.
    ///
    /// This asserts the gate, not the network read: a chain the helper has no
    /// client for must answer `None` without attempting a call, which is what
    /// keeps the fallback reachable for Solana, TON, Sui, Aptos and NEAR.
    #[tokio::test]
    async fn a_family_core_cannot_ask_falls_back_to_the_caller() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        for chain in [Chain::Solana, Chain::Ton, Chain::Sui, Chain::Aptos, Chain::Near] {
            assert_eq!(
                service.token_contract_decimals(chain, "whatever").await,
                None,
                "{} has no metadata client, so the caller's value must stand",
                chain.chain_display_name()
            );
        }
    }

    /// The families that are asked are the ones with a metadata call.
    #[test]
    fn the_families_core_asks_are_evm_and_tron() {
        let asks: Vec<_> = Chain::all()
            .filter(|c| !c.is_testnet() && (c.is_evm() || *c == Chain::Tron))
            .collect();
        assert!(asks.len() >= 24, "expected the EVM family plus Tron, got {}", asks.len());
        assert!(asks.contains(&Chain::Tron));
        assert!(asks.contains(&Chain::Ethereum));
    }
}

#[cfg(test)]
mod build_send_params_tests {
    use crate::registry::Chain;
    use crate::send::ethereum::{EvmCustomFeeConfiguration, EvmSendOverridesInput};
    use crate::send::SendExecutionRequest;
    use crate::service::send_params::{ExecuteSendParams, SendParams, SendTokenParams};
    use crate::service::WalletService;

    fn req(chain_id: &str, chain_name: &str) -> SendExecutionRequest {
        SendExecutionRequest {
            chain_id: chain_id.to_string(),
            chain_name: chain_name.to_string(),
            derivation_path: String::new(),
            seed_phrase: None,
            private_key_hex: None,
            from_address: "from".to_string(),
            to_address: "to".to_string(),
            amount: 1.5,
            amount_str: None,
            contract_address: None,
            token_decimals: None,
            fee_rate_svb: None,
            fee_sat: None,
            gas_budget: None,
            fee_amount: None,
            evm_overrides: None,
            monero_priority: None,
            derivation_overrides: None,
        }
    }

    /// Bitcoin: `amount_u64(amount, 1e8)`, and an absent `fee_rate_svb`
    /// defaults to 10 — same as the JSON builder this replaced, which set
    /// `fee_rate_svb` unconditionally rather than leaving it absent for
    /// `BitcoinSendParams`'s own `.unwrap_or(10.0)` to apply.
    #[tokio::test]
    async fn bitcoin_native_scales_by_1e8_sat() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let r = req("bitcoin", "Bitcoin");
        let params = service
            .build_send_params(Chain::Bitcoin, &r, "priv", &None)
            .await
            .expect("params");
        let ExecuteSendParams::Native(SendParams::Bitcoin(p)) = params else {
            panic!("expected Bitcoin params")
        };
        assert_eq!(p.amount_sat, 150_000_000);
        assert_eq!(p.fee_rate_svb, Some(10.0));
        assert_eq!(p.dust_threshold_sats, None);
        assert!(!p.sign_only);
    }

    /// Dogecoin took a *different* scale (a literal `1e8`, not
    /// `chain.native_decimals()`) and a fee computed from a kb-rate, not a
    /// flat `fee_sat` — the one native arm that does not share the
    /// `scaled_amount`/`utxo` shape every other UTXO chain below it uses.
    /// This is the arm most likely to have been merged into that shared
    /// shape by mistake, so it gets its own test.
    #[tokio::test]
    async fn dogecoin_uses_its_own_scale_and_kb_rate_fee() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let mut r = req("dogecoin", "Dogecoin");
        r.amount = 2.0;
        r.fee_rate_svb = Some(1.0); // 1 DOGE/kb
        let params = service
            .build_send_params(Chain::Dogecoin, &r, "priv", &None)
            .await
            .expect("params");
        let ExecuteSendParams::Native(SendParams::Utxo(p)) = params else {
            panic!("expected Utxo params")
        };
        assert_eq!(p.amount_sat, 200_000_000);
        // 1.0 DOGE/kb * 350 / 1000 = 0.35 DOGE -> * 1e8
        assert_eq!(p.fee_sat, Some(35_000_000));
    }

    /// Litecoin shares the `UtxoFixedFeeSendParams` shape with four other
    /// chains, scaled by `chain.native_decimals()` (not a literal `1e8`) —
    /// the thing that distinguishes it from Dogecoin above.
    #[tokio::test]
    async fn litecoin_scales_by_native_decimals_with_its_own_fee_default() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let mut r = req("litecoin", "Litecoin");
        r.amount = 1.0;
        let params = service
            .build_send_params(Chain::Litecoin, &r, "priv", &None)
            .await
            .expect("params");
        let ExecuteSendParams::Native(SendParams::Utxo(p)) = params else {
            panic!("expected Utxo params")
        };
        assert_eq!(p.amount_sat, 100_000_000);
        assert_eq!(p.fee_sat, Some(10_000), "Litecoin's own default, not BCH's 1_000");
    }

    /// EVM native: `value_wei` goes through the string-exact `to_raw(18)`
    /// path, and overrides convert directly from the typed
    /// `EvmSendOverridesInput` — no JSON in between.
    #[tokio::test]
    async fn evm_native_uses_string_exact_wei_and_carries_overrides() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let mut r = req("ethereum", "Ethereum");
        r.amount_str = Some("0.1".to_string());
        r.evm_overrides = Some(EvmSendOverridesInput {
            nonce: Some(9),
            custom_fees: Some(EvmCustomFeeConfiguration {
                max_fee_per_gas_gwei: 50.0,
                max_priority_fee_per_gas_gwei: 3.0,
            }),
            gas_limit: None,
            calldata_hex: None,
            sign_only: None,
            access_list_json: None,
        });
        let params = service
            .build_send_params(Chain::Ethereum, &r, "priv", &None)
            .await
            .expect("params");
        let ExecuteSendParams::Native(SendParams::Evm(p, overrides)) = params else {
            panic!("expected Evm params")
        };
        // 0.1 ETH, 18 decimals, exact string arithmetic — not the f64 path,
        // which is the whole reason `amount_str` exists.
        assert_eq!(p.value_wei, 100_000_000_000_000_000u128);
        assert_eq!(overrides.nonce, Some(9));
        assert_eq!(overrides.max_fee_per_gas_wei, Some(50_000_000_000));
        assert_eq!(overrides.max_priority_fee_per_gas_wei, Some(3_000_000_000));
    }

    /// Polkadot and Bittensor both go through the string-exact `to_raw`
    /// path (10 and 9 decimals respectively) rather than `amount_u64` — the
    /// two native chains whose smallest unit is large enough that f64
    /// scaling would lose precision on an ordinary send amount.
    #[tokio::test]
    async fn polkadot_and_bittensor_use_string_exact_raw_units() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let mut r = req("polkadot", "Polkadot");
        r.amount_str = Some("1.25".to_string());
        let params = service
            .build_send_params(Chain::Polkadot, &r, "priv", &None)
            .await
            .expect("params");
        let ExecuteSendParams::Native(SendParams::Polkadot(p)) = params else {
            panic!("expected Polkadot params")
        };
        assert_eq!(p.planck, 12_500_000_000);

        let mut r = req("bittensor", "Bittensor");
        r.amount_str = Some("1.25".to_string());
        let params = service
            .build_send_params(Chain::Bittensor, &r, "priv", &None)
            .await
            .expect("params");
        let ExecuteSendParams::Native(SendParams::Bittensor(p)) = params else {
            panic!("expected Bittensor params")
        };
        assert_eq!(p.rao, 1_250_000_000);
    }

    /// Monero: no `from` field (the send doesn't name a source address) and
    /// a default priority of 2 when the caller does not set one.
    #[tokio::test]
    async fn monero_defaults_priority_to_2() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let mut r = req("monero", "Monero");
        r.amount = 2.0;
        let params = service
            .build_send_params(Chain::Monero, &r, "priv", &None)
            .await
            .expect("params");
        let ExecuteSendParams::Native(SendParams::Monero(p)) = params else {
            panic!("expected Monero params")
        };
        assert_eq!(p.piconeros, 2_000_000_000_000);
        assert_eq!(p.priority, Some(2));
    }

    /// A non-EVM testnet is a named error, not a panic. Every non-EVM arm
    /// in the match is an exact `Chain::X` pattern (`Chain::Bitcoin`, not
    /// "Bitcoin or any of its testnets"), so `BitcoinTestnet` falls to the
    /// catch-all — the same as it did in the two matches this one replaced.
    /// EVM testnets are different: `c if c.is_evm()` is a family guard, not
    /// an exact match, and it is true for a testnet too — an EVM testnet
    /// send is supported, not an error, on both the old code and this one.
    #[tokio::test]
    async fn a_non_evm_testnet_is_a_named_error() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let r = req("bitcoin-testnet", "Bitcoin Testnet");
        let err = service
            .build_send_params(Chain::BitcoinTestnet, &r, "priv", &None)
            .await
            .expect_err("BitcoinTestnet has no exact arm");
        assert!(format!("{err:?}").contains("unsupported chain"));
    }

    /// Token sends: NEAR takes the string-exact `to_raw` path, Solana takes
    /// the f64-scaled `amount_u64` path — the same split as the native side,
    /// preserved rather than unified. Both chains have no decimals client
    /// (confirmed by `a_family_core_cannot_ask_falls_back_to_the_caller`
    /// above), so `token_decimals` is what resolves it, with no network call.
    #[tokio::test]
    async fn token_sends_preserve_the_two_different_amount_paths() {
        let service = WalletService::new_typed(Vec::new()).expect("service");

        let mut r = req("near", "NEAR");
        r.contract_address = Some("token.near".to_string());
        r.token_decimals = Some(24);
        r.amount_str = Some("0.1".to_string());
        let params = service
            .build_send_params(Chain::Near, &r, "priv", &None)
            .await
            .expect("params");
        let ExecuteSendParams::Token(SendTokenParams::Near(p)) = params else {
            panic!("expected Near token params")
        };
        assert_eq!(p.amount_raw, 100_000_000_000_000_000_000_000u128);

        let mut r = req("solana", "Solana");
        r.contract_address = Some("mint111".to_string());
        r.token_decimals = Some(6);
        r.amount = 1.5;
        let params = service
            .build_send_params(Chain::Solana, &r, "priv", &Some("pub".to_string()))
            .await
            .expect("params");
        let ExecuteSendParams::Token(SendTokenParams::Solana(p)) = params else {
            panic!("expected Solana token params")
        };
        assert_eq!(p.amount_raw, 1_500_000);
        assert_eq!(p.decimals, 6);
    }

    /// A token send with no contract-decimals source at all — no network
    /// answer (there is no network here) and no caller-supplied fallback —
    /// is a named error naming the contract, not a silent default to 6
    /// decimals (the mistake this path was written to stop making).
    #[tokio::test]
    async fn a_token_send_with_no_decimals_source_is_a_named_error() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let mut r = req("near", "NEAR");
        r.contract_address = Some("token.near".to_string());
        r.token_decimals = None;
        let err = service
            .build_send_params(Chain::Near, &r, "priv", &None)
            .await
            .expect_err("no decimals source at all");
        assert!(format!("{err:?}").contains("token.near"));
    }
}
