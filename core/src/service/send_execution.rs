use super::*;
use zeroize::Zeroizing;

/// Validate before keys or network reads; token precision is checked again
/// after metadata resolution. Native precision belongs to the registry.
fn validate_execution_amount(
    chain: Chain,
    request: &crate::send::SendExecutionRequest,
) -> Result<(), SpectraBridgeError> {
    let decimals = if request.contract_address.is_none() {
        u32::from(chain.native_decimals())
    } else {
        request
            .amount_str
            .trim()
            .split_once('.')
            .map_or(0, |(_, f)| f.len() as u32)
    };
    let raw = crate::send::amount_input::parse_raw_amount(&request.amount_str, decimals)?;
    if raw == 0 && (!chain.is_evm() || request.contract_address.is_some()) {
        return Err(SpectraBridgeError::InvalidInput {
            message: "amount must be positive".into(),
        });
    }
    Ok(())
}

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
            let chain = Chain::from_str_id(&request.chain_id).ok_or_else(|| {
                SpectraBridgeError::InvalidInput {
                    message: format!("execute_send: unsupported chain_id: {}", request.chain_id),
                }
            })?;
            // Refuse malformed overrides before reading or deriving signing material.
            if let Some(input) = &request.evm_overrides {
                input.resolve(chain)?;
            }
            validate_execution_amount(chain, &request)?;
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

    /// Resolve an exact decimal amount into the protocol's integer units.
    async fn build_send_params(
        &self,
        chain: Chain,
        req: &crate::send::SendExecutionRequest,
        priv_hex: &str,
        pub_hex: &Option<String>,
    ) -> Result<crate::service::send_params::ExecuteSendParams, SpectraBridgeError> {
        use crate::send::amount_input::parse_raw_amount;
        use crate::send::payload::amount_u64;
        use crate::service::send_params::*;

        let overrides = req
            .evm_overrides
            .as_ref()
            .map(|input| input.resolve(chain))
            .transpose()?
            .unwrap_or_default();

        let from = req.from_address.clone();
        let to = req.to_address.clone();
        let private_key_hex = priv_hex.to_string();
        let public_key_hex = pub_hex.clone();

        let raw_u128 = |dec: u32| parse_raw_amount(&req.amount_str, dec);
        let raw_u64 = |dec: u32| -> Result<u64, SpectraBridgeError> {
            u64::try_from(raw_u128(dec)?).map_err(|_| SpectraBridgeError::InvalidInput {
                message: "amount exceeds this protocol's u64 range".into(),
            })
        };
        validate_execution_amount(chain, req)?;

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
                        amount_raw: raw_u128(decimals)?,
                        private_key_hex,
                    },
                    overrides,
                ),
                Chain::Tron => SendTokenParams::Tron(TronTokenSendParams {
                    from,
                    contract: contract.clone(),
                    to,
                    amount_raw: raw_u128(decimals)?,
                    fee_limit_sun: None,
                    private_key_hex,
                }),
                Chain::Solana => SendTokenParams::Solana(SolanaTokenSendParams {
                    from_pubkey_hex: public_key_hex.unwrap_or_default(),
                    to,
                    mint: contract.clone(),
                    amount_raw: raw_u64(decimals)?,
                    decimals: u8::try_from(decimals)
                        .map_err(|_| SpectraBridgeError::from("token decimals out of range"))?,
                    private_key_hex,
                }),
                Chain::Near => SendTokenParams::Near(NearTokenSendParams {
                    from,
                    contract: contract.clone(),
                    to,
                    amount_raw: raw_u128(decimals)?,
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
        let utxo = |from: String,
                    to: String,
                    private_key_hex: String,
                    amount_sat: u64,
                    fee_sat: Option<u64>| {
            UtxoFixedFeeSendParams {
                from,
                to,
                amount_sat,
                fee_sat,
                private_key_hex,
                dust_threshold_sats: None,
            }
        };
        let scaled_amount = |fee_default: u64| -> Result<(u64, u64), SpectraBridgeError> {
            Ok((
                raw_u64(u32::from(chain.native_decimals()))?,
                req.fee_sat.unwrap_or(fee_default),
            ))
        };

        let params = match chain {
            Chain::Bitcoin => SendParams::Bitcoin(BitcoinNativeSendParams {
                from,
                to,
                amount_sat: raw_u64(8)?,
                fee_rate_svb: Some(req.fee_rate_svb.unwrap_or(10.0)),
                private_key_hex,
                dust_threshold_sats: None,
                sign_only: false,
            }),
            c if c.is_evm() => SendParams::Evm(
                EvmNativeSendParams {
                    from,
                    to,
                    value_wei: raw_u128(18)?,
                    private_key_hex,
                },
                overrides,
            ),
            Chain::Solana => SendParams::Solana(SolanaNativeSendParams {
                from_pubkey_hex: public_key_hex.unwrap_or_default(),
                to,
                lamports: raw_u64(9)?,
                private_key_hex,
            }),
            Chain::Dogecoin => {
                // Dogecoin estimates a 350-byte fee from DOGE/kB.
                let fee_rate_doge_per_kb = req.fee_rate_svb.unwrap_or(0.01);
                SendParams::Utxo(UtxoFixedFeeSendParams {
                    from,
                    to,
                    amount_sat: raw_u64(8)?,
                    fee_sat: Some(amount_u64(fee_rate_doge_per_kb * 350.0 / 1000.0, 1e8)),
                    private_key_hex,
                    dust_threshold_sats: None,
                })
            }
            Chain::Xrp => SendParams::Xrp(XrpSendParams {
                from,
                to,
                drops: raw_u64(6)?,
                private_key_hex,
                public_key_hex,
            }),
            Chain::Litecoin => {
                let (amount_sat, fee_sat) = scaled_amount(10_000)?;
                SendParams::Utxo(utxo(from, to, private_key_hex, amount_sat, Some(fee_sat)))
            }
            Chain::BitcoinCash => {
                let (amount_sat, fee_sat) = scaled_amount(1_000)?;
                SendParams::Utxo(utxo(from, to, private_key_hex, amount_sat, Some(fee_sat)))
            }
            Chain::Tron => SendParams::Tron(TronNativeSendParams {
                from,
                to,
                amount_sun: raw_u64(6)?,
                private_key_hex,
            }),
            Chain::Stellar => SendParams::Stellar(StellarSendParams {
                from,
                to,
                stroops: i64::try_from(raw_u128(7)?).map_err(|_| {
                    SpectraBridgeError::from("amount exceeds Stellar integer range")
                })?,
                private_key_hex,
                public_key_hex,
                network_passphrase: None,
            }),
            Chain::Cardano => SendParams::Cardano(CardanoSendParams {
                from,
                to,
                amount_lovelace: raw_u64(6)?,
                fee_lovelace: Some(amount_u64(req.fee_amount.unwrap_or(0.17), 1e6)),
                private_key_hex,
                public_key_hex: public_key_hex.unwrap_or_default(),
                ttl_slots: None,
                min_change_lovelace: None,
            }),
            Chain::Polkadot => SendParams::Polkadot(PolkadotSendParams {
                from,
                to,
                planck: raw_u128(10)?,
                private_key_hex,
                public_key_hex: public_key_hex.unwrap_or_default(),
                era: None,
                tip: None,
            }),
            Chain::Bittensor => SendParams::Bittensor(BittensorSendParams {
                from,
                to,
                rao: raw_u128(9)?,
                private_key_hex,
                public_key_hex: public_key_hex.unwrap_or_default(),
            }),
            Chain::Sui => SendParams::Sui(SuiSendParams {
                from,
                to,
                mist: raw_u64(9)?,
                gas_budget: Some(amount_u64(req.gas_budget.unwrap_or(0.01), 1e9)),
                private_key_hex,
                public_key_hex: public_key_hex.unwrap_or_default(),
            }),
            Chain::Aptos => SendParams::Aptos(AptosSendParams {
                from,
                to,
                octas: raw_u64(8)?,
                private_key_hex,
                public_key_hex: public_key_hex.unwrap_or_default(),
            }),
            Chain::Ton => SendParams::Ton(TonSendParams {
                from,
                to,
                nanotons: raw_u64(9)?,
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
                yocto_near: raw_u128(24)?,
                private_key_hex,
                public_key_hex: public_key_hex.unwrap_or_default(),
            }),
            Chain::Icp => SendParams::Icp(IcpSendParams {
                from,
                to,
                e8s: raw_u64(8)?,
                private_key_hex,
                public_key_hex,
            }),
            Chain::Monero => SendParams::Monero(MoneroSendParams {
                to,
                piconeros: raw_u64(12)?,
                priority: Some(u64::from(req.monero_priority.unwrap_or(2))),
            }),
            Chain::BitcoinSV => {
                let (amount_sat, fee_sat) = scaled_amount(1_000)?;
                SendParams::Utxo(utxo(from, to, private_key_hex, amount_sat, Some(fee_sat)))
            }
            Chain::Zcash => {
                let (amount_sat, fee_sat) = scaled_amount(1_000)?;
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
                let (amount_sat, fee_sat) = scaled_amount(1_000)?;
                SendParams::Utxo(utxo(from, to, private_key_hex, amount_sat, Some(fee_sat)))
            }
            Chain::Decred => {
                let (amount_sat, fee_sat) = scaled_amount(2_000)?;
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
                let (amount_sat, fee_sat) = scaled_amount(1_000)?;
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
                let (amount_sat, fee_sat) = scaled_amount(2_000)?;
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
        for chain in [
            Chain::Solana,
            Chain::Ton,
            Chain::Sui,
            Chain::Aptos,
            Chain::Near,
        ] {
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
        assert!(
            asks.len() >= 24,
            "expected the EVM family plus Tron, got {}",
            asks.len()
        );
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
            amount_str: "1.5".into(),
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

    #[tokio::test]
    async fn exact_amounts_reach_native_and_token_signing_params() {
        let service = WalletService::new_typed(vec![]).unwrap();
        let mut r = req("solana", "Solana");
        r.amount_str = "9007199.254740993".into();
        let ExecuteSendParams::Native(SendParams::Solana(p)) = service
            .build_send_params(Chain::Solana, &r, "priv", &None)
            .await
            .unwrap()
        else {
            panic!("wrong native params")
        };
        assert_eq!(p.lamports, 9_007_199_254_740_993);
        r.contract_address = Some("mint".into());
        r.token_decimals = Some(9);
        let ExecuteSendParams::Token(SendTokenParams::Solana(p)) = service
            .build_send_params(Chain::Solana, &r, "priv", &None)
            .await
            .unwrap()
        else {
            panic!("wrong token params")
        };
        assert_eq!(p.amount_raw, 9_007_199_254_740_993);
        r.amount_str = "9007199254.740993".into();
        r.token_decimals = Some(6);
        let ExecuteSendParams::Token(SendTokenParams::Tron(p)) = service
            .build_send_params(Chain::Tron, &r, "priv", &None)
            .await
            .unwrap()
        else {
            panic!("wrong Tron params")
        };
        assert_eq!(p.amount_raw, 9_007_199_254_740_993);
    }

    #[tokio::test]
    async fn amount_precision_and_integer_overflow_are_refused() {
        let service = WalletService::new_typed(vec![]).unwrap();
        for (chain, amount) in [
            (Chain::Bitcoin, "0.000000001"),
            (Chain::Solana, "18446744073.709551616"),
            (Chain::Stellar, "922337203685.4775808"),
            (Chain::Ethereum, "1.0000000000000000001"),
            (Chain::Bitcoin, "0"),
        ] {
            let mut r = req(chain.str_id(), chain.chain_display_name());
            r.amount_str = amount.into();
            assert!(
                service
                    .build_send_params(chain, &r, "priv", &None)
                    .await
                    .is_err(),
                "{chain:?}/{amount}"
            );
        }
        for invalid in ["-1", "NaN", "inf", "1e3", "1.é", ""] {
            let mut r = req("bitcoin", "Bitcoin");
            r.amount_str = invalid.into();
            assert!(
                matches!(
                    service.execute_send(r).await,
                    Err(crate::SpectraBridgeError::InvalidInput { .. })
                ),
                "{invalid}"
            );
        }
    }

    /// Bitcoin: `raw_u64(8)?`, and an absent `fee_rate_svb`
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
        r.amount_str = "2.0".into();
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
        r.amount_str = "1.0".into();
        let params = service
            .build_send_params(Chain::Litecoin, &r, "priv", &None)
            .await
            .expect("params");
        let ExecuteSendParams::Native(SendParams::Utxo(p)) = params else {
            panic!("expected Utxo params")
        };
        assert_eq!(p.amount_sat, 100_000_000);
        assert_eq!(
            p.fee_sat,
            Some(10_000),
            "Litecoin's own default, not BCH's 1_000"
        );
    }

    /// EVM native: `value_wei` goes through the string-exact `to_raw(18)`
    /// path, and overrides convert directly from the typed
    /// `EvmSendOverridesInput` — no JSON in between.
    #[tokio::test]
    async fn evm_native_uses_string_exact_wei_and_carries_overrides() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let mut r = req("ethereum", "Ethereum");
        r.amount_str = "0.1".to_string();
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

    #[tokio::test]
    async fn evm_builder_refuses_invalid_typed_fees_without_the_ui_parser() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        for (max, priority) in [
            (f64::INFINITY, 1.0),
            (30.0, f64::NAN),
            (1e100, 1.0),
            (1.0, 2.0),
            (1e-10, 1e-10),
        ] {
            let mut request = req("ethereum", "Ethereum");
            request.evm_overrides = Some(EvmSendOverridesInput {
                custom_fees: Some(EvmCustomFeeConfiguration {
                    max_fee_per_gas_gwei: max,
                    max_priority_fee_per_gas_gwei: priority,
                }),
                ..Default::default()
            });
            assert!(
                matches!(
                    service
                        .build_send_params(Chain::Ethereum, &request, "priv", &None)
                        .await,
                    Err(crate::SpectraBridgeError::InvalidInput { .. })
                ),
                "{max}/{priority} must fail before signing"
            );
        }
    }

    #[tokio::test]
    async fn evm_overrides_reach_native_and_token_builders() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        for token in [false, true] {
            let mut request = req("ethereum", "Ethereum");
            if token {
                request.contract_address =
                    Some("0x1111111111111111111111111111111111111111".into());
                request.token_decimals = Some(6);
            }
            request.evm_overrides = Some(EvmSendOverridesInput {
                nonce: Some(7),
                gas_limit: Some(50_000),
                calldata_hex: Some("0x0102ff".into()),
                access_list_json: Some(format!(
                    r#"[{{"address":"0x{}","storageKeys":["0x{}"]}}]"#,
                    "11".repeat(20),
                    "22".repeat(32)
                )),
                sign_only: Some(true),
                ..Default::default()
            });
            let params = service
                .build_send_params(Chain::Ethereum, &request, "priv", &None)
                .await
                .unwrap();
            let overrides = match params {
                ExecuteSendParams::Native(SendParams::Evm(_, overrides)) if !token => overrides,
                ExecuteSendParams::Token(SendTokenParams::Evm(_, overrides)) if token => overrides,
                other => panic!("wrong send shape: {other:?}"),
            };
            assert_eq!(overrides.nonce, Some(7));
            assert_eq!(overrides.gas_limit, Some(50_000));
            assert_eq!(overrides.calldata, Some(vec![1, 2, 255]));
            assert_eq!(overrides.access_list[0].address, [0x11; 20]);
            assert_eq!(overrides.access_list[0].storage_keys, vec![[0x22; 32]]);
            assert!(overrides.sign_only);
        }
    }

    #[tokio::test]
    async fn malformed_evm_overrides_are_refused_before_keys_or_network() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let mut request = req("ethereum", "Ethereum");
        request.evm_overrides = Some(EvmSendOverridesInput {
            nonce: Some(-1),
            ..Default::default()
        });
        assert!(matches!(service.execute_send(request).await,
            Err(crate::SpectraBridgeError::InvalidInput { message }) if message.contains("nonce")));
    }

    /// Polkadot and Bittensor both go through the string-exact `to_raw`
    /// path (10 and 9 decimals respectively) rather than `amount_u64` — the
    /// two native chains whose smallest unit is large enough that f64
    /// scaling would lose precision on an ordinary send amount.
    #[tokio::test]
    async fn polkadot_and_bittensor_use_string_exact_raw_units() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let mut r = req("polkadot", "Polkadot");
        r.amount_str = "1.25".to_string();
        let params = service
            .build_send_params(Chain::Polkadot, &r, "priv", &None)
            .await
            .expect("params");
        let ExecuteSendParams::Native(SendParams::Polkadot(p)) = params else {
            panic!("expected Polkadot params")
        };
        assert_eq!(p.planck, 12_500_000_000);

        let mut r = req("bittensor", "Bittensor");
        r.amount_str = "1.25".to_string();
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
        r.amount_str = "2.0".into();
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

    /// Both token families use exact decimal input.
    #[tokio::test]
    async fn token_sends_use_exact_integer_amounts() {
        let service = WalletService::new_typed(Vec::new()).expect("service");

        let mut r = req("near", "NEAR");
        r.contract_address = Some("token.near".to_string());
        r.token_decimals = Some(24);
        r.amount_str = "0.1".to_string();
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
        r.amount_str = "1.5".into();
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

#[cfg(test)]
mod the_router_and_the_builder_agree {
    use crate::registry::Chain;
    use crate::send::{route_send_asset, SendAssetRoutingInput};
    use crate::service::WalletService;

    /// A chain the router says can send must be one the builder can build for.
    ///
    /// These are two tables consulted in sequence: `route_send_asset` decides
    /// in the preflight whether a send is offered, and `build_send_params`
    /// turns the request into a signable shape. Nothing made them agree. A
    /// chain in the first and missing from the second passes every check the
    /// UI runs — destination valid, amount affordable, secret present,
    /// biometrics cleared — and fails at the last step, after the user has
    /// authorised it.
    ///
    /// The send path has no end-to-end coverage: the CLI acceptance run has no
    /// network and the iOS suite does not broadcast. This is the assertion that
    /// can be made offline, and it is the one that catches a chain added to one
    /// table and not the other.
    #[tokio::test]
    async fn every_routable_mainnet_builds_send_params() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let mut unbuildable = Vec::new();
        let mut checked = 0usize;

        for chain in Chain::mainnets() {
            let route = route_send_asset(&SendAssetRoutingInput {
                chain_name: chain.chain_display_name().to_string(),
                symbol: chain.coin_symbol().to_string(),
                is_evm_chain: chain.is_evm(),
                supports_solana_send_coin: chain == Chain::Solana,
                supports_near_token_send: false,
            });
            if route.submit_kind.is_none() {
                continue;
            }
            let request = crate::send::SendExecutionRequest {
                chain_id: chain.str_id().to_string(),
                chain_name: chain.chain_display_name().to_string(),
                derivation_path: String::new(),
                seed_phrase: None,
                private_key_hex: None,
                from_address: "from".to_string(),
                to_address: "to".to_string(),
                amount_str: "1.5".into(),
                contract_address: None,
                token_decimals: None,
                fee_rate_svb: None,
                fee_sat: None,
                gas_budget: None,
                fee_amount: None,
                evm_overrides: None,
                monero_priority: None,
                derivation_overrides: None,
            };
            checked += 1;
            if let Err(e) = service
                .build_send_params(chain, &request, "priv", &None)
                .await
            {
                unbuildable.push(format!("{} ({e})", chain.chain_display_name()));
            }
        }

        assert!(
            unbuildable.is_empty(),
            "the router offers these sends and the builder cannot build them: {unbuildable:#?}"
        );
        // Not a vacuous pass: every mainnet is routable, so every mainnet was
        // built. If the router stops offering one, the assertion above would
        // still hold while checking nothing.
        assert_eq!(
            checked,
            Chain::mainnets().count(),
            "some mainnet is no longer routable and was skipped here"
        );
    }
}
