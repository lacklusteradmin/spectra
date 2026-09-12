use super::*;
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
            (Chain::Solana, "5000", "0.000005"),     // 9 decimals
            (Chain::Cardano, "170000", "0.17"),      // 6 decimals
            (Chain::Sui, "1000", "0.000001"),        // 9 decimals
            (Chain::Icp, "10000", "0.0001"),         // 8 decimals
            (Chain::Polkadot, "160000000", "0.016"), // 10 decimals
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

/// A wallet holding one asset, so a probe named by wallet and holding has
/// something to look up. `token` is `None` for the chain's own asset, and
/// otherwise the contract and precision of the row that vouches for it —
/// which goes into the token preferences beside the holding, because an
/// unvouched token is a refusal rather than a probe.
#[cfg(test)]
async fn seed_probe_holding(
    service: &WalletService,
    chain: Chain,
    symbol: &str,
    token: Option<(&str, u32)>,
) -> String {
    use crate::store::state::WalletSummary;
    use crate::store::wallet_domain::{
        AssetHolding, CoreTokenHostingChain, CoreTokenPreferenceCategory, CoreTokenPreferenceEntry,
    };
    let chain_name = chain.chain_display_name().to_string();
    let mut state = service.wallet_state.write().await;
    let mut wallet = WalletSummary::single_address(
        "probe-wallet",
        "Probe",
        chain_name.clone(),
        "sender",
        None,
        false,
    );
    wallet.holdings = vec![AssetHolding {
        name: symbol.to_string(),
        symbol: symbol.to_string(),
        coin_gecko_id: String::new(),
        chain_name: chain_name.clone(),
        token_standard: String::new(),
        contract_address: token.map(|(contract, _)| contract.to_string()),
        amount: 1.0,
        price_usd: 0.0,
    }];
    state.wallets.push(wallet);
    if let Some((contract, decimals)) = token {
        let hosting =
            CoreTokenHostingChain::from_chain_name(&chain_name).expect("the chain hosts tokens");
        state.token_preferences.push(CoreTokenPreferenceEntry {
            category: CoreTokenPreferenceCategory::Stablecoin,
            is_built_in: false,
            is_enabled: true,
            token: crate::tokens::TokenEntry {
                chain: hosting.chain_name().to_string(),
                name: symbol.to_string(),
                symbol: symbol.to_string(),
                token_standard: String::new(),
                contract: contract.to_string(),
                coingecko_id: String::new(),
                decimals,
                tags: Vec::new(),
                color: String::new(),
                asset_name: String::new(),
                enabled: true,
            },
        });
    }
    format!("{chain_name}|{symbol}")
}

#[cfg(test)]
mod a_destination_probe_refuses_before_it_guesses {
    use crate::service::WalletService;

    /// An asset core cannot identify is an error, not a verdict.
    ///
    /// The shape this replaces had a `default` arm that answered
    /// `(nil, nil)` — no warning — for anything it did not recognise, so a
    /// chain the front end could not resolve looked exactly like a
    /// destination that had passed the check. Silence is the wrong answer to
    /// "is this address safe to send to"; the caller has to know the question
    /// was not asked. The composer that named the token itself had the same
    /// hole from the other side: it cleared the probe and showed nothing when
    /// it could not identify one. No network: both refusals precede the reads.
    #[tokio::test]
    async fn an_unfindable_holding_is_an_error_and_not_a_clean_verdict() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let missing = service
            .send_destination_risk(
                "no-such-wallet".into(),
                "Bitcoin|BTC".into(),
                "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq".into(),
            )
            .await;
        assert!(
            missing.is_err(),
            "a holding core does not have must not answer with a verdict"
        );

