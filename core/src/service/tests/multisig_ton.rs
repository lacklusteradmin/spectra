//! A TON multisig v2 (`ton-multisig.json`, its code the contract's build)
//! against a toncenter whose multisig and order states the test sets: the
//! account as the contract holds it, the order a session builds, and each
//! proposal and approval the contract would refuse, refused before anything
//! is sent. What a proposal and an approval attach is checked in the wallet
//! message sent, the approval against @ton/core's.

use super::*;
use crate::derivation::ton::TonWalletVersion;
use crate::store::secret_backends::InMemorySecretStore;
use crate::store::wallet_secrets::store_seed_phrase;
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;
use std::sync::Mutex;
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

const TON: u64 = 1_000_000_000;

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/ton-multisig.json")).unwrap()
}

fn mnemonic(index: usize) -> (String, [u8; 32]) {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/ton-mnemonics.json")).unwrap();
    let entry = &fixture["mnemonics"][index];
    (
        entry["mnemonic"].as_str().unwrap().into(),
        hex::decode(entry["public_key"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap(),
    )
}

fn boc(value: &Value) -> Cell {
    cell_from_boc(&hex::decode(value["boc_hex"].as_str().unwrap()).unwrap()).unwrap()
}

fn account(address: &str) -> TonAccount {
    account_of(address).unwrap()
}

/// An order deployed at its number: its approvals by index, whether it was
/// sent for execution, and what it carries.
struct Order {
    approvals: Vec<u8>,
    sent: bool,
    expiration: u64,
    order: Cell,
}

/// What the loopback toncenter holds: the multisig's balance, its order 5
/// once deployed, and every external message sent, base64.
struct Toncenter {
    balance: u64,
    order: Option<Order>,
    sent: Vec<String>,
}

/// Order 5's data as the Order contract keeps it.
fn order_data(order: &Order) -> Cell {
    let multisig = account(fixture()["multisig"]["raw"].as_str().unwrap());
    let mask = order
        .approvals
        .iter()
        .fold(0u64, |mask, index| mask | 1 << index);
    let mut data = Cell::default();
    data.address(multisig.0, &multisig.1).unwrap();
    for word in [0, 0, 0, 5] {
        data.uint(word, 64).unwrap();
    }
    data.uint(2, 8)
        .unwrap()
        .uint(u64::from(order.sent), 1)
        .unwrap()
        .reference(boc(&fixture()["multisig"]["signers_cell"]))
        .unwrap();
    for word in [0, 0, 0, mask] {
        data.uint(word, 64).unwrap();
    }
    data.uint(order.approvals.len() as u64, 8)
        .unwrap()
        .uint(order.expiration, 48)
        .unwrap()
        .reference(order.order.clone())
        .unwrap();
    data
}

async fn toncenter() -> (MockServer, Arc<Mutex<Toncenter>>) {
    let live = Arc::new(Mutex::new(Toncenter {
        balance: 3 * TON,
        order: None,
        sent: Vec::new(),
    }));
    let state = live.clone();
    let fixture = fixture();
    let multisig = account(fixture["multisig"]["raw"].as_str().unwrap());
    let order_address = ton::order_address(&multisig, 5).unwrap();
    let code = STANDARD.encode(include_bytes!(
        "../../../tests/fixtures/ton-multisig-code.boc"
    ));
    let data = STANDARD.encode(
        hex::decode(
            fixture["multisig"]["data_at_next_order_seqno"]["boc_hex"]
                .as_str()
                .unwrap(),
        )
        .unwrap(),
    );
    let (root_hash, file_hash) = Chain::Ton.ton_zero_state().unwrap();
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let mut live = state.lock().unwrap();
            let query: HashMap<String, String> = request.url.query_pairs().into_owned().collect();
            let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            let address = query
                .get("address")
                .or(body["address"].as_str().map(str::to_string).as_ref())
                .map(|address| account(address));
            let information = |address: TonAccount| {
                if address == multisig {
                    json!({"state": "active", "balance": live.balance.to_string(),
                        "code": code, "data": data})
                } else if address == order_address {
                    match &live.order {
                        Some(order) => json!({"state": "active", "balance": "0", "code": "",
                            "data": STANDARD.encode(order_data(order).to_boc().unwrap())}),
                        None => json!({"state": "uninitialized", "balance": "0",
                            "code": "", "data": ""}),
                    }
                } else {
                    // A signer's wallet.
                    json!({"state": "active", "balance": (2 * TON).to_string(),
                        "code": "", "data": ""})
                }
            };
            let result = match request.url.path() {
                "/getMasterchainInfo" => json!({"init": {"workchain": -1, "seqno": 0,
                    "root_hash": root_hash, "file_hash": file_hash}}),
                "/getAddressInformation" => information(address.unwrap()),
                "/getAddressBalance" => information(address.unwrap())["balance"].clone(),
                "/runGetMethod" => {
                    assert_eq!(body["method"], "seqno");
                    json!({"exit_code": 0, "stack": [["num", "0x4"]]})
                }
                "/sendBocReturnHash" => {
                    live.sent.push(body["boc"].as_str().unwrap().into());
                    json!({"hash": STANDARD.encode([live.sent.len() as u8; 32])})
                }
                other => panic!("unexpected {other}"),
            };
            ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": result}))
        })
        .mount(&server)
        .await;
    (server, live)
}

