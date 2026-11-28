//! Token holdings read off one listing of what an address holds.
use super::*;
use crate::service::send_stage_support::{Secret, Wallet, node};
use serde_json::{Value, json};

/// A Cardano wallet's native assets are what its outputs at its own base
/// address hold, summed, under the network's own standard: discovered
/// unnamed, and refreshed by policy and name however a token was spelled
/// when it was added.
#[tokio::test]
async fn a_cardano_wallet_holds_the_native_assets_its_base_address_outputs_hold() {
    let token = format!("{}.01", "a".repeat(56));
    let addresses = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
    let asked = addresses.clone();
    let server = node(move |path, body| {
        if let Some(listed) = body.get("_addresses") {
            asked.lock().unwrap().push(listed.clone());
        }
        let asset = |quantity: &str| json!({"policy_id": "a".repeat(56), "asset_name": "01", "quantity": quantity});
        Some(match path {
            "/address_utxos" => json!([
                {"tx_hash": "11".repeat(32), "tx_index": 0, "value": "2000000",
                    "is_spent": false, "asset_list": [asset("1")]},
                {"tx_hash": "22".repeat(32), "tx_index": 1, "value": "1500000",
                    "is_spent": false, "asset_list": [asset("2")]},
                {"tx_hash": "00".repeat(32), "tx_index": 0, "value": "1170000",
                    "is_spent": false, "asset_list": []},
            ]),
            "/asset_info" => {
                assert_eq!(body["_asset_list"], json!([["a".repeat(56), "01"]]));
                json!([{"policy_id": "a".repeat(56), "asset_name": "01",
                    "token_registry_metadata": null, "cip68_metadata": null}])
            }
            other => panic!("unexpected Koios request {other}"),
        })
    })
    .await;
    let vector: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/cardano-emurgo-witness.json"
    ))
    .unwrap();
    let wallet = Wallet::import(
        Chain::Cardano,
        &server.uri(),
        Secret::Phrase(vector["mnemonic"].as_str().unwrap()),
    )
    .await;
    // A phrase wallet holds its base address, not its key's enterprise one.
    assert!(wallet.address.starts_with("addr1q"), "{}", wallet.address);
    let discovered = wallet
        .service
        .discover_token_balances(Chain::Cardano, wallet.address.clone())
        .await
        .unwrap();
    let read = |rows: Vec<TokenBalanceResult>| {
        rows.into_iter()
            .map(|row| {
                (
                    row.contract_address,
                    row.standard,
                    row.decimals,
                    row.balance_display,
                )
            })
            .collect::<Vec<_>>()
    };
    let held = [(
        token.clone(),
        Some(crate::registry::TokenStandard::CardanoNativeToken),
        0,
        "3".to_string(),
    )];
    assert_eq!(read(discovered), held);
    let refreshed = wallet
        .service
        .known_token_balances(
            Chain::Cardano,
            wallet.address.clone(),
            vec![TokenDescriptor {
                standard: None,
                contract: token.to_uppercase(),
                symbol: "TEST".into(),
                decimals: 0,
                name: None,
            }],
        )
        .await
        .unwrap();
    assert_eq!(read(refreshed), held);
    // Every read named the wallet's base address, and only it.
    let asked = addresses.lock().unwrap();
    assert!(!asked.is_empty());
    assert!(
        asked
            .iter()
            .all(|listed| *listed == json!([wallet.address.clone()])),
        "{asked:?}"
    );
}

mod a_listing_answers_for_every_known_token {
    use super::holding_key;
    use crate::registry::Chain;
    use crate::service::{TokenDescriptor, WalletService};

    /// Offline nothing is configured, so nothing can list holdings, and the
    /// answer says so rather than returning a list that reads as "holds
    /// nothing".
    #[tokio::test]
    async fn a_chain_nothing_can_list_says_so() {
        let service = WalletService::new(Vec::new()).expect("service");
        for chain in Chain::all() {
            let err = service
                .discover_token_balances(chain, "whatever".into())
                .await
                .expect_err("no service is configured, so nothing can list");
            assert!(
                err.to_string()
                    .contains("needs an address indexer to list the tokens"),
                "{}: {err}",
                chain.str_id()
            );
        }
    }