        // The wallet is there and holds the asset, but nothing vouches for the
        // contract — so there is no balance to ask about, and saying so is the
        // answer.
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let key = super::seed_probe_holding(
            &service,
            crate::registry::Chain::Ethereum,
            "TOK",
            Some((&format!("0x{}", "22".repeat(20)), 6)),
        )
        .await;
        service.wallet_state.write().await.token_preferences.clear();
        let untracked = service
            .send_destination_risk("probe-wallet".into(), key, format!("0x{}", "33".repeat(20)))
            .await;
        let refusal = untracked
            .expect_err("an unvouched token has no balance to report")
            .to_string();
        assert!(
            refusal.contains("TOK") && refusal.contains("tracks"),
            "the refusal names the asset and why: {refusal}"
        );
    }
}

/// What a destination probe asks about, from the holding being sent.
///
/// `None` is the chain's own asset. A token the user does not track has no
/// descriptor and is a refusal rather than a fallback: the probe would
/// otherwise read the *chain's* balance and report it as the token's, which is
/// what three of Swift's four arms did, or read nothing and show no verdict,
/// which is what the EVM arm did. Neither says "we could not check".

#[cfg(test)]
mod destination_resolution_tests {
    use crate::service::WalletService;

    /// A valid address comes back in the chain's own form, and says no name
    /// was involved.
    ///
    /// The EVM branch this replaces lowercased through `normalizeEVMAddress`,
    /// which is the same answer here — but it was Swift's spelling of a
    /// registry rule, applied on EVM chains only. Offline: no branch that
    /// touches the network is reached.
    #[tokio::test]
    async fn a_valid_address_is_normalized_and_not_a_name() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let resolved = service
            .resolve_send_destination(
                "ethereum".into(),
                "  0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA  ".into(),
            )
            .await
            .expect("a valid EVM address resolves to itself");
        assert_eq!(
            resolved.address,
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert!(!resolved.used_ens);
    }

    /// Nothing typed is refused rather than resolved to the empty string.
    #[tokio::test]
    async fn an_empty_destination_is_refused() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let err = service
            .resolve_send_destination("bitcoin".into(), "   ".into())
            .await
            .expect_err("an empty destination is not an address");
        assert!(format!("{err:?}").contains("Bitcoin"));
    }

    /// A `.eth` name on a chain that does not run the registry is refused
    /// before any lookup, which is the stricter of the two readings and what
    /// `Chain::resolves_ens_names` now states once.
    ///
    /// Offline by construction: the refusal happens before the resolver is
    /// called, so a network-less test proves the branch and not the timeout.
    #[tokio::test]
    async fn a_name_is_not_looked_up_off_the_chain_that_registers_it() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        for chain_id in ["arbitrum", "base", "polygon", "bitcoin"] {
            let err = service
                .resolve_send_destination(chain_id.into(), "vitalik.eth".into())
                .await
                .expect_err("a name off Ethereum is not a destination");
            assert!(
                format!("{err:?}").contains("valid"),
                "{chain_id} should refuse the name, got {err:?}"
            );
        }
    }

    /// An unknown chain is an error, not a destination.
    #[tokio::test]
    async fn an_unknown_chain_resolves_nothing() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        assert!(service
            .resolve_send_destination("not-a-chain".into(), "0xabc".into())
            .await
            .is_err());
    }
}

#[cfg(test)]
mod failed_reads {
    use super::*;
    use wiremock::{matchers::any, Mock, MockServer, Request, ResponseTemplate};