/// The watched multisig; signers 0 and 2's W5 wallets; and a v4R2 wallet
/// of signer 1's key, which is not the signer's account.
async fn wallets() -> (Arc<WalletService>, MockServer, Arc<Mutex<Toncenter>>) {
    let (server, live) = toncenter().await;
    let service = WalletService::new(vec![ChainEndpoints {
        capabilities: EndpointCapability::ALL.to_vec(),
        chain_id: Chain::Ton,
        endpoints: vec![server.uri()],
    }])
    .unwrap();
    let db = std::env::temp_dir().join(format!(
        "ton-multisig-{}.sqlite",
        crate::store::new_event_id()
    ));
    service
        .open_state(db.to_string_lossy().into())
        .await
        .unwrap();
    let secrets = Arc::new(InMemorySecretStore::new());
    service.set_secret_store(secrets.clone());
    let fixture = fixture();
    let mut wallets = vec![(
        WalletState::single_address(
            "vault",
            "Vault",
            Chain::Ton,
            fixture["multisig"]["address"].as_str().unwrap(),
            None,
            true,
        ),
        None,
    )];
    for (id, index, version) in [
        ("t0", 0, TonWalletVersion::W5),
        ("t2", 2, TonWalletVersion::W5),
        ("outsider", 1, TonWalletVersion::V4R2),
    ] {
        let (phrase, public) = mnemonic(index);
        let address = version.address(&public, Chain::Ton).unwrap();
        assert_eq!(
            account(&address) == account(fixture["signers"][index]["raw"].as_str().unwrap()),
            version == TonWalletVersion::W5
        );
        wallets.push((
            WalletState::single_address(id, id, Chain::Ton, address, None, false),
            Some(phrase),
        ));
    }
    for (wallet, phrase) in wallets {
        if let Some(phrase) = phrase {
            store_seed_phrase(&*secrets, &wallet.id, &phrase, None).unwrap();
        }
        service
            .apply_state_command(StateCommand::UpsertWallet { wallet })
            .await
            .unwrap();
    }
    (service, server, live)
}

fn destination() -> TonAccount {
    (0, [0x55; 32])
}

fn spend(tons: u64) -> MultisigSpend {
    MultisigSpend {
        to_address: friendly_address(0, &destination().1, false),
        amount: tons.to_string(),
        fee_rate: None,
        expires_in_secs: None,
        memo: None,
    }
}