    /// A known token absent from a listing holds none; one present takes the
    /// listing's balance and decimals; a listing row nobody knows is ignored.
    #[tokio::test]
    async fn absent_is_zero_and_present_is_the_listed_balance() {
        let service = WalletService::new(Vec::new()).expect("service");
        let usdc = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
        let usdt = "0xdAC17F958D2ee523a2206206994597C13D831ec7";
        let descriptor = |contract: &str| TokenDescriptor {
            standard: None,
            contract: contract.into(),
            symbol: "TKN".into(),
            decimals: 18,
            name: None,
        };
        let rows = service
            .balances_from_holdings(
                Chain::Ethereum,
                vec![
                    crate::api::HeldToken {
                        contract: usdc.to_lowercase(),
                        balance_raw: 2_500_000,
                        decimals: Some(6),
                    },
                    crate::api::HeldToken {
                        contract: format!("0x{}", "99".repeat(20)),
                        balance_raw: 1,
                        decimals: Some(0),
                    },
                ],
                &[descriptor(usdc), descriptor(usdt)],
            )
            .await;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].balance_display, "2.5");
        assert_eq!(
            rows[0].decimals, 6,
            "the listing's decimals, not the caller's"
        );
        assert_eq!(rows[1].balance_raw, "0");
    }

    /// An indexer lists Arc's USDC ERC-20 face beside the coin it is: the
    /// same balance, which the native row already shows, so discovery leaves
    /// it out and keeps every other token; Metis's face is matched in any
    /// case, and a network without such a face keeps the whole listing.
    #[test]
    fn the_coins_own_erc20_face_is_not_discovered_as_a_token() {
        let held = |contract: &str| crate::api::HeldToken {
            contract: contract.into(),
            balance_raw: 2_500_000,
            decimals: Some(6),
        };
        let eurc = "0xbef5f6d51cb62b58e6a8f77868681825c6fe21c1";
        let listing = || {
            vec![
                held("0x3600000000000000000000000000000000000000"),
                held("0xDEADDEADDEADDEADDEADDEADDEADDEADDEAD0000"),
                held(eurc),
            ]
        };
        let kept = |chain| {
            super::without_native_coin_face(chain, listing())
                .into_iter()
                .map(|h| h.contract)
                .collect::<Vec<_>>()
        };
        assert_eq!(kept(Chain::Arc).len(), 2);
        assert!(!kept(Chain::Arc).iter().any(|c| c.starts_with("0x3600")));
        assert!(!kept(Chain::Metis).iter().any(|c| c.starts_with("0xDEAD")));
        assert_eq!(kept(Chain::Ethereum).len(), 3);
    }

    /// The catalog spells a jetton master user-friendly; the indexer, raw.
    #[test]
    fn a_ton_master_matches_in_either_spelling() {
        assert_eq!(
            holding_key(
                Chain::Ton,
                "EQCxE6mUtQJKFnGfaROTKOt1lZbDiiX1kCixRv7Nw2Id_sDs"
            ),
            holding_key(
                Chain::Ton,
                "0:B113A994B5024A16719F69139328EB759596C38A25F59028B146FECDC3621DFE"
            )
        );
        assert_eq!(
            holding_key(Chain::Ethereum, "0xABC"),
            holding_key(Chain::Ethereum, "0xabc")
        );
    }

    /// A devnet wallet's tokens are listed by the devnet's own endpoints,
    /// both token programs asked about the owner; the mainnet's, for the
    /// same address, are not asked at all.
    #[tokio::test]
    async fn a_devnet_listing_asks_only_the_devnets_endpoints() {
        use serde_json::{Value, json};
        use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};
        let owner = "BLeUXTx9thHGT7VJUtF9vHEmfMDgW1nnKZ9UVer2CoLX";
        let mint = "So11111111111111111111111111111111111111112";
        let devnet = MockServer::start().await;
        Mock::given(any())
            .respond_with(move |request: &Request| {
                let call: Value = request.body_json().unwrap();
                let program = call["params"][1]["programId"].as_str().unwrap_or_default();
                let value = if program.starts_with("Tokenkeg") {
                    json!([{"account": {"data": {"parsed": {"info": {
                        "mint": mint,
                        "tokenAmount": {"amount": "2500000", "decimals": 6},
                    }}}}}])
                } else {
                    json!([])
                };
                ResponseTemplate::new(200).set_body_json(
                    json!({"jsonrpc": "2.0", "id": call["id"], "result": {"value": value}}),
                )
            })
            .mount(&devnet)
            .await;
        let mainnet = MockServer::start().await;
        let endpoints = |chain_id, server: &MockServer| crate::service::ChainEndpoints {
            capabilities: crate::api::endpoint::EndpointCapability::ALL.to_vec(),
            chain_id,
            endpoints: vec![server.uri()],
        };
        let service = WalletService::serving(vec![
            endpoints(Chain::SolanaDevnet, &devnet),
            endpoints(Chain::Solana, &mainnet),
        ])
        .unwrap();
        let held = service
            .discover_token_balances(Chain::SolanaDevnet, owner.into())
            .await
            .unwrap();
        let read: Vec<_> = held
            .iter()
            .map(|t| {
                (
                    t.contract_address.as_str(),
                    t.decimals,
                    t.balance_display.as_str(),
                )
            })
            .collect();
        assert_eq!(read, [(mint, 6, "2.5")]);
        let calls: Vec<Value> = devnet
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|request| request.body_json().unwrap())
            .collect();
        assert_eq!(calls.len(), 2, "{calls:?}");
        assert!(
            calls
                .iter()
                .all(|call| call["method"] == "getTokenAccountsByOwner"
                    && call["params"][0] == owner)
        );
        assert!(mainnet.received_requests().await.unwrap().is_empty());
    }
}