    /// The send path is the caller that must not read an absent balance as a
    /// zero. `fetch_token_balances` leaves out a token it could not read so a
    /// refresh keeps the last known amount on screen; here that same absence
    /// has to be a refusal, because everything a send decides — whether this
    /// is the whole balance, whether the amount fits — is computed from it.
    #[tokio::test]
    async fn an_unreadable_token_balance_is_not_an_empty_wallet() {
        let server = MockServer::start().await;
        // Only the token read fails. The balance and the history run
        // concurrently and `try_join!` reports whichever errors first, so a
        // mock that failed both would be asserting on which future lost a
        // race — and did, intermittently, under a loaded test run.
        Mock::given(any())
            .respond_with(|req: &Request| {
                let body: serde_json::Value = req.body_json().unwrap();
                let is_token_read = body["method"] == "eth_call";
                ResponseTemplate::new(200).set_body_json(if is_token_read {
                    json!({
                        "jsonrpc": "2.0", "id": body["id"],
                        "error": {"code": -32000, "message": "no code at address"},
                    })
                } else {
                    json!({"jsonrpc": "2.0", "id": body["id"], "result": "0x1"})
                })
            })
            .mount(&server)
            .await;
        let service = WalletService::new_typed(vec![ChainEndpoints {
            chain_id: "ethereum".into(),
            endpoints: vec![server.uri()],
            api_key: None,
        }])
        .unwrap();
        let contract = format!("0x{}", "22".repeat(20));
        let key =
            seed_probe_holding(&service, Chain::Ethereum, "TEST", Some((&contract, 18))).await;
        let risk = service
            .send_destination_risk("probe-wallet".into(), key, format!("0x{}", "33".repeat(20)))
            .await;
        assert!(
            risk.unwrap_err().to_string().contains("unavailable"),
            "an unread balance must not reach a send as zero"
        );
    }

    #[tokio::test]
    async fn evm_preview_requires_every_rpc_and_valid_amount() {
        for failed in [
            "",
            "eth_getTransactionCount",
            "eth_getBalance",
            "eth_estimateGas",
            "eth_feeHistory",
            "reward",
        ] {
            let server = MockServer::start().await;
            Mock::given(any()).respond_with(move |req: &Request| {
                let body: serde_json::Value = req.body_json().unwrap();
                let method = body["method"].as_str().unwrap();
                let result = match method {
                    "eth_getTransactionCount" => json!("0x7"),
                    "eth_getBalance" => json!("0xde0b6b3a7640000"),
                    "eth_estimateGas" => json!("0x7530"),
                    "eth_feeHistory" if failed == "reward" => json!({"baseFeePerGas":["0x1"]}),
                    "eth_feeHistory" => json!({"baseFeePerGas":["0x1"],"reward":[["0x2"]]}),
                    _ => panic!("unexpected method {method}"),
                };
                let response = if failed == method {
                    json!({"jsonrpc":"2.0","id":body["id"],"error":{"code":-32000,"message":"unavailable"}})
                } else { json!({"jsonrpc":"2.0","id":body["id"],"result":result}) };
                ResponseTemplate::new(200).set_body_json(response)
            }).mount(&server).await;
            let service = WalletService::new_typed(vec![ChainEndpoints {
                chain_id: "ethereum".into(),
                endpoints: vec![server.uri()],
                api_key: None,
            }])
            .unwrap();
            let preview = service
                .fetch_evm_send_preview(
                    "ethereum",
                    "from".into(),
                    "to".into(),
                    "1".into(),
                    "0x".into(),
                )
                .await;
            if failed.is_empty() {
                let value: serde_json::Value = serde_json::from_str(&preview.unwrap()).unwrap();
                assert_eq!(value["nonce"], 7);
                assert_eq!(value["gas_limit"], 30000);
            } else {
                assert!(preview.is_err(), "{failed}");
            }
            let before = server.received_requests().await.unwrap().len();
            for value in ["bad", "-1", "+1", "340282366920938463463374607431768211456"] {
                assert!(service
                    .fetch_evm_send_preview(
                        "ethereum",
                        "from".into(),
                        "to".into(),
                        value.into(),
                        "0x".into()
                    )
                    .await
                    .is_err());
            }
            assert_eq!(server.received_requests().await.unwrap().len(), before);
        }
    }

