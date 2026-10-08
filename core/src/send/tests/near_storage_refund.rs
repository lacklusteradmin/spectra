use super::*;
use crate::send::near::PreparedNearFunctionCall;

const ACCOUNT: &str = "alice.near";
const TOKEN: &str = "usdt.tether-token.near";

/// A stored storage refund exactly as `build_token_storage_refund` stores
/// one: 0.00125 NEAR back, at most 0.0003 NEAR in fees.
fn refund() -> StoredSend {
    let prepared = PreparedPayload::NearFunctionCall(PreparedNearFunctionCall::storage_unregister(
        ACCOUNT,
        [1; 32],
        43,
        TOKEN,
        30_000_000_000_000,
        [2; 32],
        300_000_000_000_000_000_000,
    ));
    let mut stored = StoredSend {
        view: SendArtifact {
            id: "refund".into(),
            revision: 0,
            stage: SendStage::Prepared,
            wallet_id: "wallet".into(),
            chain_id: crate::registry::Chain::Near,
            sender: ACCOUNT.into(),
            recipient: ACCOUNT.into(),
            amount: "0.00125".into(),
            asset: "NEAR".into(),
            symbol: "NEAR".into(),
            staking: None,
            operation: Some(WalletOperation::RefundTokenStorage {
                contract: TOKEN.into(),
                refund: "0.00125".into(),
                network_fee: "0.0003".into(),
            }),
            created_at: 1.0,
            review_digest: String::new(),
            review: SendArtifactReview::default(),
            prepared_details: String::new(),
            signing_payload_hex: String::new(),
            signed_payload: None,
            transaction_hash: None,
            attempts: vec![],
            selected_endpoints: vec![],
            memo: None,
        },
        request: super::super::SendExecutionRequest {
            chain_id: crate::registry::Chain::Near,
            wallet_id: "wallet".into(),
            password: None,
            to_address: ACCOUNT.into(),
            amount_str: "0.00125".into(),
            contract_address: None,
            token_standard: None,
            token_decimals: None,
            fee_rate_svb: None,
            fee_sat: None,
            gas_budget: None,
            fee_amount: Some("0.0003".into()),
            evm_overrides: None,
            sign_only: false,
            memo: None,
        },
        prepared,
        submission: None,
        signed_digest: None,
        substrate_verified_through: None,
        icp_staking_receipts: vec![],
    };
    reseal(&mut stored);
    stored
}

/// What a tamperer with the database would do after an edit: make the
/// stored artifact consistent with itself again.
fn reseal(stored: &mut StoredSend) {
    stored.view.prepared_details = serde_json::to_string_pretty(&stored.prepared).unwrap();
    stored.view.review_digest = stored.digest().unwrap();
}

fn call(stored: &mut StoredSend) -> &mut PreparedNearFunctionCall {
    match &mut stored.prepared {
        PreparedPayload::NearFunctionCall(prepared) => prepared,
        _ => unreachable!(),
    }
}

/// A storage refund signs exactly the reviewed call: the account
/// unregistering itself from the token it names, without `force`, with one
/// yoctoNEAR, its deposit coming back to it. Any other call under its name,
/// even one consistent with itself, is refused.
#[test]
fn only_the_reviewed_unregistration_is_signed() {
    refund().validate().unwrap();
    let tampers: Vec<(&str, Box<dyn Fn(&mut StoredSend)>)> = vec![
        (
            "force",
            Box::new(|s| call(s).args = br#"{"force":true}"#.to_vec()),
        ),
        ("no yoctoNEAR", Box::new(|s| call(s).deposit = "0".into())),
        (
            "another contract",
            Box::new(|s| call(s).receiver = "wrap.near".into()),
        ),
        (
            "another method",
            Box::new(|s| call(s).method = "storage_withdraw".into()),
        ),
        ("another fee", Box::new(|s| call(s).fee_budget = "1".into())),
        (
            "another refund",
            Box::new(|s| {
                s.view.amount = "1".into();
                s.request.amount_str = "1".into();
            }),
        ),
        (
            "another recipient",
            Box::new(|s| {
                s.view.recipient = "bob.near".into();
                s.request.to_address = "bob.near".into();
            }),
        ),
    ];
    for (what, tamper) in tampers {
        let mut stored = refund();
        tamper(&mut stored);
        reseal(&mut stored);
        let error = stored.validate().unwrap_err().to_string();
        assert!(error.contains("operation was altered"), "{what}: {error}");
    }
    // A NEAR call with its operation dropped is no staking and no
    // operation: refused, however consistent.
    let mut stored = refund();
    stored.view.operation = None;
    reseal(&mut stored);
    assert!(stored.validate().is_err());
}
