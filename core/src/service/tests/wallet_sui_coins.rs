//! A Sui wallet's coin objects by type, and what merging them costs,
//! against a node holding three SUI coins, three of a token and one of
//! another.

use super::*;
use crate::derivation::setup::WalletSetupMethod;
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

fn token() -> String {
    format!("0x{}::usdc::USDC", "aa".repeat(32))
}

fn single() -> String {
    format!("0x{}::one::ONE", "bb".repeat(32))
}

fn coins(coin_type: &str) -> Vec<Value> {
    let coin = |byte: &str, version: u64, balance: u64| {
        json!({"coinObjectId": format!("0x{}", byte.repeat(32)), "version": version.to_string(),
            "digest": "1".repeat(32), "balance": balance.to_string()})
    };
    if coin_type == SUI {
        vec![
            coin("33", 7, 4_000_000),
            coin("44", 8, 4_000_000),
            coin("55", 9, 4_000_000),
        ]
    } else if coin_type == token() {
        vec![
            coin("66", 10, 1_000_000),
            coin("77", 11, 1_000_000),
            coin("88", 12, 1_000_000),
        ]
    } else {
        vec![coin("99", 13, 5)]
    }
}

/// The wallet and a node whose dry run reports 1 MIST of computation and
/// 2 of storage for every million.
async fn wallet() -> (
    crate::service::loopback_service::OpenService,
    MockServer,
    String,
) {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(|request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let (identifier, checkpoint) = Chain::Sui.sui_network_identity().unwrap();
            let result = match body["method"].as_str().unwrap() {
                "sui_getChainIdentifier" => json!(identifier),
                "sui_getCheckpoint" => json!({"sequenceNumber": "0", "digest": checkpoint}),
                "suix_getAllBalances" => json!(
                    [SUI.to_string(), token(), single()]
                        .iter()
                        .map(|kind| {
                            let rows = coins(kind);
                            let total: u64 = rows
                                .iter()
                                .map(|row| row["balance"].as_str().unwrap().parse::<u64>().unwrap())
                                .sum();
                            json!({"coinType": kind, "coinObjectCount": rows.len(),
                            "totalBalance": total.to_string()})
                        })
                        .collect::<Vec<_>>()
                ),
                "suix_getCoins" => json!({"data": coins(body["params"][1].as_str().unwrap()),
                    "nextCursor": null, "hasNextPage": false}),
                "suix_getBalance" => json!({"totalBalance": "12000000"}),
                "suix_getCoinMetadata" => json!({"decimals": 6}),
                "suix_getReferenceGasPrice" => json!("1000"),
                "sui_dryRunTransactionBlock" => json!({"effects": {
                    "status": {"status": "success"},
                    "gasUsed": {"computationCost": "1000000", "storageCost": "2000000",
                        "storageRebate": "2500000"}}}),
                other => panic!("unexpected Sui call {other}"),
            };
            ResponseTemplate::new(200)
                .set_body_json(json!({"jsonrpc": "2.0", "id": body["id"], "result": result}))
        })
        .mount(&server)
        .await;
    let service = crate::service::loopback_service::open().await;
    let wallet = service
        .import(crate::derivation::setup::tests::fixture(
            Chain::Sui,
            WalletSetupMethod::ImportPhrase,
        ))
        .await;
    use EndpointCapability::*;
    service
        .use_endpoint(
            Chain::Sui,
            crate::EndpointApi::SuiJsonRpc,
            &[Balance, Fee, Broadcast, Verification],
            &server.uri(),
        )
        .await;
    (service, server, wallet)
}

/// Each coin type the wallet holds, SUI first and then by object count,
/// with its objects, its total in the type's decimals and whether a merge
/// would join any.
#[tokio::test]
async fn coin_objects_are_listed_by_type_sui_first() {
    let (service, _server, wallet) = wallet().await;
    let types = service.wallet_coin_objects(wallet).await.unwrap();
    let listed: Vec<_> = types
        .iter()
        .map(|t| {
            (
                t.coin_type.clone(),
                t.objects,
                t.balance.as_str(),
                t.mergeable,
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            (SUI.to_string(), 3, "0.012", true),
            (token(), 3, "3", true),
            (single(), 1, "0.000005", false),
        ]
    );
}

/// A merge's budget is its dry run's computation and storage with a fifth
/// to spare; a type one object holds has nothing to merge, and nothing is
/// stored for it.
#[tokio::test]
async fn a_merge_budgets_its_dry_run_cost_with_a_fifth_to_spare() {
    let (service, _server, wallet) = wallet().await;
    let refusal = service
        .build_coin_merge(wallet.clone(), single())
        .await
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("there is nothing to merge"), "{refusal}");
    assert!(service.list_sends().await.unwrap().is_empty());
    for coin_type in [token(), SUI.to_string()] {
        let built = service
            .build_coin_merge(wallet.clone(), coin_type.clone())
            .await
            .unwrap();
        // (1_000_000 + 2_000_000) × 6/5 MIST.
        assert_eq!(
            built.operation,
            Some(crate::send::stages::WalletOperation::MergeCoins {
                coin_type,
                objects: 3,
                network_fee: "0.0036".into(),
            })
        );
        let prepared: Value = serde_json::from_str(&built.prepared_details).unwrap();
        assert_eq!(prepared["SuiMerge"]["transaction"]["gas_budget"], 3_600_000);
    }
}
