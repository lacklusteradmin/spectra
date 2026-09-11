use super::*;
#[cfg(test)]
mod token_decimals_come_from_the_contract {
    use crate::registry::Chain;
    use crate::service::WalletService;

    /// Which families core can ask, and which still take the caller's word.
    ///
    /// `build_execute_send_payload` used `req.token_decimals.unwrap_or(6)`, so
    /// a caller that supplied nothing denominated its transfer at six places
    /// whatever the contract said. It reads `decimals()` off the token now,
    /// and only a family without a reader may fall back to caller precision.
    ///
    /// This asserts the gate, not the network read: a chain the helper has no
    /// client for must answer `None` without attempting a call, which is what
    /// keeps the fallback reachable for Solana, TON, Sui and Aptos.
    #[tokio::test]
    async fn a_family_core_cannot_ask_falls_back_to_the_caller() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        for chain in [Chain::Solana, Chain::Ton, Chain::Sui, Chain::Aptos] {
            assert_eq!(
                service
                    .token_contract_decimals(chain, "whatever")
                    .await
                    .unwrap(),
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

    pub(super) fn req(chain_id: &str, _chain_name: &str) -> SendExecutionRequest {
        SendExecutionRequest {
            chain_id: chain_id.to_string(),
            wallet_id: "w".into(),
            password: None,
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
            sign_only: false,
        }
    }

    #[tokio::test]
    async fn invalid_fees_are_refused_before_signing_identity() {
        let service = WalletService::new_typed(vec![]).unwrap();
        for bad in [f64::NAN, f64::INFINITY, -1.0, 0.0, 0.0000000001, f64::MAX] {
            for chain in [Chain::Cardano, Chain::Sui, Chain::Dogecoin] {
                let mut request = req(chain.str_id(), "");
                match chain {
                    Chain::Cardano => request.fee_amount = Some(bad),
                    Chain::Sui => request.gas_budget = Some(bad),
                    _ => request.fee_rate_svb = Some(bad),
                }
                let error = service.execute_send(request).await.unwrap_err();
                assert!(matches!(
                    error,
                    crate::SpectraBridgeError::InvalidInput { .. }
                ));
                assert!(error.to_string().contains("fee"), "{chain:?}: {error}");
            }
        }
    }

    async fn metadata_service(
        chain: Chain,
    ) -> (std::sync::Arc<WalletService>, wiremock::MockServer) {
        use wiremock::{matchers::any, Mock, MockServer, Request, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(any()).respond_with(move |request: &Request| {
            let body: serde_json::Value = request.body_json().unwrap();
            let decimals = format!("{:064x}", 6);
            let symbol = format!("{:0<64}", hex::encode("TOKEN"));
            let response = if let Some(batch) = body.as_array() {
                serde_json::Value::Array(batch.iter().enumerate().map(|(i, item)| serde_json::json!({
                    "jsonrpc":"2.0", "id":item["id"], "result":format!("0x{}", if i == 0 { &decimals } else { &symbol })
                })).collect())
            } else if chain == Chain::Near {
                let metadata = serde_json::json!({"spec":"ft-1.0.0", "name":"Token", "symbol":"TOKEN", "decimals":6});
                serde_json::json!({"jsonrpc":"2.0", "id":body["id"], "result":{"result":serde_json::to_vec(&metadata).unwrap()}})
            } else {
                serde_json::json!({"constant_result": [if body["function_selector"] == "decimals()" { decimals } else { symbol }]})
            };
            ResponseTemplate::new(200).set_body_json(response)
        }).mount(&server).await;
        let service = WalletService::new_typed(vec![crate::service::ChainEndpoints {
            chain_id: chain.str_id().into(),
            endpoints: vec![server.uri()],
            api_key: None,
        }])
        .unwrap();
        (service, server)
    }

    #[tokio::test]
    async fn failed_metadata_never_uses_supplied_precision() {
        let service = WalletService::new_typed(vec![]).unwrap();
        for chain in [Chain::Ethereum, Chain::Tron, Chain::Near] {
            let mut request = req(chain.str_id(), chain.chain_display_name());
            request.contract_address = Some("contract".into());
            request.token_decimals = Some(18);
            assert!(service
                .build_send_params(chain, &request, "from", "priv", &None)
                .await
                .is_err());
        }
    }

    #[tokio::test]
    async fn exact_amounts_reach_native_and_token_signing_params() {
        let service = WalletService::new_typed(vec![]).unwrap();
        let mut r = req("solana", "Solana");
        r.amount_str = "9007199.254740993".into();
        let ExecuteSendParams::Native(SendParams::Solana(p)) = service
            .build_send_params(Chain::Solana, &r, "from", "priv", &None)
            .await
            .unwrap()
        else {
            panic!("wrong native params")
        };
        assert_eq!(p.lamports, 9_007_199_254_740_993);
        r.contract_address = Some("mint".into());
        r.token_decimals = Some(9);
        let ExecuteSendParams::Token(SendTokenParams::Solana(p)) = service
            .build_send_params(Chain::Solana, &r, "from", "priv", &None)
            .await
            .unwrap()
        else {
            panic!("wrong token params")
        };
        assert_eq!(p.amount_raw, 9_007_199_254_740_993);
        r.amount_str = "9007199254.740993".into();
        r.token_decimals = Some(18); // The chain's 6 must win over stale caller data.
        let (service, _server) = metadata_service(Chain::Tron).await;
        let ExecuteSendParams::Token(SendTokenParams::Tron(p)) = service
            .build_send_params(Chain::Tron, &r, "from", "priv", &None)
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
                    .build_send_params(chain, &r, "from", "priv", &None)
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
            .build_send_params(Chain::Bitcoin, &r, "from", "priv", &None)
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
            .build_send_params(Chain::Dogecoin, &r, "from", "priv", &None)
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
    ///
    /// The fee a caller does not supply is the chain's own, and "the chain's
    /// own" means the registry's: the fee estimate the user was shown comes
    /// from `static_fee_units`, so anything else signs for a different amount
    /// than the send screen quoted. This asserted a literal `10_000` against a
    /// registry that says `1_000`.
    #[tokio::test]
    async fn litecoin_scales_by_native_decimals_with_its_own_fee_default() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let mut r = req("litecoin", "Litecoin");
        r.amount_str = "1.0".into();
        let params = service
            .build_send_params(Chain::Litecoin, &r, "from", "priv", &None)
            .await
            .expect("params");
        let ExecuteSendParams::Native(SendParams::Utxo(p)) = params else {
            panic!("expected Utxo params")
        };
        assert_eq!(p.amount_sat, 100_000_000);
        assert_eq!(
            p.fee_sat.map(u128::from),
            Chain::Litecoin.static_fee_units(),
            "the signed fee is the one the estimate quoted"
        );
    }

    /// Every chain on the shared UTXO shape falls back to the fee its own
    /// estimate reports. Litecoin used to sign 10_000 against a 1_000 quote
    /// and Bitcoin Cash 1_000 against 2_000.
    #[tokio::test]
    async fn an_unquoted_fee_falls_back_to_what_the_estimate_reports() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        for (chain, id, name) in [
            (Chain::Litecoin, "litecoin", "Litecoin"),
            (Chain::BitcoinCash, "bitcoin-cash", "Bitcoin Cash"),
            (Chain::BitcoinSV, "bitcoin-sv", "Bitcoin SV"),
            (Chain::BitcoinGold, "bitcoin-gold", "Bitcoin Gold"),
            (Chain::Zcash, "zcash", "Zcash"),
            (Chain::Decred, "decred", "Decred"),
            (Chain::Kaspa, "kaspa", "Kaspa"),
            (Chain::Dash, "dash", "Dash"),
        ] {
            let mut r = req(id, name);
            r.amount_str = "1.0".into();
            r.fee_sat = None;
            let params = service
                .build_send_params(chain, &r, "from", "priv", &None)
                .await
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            let fee = match params {
                ExecuteSendParams::Native(SendParams::Utxo(p)) => p.fee_sat,
                ExecuteSendParams::Native(SendParams::Zcash(p)) => p.fee_sat,
                ExecuteSendParams::Native(SendParams::Decred(p)) => p.fee_sat,
                ExecuteSendParams::Native(SendParams::Kaspa(p)) => p.fee_sat,
                other => panic!("{name}: unexpected shape {other:?}"),
            };
            assert_eq!(
                fee.map(u128::from),
                chain.static_fee_units(),
                "{name} signs a fee its own estimate does not report"
            );
        }
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
            .build_send_params(Chain::Ethereum, &r, "from", "priv", &None)
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
                        .build_send_params(Chain::Ethereum, &request, "from", "priv", &None)
                        .await,
                    Err(crate::SpectraBridgeError::InvalidInput { .. })
                ),
                "{max}/{priority} must fail before signing"
            );
        }
    }

    #[tokio::test]
    async fn evm_overrides_reach_native_and_token_builders() {
        let (service, _server) = metadata_service(Chain::Ethereum).await;
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
                .build_send_params(Chain::Ethereum, &request, "from", "priv", &None)
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
            .build_send_params(Chain::Polkadot, &r, "from", "priv", &None)
            .await
            .expect("params");
        let ExecuteSendParams::Native(SendParams::Polkadot(p)) = params else {
            panic!("expected Polkadot params")
        };
        assert_eq!(p.planck, 12_500_000_000);

        let mut r = req("bittensor", "Bittensor");
        r.amount_str = "1.25".to_string();
        let params = service
            .build_send_params(Chain::Bittensor, &r, "from", "priv", &None)
            .await
            .expect("params");
        let ExecuteSendParams::Native(SendParams::Bittensor(p)) = params else {
            panic!("expected Bittensor params")
        };
        assert_eq!(p.rao, 1_250_000_000);
    }

    /// Monero binds wallet-rpc to the resolved sender and defaults to priority 2.
    #[tokio::test]
    async fn monero_defaults_priority_to_2() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let mut r = req("monero", "Monero");
        r.amount_str = "2.0".into();
        let params = service
            .build_send_params(Chain::Monero, &r, "from", "priv", &None)
            .await
            .expect("params");
        let ExecuteSendParams::Native(SendParams::Monero(p)) = params else {
            panic!("expected Monero params")
        };
        assert_eq!(p.from, "from");
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
            .build_send_params(Chain::BitcoinTestnet, &r, "from", "priv", &None)
            .await
            .expect_err("BitcoinTestnet has no exact arm");
        assert!(format!("{err:?}").contains("unsupported chain"));
    }

    /// Both token families use exact decimal input.
    #[tokio::test]
    async fn token_sends_use_exact_integer_amounts() {
        let (service, _server) = metadata_service(Chain::Near).await;

        let mut r = req("near", "NEAR");
        r.contract_address = Some("token.near".to_string());
        r.token_decimals = Some(24);
        r.amount_str = "0.1".to_string();
        let params = service
            .build_send_params(Chain::Near, &r, "from", "priv", &None)
            .await
            .expect("params");
        let ExecuteSendParams::Token(SendTokenParams::Near(p)) = params else {
            panic!("expected Near token params")
        };
        assert_eq!(p.amount_raw, 100_000); // Contract says 6, caller says 24.

        let mut r = req("solana", "Solana");
        r.contract_address = Some("mint111".to_string());
        r.token_decimals = Some(6);
        r.amount_str = "1.5".into();
        let params = service
            .build_send_params(Chain::Solana, &r, "from", "priv", &Some("pub".to_string()))
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
        let mut r = req("solana", "Solana");
        r.contract_address = Some("mint".to_string());
        r.token_decimals = None;
        let err = service
            .build_send_params(Chain::Solana, &r, "from", "priv", &None)
            .await
            .expect_err("no decimals source at all");
        assert!(format!("{err:?}").contains("mint"));
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
                wallet_id: "w".into(),
                password: None,
                to_address: if chain == Chain::Ton {
                    format!("0:{}", "22".repeat(32))
                } else {
                    "to".to_string()
                },
                amount_str: "1.5".into(),
                contract_address: None,
                token_decimals: None,
                fee_rate_svb: None,
                fee_sat: None,
                gas_budget: None,
                fee_amount: None,
                evm_overrides: None,
                monero_priority: None,
                sign_only: false,
            };
            checked += 1;
            if let Err(e) = service
                .build_send_params(chain, &request, "from", "priv", &None)
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

