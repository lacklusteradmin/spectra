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
    let service = WalletService::serving(vec![ChainEndpoints {
        capabilities: EndpointCapability::ALL.to_vec(),
        chain_id: Chain::Ethereum,
        endpoints,
    }])
    .unwrap();
    let database = std::env::temp_dir()
        .join(format!("owned-send-{}.sqlite", crate::store::new_id()))
        .to_string_lossy()
        .into_owned();
    service.opened(database).await.unwrap();
    let mut source =
        WalletState::single_address("source", "Source", Chain::Ethereum, SOURCE, None, true);
    let mut ether = Chain::Ethereum.native_holding_template();
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
        service.upsert_wallet(wallet).await.unwrap();
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
    let quoted_fee = quote
        .preview
        .as_ref()
        .unwrap()
        .estimated_network_fee
        .clone();
    let built = service.build_owned_send(input(OTHER, "1")).await.unwrap();
    let reviewed = crate::send::stages::SendArtifactReview::of_transfer(
        quote.warnings.clone(),
        quote.recipient_warnings.clone(),
        true,
    );
    assert!(
        built
            .review
            .warnings
            .contains(&crate::send::stages::ReviewWarning::OwnAddress)
    );
    assert_eq!(built.review.warnings, reviewed.warnings);
    let fee = built.review.network_fee.clone().unwrap();
    assert_eq!(
        (fee.amount.as_str(), fee.bound),
        (quoted_fee.as_str(), crate::send::stages::FeeBound::Estimate)
    );
    let PreparedPayload::Evm(prepared) =
        service.load_send_artifact(built.id).await.unwrap().prepared
    else {
        panic!("not an EVM transaction")
    };
    assert_eq!(prepared.nonce, 7);
    assert_eq!(
        crate::amount::decimal::from_units(prepared.maximum_fee_wei().unwrap(), 18),
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
        .upsert_history_records(vec![
            pending_send("to-other", OTHER, "1", &format!("0x{}", "aa".repeat(32))),
            pending_send("to-alice", CONTACT, "1", &format!("0x{}", "bb".repeat(32))),
        ])
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
        .upsert_history_records(vec![pending_send(
            "pending",
            OTHER,
            "0.123456789012",
            &format!("0x{}", "aa".repeat(32)),
        )])
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

mod shortcut_amount_tests {
    use super::{send_total, shortcut_amount};

    #[test]
    fn shares_are_cut_to_display_precision_and_the_maximum_is_exact() {
        let exact = "0.012490233065563998".to_string();
        assert_eq!(shortcut_amount(exact.clone(), 25, 18), "0.0124902");
        assert_eq!(shortcut_amount(exact.clone(), 100, 18), exact);
        // Dust has no display form; it stays exact rather than become zero.
        let dust = "0.000000000000000001".to_string();
        assert_eq!(shortcut_amount(dust.clone(), 25, 18), dust);
    }

    /// A coin's total is what leaves the wallet, exactly; a token's fee is
    /// another asset's, so it has none, and nor does an unknown fee.
    #[test]
    fn a_total_adds_the_fee_only_where_the_coin_pays_it() {
        assert_eq!(
            send_total(true, "0.001", Some("0.000021")),
            Some("0.001021".to_string())
        );
        assert_eq!(
            send_total(true, "1", Some("0.000000000000000001")),
            Some("1.000000000000000001".to_string())
        );
        assert_eq!(send_total(false, "20", Some("0.000021")), None);
        assert_eq!(send_total(true, "0.001", None), None);
    }
}

mod provisional_amount_tests {
    use super::provisional_amount;

    /// A tenth of the holding, cut to its precision; a holding too small for
    /// a tenth to count is quoted at its smallest unit.
    #[test]
    fn an_empty_amount_is_quoted_for_a_tenth_of_the_holding() {
        assert_eq!(provisional_amount("2.5", 8), "0.25");
        assert_eq!(provisional_amount("0.123456789", 8), "0.01234567");
        assert_eq!(provisional_amount("0.00000009", 8), "0.00000001");
        assert_eq!(provisional_amount("0", 6), "0.000001");
        assert_eq!(provisional_amount("7", 0), "1");
    }
}

mod watch_only_preview {
    //! Exercise app-facing adapters through protocol fixtures, not their decoders.
    use super::*;
    use wiremock::matchers::{body_partial_json, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn service(chain: crate::registry::Chain, server: &MockServer) -> Arc<WalletService> {
        WalletService::serving(vec![ChainEndpoints {
            capabilities: EndpointCapability::ALL.to_vec(),
            chain_id: chain,
            endpoints: vec![server.uri()],
        }])
        .unwrap()
    }

    #[tokio::test]
    async fn owned_non_evm_preview_needs_only_stored_watch_address_and_valid_input() {
        let server = MockServer::start().await;
        let svc = service(crate::registry::Chain::Solana, &server);
        let address = "11111111111111111111111111111111";
        let mut wallet = crate::store::state::WalletState::single_address(
            "watch",
            "Watch",
            crate::registry::Chain::Solana,
            address,
            None,
            true,
        );
        wallet
            .holdings
            .push(crate::store::wallet_domain::AssetHolding {
                name: "Solana".into(),
                symbol: "SOL".into(),
                chain_id: crate::registry::Chain::Solana,
                token_standard: None,
                amount: "2".into(),
                id: String::new(),
                coingecko_id: String::new(),
                contract_address: None,
            });
        svc.wallet_state.write().await.wallets.push(wallet);
        for (amount, nonce) in [("0.0000000001", None), ("NaN", None), ("1", Some(1))] {
            assert!(
                svc.preview_owned_send(
                    "watch".into(),
                    "solana:native".into(),
                    amount.into(),
                    "".into(),
                    nonce,
                    None
                )
                .await
                .is_err()
            );
        }
        assert!(server.received_requests().await.unwrap().is_empty());
        Mock::given(method("POST"))
            .and(body_partial_json(
                json!({"method":"getBalance","params":[address]}),
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"jsonrpc":"2.0","id":1,"result":{"value":2_000_000_000u64}}),
                ),
            )
            .mount(&server)
            .await;
        let result = svc
            .preview_owned_send(
                "watch".into(),
                "solana:native".into(),
                "1".into(),
                "".into(),
                None,
                None,
            )
            .await
            .unwrap();
        let quote = result.expect("quote");
        assert!(quote.shortcuts.contains_key(&100));
        assert_eq!(quote.chain_id, crate::registry::Chain::Solana);
        assert_eq!(quote.preview.max_sendable, "1.999995");
        svc.wallet_state.write().await.wallets[0].address = None;
        let count = server.received_requests().await.unwrap().len();
        assert!(
            svc.preview_owned_send(
                "watch".into(),
                "solana:native".into(),
                "1".into(),
                "".into(),
                None,
                None
            )
            .await
            .is_err()
        );
        assert_eq!(server.received_requests().await.unwrap().len(), count);
    }
}
