//! Each prerequisite the network enforces on closing an account, refused
//! before anything is built.

use super::*;
use crate::api::horizon::StellarMergeableAccount;
use crate::api::xrpl_json_rpc::XrpDeletableAccount;

const DESTINATION: &str = "rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe";

fn xrp_state() -> XrpDeletionState {
    XrpDeletionState {
        source: Some(XrpDeletableAccount {
            balance_drops: 25_000_000,
            sequence: 1_000,
            owner_count: 2,
            minted_nft_sequence: None,
            blockers: 0,
        }),
        destination_flags: Some(0),
        ledger_index: 1_256,
        reserve_base: 1_000_000,
        reserve_increment: 200_000,
    }
}

fn refusal<T: std::fmt::Debug>(result: Result<T, SpectraBridgeError>) -> String {
    result.unwrap_err().to_string()
}

#[test]
fn an_xrp_account_with_nothing_blocking_deletes_for_one_owner_reserve() {
    // Two deletable objects go with it; its reserve is the base and theirs.
    assert_eq!(
        xrp_deletion(&xrp_state(), DESTINATION, false).unwrap(),
        (25_000_000, 1_000, 2, 1_400_000, 200_000)
    );
}

#[test]
fn xrp_refuses_what_the_network_would() {
    let mut missing = xrp_state();
    missing.source = None;
    assert!(refusal(xrp_deletion(&missing, DESTINATION, false)).contains("nothing to close"));

    let mut blocked = xrp_state();
    blocked.source.as_mut().unwrap().blockers = 1;
    assert!(refusal(xrp_deletion(&blocked, DESTINATION, false)).contains("trust lines"));

    let mut too_many = xrp_state();
    too_many.source.as_mut().unwrap().owner_count = 1_001;
    assert!(refusal(xrp_deletion(&too_many, DESTINATION, false)).contains("1001 objects"));

    // Sequence + 256 must not pass the validated ledger, nor must the
    // minted-NFT sequence.
    let mut recent = xrp_state();
    recent.ledger_index = 1_255;
    assert!(refusal(xrp_deletion(&recent, DESTINATION, false)).contains("after ledger 1256"));
    let mut minted = xrp_state();
    minted.source.as_mut().unwrap().minted_nft_sequence = Some(1_100);
    assert!(refusal(xrp_deletion(&minted, DESTINATION, false)).contains("after ledger 1356"));

    let mut absent = xrp_state();
    absent.destination_flags = None;
    assert!(refusal(xrp_deletion(&absent, DESTINATION, false)).contains("not on the network"));
    let mut tagged = xrp_state();
    tagged.destination_flags = Some(LSF_REQUIRE_DEST_TAG);
    assert!(refusal(xrp_deletion(&tagged, DESTINATION, false)).contains("destination tag"));
    // With the tag it asks for, it takes the account.
    assert!(xrp_deletion(&tagged, DESTINATION, true).is_ok());
    let mut authorized = xrp_state();
    authorized.destination_flags = Some(XRP_DEPOSIT_AUTH);
    assert!(refusal(xrp_deletion(&authorized, DESTINATION, false)).contains("authorized"));

    let mut poor = xrp_state();
    poor.source.as_mut().unwrap().balance_drops = 200_000;
    assert!(refusal(xrp_deletion(&poor, DESTINATION, false)).contains("does not cover the fee"));
}

fn stellar_state() -> StellarMergeState {
    StellarMergeState {
        source: Some(StellarMergeableAccount {
            balance_stroops: 50_000_000,
            sequence: 4_294_967_296 * 10 + 5,
            subentries: 0,
            sponsoring: 0,
            sponsored: 0,
            auth_immutable: false,
        }),
        destination_memo_required: Some(false),
        ledger: 11,
        base_reserve: 5_000_000,
    }
}

#[test]
fn a_bare_stellar_account_merges_with_the_next_sequence() {
    assert_eq!(
        stellar_merge(&stellar_state(), DESTINATION, false).unwrap(),
        (50_000_000, 4_294_967_296 * 10 + 6, 10_000_000)
    );
    // A sponsored account's own reserve was its sponsor's.
    let mut sponsored = stellar_state();
    sponsored.source.as_mut().unwrap().sponsored = 2;
    assert_eq!(stellar_merge(&sponsored, DESTINATION, false).unwrap().2, 0);
}

#[test]
fn stellar_refuses_what_the_network_would() {
    let mut missing = stellar_state();
    missing.source = None;
    assert!(refusal(stellar_merge(&missing, DESTINATION, false)).contains("nothing to close"));
    let mut entries = stellar_state();
    entries.source.as_mut().unwrap().subentries = 1;
    assert!(refusal(stellar_merge(&entries, DESTINATION, false)).contains("trust lines"));
    let mut sponsor = stellar_state();
    sponsor.source.as_mut().unwrap().sponsoring = 1;
    assert!(refusal(stellar_merge(&sponsor, DESTINATION, false)).contains("sponsors"));
    let mut immutable = stellar_state();
    immutable.source.as_mut().unwrap().auth_immutable = true;
    assert!(refusal(stellar_merge(&immutable, DESTINATION, false)).contains("immutable"));
    // The merge's sequence must stay below the ledger's starting sequence.
    let mut ahead = stellar_state();
    ahead.ledger = 10;
    assert!(refusal(stellar_merge(&ahead, DESTINATION, false)).contains("later ledger"));
    let mut absent = stellar_state();
    absent.destination_memo_required = None;
    assert!(refusal(stellar_merge(&absent, DESTINATION, false)).contains("not on the network"));
    let mut memo = stellar_state();
    memo.destination_memo_required = Some(true);
    assert!(refusal(stellar_merge(&memo, DESTINATION, false)).contains("memo"));
    // With the memo it asks for, it takes the account.
    assert!(stellar_merge(&memo, DESTINATION, true).is_ok());
}

