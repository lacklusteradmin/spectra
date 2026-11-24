use super::*;
use serde_json::json;

const ADDRESS: &str = "tz1LHQgG3wJPBtxqUDJSbN4FX2WNPiuurj2M";
const OTHER: &str = "tz1cvjyVyd2T5Wozmx7CTNBH8yofzyLyJpi7";

/// `/v1/accounts/tz1LHQ…/operations?type=transaction`, read 2026-11-24.
fn mainnet_page() -> Vec<Value> {
    serde_json::from_value(json!([
        {
            "type": "transaction", "id": 3090991294709760_u64, "level": 15953142,
            "timestamp": "2026-11-24T11:45:43Z", "block": "BLENw5Q7a6vKjLMoJBXjVYN8xFfWSGpgvgy7dQuCibaPF4bgiuy",
            "hash": "ootbVsgxdptx4MrpHNkRjLSkRTR5PbDeAH9XVVR9xc8N69UHFsm", "counter": 240753240,
            "sender": {"address": ADDRESS}, "gasLimit": 2201, "gasUsed": 2101, "storageLimit": 0,
            "storageUsed": 0, "bakerFee": 578, "storageFee": 0, "allocationFee": 0,
            "target": {"address": OTHER}, "amount": 28419350, "status": "applied", "hasInternals": false,
        },
        {
            "type": "transaction", "id": 3090980074946560_u64, "level": 15953063,
            "timestamp": "2026-11-24T11:37:49Z", "block": "BMJCxHXdWUAj1pfuyiYXRopPcJFjWKBjMDU6YXNTRTKVHtFQzeZ",
            "hash": "oohQEV43tdNCTevBU6rimhSqgrF8tMXnLGvnvzjR9WxKHGasUkz", "counter": 240678902,
            "sender": {"address": "tz1YPH6hLFRBruDdwbAAKeAovd6Kmewv3HhB"}, "gasLimit": 2201, "gasUsed": 2101,
            "storageLimit": 0, "storageUsed": 0, "bakerFee": 578, "storageFee": 0, "allocationFee": 0,
            "target": {"address": ADDRESS}, "amount": 19993056, "status": "applied", "hasInternals": false,
        },
        {
            "type": "transaction", "id": 3037219755393024_u64, "level": 15643566,
            "timestamp": "2026-11-02T17:56:37Z", "block": "BL1QKZiEycaJw9AAtfHzwNrAQYYzKzCW7KmPrVE2BH489c7mRHv",
            "hash": "oo4FAfd2K2yyjSM36y57YCVoiPuJbV5tm1CLCvgiFvn61KgDfSc", "counter": 240752053,
            "sender": {"address": "tz1cptDmCNpmwHahGx2rVf5aZdmMHWjqoxaa"}, "gasLimit": 2101, "gasUsed": 2101,
            "storageLimit": 277, "storageUsed": 0, "bakerFee": 652, "storageFee": 0, "allocationFee": 64250,
            "target": {"address": ADDRESS}, "amount": 10000000, "status": "applied", "hasInternals": false,
        },
    ]))
    .unwrap()
}

#[test]
fn history_reads_what_each_transfer_did_to_the_balance() {
    let page = history_page(&mainnet_page(), ADDRESS).unwrap();
    assert_eq!(page.next_cursor, None, "a short page is the last");
    assert_eq!(page.items.len(), 3);
    let sent = &page.items[0];
    assert_eq!(
        sent.txid,
        "ootbVsgxdptx4MrpHNkRjLSkRTR5PbDeAH9XVVR9xc8N69UHFsm"
    );
    assert_eq!(sent.amount, HistoryAmount::Units(28_419_350));
    assert!(!sent.incoming);
    assert_eq!(sent.counterparty, OTHER);
    assert_eq!(sent.outcome, HistoryOutcome::Confirmed);
    assert_eq!(sent.block_height, Some(15_953_142));
    assert!(sent.timestamp.is_some());
    let received = &page.items[2];
    assert_eq!(received.amount, HistoryAmount::Units(10_000_000));
    assert!(received.incoming);
    assert_eq!(
        received.counterparty,
        "tz1cptDmCNpmwHahGx2rVf5aZdmMHWjqoxaa"
    );
}

fn transfer(id: u64, hash: &str, sender: &str, target: &str, amount: u64, status: &str) -> Value {
    json!({
        "type": "transaction", "id": id, "level": 100, "timestamp": "2026-11-24T00:00:00Z",
        "hash": hash, "sender": {"address": sender}, "target": {"address": target},
        "amount": amount, "status": status,
    })
}