#[cfg(test)]
mod sign_only_tests {
    use super::build_send_params_tests::req;

    /// "Sign and stop" is one question however it was asked.
    ///
    /// It had two routes and four readers, and they disagreed: the refusal
    /// read both routes, the result field read only `sign_only`, and the
    /// Bitcoin builder read only `sign_only` too. So a caller asking through
    /// the EVM overrides — the route that existed first, and the one the
    /// field's own doc comment still points at — got a signed transaction
    /// back with `signed_payload: None`.
    #[test]
    fn either_route_asks_the_same_thing() {
        let plain = req("ethereum", "Ethereum");
        assert!(!plain.wants_sign_only(), "a send is not a dry run");

        let mut by_field = req("ethereum", "Ethereum");
        by_field.sign_only = true;
        assert!(by_field.wants_sign_only());

        let mut by_overrides = req("ethereum", "Ethereum");
        by_overrides.evm_overrides = Some(crate::send::ethereum::EvmSendOverridesInput {
            sign_only: Some(true),
            ..Default::default()
        });
        assert!(
            by_overrides.wants_sign_only(),
            "the older route asks for a dry run just as much"
        );

        // Overrides that say nothing about it do not unsay the field.
        let mut both = req("ethereum", "Ethereum");
        both.sign_only = true;
        both.evm_overrides = Some(crate::send::ethereum::EvmSendOverridesInput {
            sign_only: None,
            ..Default::default()
        });
        assert!(both.wants_sign_only());
    }
}

