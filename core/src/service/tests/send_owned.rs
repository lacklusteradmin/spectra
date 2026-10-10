//! Sends from a wallet the user holds, judged against every wallet and
//! contact they hold: which destinations are their own, what an owned build
//! keeps of its review, who each end of a transfer is, and what a
//! replacement of a pending send pays.
use super::*;
use crate::send::stages::PreparedPayload;
use crate::service::history_query::{EndpointHolder, TransactionEndpoint, TransactionEndpoints};
use crate::store::persistence_models::TransactionRecord;
use crate::store::state::WalletState;
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

const SOURCE: &str = "0x1111111111111111111111111111111111111111";
const OTHER: &str = "0x2222222222222222222222222222222222222222";
const CONTACT: &str = "0x3333333333333333333333333333333333333333";
const HOLDING: &str = "ethereum:native";

/// An Ethereum node that answers every read an owned send makes. A pending
/// transaction it is asked about has nonce 7.
async fn node() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(|request: &Request| {
            let body: Value = request.body_json().unwrap();
            let answer = |call: &Value| {
                let result = match call["method"].as_str().unwrap() {
                    "eth_chainId" => json!("0x1"),
                    "eth_getTransactionCount" => json!("0x7"),
                    "eth_getBalance" => json!("0x8ac7230489e80000"),
                    "eth_estimateGas" => json!("0x5208"),
                    "eth_getCode" => json!("0x"),
                    "eth_feeHistory" => {
                        json!({"baseFeePerGas":["0x3b9aca00"],"reward":[["0x77359400"]]})
                    }
                    "eth_getTransactionByHash" => json!({"nonce": "0x7"}),
                    other => panic!("unexpected RPC {other}"),
                };
                json!({"jsonrpc":"2.0","id":call["id"],"result":result})
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

/// Two watched Ethereum wallets, "Source" holding ten ether and "Other",
/// and an Arbitrum wallet "Elsewhere" at `elsewhere`.
async fn wallets(endpoints: Vec<String>, elsewhere: &str) -> Arc<WalletService> {
    let service = WalletService::new(vec![ChainEndpoints {
        capabilities: EndpointCapability::ALL.to_vec(),
        chain_id: Chain::Ethereum,
        endpoints,
    }])
    .unwrap();
    let database = std::env::temp_dir()
        .join(format!(
            "owned-send-{}.sqlite",
            crate::store::new_event_id()
        ))
        .to_string_lossy()
        .into_owned();
    service.open_state(database).await.unwrap();
    let mut source =
        WalletState::single_address("source", "Source", Chain::Ethereum, SOURCE, None, true);
    let mut ether = native_coin_template(Chain::Ethereum).unwrap();
    ether.amount = "10".into();
    source.holdings.push(ether);
    for wallet in [
        source,
        WalletState::single_address("other", "Other", Chain::Ethereum, OTHER, None, true),
        WalletState::single_address(
            "elsewhere",
            "Elsewhere",
            Chain::Arbitrum,
            elsewhere,
            None,
            true,
        ),
    ] {
        service
            .apply_state_command(StateCommand::UpsertWallet { wallet })
            .await
            .unwrap();
    }
    service
}

fn input(destination: &str, amount: &str) -> super::super::send_review::SendReviewInput {
    super::super::send_review::SendReviewInput {
        wallet_id: "source".into(),
        holding_key: HOLDING.into(),
        amount: amount.into(),
        destination: destination.into(),
        overrides: None,
        memo: None,
    }
}

/// A pending send of `amount` ether from Source to `address`, as a
/// broadcast stores it.
fn pending_send(id: &str, address: &str, amount: &str, hash: &str) -> TransactionRecord {
    serde_json::from_value(json!({
        "id": id, "walletId": "source", "kind": "send", "status": "pending",
        "walletName": "Source", "assetDisplayName": "Ether", "symbol": "ETH",
        "chainId": "ethereum", "deploymentId": HOLDING, "amount": amount,
        "address": address, "transactionHash": hash, "createdAtUnix": 1234.0
    }))
    .unwrap()
}

/// A send to any of the user's wallets on the network is to their own
/// address, in whatever case it is typed; a stranger's is not.
#[tokio::test]
async fn another_of_the_users_wallets_is_an_own_destination() {
    let service = wallets(vec![], &format!("0x{}", "44".repeat(20))).await;
    let own = |destination: String| {
        let service = service.clone();
        async move {
            service
                .is_own_send_destination("source".into(), HOLDING.into(), destination)
                .await
                .unwrap()
        }
    };
    assert!(own(SOURCE.into()).await);
    assert!(own(OTHER.into()).await);
    assert!(own(OTHER.to_uppercase().replacen("0X", "0x", 1)).await);
    assert!(!own(CONTACT.into()).await);
}

/// An owned build keeps what its review said: the self-send confirmation,
/// the warnings, and the network fee the quote showed, which bounds the
/// transaction it stores.
#[tokio::test]
async fn an_owned_build_keeps_its_reviews_advisories_and_fee() {
    let server = node().await;
    let service = wallets(vec![server.uri()], &format!("0x{}", "44".repeat(20))).await;
    let quote = service.review_owned_send(input(OTHER, "1")).await.unwrap();
    assert!(quote.requires_self_send_confirmation);
    let quoted_fee = quote.preview.as_ref().unwrap().network_fee().to_string();
    let built = service.build_owned_send(input(OTHER, "1")).await.unwrap();
    assert!(built.review.requires_self_send_confirmation);
    assert_eq!(
        serde_json::to_value(&built.review.warnings).unwrap(),
        serde_json::to_value(&quote.warnings).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&built.review.recipient_warnings).unwrap(),
        serde_json::to_value(&quote.recipient_warnings).unwrap()
    );
    assert_eq!(
        built.review.network_fee.as_deref(),
        Some(quoted_fee.as_str())
    );
    let PreparedPayload::Evm(prepared) =
        service.load_send_artifact(built.id).await.unwrap().prepared
    else {
        panic!("not an EVM transaction")
    };
    assert_eq!(prepared.nonce, 7);
    assert_eq!(
        crate::decimal::from_units(prepared.maximum_fee_wei().unwrap(), 18),
        quoted_fee
    );
}

/// Each end of a stored transfer, and an address about to be sent to, is
/// named by who holds it: the sending wallet, another of the user's wallets
/// on the network, or a saved contact. A wallet on another network is not
/// named, though it shares the address.
#[tokio::test]
async fn transfer_ends_and_destinations_are_named_by_their_holder() {
    let elsewhere = format!("0x{}", "44".repeat(20));
    let service = wallets(vec![], &elsewhere).await;
    service
        .apply_state_command(StateCommand::AddAddressBookEntry {
            name: "Alice".into(),
            chain_id: Chain::Ethereum,
            address: CONTACT.into(),
            note: String::new(),
        })
        .await
        .unwrap();
    service
        .apply_transaction_command(TransactionCommand::Upsert {
            records: vec![
                pending_send("to-other", OTHER, "1", &format!("0x{}", "aa".repeat(32))),
                pending_send("to-alice", CONTACT, "1", &format!("0x{}", "bb".repeat(32))),
            ],
        })
        .await
        .unwrap();
    let wallet = |name: &str| Some(EndpointHolder::Wallet { name: name.into() });
    let alice = Some(EndpointHolder::Contact {
        name: "Alice".into(),
    });
    assert_eq!(
        service
            .transaction_endpoints("to-other".into())
            .await
            .unwrap(),
        Some(TransactionEndpoints {
            from: None,
            to: Some(TransactionEndpoint {
                address: OTHER.into(),
                is_mine: false,
                holder: wallet("Other"),
            }),
        })
    );
    assert_eq!(
        service
            .transaction_endpoints("to-alice".into())
            .await
            .unwrap()
            .and_then(|ends| ends.to)
            .and_then(|to| to.holder),
        alice
    );
    let holder = |address: String| {
        let service = service.clone();
        async move {
            service
                .address_holder("source".into(), Chain::Ethereum, address)
                .await
                .unwrap()
        }
    };
    assert_eq!(holder(SOURCE.into()).await, wallet("Source"));
    assert_eq!(
        holder(OTHER.to_uppercase().replacen("0X", "0x", 1)).await,
        wallet("Other")
    );
    assert_eq!(holder(CONTACT.into()).await, alice);
    assert_eq!(holder(elsewhere).await, None);
}

/// A replacement re-sends the pending transfer's exact amount to its
/// recipient at its nonce; a cancellation sends nothing, to the wallet
/// itself.
#[tokio::test]
async fn a_replacement_resends_the_exact_amount_and_a_cancellation_sends_nothing_home() {
    let server = node().await;
    let service = wallets(vec![server.uri()], &format!("0x{}", "44".repeat(20))).await;
    service
        .apply_transaction_command(TransactionCommand::Upsert {
            records: vec![pending_send(
                "pending",
                OTHER,
                "0.123456789012",
                &format!("0x{}", "aa".repeat(32)),
            )],
        })
        .await
        .unwrap();
    let speed_up = service
        .replacement_draft("pending".into(), false)
        .await
        .unwrap();
    assert_eq!(
        (
            speed_up.wallet_id.as_str(),
            speed_up.holding_key.as_str(),
            speed_up.destination.as_str(),
            speed_up.amount.as_str(),
            speed_up.nonce,
        ),
        ("source", HOLDING, OTHER, "0.123456789012", 7)
    );
    let cancel = service
        .replacement_draft("pending".into(), true)
        .await
        .unwrap();
    assert_eq!(
        (
            cancel.destination.as_str(),
            cancel.amount.as_str(),
            cancel.nonce
        ),
        (SOURCE, "0", 7)
    );
}