#[test]
fn one_operation_group_is_one_entry_and_a_failed_send_shows_its_amount() {
    let contract = "KT1H5b7LxEExkFd2Tng77TfuWbM5aPvHstPr";
    let operations = [
        // A contract call that pays the address back more than it sent.
        transfer(9, "opA", ADDRESS, contract, 1_000, "applied"),
        transfer(8, "opA", contract, ADDRESS, 5_000, "applied"),
        // A failed send: only its fee moved.
        transfer(7, "opB", ADDRESS, OTHER, 42, "failed"),
        // A failed payment to the address moved nothing to it.
        transfer(6, "opC", OTHER, ADDRESS, 42, "backtracked"),
        // To itself, and a zero-tez contract call: nothing moved.
        transfer(5, "opD", ADDRESS, ADDRESS, 9, "applied"),
        transfer(4, "opE", ADDRESS, contract, 0, "applied"),
    ];
    let page = history_page(&operations, ADDRESS).unwrap();
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.items[0].txid, "opA");
    assert_eq!(page.items[0].amount, HistoryAmount::Units(4_000));
    assert!(page.items[0].incoming);
    assert_eq!(page.items[1].txid, "opB");
    assert_eq!(page.items[1].amount, HistoryAmount::Units(42));
    assert!(!page.items[1].incoming);
    assert_eq!(page.items[1].outcome, HistoryOutcome::Failed);
}

#[test]
fn a_full_page_leaves_its_last_group_for_the_next() {
    let mut operations: Vec<Value> = (0..HISTORY_PAGE_SIZE as u64 - 2)
        .map(|n| transfer(1_000 - n, &format!("op{n}"), OTHER, ADDRESS, 1, "applied"))
        .collect();
    // The group that straddles the page's end.
    operations.push(transfer(900, "opLast", OTHER, ADDRESS, 1, "applied"));
    operations.push(transfer(899, "opLast", OTHER, ADDRESS, 1, "applied"));
    let page = history_page(&operations, ADDRESS).unwrap();
    assert_eq!(page.items.len(), HISTORY_PAGE_SIZE - 2);
    assert!(page.items.iter().all(|item| item.txid != "opLast"));
    assert_eq!(page.next_cursor.as_deref(), Some("953"));

    // A page that is all one group is read as it is.
    let group: Vec<Value> = (0..HISTORY_PAGE_SIZE as u64)
        .map(|n| transfer(500 - n, "opBig", OTHER, ADDRESS, 1, "applied"))
        .collect();
    let page = history_page(&group, ADDRESS).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(
        page.items[0].amount,
        HistoryAmount::Units(HISTORY_PAGE_SIZE as u128)
    );
    assert_eq!(page.next_cursor.as_deref(), Some("451"));
}

/// `/v1/operations/ootbVs…`, read 2026-11-24: a reveal and its transfer.
#[test]
fn an_operation_is_confirmed_once_every_part_of_it_applied() {
    let hash = "ootbVsgxdptx4MrpHNkRjLSkRTR5PbDeAH9XVVR9xc8N69UHFsm";
    let applied = json!([
        {"type": "reveal", "id": 3090991293661184_u64, "level": 15953142, "hash": hash,
         "sender": {"address": ADDRESS}, "counter": 240753239, "gasLimit": 270, "gasUsed": 171,
         "storageLimit": 0, "bakerFee": 291, "status": "applied",
         "publicKey": "edpkuc4t4nhvBkMHa3bDUAKL8hpzT6jQGUFfzwVa12nGKZDrykjonK"},
        {"type": "transaction", "id": 3090991294709760_u64, "level": 15953142, "hash": hash,
         "counter": 240753240, "sender": {"address": ADDRESS}, "target": {"address": OTHER},
         "amount": 28419350, "status": "applied"},
    ]);
    assert_eq!(
        status_from(&applied, hash).unwrap(),
        TransactionStatus::Confirmed {
            succeeded: true,
            block: Some(15_953_142),
        }
    );
    let mut failed = applied.clone();
    failed[1]["status"] = json!("failed");
    failed[0]["status"] = json!("backtracked");
    assert_eq!(
        status_from(&failed, hash).unwrap(),
        TransactionStatus::Confirmed {
            succeeded: false,
            block: Some(15_953_142),
        }
    );
    // Not yet indexed: pending. Another operation's answer is no answer.
    assert_eq!(
        status_from(&json!([]), hash).unwrap(),
        TransactionStatus::Pending
    );
    assert!(
        status_from(
            &applied,
            "opac2pJnnSfuYU69ou5P7neHjD5oy2CbqmGeo6ae5MwxddBoAbF"
        )
        .is_err()
    );
}