mod decimals_come_from_the_chain {
    use crate::registry::Chain;
    use crate::service::{ChainEndpoints, TokenDescriptor, WalletService};
    use serde_json::json;

    /// A balance's decimals are the contract's, not the caller's: where the
    /// catalog disagrees with the contract, the catalog is the one that can
    /// only be wrong.
    ///
    /// A provider failure must remain an error, not an invented zero balance.
    #[tokio::test]
    async fn an_unreadable_contract_is_left_out_rather_than_reported_as_zero() {
        let service = WalletService::new(Vec::new()).expect("service");
        for chain in Chain::all().filter(|chain| {
            chain.hosts_tokens()
                && (chain.is_evm()
                    || matches!(
                        chain.mainnet_counterpart(),
                        Chain::Tron
                            | Chain::Near
                            | Chain::Ton
                            | Chain::Sui
                            | Chain::Aptos
                            | Chain::Solana
                    ))
        }) {
            let contract = match chain.mainnet_counterpart() {
                Chain::Tron => "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t",
                Chain::Solana => "11111111111111111111111111111111",
                Chain::Sui => "0x2::coin::T",
                Chain::Aptos => "0x1",
                Chain::Near => "token.near",
                Chain::Ton => "EQCxE6mUtQJKFnGfaROTKOt1lZbDiiX1kCixRv7Nw2Id_sDs",
                _ => "0x1111111111111111111111111111111111111111",
            };
            let results = service
                .fetch_token_balances(
                    chain,
                    "whoever".into(),
                    vec![TokenDescriptor {
                        standard: None,
                        contract: contract.into(),
                        symbol: "TEST".into(),
                        decimals: 18,
                        name: None,
                    }],
                )
                .await;
            match chain.mainnet_counterpart() {
                // TON reads every jetton the address holds in one call, so a
                // failure there is the chain's answer and not one token's.
                Chain::Ton => assert!(results.is_err(), "ton"),
                _ => assert!(
                    results
                        .unwrap_or_else(|e| panic!("{}: {e}", chain.str_id()))
                        .is_empty(),
                    "{} fabricated a balance for a contract it could not read",
                    chain.str_id()
                ),
            }
        }
    }