    #[tokio::test]
    async fn trc20_zero_is_valid_but_failed_metadata_is_not_a_zero_balance() {
        use wiremock::matchers::body_partial_json;
        for valid in [true, false] {
            let server = MockServer::start().await;
            for (selector, result) in [
                ("balanceOf(address)", "0".repeat(64)),
                ("decimals()", format!("{:064x}", 6)),
                ("symbol()", format!("{:0<64}", hex::encode("TEST"))),
            ] {
                let response = if !valid && selector == "symbol()" {
                    json!({})
                } else {
                    json!({"constant_result":[result]})
                };
                Mock::given(body_partial_json(json!({"function_selector":selector})))
                    .respond_with(ResponseTemplate::new(200).set_body_json(response))
                    .mount(&server)
                    .await;
            }
            let service = WalletService::new_typed(vec![ChainEndpoints {
                chain_id: "tron".into(),
                endpoints: vec![server.uri()],
                api_key: None,
            }])
            .unwrap();
            let result = service
                .fetch_token_balances(
                    "tron".into(),
                    "TLa2f6VPqDgRE67v1736s7bJ8Ray5wYjU7".into(),
                    vec![TokenDescriptor {
                        contract: "TR7NHqjeKQxGTCi8q8ZY4pL8otgjLj6t".into(),
                        symbol: "TEST".into(),
                        decimals: 18,
                        name: None,
                    }],
                )
                .await;
            let rows = result.expect("one token's failure is not the request's");
            if valid {
                // A contract that answers zero holds zero.
                assert_eq!(rows[0].balance_raw, "0");
                assert_eq!(rows[0].decimals, 6);
            } else {
                // A contract that does not answer is left out. It must never
                // arrive as a zero: `send_destination_risk` reads this row and
                // would take the absence for an empty wallet.
                assert!(rows.is_empty(), "{rows:?}");
            }
        }
    }
}

/// A send preview's "spendable" is a fact about the asset the amount field
/// moves, not about whatever the chain pays gas in. Both previews here used to
/// answer with the gas coin's own arithmetic whatever was being sent.
#[cfg(test)]
mod a_preview_quotes_the_asset_it_moves {
    use super::*;
    use wiremock::{matchers::any, Mock, MockServer, Request, ResponseTemplate};

    /// One ETH held, and 250 of an 8-decimal token.
    const ETH_BALANCE_WEI: &str = "0xde0b6b3a7640000";
    const TOKEN_DECIMALS: u128 = 8;
    const TOKEN_RAW: u128 = 25_000_000_000; // 250.0 at 8 decimals
    const TOKEN_DISPLAY: f64 = 250.0;

    /// Answer one JSON-RPC call. `eth_call` dispatches on the ABI selector so
    /// the token contract can hold a balance the account does not.
    fn rpc_result(method: &str, params: &serde_json::Value) -> serde_json::Value {
        match method {
            "eth_getTransactionCount" => json!("0x7"),
            "eth_getBalance" => json!(ETH_BALANCE_WEI),
            "eth_estimateGas" => json!("0x7530"),
            "eth_feeHistory" => json!({"baseFeePerGas": ["0x1"], "reward": [["0x2"]]}),
            "eth_call" => {
                let data = params[0]["data"].as_str().unwrap_or_default();
                let selector = data.trim_start_matches("0x").get(..8).unwrap_or_default();
                match selector {
                    "70a08231" => json!(format!("0x{TOKEN_RAW:064x}")),
                    "313ce567" => json!(format!("0x{TOKEN_DECIMALS:064x}")),
                    "95d89b41" => json!(format!("0x{:0<64}", hex::encode("TEST"))),
                    other => panic!("unexpected eth_call selector {other}"),
                }
            }
            other => panic!("unexpected method {other}"),
        }
    }

