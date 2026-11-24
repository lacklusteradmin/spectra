use super::*;
use serde_json::json;

const ADDRESS: &str = "f1qode47ievxlxzk6z2viuovedabmn3tq6t57uqhq";

/// `/address/{ADDRESS}/transfers?pageSize=100&page=0&type=transfer`, as
/// filfox.info answered it on 2026-11-24.
fn page() -> Value {
    json!({
        "totalCount": 3,
        "transfers": [
            {"height": 6242968, "timestamp": 1785595440,
             "message": "bafy2bzacedq23shsjgxqckusrr6cx7uvn3quyfgumf3tpycdensvwgdlvj3dw",
             "from": ADDRESS, "to": "f1mu7f2m4nbhnqsuphkeeujnecezbuxkwgru7el5a",
             "value": "-5006722643880", "type": "send"},
            {"height": 6217419, "timestamp": 1784828970,
             "message": "bafy2bzacedh35fgaloaeil4646zbps562pv3d7jroii6vhvuwipc4x5ed3nwi",
             "from": ADDRESS, "to": "f1sbxnvvt7akhukrto6puch5eafmkgnm3udu3h5gq",
             "value": "-9899350480580560", "type": "send"},
            {"height": 6211690, "timestamp": 1784657100,
             "message": "bafy2bzacea5ifdp3c5r46uio5b5mnyfqhrl53zc7dytfpvgngnronvd3lomqk",
             "from": "f3slmbmyitgwzx3y6vxndfu6qr6m2lvbkvzfvumahpjzf7a7p5wzvejfhh5ta46wcfhy5kdlsxv4klxijzhhyq",
             "to": ADDRESS, "value": "10000000000000000", "type": "receive"},
        ],
        "types": ["burn-fee", "miner-fee", "transfer"],
    })
}

#[test]
fn transfers_read_as_one_entry_per_message_from_the_addresss_side() {
    let page = page();
    let entries = history_entries(page["transfers"].as_array().unwrap()).unwrap();
    assert_eq!(entries.len(), 3);
    let sent = &entries[0];
    assert_eq!(
        sent.txid,
        "bafy2bzacedq23shsjgxqckusrr6cx7uvn3quyfgumf3tpycdensvwgdlvj3dw"
    );
    assert_eq!(sent.amount, HistoryAmount::Units(5_006_722_643_880));
    assert!(!sent.incoming);
    assert_eq!(
        sent.counterparty,
        "f1mu7f2m4nbhnqsuphkeeujnecezbuxkwgru7el5a"
    );
    assert_eq!(sent.block_height, Some(6_242_968));
    assert_eq!(sent.timestamp, Some(1_785_595_440.0));
    assert_eq!(sent.outcome, HistoryOutcome::Confirmed);
    let received = &entries[2];
    assert_eq!(
        received.amount,
        HistoryAmount::Units(10_000_000_000_000_000)
    );
    assert!(received.incoming);
    assert!(received.counterparty.starts_with("f3slmbmy"));
}

#[test]
fn a_messages_transfers_are_summed_and_its_fees_left_aside() {
    let message = "bafy2bzaceakqouboqyxtz6mp2up2bgd2qyd5qk6vjklcveik3v3fqf2taqq7g";
    let transfer = |value: &str, kind: &str| {
        json!({"height": 7, "timestamp": 1, "message": message,
               "from": ADDRESS, "to": "f099", "value": value, "type": kind})
    };
    // Two payouts of one message, and the fees an unfiltered page lists.
    let transfers = [
        transfer("-3", "send"),
        transfer("-4", "send"),
        transfer("-1000000000", "burn-fee"),
        transfer("-5000000000000", "miner-fee"),
    ];
    let entries = history_entries(&transfers).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].amount, HistoryAmount::Units(7));
    // A send to itself moved nothing.
    let to_itself = [transfer("-9", "send"), transfer("9", "receive")];
    assert!(history_entries(&to_itself).unwrap().is_empty());
    // An amount that is not whole attoFIL is not this API's answer.
    assert!(history_entries(&[transfer("1.5", "receive")]).is_err());
    assert!(history_entries(&[transfer("", "receive")]).is_err());
}