/// The internal message an external W5 message carries: its destination,
/// bounce flag, value, body opcode, and its cell's hash.
fn carried(external: &str) -> (TonAccount, bool, u128, u64, String) {
    let external = cell_from_boc(&STANDARD.decode(external).unwrap()).unwrap();
    let body = external.reader().reference().unwrap();
    let actions = body.reader().reference().unwrap();
    let mut list = actions.reader();
    list.reference().unwrap();
    let message = list.reference().unwrap();
    let mut read = message.reader();
    assert_eq!(read.uint(1).unwrap(), 0, "an internal message");
    read.bit().unwrap();
    let bounce = read.bit().unwrap();
    read.bit().unwrap();
    assert_eq!(read.uint(2).unwrap(), 0);
    let destination = read.address().unwrap();
    let value = read.coins().unwrap();
    assert!(!read.bit().unwrap(), "no other currencies");
    read.coins().unwrap();
    read.coins().unwrap();
    read.uint(64).unwrap();
    read.uint(32).unwrap();
    assert!(!read.bit().unwrap(), "no state init");
    let op = match read.bit().unwrap() {
        true => read.reference().unwrap().reader().uint(32).unwrap(),
        false => read.uint(32).unwrap(),
    };
    (
        destination,
        bounce,
        value,
        op,
        hex::encode(message.hash_depth().0),
    )
}

async fn sign(
    service: &WalletService,
    session: &MultisigSession,
    signer: &str,
) -> Result<MultisigSession, String> {
    service
        .sign_multisig(
            session.id.clone(),
            session.review_digest.clone(),
            Some(signer.into()),
            None,
        )
        .await
        .map_err(|error| error.to_string())
}

#[tokio::test]
async fn the_contracts_signers_read_each_named_by_its_own_wallet_here() {
    let (service, _server, _) = wallets().await;
    let account = service.multisig_account("vault".into()).await.unwrap();
    let fixture = fixture();
    assert_eq!(account.scheme, MultisigScheme::TonMultisig);
    let [permission] = account.permissions.as_slice() else {
        panic!("{account:?}");
    };
    assert_eq!(permission.threshold, 2);
    // Signer 1's key is held here, but in another wallet version: that
    // wallet is not the signer's account.
    assert_eq!(
        permission
            .signers
            .iter()
            .map(|signer| (signer.signer.clone(), signer.wallet_id.clone()))
            .collect::<Vec<_>>(),
        [
            (
                fixture["signers"][0]["address"]
                    .as_str()
                    .unwrap()
                    .to_string(),
                Some("t0".into())
            ),
            (
                fixture["signers"][1]["address"]
                    .as_str()
                    .unwrap()
                    .to_string(),
                None
            ),
            (
                fixture["signers"][2]["address"]
                    .as_str()
                    .unwrap()
                    .to_string(),
                Some("t2".into())
            ),
        ]
    );
    assert_eq!(account.signer_wallet_ids, ["t0", "t2"]);
    let [warning] = account.warnings.as_slice() else {
        panic!("{account:?}");
    };
    assert!(warning.to_string().starts_with("Proposers"));
}

#[tokio::test]
async fn an_order_is_built_only_within_what_the_multisig_holds_and_awaits_its_proposal() {
    let (service, _server, live) = wallets().await;
    let refusal = service
        .create_multisig("vault".into(), spend(4))
        .await
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("cannot pay this"), "{refusal}");
    let brief = MultisigSpend {
        expires_in_secs: Some(60),
        ..spend(1)
    };
    let refusal = service
        .create_multisig("vault".into(), brief)
        .await
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("ten minutes to a year"), "{refusal}");
    assert!(
        service
            .multisig_sessions("vault".into())
            .await
            .unwrap()
            .is_empty()
    );

    let created = service
        .create_multisig("vault".into(), spend(1))
        .await
        .unwrap();
    // No number until proposed; the destination's own bounce flag; a week to
    // gather approvals.
    assert_eq!(created.sequence, None);
    let [output] = created.outputs.as_slice() else {
        panic!("{created:?}");
    };
    assert_eq!(output.address, friendly_address(0, &destination().1, false));
    assert_eq!(output.value, TON.to_string());
    let now = crate::store::now_unix() as u64;
    let expires = created.expires_at.unwrap();
    assert!(
        (now + 7 * 86_400 - 10..=now + 7 * 86_400).contains(&expires),
        "{expires}"
    );
    assert_eq!((created.threshold, created.signed_weight), (2, 0));
    assert!(!created.complete && created.submitted_txid.is_none());
    assert!(live.lock().unwrap().sent.is_empty());
}