/// The XRP wallet of the payment fixture's phrase, and an XRPL node where
/// its account and the destination exist and `blockers` stand in the
/// way of deleting it.
async fn xrp_wallet(
    blockers: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
) -> (
    crate::service::loopback_service::OpenService,
    wiremock::MockServer,
    String,
) {
    use serde_json::{Value, json};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};
    let vector: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/xrp-mnemonic-payment.json"
    ))
    .unwrap();
    let sender = vector["transaction"]["Account"]
        .as_str()
        .unwrap()
        .to_string();
    let server = MockServer::start().await;
    let source = sender.clone();
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let params = &body["params"][0];
            let result = match body["method"].as_str().unwrap() {
                "server_info" => json!({"info": {"network_id": 0}}),
                "server_state" => json!({"state": {"validated_ledger": {
                    "seq": 2_000, "reserve_base": 1_000_000, "reserve_inc": 200_000}}}),
                "account_info" if params["account"] == source.as_str() => json!({"account_data": {
                    "Balance": "25000000", "Sequence": 1_000, "OwnerCount": 2, "Flags": 0}}),
                "account_info" if params["account"] == DESTINATION => json!({"account_data": {
                    "Balance": "50000000", "Sequence": 5, "OwnerCount": 0, "Flags": 0}}),
                "account_objects" => {
                    assert_eq!(params["deletion_blockers_only"], true);
                    json!({"account_objects": *blockers.lock().unwrap()})
                }
                other => panic!("unexpected XRPL call {other} {params}"),
            };
            ResponseTemplate::new(200).set_body_json(json!({"result": result}))
        })
        .mount(&server)
        .await;
    let service = crate::service::loopback_service::open().await;
    let mut commit = crate::derivation::setup::tests::fixture(
        Chain::Xrp,
        crate::derivation::setup::WalletSetupMethod::ImportPhrase,
    );
    commit.seed_phrase = Some(vector["mnemonic"].as_str().unwrap().into());
    commit.derivation_path = Some(vector["derivation_path"].as_str().unwrap().into());
    let wallet = service.import(commit).await;
    assert_eq!(service.address(&wallet, Chain::Xrp).await, sender);
    use EndpointCapability::*;
    service
        .use_endpoint(
            Chain::Xrp,
            crate::EndpointApi::XrplJsonRpc,
            &[Balance, Fee, Broadcast, Verification],
            &server.uri(),
        )
        .await;
    (service, server, wallet)
}

/// Signing reads every prerequisite again: an escrow created after the
/// review stops the deletion, which stays unsigned until it is gone.
#[tokio::test]
async fn a_blocker_that_appears_after_the_review_is_refused_at_signing() {
    let blockers = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (service, _server, wallet) = xrp_wallet(blockers.clone()).await;
    let built = service
        .build_account_closing(wallet, DESTINATION.into(), None)
        .await
        .unwrap();
    *blockers.lock().unwrap() = vec![serde_json::json!({"LedgerEntryType": "Escrow"})];
    let refusal = service
        .sign_send(built.id.clone(), built.review_digest.clone(), None)
        .await
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("escrows"), "{refusal}");
    let stored = service.inspect_send(built.id.clone()).await.unwrap();
    assert_eq!(stored.stage, SendStage::Prepared);
    blockers.lock().unwrap().clear();
    let signed = service
        .sign_send(built.id, built.review_digest, None)
        .await
        .unwrap();
    assert_eq!(signed.stage, SendStage::Signed);
}

/// Only XRP and Stellar accounts close, and only from a wallet that signs:
/// another network's account and a watched one are refused before any
/// network is read, and nothing is stored.
#[tokio::test]
async fn other_networks_and_watched_accounts_do_not_close() {
    let service = crate::service::loopback_service::open().await;
    let solana = service
        .import(crate::derivation::setup::tests::fixture(
            Chain::Solana,
            crate::derivation::setup::WalletSetupMethod::ImportPhrase,
        ))
        .await;
    let refused = service
        .build_account_closing(
            solana,
            "BLeUXTx9thHGT7VJUtF9vHEmfMDgW1nnKZ9UVer2CoLX".into(),
            None,
        )
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(refused, "A Solana account cannot be closed.");
    let mut watch = crate::derivation::setup::tests::fixture(
        Chain::Xrp,
        crate::derivation::setup::WalletSetupMethod::WatchAddresses,
    );
    watch.request.kind = crate::derivation::import::WalletImportKind::WatchAddresses {
        addresses: vec![DESTINATION.into()],
    };
    let watched = service.import(watch).await;
    let refused = service
        .build_account_closing(watched, "rHsMGQEkVNJmpGWs8XUBoTBiAAbwxZN5v3".into(), None)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("a watch-only wallet cannot send"),
        "{refused}"
    );
    assert!(service.list_sends().await.unwrap().is_empty());
}