    /// One unreadable contract is that contract's answer, not the wallet's.
    /// Collecting the batch into a single `Result` meant a self-destructed
    /// token stopped every other token in the wallet from refreshing, and the
    /// only caller discards the error, so it stopped silently.
    #[tokio::test]
    async fn an_unreadable_token_does_not_take_the_readable_ones_with_it() {
        use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};
        let holder = format!("0x{}", "33".repeat(20));
        let good = format!("0x{}", "11".repeat(20));
        let bad = format!("0x{}", "22".repeat(20));
        let server = MockServer::start().await;
        let refused = bad.clone();
        Mock::given(any())
            .respond_with(move |req: &Request| {
                let body: serde_json::Value = req.body_json().unwrap();
                let calls = body
                    .as_array()
                    .cloned()
                    .unwrap_or_else(|| vec![body.clone()]);
                let replies: Vec<_> = calls
                    .iter()
                    .map(|call| {
                        let to = call["params"][0]["to"].as_str().unwrap_or_default();
                        if to.eq_ignore_ascii_case(&refused) {
                            return json!({"jsonrpc":"2.0","id":call["id"],
                                   "error":{"code":-32000,"message":"no code at address"}});
                        }
                        // balanceOf and decimals are both plain uint256 words;
                        // symbol decodes to empty, which the catalog covers.
                        json!({"jsonrpc":"2.0","id":call["id"],
                               "result":format!("0x{:064x}", 6)})
                    })
                    .collect();
                ResponseTemplate::new(200).set_body_json(if body.is_array() {
                    json!(replies)
                } else {
                    replies[0].clone()
                })
            })
            .mount(&server)
            .await;

