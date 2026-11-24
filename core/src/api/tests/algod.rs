use super::*;
use serde_json::json;

/// Answers read from mainnet-api.4160.nodely.dev on 2026-11-24.
fn params() -> Value {
    json!({
        "consensus-version": "https://github.com/algorandfoundation/specs/tree/73c43b4a9699390884c51e2a9598d49e28373d10",
        "fee": 0,
        "genesis-hash": "wGHE2Pwdvd7S12BL5FaOP20EGYesN73ktiC1qzkkit8=",
        "genesis-id": "mainnet-v1.0",
        "last-round": 67257603,
        "min-fee": 1000
    })
}

const EMPTY: &str = "JA2PBBITKPRF6J3DPSOS65GLXRZTOMYVJ6DK3GSDYWYRQJKIZVSRESQ5WY";

fn empty_account() -> Value {
    json!({
        "address": EMPTY,
        "amount": 0,
        "amount-without-pending-rewards": 0,
        "apps-total-schema": {"num-byte-slice": 0, "num-uint": 0},
        "min-balance": 100000,
        "pending-rewards": 0,
        "reward-base": 218288,
        "rewards": 0,
        "round": 67257847,
        "status": "Offline",
        "total-apps-opted-in": 0,
        "total-assets-opted-in": 0,
        "total-created-apps": 0,
        "total-created-assets": 0
    })
}

#[test]
fn parameters_are_read_only_from_mainnet() {
    assert_eq!(
        params_from(Chain::Algorand, &params()).unwrap(),
        AlgorandParams {
            last_round: 67_257_603,
            min_fee: 1_000,
            fee_per_byte: 0,
        }
    );
    let mut testnet = params();
    testnet["genesis-id"] = json!("testnet-v1.0");
    testnet["genesis-hash"] = json!("SGO1GKSzyE7IEPItTxCByw9x8FmnrCDexi9/cOUJOiI=");
    assert!(params_from(Chain::Algorand, &testnet).is_err());
    // The right name over another genesis is another network.
    let mut renamed = params();
    renamed["genesis-hash"] = json!("SGO1GKSzyE7IEPItTxCByw9x8FmnrCDexi9/cOUJOiI=");
    assert!(params_from(Chain::Algorand, &renamed).is_err());
    let mut unnamed = params();
    unnamed.as_object_mut().unwrap().remove("genesis-id");
    assert!(params_from(Chain::Algorand, &unnamed).is_err());
    let mut no_fee = params();
    no_fee.as_object_mut().unwrap().remove("min-fee");
    assert!(params_from(Chain::Algorand, &no_fee).is_err());
    assert!(params_from(Chain::Ethereum, &params()).is_err());
}

#[test]
fn an_account_is_read_with_its_minimum_and_signer() {
    let account = account_from(&empty_account(), EMPTY).unwrap();
    assert_eq!(account.amount, 0);
    assert_eq!(account.min_balance, 100_000);
    assert_eq!(account.spendable(), 0);
    assert_eq!(account.auth_address, None);

    let mut funded = empty_account();
    funded["amount"] = json!(5_000_000);
    funded["min-balance"] = json!(200_000);
    let account = account_from(&funded, EMPTY).unwrap();
    assert_eq!(account.spendable(), 4_800_000);

    // A rekeyed account signs with another address's key.
    let auth = "BHYSAPV7W4FSPMKOVS4RVYCARBIANFQDQGCOTE3FN24THEMRE6OFLCUCYE";
    funded["auth-addr"] = json!(auth);
    assert_eq!(
        account_from(&funded, EMPTY)
            .unwrap()
            .auth_address
            .as_deref(),
        Some(auth)
    );
    assert!(account_from(&empty_account(), auth).is_err());
}

#[test]
fn a_pending_answer_says_whether_the_transaction_is_committed() {
    // A payment a node still remembered, read on 2026-11-24.
    let confirmed = json!({
        "confirmed-round": 67258005,
        "pool-error": "",
        "txn": {
            "sig": "oQAxcsTrHTsvSAJSXZx0JPfmp4QuSxuH0TtTbrIBV0rDliV1cKmYd21CSj4DuGMummJR35V6JrODn6woI4VyCg==",
            "txn": {
                "fee": 1000, "fv": 67258003, "gen": "mainnet-v1.0",
                "gh": "wGHE2Pwdvd7S12BL5FaOP20EGYesN73ktiC1qzkkit8=",
                "grp": "4bAa2wp1pM4L9R1l/9W90MJqMegRn9rPfhXmDXRj4ac=",
                "lv": 67259003,
                "rcv": "JM2AUDGBHZYIIN5MFP4VPKKAGJH27PWI3XXITX4UAHSQQ5ZL7BAGD72YOQ",
                "snd": "F2IBRL7FBPC7IESEIWYD2C7NPOLGYUSRFN4EQIYELESQGTBXTDVLSCELNY",
                "type": "pay"
            }
        }
    });
    assert_eq!(
        pending_status(&confirmed).unwrap(),
        TransactionStatus::Confirmed {
            succeeded: true,
            block: Some(67_258_005)
        }
    );
    // In the pool, or evicted from this node's: still pending.
    let mut pooled = confirmed.clone();
    pooled["confirmed-round"] = json!(0);
    assert_eq!(pending_status(&pooled).unwrap(), TransactionStatus::Pending);
    pooled["pool-error"] = json!("transaction already in ledger");
    assert_eq!(pending_status(&pooled).unwrap(), TransactionStatus::Pending);
    assert!(pending_status(&json!({"confirmed-round": 5})).is_err());
}

#[test]
fn a_transaction_id_is_52_letters_of_base32() {
    assert!(validate_txid("5UQL3WNONSVCAIN3PEPEL7OXGDDCA6KDHSLESV4DVPJGSZSGRNSA").is_ok());
    for bad in [
        "5UQL3WNONSVCAIN3PEPEL7OXGDDCA6KDHSLESV4DVPJGSZSGRNS",
        "5UQL3WNONSVCAIN3PEPEL7OXGDDCA6KDHSLESV4DVPJGSZSGRNS1",
        "../v2/status/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        &"ab".repeat(32),
    ] {
        assert!(validate_txid(bad).is_err(), "{bad}");
    }
}

#[test]
fn a_node_message_is_read_from_its_error_body() {
    assert_eq!(
        node_message(r#"{"message":"missing required field: txn"}"#),
        "missing required field: txn"
    );
    assert_eq!(node_message("bad gateway"), "bad gateway");
}