    /// A node that answers single calls and JSON-RPC batches alike —
    /// `fetch_erc20_metadata` batches its two reads and `balanceOf` does not.
    async fn evm_node() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(|req: &Request| {
                let body: serde_json::Value = req.body_json().unwrap();
                let answer = |call: &serde_json::Value| {
                    json!({
                        "jsonrpc": "2.0",
                        "id": call["id"],
                        "result": rpc_result(call["method"].as_str().unwrap(), &call["params"]),
                    })
                };
                ResponseTemplate::new(200).set_body_json(match body.as_array() {
                    Some(batch) => json!(batch.iter().map(answer).collect::<Vec<_>>()),
                    None => answer(&body),
                })
            })
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn owned_preview_uses_wallet_network_and_exact_amount_without_secrets() {
        let server = evm_node().await;
        let service = WalletService::new_typed(vec![ChainEndpoints {
            chain_id: "ethereum-sepolia".into(),
            endpoints: vec![server.uri()],
            api_key: None,
        }])
        .unwrap();
        let key = super::seed_probe_holding(&service, Chain::Ethereum, "ETH", None).await;
        {
            let mut state = service.wallet_state.write().await;
            state.wallets[0].network_mode = Some("ethereum-sepolia".into());
            state.wallets[0].addresses[0].address = format!("0x{}", "11".repeat(20));
        }
        for amount in ["NaN", "-1", "0.0000000000000000001"] {
            assert!(matches!(
                service
                    .preview_owned_evm_send(
                        "probe-wallet".into(),
                        key.clone(),
                        amount.into(),
                        "".into(),
                        None,
                        None
                    )
                    .await,
                Err(crate::SpectraBridgeError::InvalidInput { .. })
            ));
        }
        assert!(server.received_requests().await.unwrap().is_empty());
        let preview = service
            .preview_owned_evm_send(
                "probe-wallet".into(),
                key,
                "1.1".into(),
                "".into(),
                None,
                None,
            )
            .await
            .unwrap();
        assert!(preview.is_some());
        let requests = server.received_requests().await.unwrap();
        let estimate = requests
            .iter()
            .map(|r| r.body_json::<serde_json::Value>().unwrap())
            .find(|r| r["method"] == "eth_estimateGas")
            .unwrap();
        assert_eq!(estimate["params"][0]["value"], "0xf43fc2c04ee0000");
    }

    /// `value_wei` and `data_hex` are what `prepare_evm_send_assembly` hands
    /// this call for the send in question — a token transfer carries its
    /// amount in the calldata and moves no ether, so its value is zero.
    async fn preview(
        server: &MockServer,
        to: String,
        value_wei: &str,
        data_hex: String,
    ) -> serde_json::Value {
        let service = WalletService::new_typed(vec![ChainEndpoints {
            chain_id: "ethereum".into(),
            endpoints: vec![server.uri()],
            api_key: None,
        }])
        .unwrap();
        let raw = service
            .fetch_evm_send_preview(
                "ethereum",
                format!("0x{}", "11".repeat(20)),
                to,
                value_wei.into(),
                data_hex,
            )
            .await
            .expect("preview");
        serde_json::from_str(&raw).unwrap()
    }

    /// An ERC-20 send moves the token, so the token's balance — scaled by the
    /// contract's own decimals — is what is spendable. This answered with the
    /// sender's ETH balance, which the send sheet then rendered through the
    /// token's formatter: 1 ETH shown as "1 USDC", and "Max" filling in a
    /// number the token transfer could not move.
    #[tokio::test]
    async fn an_erc20_send_is_limited_by_the_token_and_not_by_the_ether() {
        let server = evm_node().await;
        let contract = format!("0x{}", "22".repeat(20));
        // transfer(0x33…, 1)
        let data = format!("0xa9059cbb{:0>64}{:0>64}", "33".repeat(20), "1");
        let value = preview(&server, contract, "0", data).await;

        assert_eq!(value["spendable_balance"], json!(TOKEN_DISPLAY));
        assert_ne!(
            value["spendable_balance"].as_f64().unwrap().round(),
            1.0,
            "the gas coin's balance is not the token's"
        );
        // The fee is still quoted in the gas coin: it is a separate claim.
        assert!(value["estimated_fee_eth"].as_f64().unwrap() > 0.0);
    }