#[cfg(test)]
mod send_chain_tests {
    use super::send_chain_for;
    use crate::registry::Chain;
    use crate::store::state::{CoreAppState, WalletSummary};

    fn wallet(id: &str, chain: Chain, network_mode: Option<&str>) -> WalletSummary {
        WalletSummary {
            id: id.to_string(),
            name: id.to_string(),
            is_watch_only: false,
            chain_name: chain.chain_display_name().to_string(),
            include_in_portfolio_total: true,
            network_mode: network_mode.map(str::to_string),
            xpub: None,
            derivation_preset: "standard".to_string(),
            derivation_path: None,
            derivation_overrides: Default::default(),
            holdings: Vec::new(),
            addresses: Vec::new(),
        }
    }

    /// A send is signed for the network the wallet is on.
    ///
    /// It used to be signed for the family's mainnet whatever network was
    /// selected: with the app on Sepolia, a send still signed chain id 1 and
    /// read mainnet endpoints, so what the user believed was a testnet
    /// transaction was a valid mainnet one. `spectra send broadcast
    /// --sign-only` prints the signed chain id, which is how it was found.
    #[test]
    fn a_send_follows_the_network_the_wallet_is_on() {
        let mut state = CoreAppState::default();
        state.wallets = vec![wallet("w1", Chain::Ethereum, None)];
        assert_eq!(
            send_chain_for(&state, "w1", Chain::Ethereum),
            Chain::Ethereum
        );

        // The app's selection moves the family.
        state.settings.network_chain_by_family.insert(
            Chain::Ethereum.str_id().to_string(),
            Chain::EthereumSepolia.str_id().to_string(),
        );
        assert_eq!(
            send_chain_for(&state, "w1", Chain::Ethereum),
            Chain::EthereumSepolia
        );
        // Ids cross the boundary in whichever case a front end holds them.
        assert_eq!(
            send_chain_for(&state, "W1", Chain::Ethereum),
            Chain::EthereumSepolia
        );

        // A wallet's own record wins over the app's selection.
        state.wallets[0].network_mode = Some(Chain::Ethereum.str_id().to_string());
        assert_eq!(
            send_chain_for(&state, "w1", Chain::Ethereum),
            Chain::Ethereum
        );

        // A selection for another family does not move this one, and an
        // unknown wallet keeps the requested chain rather than guessing.
        state.wallets[0].network_mode = None;
        assert_eq!(
            send_chain_for(&state, "w1", Chain::Bitcoin),
            Chain::Bitcoin,
            "the Ethereum selection must not move a Bitcoin send"
        );
        assert_eq!(
            send_chain_for(&state, "nobody", Chain::Ethereum),
            Chain::Ethereum
        );
    }
}