#[tokio::test]
async fn a_signer_proposes_and_approves_only_what_the_contract_would_take() {
    let (service, _server, live) = wallets().await;
    let fixture = fixture();
    let multisig = account(fixture["multisig"]["raw"].as_str().unwrap());
    let created = service
        .create_multisig("vault".into(), spend(1))
        .await
        .unwrap();
    let refusal = sign(&service, &created, "outsider").await.unwrap_err();
    assert!(
        refusal.contains("not one of the multisig's signers"),
        "{refusal}"
    );
    assert!(live.lock().unwrap().sent.is_empty());

    // Signer 0 proposes order 5 to the multisig, attaching what deploying
    // and executing it costs.
    let proposed = sign(&service, &created, "t0").await.unwrap();
    assert_eq!(proposed.sequence.as_deref(), Some("5"));
    let order_address = ton::order_address(&multisig, 5).unwrap();
    assert_eq!(
        proposed.transaction_id,
        friendly_address(order_address.0, &order_address.1, true)
    );
    assert_eq!(proposed.signed_weight, 1);
    assert!(!proposed.complete && proposed.submitted_txid.is_none());
    let [proposal] = live.lock().unwrap().sent.clone().try_into().unwrap();
    let (to, bounce, value, op, _) = carried(&proposal);
    assert_eq!(
        (to, bounce, value, op),
        (multisig, true, 200_000_000, 0xf718_510f)
    );

    // Until the multisig deploys it, there is nothing to approve.
    let refusal = sign(&service, &created, "t2").await.unwrap_err();
    assert!(refusal.contains("not on the network yet"), "{refusal}");
    let shared: Value = serde_json::from_str(&proposed.data).unwrap();
    let order =
        cell_from_boc(&STANDARD.decode(shared["order"].as_str().unwrap()).unwrap()).unwrap();
    live.lock().unwrap().order = Some(Order {
        approvals: vec![0],
        sent: false,
        expiration: proposed.expires_at.unwrap(),
        order,
    });
    let refusal = sign(&service, &created, "t0").await.unwrap_err();
    assert!(refusal.contains("already approved"), "{refusal}");
    for refusal in [
        service
            .submit_multisig(created.id.clone(), None, None)
            .await
            .unwrap_err(),
        service
            .finalize_multisig(created.id.clone())
            .await
            .unwrap_err(),
    ] {
        assert!(
            refusal
                .to_string()
                .contains("goes to the network as it signs")
        );
    }
    live.lock().unwrap().order.as_mut().unwrap().sent = true;
    let refusal = sign(&service, &created, "t2").await.unwrap_err();
    assert!(refusal.contains("already executed"), "{refusal}");
    assert_eq!(live.lock().unwrap().sent.len(), 1);

    // Signer 2's approval meets the threshold: @ton/core's message to the
    // order, and the session is done.
    live.lock().unwrap().order.as_mut().unwrap().sent = false;
    let approved = sign(&service, &created, "t2").await.unwrap();
    assert!(approved.complete && approved.submitted_txid.is_some());
    let [_, approval] = live.lock().unwrap().sent.clone().try_into().unwrap();
    let (to, bounce, value, op, hash) = carried(&approval);
    assert_eq!(
        (to, bounce, value, op),
        (order_address, true, 100_000_000, 0xa762_230f)
    );
    assert_eq!(
        hash,
        fixture["wallet_messages"]["approve"]["hash"]
            .as_str()
            .unwrap()
    );
}