    /// A native send pays its fee out of the balance it is moving, so the fee
    /// still comes off the top. Unchanged — asserted here so the token arm
    /// cannot be made to swallow this one.
    #[tokio::test]
    async fn a_native_send_still_nets_the_fee_off_its_own_balance() {
        let server = evm_node().await;
        let value = preview(
            &server,
            format!("0x{}", "33".repeat(20)),
            "1000000000000000",
            "0x".into(),
        )
        .await;

        let fee = value["estimated_fee_eth"].as_f64().unwrap();
        assert!(fee > 0.0);
        assert_eq!(value["spendable_balance"].as_f64().unwrap(), 1.0 - fee);
    }

    /// TRC-20 decimals are the contract's. The fixed `1e6` that stood here is
    /// TRX's own scale, so an 18-decimal token was quoted at 10^12 times the
    /// holding it is — and "Max" offered it.
    #[tokio::test]
    async fn a_trc20_preview_scales_by_the_contract_and_not_by_trx() {
        use wiremock::matchers::body_partial_json;

        let server = MockServer::start().await;
        // 4.2 of an 18-decimal token.
        let raw: u128 = 4_200_000_000_000_000_000;
        for (selector, result) in [
            ("balanceOf(address)", format!("{raw:064x}")),
            ("decimals()", format!("{:064x}", 18)),
            ("symbol()", format!("{:0<64}", hex::encode("TEST"))),
        ] {
            Mock::given(body_partial_json(json!({"function_selector": selector})))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"constant_result": [result]})),
                )
                .mount(&server)
                .await;
        }
        let service = WalletService::new_typed(vec![ChainEndpoints {
            chain_id: "tron".into(),
            endpoints: vec![server.uri()],
            api_key: None,
        }])
        .unwrap();
        let value = service
            .fetch_tron_send_preview_typed(
                "TLa2f6VPqDgRE67v1736s7bJ8Ray5wYjU7".into(),
                "TEST".into(),
                "TR7NHqjeKQxGTCi8q8ZY4pL8otgjLj6t".into(),
            )
            .await
            .expect("preview").expect("valid typed preview");

        assert_eq!(value.spendableBalance, 4.2);
        assert_eq!(value.maxSendable, 4.2);
    }

    /// A token balance nobody could read is not a zero holding — the send
    /// sheet decides whether the amount fits from this number.
    #[tokio::test]
    async fn an_unreadable_trc20_balance_refuses_rather_than_quoting_zero() {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let service = WalletService::new_typed(vec![ChainEndpoints {
            chain_id: "tron".into(),
            endpoints: vec![server.uri()],
            api_key: None,
        }])
        .unwrap();
        let result = service
            .fetch_tron_send_preview_typed(
                "TLa2f6VPqDgRE67v1736s7bJ8Ray5wYjU7".into(),
                "TEST".into(),
                "TR7NHqjeKQxGTCi8q8ZY4pL8otgjLj6t".into(),
            )
            .await;
        assert!(
            result.is_err(),
            "an unread balance must not quote a maximum"
        );
    }
}

