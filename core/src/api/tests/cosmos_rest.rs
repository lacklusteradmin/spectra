use super::*;

const ADDRESS: &str = "cosmos1qypqxpq9qcrsszg2pvxq6rs0zqg3yyc5lzv7xu";
const OTHER: &str = "cosmos1zg69v7ys40x77y352eufp27daufrg4ncnjqz7q";

#[test]
fn an_account_is_read_from_every_type_that_holds_one() {
    let base = json!({
        "@type": "/cosmos.auth.v1beta1.BaseAccount",
        "address": ADDRESS, "account_number": "12", "sequence": "3",
    });
    let eth = json!({
        "@type": "/injective.types.v1beta1.EthAccount",
        "base_account": {"address": ADDRESS, "account_number": "12", "sequence": "3"},
        "code_hash": "xdJGAYb3IzySfn2y3McDwOUAtlPKgic7e/rYBF2FpHA=",
    });
    let vesting = json!({
        "@type": "/cosmos.vesting.v1beta1.ContinuousVestingAccount",
        "base_vesting_account": {
            "base_account": {"address": ADDRESS, "account_number": "12", "sequence": "3"},
            "original_vesting": [],
        },
    });
    let expected = CosmosAccount {
        account_number: 12,
        sequence: 3,
    };
    for account in [base, eth, vesting] {
        assert_eq!(account_from(&account, ADDRESS).unwrap(), expected);
    }
    // A module account holds no base account a key signs for, and an
    // answer for another address is not this one's.
    let module = json!({"@type": "/cosmos.auth.v1beta1.ModuleAccount", "name": "distribution"});
    assert!(account_from(&module, ADDRESS).is_err());
    let other = json!({"address": OTHER, "account_number": "1", "sequence": "0"});
    assert!(account_from(&other, ADDRESS).is_err());
}

#[test]
fn gas_prices_are_exact_fractions_of_their_decimals() {
    let price = decimal_gas_price("0.005000000000000000").unwrap();
    assert_eq!((price.numerator, price.denominator), (5, 1000));
    let price = decimal_gas_price("160000000").unwrap();
    assert_eq!((price.numerator, price.denominator), (160_000_000, 1));
    assert_eq!(
        decimal_gas_price("0.0025").unwrap().fee_for(100_001),
        Some(251)
    );
    for text in ["", ".", "-1", "1e5", "0.5x", "1.2.3"] {
        assert!(decimal_gas_price(text).is_none(), "{text}");
    }
}

#[test]
fn a_coin_list_is_summed_for_one_denomination() {
    assert_eq!(
        coins_of("100uatom,5ibc/27394FB0,7uatom", "uatom").unwrap(),
        107
    );
    assert_eq!(coins_of("5ibc/27394FB0", "uatom").unwrap(), 0);
    assert_eq!(coins_of("", "uatom").unwrap(), 0);
    assert!(coins_of("uatom", "uatom").is_err());
}

fn attribute(key: &str, value: &str) -> Value {
    json!({"key": key, "value": value, "index": true})
}

#[test]
fn history_reads_what_messages_moved_and_leaves_the_fee_aside() {
    // An SDK 0.50 answer: the fee's transfer has no `msg_index`.
    let sent = json!({
        "txhash": "ab".repeat(32), "height": "100", "code": 0,
        "timestamp": "2026-11-24T08:00:00Z",
        "events": [
            {"type": "transfer", "attributes": [
                attribute("recipient", "cosmos17xpfvakm2amg962yls6f84z3kell8c5lserqta"),
                attribute("sender", ADDRESS),
                attribute("amount", "5000uatom"),
            ]},
            {"type": "transfer", "attributes": [
                attribute("recipient", OTHER),
                attribute("sender", ADDRESS),
                attribute("amount", "1000000uatom"),
                attribute("msg_index", "0"),
            ]},
        ],
    });
    let entry = history_entry(&sent, ADDRESS, "uatom").unwrap().unwrap();
    assert_eq!(entry.txid, "AB".repeat(32));
    assert_eq!(entry.amount, HistoryAmount::Units(1_000_000));
    assert!(!entry.incoming);
    assert_eq!(entry.outcome, HistoryOutcome::Confirmed);
    assert_eq!(entry.counterparty, OTHER);
    assert_eq!(entry.block_height, Some(100));
    assert!(entry.timestamp.is_some());

    // An older node lists each message's events under its log.
    let received = json!({
        "txhash": "cd".repeat(32), "height": "7", "code": 0,
        "logs": [{"msg_index": 0, "events": [
            {"type": "transfer", "attributes": [
                attribute("recipient", ADDRESS),
                attribute("sender", OTHER),
                attribute("amount", "250uatom,9ibc/27394FB0"),
            ]},
        ]}],
        "events": [],
    });
    let entry = history_entry(&received, ADDRESS, "uatom").unwrap().unwrap();
    assert_eq!(entry.amount, HistoryAmount::Units(250));
    assert!(entry.incoming);
    assert_eq!(entry.counterparty, OTHER);

    // Another coin only: nothing happened to this coin's balance.
    let other_coin = json!({
        "txhash": "ef".repeat(32), "height": "8", "code": 0,
        "events": [{"type": "transfer", "attributes": [
            attribute("recipient", ADDRESS), attribute("sender", OTHER),
            attribute("amount", "9ibc/27394FB0"), attribute("msg_index", "0"),
        ]}],
    });
    assert!(
        history_entry(&other_coin, ADDRESS, "uatom")
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_failed_send_is_shown_with_the_amount_it_asked_for() {
    let failed = json!({
        "txhash": "01".repeat(32), "height": "9", "code": 5,
        "tx": {"body": {"messages": [{
            "@type": "/cosmos.bank.v1beta1.MsgSend",
            "from_address": ADDRESS, "to_address": OTHER,
            "amount": [{"denom": "uatom", "amount": "42"}],
        }]}},
        "events": [],
    });
    let entry = history_entry(&failed, ADDRESS, "uatom").unwrap().unwrap();
    assert_eq!(entry.amount, HistoryAmount::Units(42));
    assert_eq!(entry.outcome, HistoryOutcome::Failed);
    assert!(!entry.incoming);
}

#[test]
fn a_missing_account_or_transaction_is_not_found_and_nothing_else_is() {
    assert!(is_not_found(&ApiError::Status {
        status: 404,
        body: String::new()
    }));
    assert!(is_not_found(&ApiError::Status {
        status: 500,
        body:
            r#"{"code":5,"message":"rpc error: code = NotFound desc = account cosmos1… not found"}"#
                .into()
    }));
    assert!(!is_not_found(&ApiError::Status {
        status: 500,
        body: r#"{"code":13,"message":"internal"}"#.into()
    }));
    assert!(!is_not_found(&ApiError::Transport("timed out".into())));
}
