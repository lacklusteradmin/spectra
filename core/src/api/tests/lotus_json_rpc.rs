use super::*;

const SIGNED_CID: &str = "bafy2bzaceakqouboqyxtz6mp2up2bgd2qyd5qk6vjklcveik3v3fqf2taqq7g";

fn asked() -> FilecoinMessage {
    FilecoinMessage {
        to: "f1nzasvvzledaucnnjn6i4wsvydqu37nnhypdlnfq".into(),
        from: "f1up5qywgfhygtqmxlvke3syvgldkqavicn5derri".into(),
        nonce: 0,
        value: "1000000000000".into(),
        gas_limit: 0,
        gas_fee_cap: "0".into(),
        gas_premium: "0".into(),
        method: 0,
    }
}

#[test]
fn a_gas_estimate_is_read_only_for_the_message_asked_about() {
    // api.node.glif.io's answer on 2026-11-24.
    let answer = json!({
        "Version": 0, "To": "f1nzasvvzledaucnnjn6i4wsvydqu37nnhypdlnfq",
        "From": "f1up5qywgfhygtqmxlvke3syvgldkqavicn5derri", "Nonce": 0,
        "Value": "1000000000000", "GasLimit": 1526328, "GasFeeCap": "101213",
        "GasPremium": "100022", "Method": 0, "Params": null,
        "CID": {"/": "bafy2bzaceb5ljgdwbqju55doirqprgmyfqsdskmc5c5r2naqdqmkfbnoinkei"},
    });
    assert_eq!(
        gas_from(&answer, &asked()).unwrap(),
        FilecoinGas {
            limit: 1_526_328,
            fee_cap: 101_213,
            premium: 100_022
        }
    );
    for (field, value) in [
        ("To", json!("f1up5qywgfhygtqmxlvke3syvgldkqavicn5derri")),
        ("Value", json!("1")),
        ("Method", json!(3844450837u64)),
        ("GasLimit", json!(0)),
        ("GasFeeCap", json!("-1")),
        ("GasPremium", json!(100022)),
    ] {
        let mut altered = answer.clone();
        altered[field] = value;
        assert!(gas_from(&altered, &asked()).is_err(), "{field}");
    }
}

#[test]
fn a_mined_message_is_confirmed_by_its_receipt_and_an_unknown_one_pending() {
    // api.node.glif.io's `StateSearchMsg` answer on 2026-11-24.
    let found = json!({
        "Message": {"/": SIGNED_CID},
        "Receipt": {"ExitCode": 0, "Return": null, "GasUsed": 1226263, "EventsRoot": null},
        "ReturnDec": null,
        "TipSet": [
            {"/": "bafy2bzaceb67vico2p7pkprydl5aq2eswryku4sgjznhwx4u5mcqhk3wrb7hw"},
            {"/": "bafy2bzaceds2inzxkw525phbvm4fhzrlsahzoj3pbxli7hz73t4uahfu63dii"},
        ],
        "Height": 6573568,
    });
    assert_eq!(
        message_status(&found, SIGNED_CID).unwrap(),
        TransactionStatus::Confirmed {
            succeeded: true,
            block: Some(6_573_568)
        }
    );
    let mut failed = found.clone();
    failed["Receipt"]["ExitCode"] = json!(6);
    assert_eq!(
        message_status(&failed, SIGNED_CID).unwrap(),
        TransactionStatus::Confirmed {
            succeeded: false,
            block: Some(6_573_568)
        }
    );
    assert_eq!(
        message_status(&Value::Null, SIGNED_CID).unwrap(),
        TransactionStatus::Pending
    );
    let other = "bafy2bzacechqqlkfsb2lu3cgjliyfsao2sprjk276vawiiji5f4jlhx5pvc7c";
    assert!(message_status(&found, other).is_err());
}