        let service = WalletService::serving(vec![ChainEndpoints {
            capabilities: crate::api::endpoint::EndpointCapability::ALL.to_vec(),
            chain_id: crate::registry::Chain::Ethereum,
            endpoints: vec![server.uri()],
        }])
        .unwrap();
        let descriptor = |contract: &str| TokenDescriptor {
            standard: None,
            contract: contract.into(),
            symbol: "TEST".into(),
            decimals: 18,
            name: None,
        };
        let rows = service
            .fetch_token_balances(
                crate::registry::Chain::Ethereum,
                holder,
                vec![descriptor(&bad), descriptor(&good)],
            )
            .await
            .expect("one bad contract is not a failed request");
        assert_eq!(rows.len(), 1, "only the readable contract answers");
        assert!(rows[0].contract_address.eq_ignore_ascii_case(&good));
    }

    /// An Aptos fungible asset is read by its metadata address through the
    /// primary store, and a legacy coin by its type through `coin::balance`.
    /// Both used to be read as a `CoinStore<T>`, which no fungible asset has,
    /// so every catalog Aptos token was left out.
    #[tokio::test]
    async fn aptos_reads_a_fungible_asset_and_a_coin_through_view_functions() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{body_json, method, path},
        };
        let owner = "0x6a2c9b3d1f64c2ae7c0e79c56b4b2f8e07c1b5f2aa7ec8bb0e0f2e7c4b6d8e1a";
        let usdc = "0xbae207659db88bea0cbead6da0ed00aac12edcdda169e591cd41c94180b46f3b";
        let coin = "0x5e156f1207d0ebfa19a9eeff00d62a282278fb8719f4fab3a586a0a2c0fffbea::coin::T";
        let metadata = "0x1::fungible_asset::Metadata";
        let server = MockServer::start().await;
        for (request, reply) in [
            (
                json!({"function": "0x1::primary_fungible_store::balance",
                       "type_arguments": [metadata], "arguments": [owner, usdc]}),
                json!(["2500000"]),
            ),
            (
                json!({"function": "0x1::fungible_asset::decimals",
                       "type_arguments": [metadata], "arguments": [usdc]}),
                json!([6]),
            ),
            (
                json!({"function": "0x1::coin::balance",
                       "type_arguments": [coin], "arguments": [owner]}),
                json!(["700"]),
            ),
            (
                json!({"function": "0x1::coin::decimals",
                       "type_arguments": [coin], "arguments": []}),
                json!([2]),
            ),
        ] {
            Mock::given(method("POST"))
                .and(path("/view"))
                .and(body_json(request))
                .respond_with(ResponseTemplate::new(200).set_body_json(reply))
                .expect(1)
                .mount(&server)
                .await;
        }

        let service = WalletService::serving(vec![ChainEndpoints {
            capabilities: crate::api::endpoint::EndpointCapability::ALL.to_vec(),
            chain_id: Chain::Aptos,
            endpoints: vec![server.uri()],
        }])
        .unwrap();
        let descriptor = |contract: &str| TokenDescriptor {
            standard: None,
            contract: contract.into(),
            symbol: "TEST".into(),
            // Wrong on purpose: the token's own count is the one used.
            decimals: 18,
            name: None,
        };
        let rows = service
            .fetch_token_balances(
                Chain::Aptos,
                owner.into(),
                vec![descriptor(usdc), descriptor(coin)],
            )
            .await
            .expect("both tokens are readable");
        let read: Vec<_> = rows
            .iter()
            .map(|r| {
                (
                    r.contract_address.as_str(),
                    r.decimals,
                    r.balance_raw.as_str(),
                    r.balance_display.as_str(),
                )
            })
            .collect();
        assert_eq!(read, [(usdc, 6, "2500000", "2.5"), (coin, 2, "700", "7")]);
        assert_eq!(
            rows[0].standard,
            Some(crate::registry::TokenStandard::Aip21)
        );
        assert_eq!(
            rows[1].standard,
            Some(crate::registry::TokenStandard::AptosCoin)
        );
    }

    #[tokio::test]
    async fn unsupported_or_wrong_protocol_is_rejected_before_requests_and_cannot_become_listing_zero()
     {
        use wiremock::MockServer;
        let server = MockServer::start().await;
        let service = WalletService::serving(vec![ChainEndpoints {
            chain_id: Chain::Tron,
            endpoints: vec![server.uri()],
            capabilities: crate::api::endpoint::EndpointCapability::ALL.to_vec(),
        }])
        .unwrap();
        let token = TokenDescriptor {
            standard: Some(crate::registry::TokenStandard::Trc20),
            contract: "1002000".into(),
            symbol: "T10".into(),
            decimals: 6,
            name: None,
        };
        let mut legacy = [TokenDescriptor {
            standard: Some(crate::registry::TokenStandard::Trc10),
            ..token.clone()
        }];
        super::validate_token_reads(Chain::Tron, &mut legacy).expect("TRC-10 is supported on Tron");
        for result in [
            service
                .fetch_token_balances(Chain::Tron, "owner".into(), vec![token.clone()])
                .await,
            service
                .known_token_balances(Chain::Tron, "owner".into(), vec![token.clone()])
                .await,
        ] {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("invalid token identifier for protocol")
            );
        }
        assert!(
            service
                .balances_from_holdings(Chain::Tron, Vec::new(), &[token])
                .await
                .is_empty()
        );
        assert!(server.received_requests().await.unwrap().is_empty());
        let descriptor = TokenDescriptor {
            standard: Some(crate::registry::TokenStandard::Erc20),
            contract: "11111111111111111111111111111111".into(),
            symbol: "BAD".into(),
            decimals: 6,
            name: None,
        };
        assert!(
            service
                .fetch_token_balances(Chain::Solana, "owner".into(), vec![descriptor])
                .await
                .unwrap_err()
                .to_string()
                .contains("protocol does not belong")
        );
    }

    /// A NEP-141 balance on NEAR's test network is read from that network's
    /// node alone, by `ft_balance_of` for the holder and `ft_metadata`, and
    /// scaled by the contract's own decimals rather than the caller's.
    #[tokio::test]
    async fn a_near_testnet_token_is_read_from_the_testnets_own_node() {
        use base64::Engine;
        use serde_json::Value;
        use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};
        let view = |request: &Request| -> (Value, String, Value) {
            let call: Value = request.body_json().unwrap();
            let params = &call["params"];
            let args = base64::engine::general_purpose::STANDARD
                .decode(params["args_base64"].as_str().unwrap_or_default())
                .unwrap();
            (
                params["account_id"].clone(),
                params["method_name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                serde_json::from_slice(&args).unwrap_or(Value::Null),
            )
        };
        let testnet = MockServer::start().await;
        Mock::given(any())
            .respond_with(move |request: &Request| {
                let call: Value = request.body_json().unwrap();
                let value = match view(request).1.as_str() {
                    "ft_balance_of" => json!("2500000"),
                    "ft_metadata" => {
                        json!({"spec": "ft-1.0.0", "name": "Fixture", "symbol": "TST", "decimals": 6})
                    }
                    _ => Value::Null,
                };
                let bytes = serde_json::to_vec(&value).unwrap();
                ResponseTemplate::new(200).set_body_json(
                    json!({"jsonrpc": "2.0", "id": call["id"], "result": {"result": bytes}}),
                )
            })
            .mount(&testnet)
            .await;
        let mainnet = MockServer::start().await;
        let endpoints = |chain_id, server: &MockServer| ChainEndpoints {
            capabilities: crate::api::endpoint::EndpointCapability::ALL.to_vec(),
            chain_id,
            endpoints: vec![server.uri()],
        };
        let service = WalletService::serving(vec![
            endpoints(Chain::NearTestnet, &testnet),
            endpoints(Chain::Near, &mainnet),
        ])
        .unwrap();
        let rows = service
            .fetch_token_balances(
                Chain::NearTestnet,
                "owner.testnet".into(),
                vec![TokenDescriptor {
                    standard: None,
                    contract: "fixture.testnet".into(),
                    symbol: "TST".into(),
                    decimals: 18,
                    name: None,
                }],
            )
            .await
            .unwrap();
        let read: Vec<_> = rows
            .iter()
            .map(|r| {
                (
                    r.standard.map_or("", |standard| standard.as_str()),
                    r.contract_address.as_str(),
                    r.decimals,
                    r.balance_raw.as_str(),
                    r.balance_display.as_str(),
                )
            })
            .collect();
        assert_eq!(read, [("NEP-141", "fixture.testnet", 6, "2500000", "2.5")]);
        let mut calls: Vec<_> = testnet
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(view)
            .collect();
        calls.sort_by(|a, b| a.1.cmp(&b.1));
        assert_eq!(
            calls,
            [
                (
                    json!("fixture.testnet"),
                    "ft_balance_of".to_string(),
                    json!({"account_id": "owner.testnet"})
                ),
                (
                    json!("fixture.testnet"),
                    "ft_metadata".to_string(),
                    json!({})
                ),
            ]
        );
        assert!(mainnet.received_requests().await.unwrap().is_empty());
    }

    /// An empty list is not a fetch.
    #[tokio::test]
    async fn no_tokens_is_no_round_trip() {
        let service = WalletService::new(Vec::new()).expect("service");
        let out = service
            .fetch_token_balances(
                crate::registry::Chain::Ethereum,
                "whoever".into(),
                Vec::new(),
            )
            .await
            .expect("an empty request cannot fail");
        assert!(out.is_empty());
    }
}
