use super::*;
use serde_json::json;

const ADDRESS: &str = "4TPMQLUIBMQ6ILR4FSBEWJACEOYJVYZ7PWL333KL47DHVT6TJHH55E5WWE";
const FEE_SINK: &str = "Y76M3MSY6DKBRHBL7C3NNDXGS5IIMQVQVUAB6MP4XEMMGVF2QWNPL226CA";
const OTHER: &str = "4MBB6O7EV2ZRIUKJT47B2NJ2BQPNJ3BQEQBPA7UN7MXWRG7U4OMPP6VOTY";

fn pay(sender: &str, receiver: &str, amount: u64) -> Value {
    json!({
        "tx-type": "pay",
        "sender": sender,
        "fee": 0,
        "payment-transaction": {"amount": amount, "close-amount": 0, "receiver": receiver},
    })
}

fn root(id: &str, mut transaction: Value) -> Value {
    let fields = transaction.as_object_mut().unwrap();
    fields.insert("id".into(), json!(id));
    fields.insert("confirmed-round".into(), json!(67257669));
    fields.insert("round-time".into(), json!(1795521298));
    fields.insert("genesis-id".into(), json!("mainnet-v1.0"));
    fields.insert(
        "genesis-hash".into(),
        json!("wGHE2Pwdvd7S12BL5FaOP20EGYesN73ktiC1qzkkit8="),
    );
    transaction
}

#[test]
fn a_payment_to_the_address_is_received_from_its_sender() {
    // A proposer payout from the fee sink, read from the indexer on
    // 2026-11-24.
    let payout = root(
        "AYPBZGZCSGBSZVZ3R42DZC3CS7T7D5MVODOI4QVQULGCMU7Z2ITQ",
        pay(FEE_SINK, ADDRESS, 8_108_276),
    );
    check_genesis(Chain::Algorand, &payout).unwrap();
    let entry = history_entry(&payout, ADDRESS).unwrap().unwrap();
    assert_eq!(
        entry.txid,
        "AYPBZGZCSGBSZVZ3R42DZC3CS7T7D5MVODOI4QVQULGCMU7Z2ITQ"
    );
    assert_eq!(entry.amount, HistoryAmount::Units(8_108_276));
    assert!(entry.incoming);
    assert_eq!(entry.counterparty, FEE_SINK);
    assert_eq!(entry.outcome, HistoryOutcome::Confirmed);
    assert_eq!(entry.block_height, Some(67_257_669));
    assert_eq!(entry.timestamp, Some(1_795_521_298.0));

    let sent = history_entry(&root("A", pay(ADDRESS, OTHER, 5)), ADDRESS)
        .unwrap()
        .unwrap();
    assert!(!sent.incoming);
    assert_eq!(sent.amount, HistoryAmount::Units(5));
    assert_eq!(sent.counterparty, OTHER);
}

#[test]
fn inner_payments_count_and_other_transactions_do_not() {
    // An application call whose inner call pays from the address, as the
    // indexer listed it on 2026-11-24, with its asset transfers.
    let call = root(
        "ETQMKTQKZS4VXIPKBUVEMOGFUGGUPJQDSH63BVUCLKBJEAP3Q6MQ",
        json!({
            "tx-type": "appl",
            "sender": "NKBDACORTZ7LGES4UUHLGQ6UE3WFG7YSMMP76YDWOFCAR5SPOEW447TBYU",
            "fee": 23000,
            "inner-txns": [
                {"tx-type": "appl", "sender": "NKBDACORTZ7LGES4UUHLGQ6UE3WFG7YSMMP76YDWOFCAR5SPOEW447TBYU", "inner-txns": [
                    pay("VVU2LEKHHCF2PACOOIMSH6WY6AM5XMFWZRGWVHR65CILVVS6F4PHNIH35Y", OTHER, 3_954_166),
                    pay(ADDRESS, OTHER, 31_751_723),
                ]},
                {"tx-type": "axfer", "sender": ADDRESS, "asset-transfer-transaction": {
                    "amount": 28_873_693, "asset-id": 1_134_696_561, "close-amount": 0, "receiver": OTHER,
                }},
            ],
        }),
    );
    let entry = history_entry(&call, ADDRESS).unwrap().unwrap();
    assert_eq!(entry.amount, HistoryAmount::Units(31_751_723));
    assert!(!entry.incoming);
    assert_eq!(entry.counterparty, OTHER);

    // An asset transfer, a call naming the address and a payment to itself
    // move none of its ALGO.
    let asset = root(
        "B",
        json!({"tx-type": "axfer", "sender": OTHER, "asset-transfer-transaction": {
            "amount": 1, "asset-id": 31_566_704, "close-amount": 0, "receiver": ADDRESS,
        }}),
    );
    let named = root("C", json!({"tx-type": "appl", "sender": OTHER}));
    let to_itself = root("D", pay(ADDRESS, ADDRESS, 9));
    for transaction in [asset, named, to_itself] {
        assert!(history_entry(&transaction, ADDRESS).unwrap().is_none());
    }
}

#[test]
fn a_closed_account_sends_its_remainder() {
    let mut close = pay(OTHER, FEE_SINK, 100);
    close["payment-transaction"]["close-remainder-to"] = json!(ADDRESS);
    close["payment-transaction"]["close-amount"] = json!(2_500_000);
    let received = history_entry(&root("E", close.clone()), ADDRESS)
        .unwrap()
        .unwrap();
    assert!(received.incoming);
    assert_eq!(received.amount, HistoryAmount::Units(2_500_000));
    assert_eq!(received.counterparty, OTHER);

    // Closing out of the address sends both the amount and the remainder.
    let closed = history_entry(&root("F", close), OTHER).unwrap().unwrap();
    assert!(!closed.incoming);
    assert_eq!(closed.amount, HistoryAmount::Units(2_500_100));
}

#[test]
fn a_transaction_is_refused_without_its_network_or_amount() {
    let mut testnet = root("G", pay(OTHER, ADDRESS, 1));
    testnet["genesis-id"] = json!("testnet-v1.0");
    testnet["genesis-hash"] = json!("SGO1GKSzyE7IEPItTxCByw9x8FmnrCDexi9/cOUJOiI=");
    assert!(check_genesis(Chain::Algorand, &testnet).is_err());
    let mut unpriced = root("H", pay(OTHER, ADDRESS, 1));
    unpriced["payment-transaction"]
        .as_object_mut()
        .unwrap()
        .remove("amount");
    assert!(history_entry(&unpriced, ADDRESS).is_err());
    assert!(history_entry(&json!({"tx-type": "pay"}), ADDRESS).is_err());
}