#[test]
fn an_actor_names_its_ethereum_address_and_a_missing_one_is_none() {
    // `StateGetActor` for an EthAccount and for an f1 account, 2026-11-24.
    let eth_account = json!({
        "Code": {"/": "bafk2bzaceajtlll5ms37steebrk7r2mm3duyogisqo5p4yk2lbxy7wqjxgyfm"},
        "Head": {"/": "bafy2bzacebc3bt6cedhoyw34drrmjvazhu4oj25er2ebk4u445pzycvq4ta4a"},
        "Nonce": 9158, "Balance": "3469462660367975677",
        "DelegatedAddress": "f410feaavpcsvq5wkp5g5yhg56nkm3x524ubef3sbzei",
    });
    assert_eq!(
        actor_from(&eth_account).unwrap(),
        Some(FilecoinActor {
            nonce: 9158,
            delegated_address: Some("f410feaavpcsvq5wkp5g5yhg56nkm3x524ubef3sbzei".into()),
        })
    );
    let account = json!({
        "Code": {"/": "bafk2bzacecgttsyulmd4r2y64iw64x2t4w7tkh636tbtkemy37ume6fxtcjuu"},
        "Nonce": 6534, "Balance": "55834046140000000", "DelegatedAddress": null,
    });
    assert_eq!(
        actor_from(&account).unwrap().unwrap().delegated_address,
        None
    );
    assert_eq!(actor_from(&Value::Null).unwrap(), None);
    // What an ID with no actor answers is `null`; a key address with none
    // answers this error.
    assert!(is_actor_not_found(&ApiError::Rejected(
        r#"lotus-json-rpc rpc error: {"code":1,"message":"resolution lookup failed (f13snrccungbofkjt377g2tr74q25j6gvnusz3s2a): resolve address f13snrccungbofkjt377g2tr74q25j6gvnusz3s2a: actor not found"}"#.into()
    )));
    assert!(!is_actor_not_found(&ApiError::Transport(
        "timed out".into()
    )));
}

#[test]
fn a_push_answers_the_cid_or_the_nodes_refusal() {
    assert_eq!(
        pushed_cid(&json!({"jsonrpc": "2.0", "id": 1, "result": {"/": SIGNED_CID}})).unwrap(),
        SIGNED_CID
    );
    let refused = json!({"jsonrpc": "2.0", "id": 1,
        "error": {"code": 1, "message": "message nonce too low: 6533 < 6534"}});
    assert!(
        matches!(pushed_cid(&refused), Err(ApiError::Rejected(message)) if message.contains("nonce too low"))
    );
    assert!(pushed_cid(&json!({"result": {"/": "not-a-cid"}})).is_err());
}

#[test]
fn balances_are_whole_attofil() {
    assert_eq!(
        attofil(&json!("55834046140000000")),
        Some(55_834_046_140_000_000)
    );
    assert_eq!(attofil(&json!("0")), Some(0));
    for value in [json!(""), json!("-1"), json!("1.5"), json!(7)] {
        assert_eq!(attofil(&value), None, "{value}");
    }
}

#[test]
fn a_signed_message_is_pushed_in_lotus_json() {
    let signed = FilecoinSignedMessage {
        message: FilecoinMessage {
            gas_limit: 1_526_328,
            gas_fee_cap: "101213".into(),
            gas_premium: "100022".into(),
            nonce: 6534,
            ..asked()
        },
        signature: vec![7; 65],
    };
    let pushed = signed.lotus_json();
    assert_eq!(pushed["Message"]["Version"], 0);
    assert_eq!(pushed["Message"]["Value"], "1000000000000");
    assert_eq!(pushed["Message"]["GasLimit"], 1_526_328);
    assert_eq!(pushed["Message"]["Params"], Value::Null);
    assert_eq!(pushed["Signature"]["Type"], 1);
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(pushed["Signature"]["Data"].as_str().unwrap())
            .unwrap(),
        vec![7; 65]
    );
}
