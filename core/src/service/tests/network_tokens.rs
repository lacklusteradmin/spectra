//! Token holdings read off one listing of what an address holds.
use super::*;
use crate::service::send_stage_protocols::send_stage_support::{Secret, Wallet, node};
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
        "Cardano Native Token".to_string(),
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
                standard: String::new(),
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
