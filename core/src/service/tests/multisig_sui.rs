//! A Sui multisig account watched from its policy (`sui-multisig.json`): the
//! wallets here that hold its members' keys, and the gas a session pays with,
//! read again before a signature and before execution.

use super::*;
use crate::store::secret_backends::InMemorySecretStore;
use crate::store::wallet_secrets::store_seed_phrase;
use serde_json::Value;
use std::sync::Mutex;
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

const OUTSIDER: &str = "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong";

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/sui-multisig.json")).unwrap()
}

fn policy() -> String {
    let fixture = fixture();
    let keys = fixture["keys"].as_array().unwrap();
    json!({"threshold": fixture["multisig"]["threshold"], "publicKeys": keys
        .iter()
        .map(|key| json!({"publicKey": key["sui_public_key"], "weight": key["weight"]}))
        .collect::<Vec<_>>()})
    .to_string()
}

/// What the loopback node holds: the version of the account's one gas coin,
/// and every method asked.
struct Node {
    version: &'static str,
    methods: Vec<String>,
}

async fn node() -> (MockServer, Arc<Mutex<Node>>) {
    let live = Arc::new(Mutex::new(Node {
        version: "7",
        methods: Vec::new(),
    }));
    let state = live.clone();
    let account = fixture()["multisig"]["address"].clone();
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let mut live = state.lock().unwrap();
            let method = body["method"].as_str().unwrap().to_string();
            live.methods.push(method.clone());
            let result = match method.as_str() {
                "sui_getChainIdentifier" => json!(Chain::Sui.sui_network_identity().unwrap().0),
                "sui_getCheckpoint" => json!({"sequenceNumber": "0",
                    "digest": Chain::Sui.sui_network_identity().unwrap().1}),
                "suix_getReferenceGasPrice" => json!("1000"),
                "suix_getCoins" => {
                    assert_eq!(body["params"][0], account);
                    json!({"data": [{"coinObjectId": format!("0x{}", "33".repeat(32)),
                        "version": live.version, "digest": "1".repeat(32), "balance": "200000000"}],
                        "hasNextPage": false, "nextCursor": null})
                }
                other => panic!("unexpected {other}"),
            };
            ResponseTemplate::new(200)
                .set_body_json(json!({"jsonrpc": "2.0", "id": body["id"], "result": result}))
        })
        .mount(&server)
        .await;
    (server, live)
}

/// The watched account; `signer0`, holding its Ed25519 member's phrase; a
/// wallet whose phrase holds none of its keys; and a watched wallet at its
/// secp256k1 member's own address.
async fn wallets(endpoint: String) -> Arc<WalletService> {
    let service = WalletService::new(vec![ChainEndpoints {
        capabilities: EndpointCapability::ALL.to_vec(),
        chain_id: Chain::Sui,
        endpoints: vec![endpoint],
    }])
    .unwrap();
    let db = std::env::temp_dir().join(format!(
        "sui-multisig-{}.sqlite",
        crate::store::new_event_id()
    ));
    service
        .open_state(db.to_string_lossy().into())
        .await
        .unwrap();
    let secrets = Arc::new(InMemorySecretStore::new());
    service.set_secret_store(secrets.clone());
    let fixture = fixture();
    let mut vault = WalletState::single_address(
        "vault",
        "Vault",
        Chain::Sui,
        fixture["multisig"]["address"].as_str().unwrap(),
        None,
        true,
    );
    vault.multisig_policy = Some(policy());
    let path = fixture["keys"][0]["path"].as_str().unwrap();
    let mut wallets = vec![
        vault,
        WalletState::single_address(
            "watched-k1",
            "Watched",
            Chain::Sui,
            fixture["keys"][1]["address"].as_str().unwrap(),
            None,
            true,
        ),
    ];
    for (id, phrase) in [
        ("signer0", fixture["keys"][0]["phrase"].as_str().unwrap()),
        ("outsider", OUTSIDER),
    ] {
        let address = crate::derivation::dispatch::derive_for_chain(
            Chain::Sui,
            phrase,
            path,
            None,
            None,
            None,
            true,
            false,
            false,
        )
        .unwrap()
        .address
        .unwrap();
        wallets.push(WalletState::single_address(
            id,
            id,
            Chain::Sui,
            address,
            Some(path.into()),
            false,
        ));
        store_seed_phrase(&*secrets, id, phrase, None).unwrap();
    }
    assert_eq!(
        wallets[2].primary_address(),
        fixture["keys"][0]["address"].as_str()
    );
    for wallet in wallets {
        service
            .apply_state_command(StateCommand::UpsertWallet { wallet })
            .await
            .unwrap();
    }
    service
}

#[tokio::test]
async fn a_member_key_a_wallet_here_holds_is_named_by_that_wallet() {
    let (server, _) = node().await;
    let service = wallets(server.uri()).await;
    let account = service.multisig_account("vault".into()).await.unwrap();
    let [permission] = account.permissions.as_slice() else {
        panic!("{account:?}");
    };
    // Only a wallet with keys at the member's own address signs as it: not
    // a watched one, nor one holding another phrase.
    assert_eq!(
        permission
            .signers
            .iter()
            .map(|signer| (signer.weight, signer.wallet_id.as_deref()))
            .collect::<Vec<_>>(),
        [(1, Some("signer0")), (1, None), (2, None)]
    );
    assert_eq!(account.signer_wallet_ids, ["signer0"]);
}

#[tokio::test]
async fn a_transfer_whose_gas_coin_changed_is_neither_signed_nor_executed() {
    let (server, live) = node().await;
    let service = wallets(server.uri()).await;
    let fixture = fixture();
    let created = service
        .create_multisig(
            "vault".into(),
            MultisigSpend {
                to_address: fixture["transaction"]["recipient"].as_str().unwrap().into(),
                amount: "0.001".into(),
                fee_rate: None,
                expires_in_secs: None,
                memo: None,
            },
        )
        .await
        .unwrap();
    let sign = || {
        service.sign_multisig(
            created.id.clone(),
            created.review_digest.clone(),
            Some("signer0".into()),
            None,
        )
    };
    live.lock().unwrap().version = "8";
    let refusal = sign().await.unwrap_err().to_string();
    assert!(refusal.contains("gas object"), "{refusal}");
    assert_eq!(
        service
            .multisig_session(created.id.clone())
            .await
            .unwrap()
            .signed_weight,
        0
    );
    live.lock().unwrap().version = "7";
    sign().await.unwrap();
    // The secp256k1 member's signature, made by the SDK, completes it.
    let joined = service
        .import_multisig(
            "vault".into(),
            json!({"transaction": fixture["transaction"]["raw_base64"],
                "signatures": [fixture["partial_signatures"][1]["signature"]]})
            .to_string(),
        )
        .await
        .unwrap();
    assert_eq!(joined.id, created.id);
    assert!(joined.complete);
    live.lock().unwrap().version = "8";
    let refusal = service
        .submit_multisig(created.id.clone(), None, None)
        .await
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("gas object"), "{refusal}");
    assert!(
        !live
            .lock()
            .unwrap()
            .methods
            .iter()
            .any(|method| method == "sui_executeTransactionBlock")
    );
    let stored = service.multisig_session(created.id).await.unwrap();
    assert_eq!(stored.submitted_txid, None);
}