#[cfg(test)]
mod destination_probe_tests {
    use super::*;
    use wiremock::{matchers::any, Mock, MockServer, Request, ResponseTemplate};
    #[tokio::test]
    async fn evm_activity_uses_one_balance_read_and_never_guesses_after_failure() {
        for nonce in [Some("0x1"), Some("0x0"), None] {
            let server = MockServer::start().await;
            Mock::given(any()).respond_with(move |request: &Request| {
                let body: serde_json::Value = request.body_json().unwrap();
                let method = body["method"].as_str().unwrap();
                let value = match method {
                    "eth_getBalance" => Some("0x0"),
                    "eth_getTransactionCount" => nonce,
                    _ => panic!("unexpected RPC {method}")
                };
                ResponseTemplate::new(200).set_body_json(match value {
                    Some(value) => json!({"jsonrpc":"2.0","id":body["id"],"result":value}),
                    None => json!({"jsonrpc":"2.0","id":body["id"],"error":{"code":-32000,"message":"offline"}})
                })
            }).mount(&server).await;
            // Zero nonce on BNB needs its keyed explorer: without a key the
            // result is unknown/error, rather than an invented empty history.
            let service = WalletService::new_typed(vec![ChainEndpoints {
                chain_id: Chain::BnbChain.str_id().into(),
                endpoints: vec![server.uri()],
                api_key: None,
            }])
            .unwrap();
            let key = seed_probe_holding(&service, Chain::BnbChain, "BNB", None).await;
            let result = service
                .send_destination_risk("probe-wallet".into(), key, format!("0x{}", "44".repeat(20)))
                .await;
            if nonce == Some("0x1") {
                let risk = result.unwrap();
                assert!(risk.has_history);
                assert!(risk.balance_is_zero);
            } else {
                assert!(result.is_err());
            }
            let requests = server.received_requests().await.unwrap();
            let balances = requests
                .iter()
                .filter(|r| {
                    r.body_json::<serde_json::Value>().unwrap()["method"] == "eth_getBalance"
                })
                .count();
            assert!(balances <= 1, "duplicate balance request");
            if nonce == Some("0x1") {
                assert_eq!(balances, 1);
                assert_eq!(requests.len(), 2);
            }
        }
    }
    #[tokio::test]
    async fn successful_empty_history_is_distinct_from_unknown() {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(|request: &Request| {
                let is_history = request.url.query().unwrap_or("").contains("details=txs");
                ResponseTemplate::new(200).set_body_json(if is_history {
                    json!({"transactions":[]})
                } else {
                    json!({"balance":"0"})
                })
            })
            .mount(&server)
            .await;
        let service = WalletService::new_typed(vec![ChainEndpoints {
            chain_id: "litecoin".into(),
            endpoints: vec![server.uri()],
            api_key: None,
        }])
        .unwrap();
        let key = seed_probe_holding(&service, Chain::Litecoin, "LTC", None).await;
        let risk = service
            .send_destination_risk(
                "probe-wallet".into(),
                key,
                "ltc1qw508d6qejxtdg4y5r3zarvary0c5xw7kgmn4n9".into(),
            )
            .await
            .unwrap();
        assert!(!risk.has_history);
        assert!(risk.balance_is_zero);
    }
}

#[cfg(test)]
mod fresh_destination_tests {
    use super::*;
    #[tokio::test]
    async fn changed_and_failed_ens_reads_cannot_reuse_a_reviewed_address() {
        let old = format!("0x{}", "11".repeat(20));
        let new = format!("0x{}", "22".repeat(20));
        let first = resolve_destination(Chain::Ethereum, "alice.eth".into(), |_| async {
            Ok(Some(old.clone()))
        })
        .await
        .unwrap();
        assert_eq!(first.address, old);
        let second = resolve_destination(Chain::Ethereum, "alice.eth".into(), |_| async {
            Ok(Some(new.clone()))
        })
        .await
        .unwrap();
        assert_eq!(second.address, new);
        assert!(verify_reviewed_destination(Chain::Ethereum, second, &old).is_err());
        assert!(
            resolve_destination(Chain::Ethereum, "alice.eth".into(), |_| async {
                Err(SpectraBridgeError::from("offline"))
            })
            .await
            .is_err()
        );
        assert!(
            resolve_destination(Chain::Ethereum, "alice.eth".into(), |_| async { Ok(None) })
                .await
                .is_err()
        );
        let third = resolve_destination(Chain::Ethereum, "alice.eth".into(), |_| async {
            Ok(Some(new.clone()))
        })
        .await
        .unwrap();
        assert!(verify_reviewed_destination(Chain::Ethereum, third, &new).is_ok());
    }
}
