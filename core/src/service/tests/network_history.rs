//! Each adapter's answer, as it serializes, against the row it is stored as.
use super::*;
use serde_json::Value;

/// What `chain`'s history adapter makes of `raw`, its serialized answer.
fn entries(chain: Chain, raw: &str) -> Vec<HistoryEntry> {
    fn read<T: serde::de::DeserializeOwned>(
        raw: &str,
        convert: fn(T) -> Option<HistoryEntry>,
    ) -> Vec<HistoryEntry> {
        serde_json::from_str::<Vec<T>>(raw)
            .expect("an adapter's own serialization")
            .into_iter()
            .filter_map(convert)
            .collect()
    }
    use crate::api::*;
    if chain.uses_utxo_client() {
        return read(raw, utxo::UtxoHistoryEntry::history_entry);
    }
    if chain.is_evm() {
        return read(raw, blockscout::EvmHistoryEntry::history_entry);
    }
    match chain.mainnet_counterpart() {
        Chain::Xrp => read(raw, xrpl_json_rpc::XrpHistoryEntry::history_entry),
        Chain::Stellar => read(raw, horizon::StellarHistoryEntry::history_entry),
        Chain::Cardano => read(raw, koios::CardanoHistoryEntry::history_entry),
        Chain::Solana => read(raw, solana_json_rpc::SolanaTransfer::history_entry),
        Chain::Tron => read(raw, trongrid_v1::TronTransfer::history_entry),
        Chain::Sui => read(raw, sui_json_rpc::SuiHistoryEntry::history_entry),
        Chain::Ton => read(raw, toncenter_v2::TonHistoryEntry::history_entry),
        Chain::Near => read(raw, nearblocks::NearHistoryEntry::history_entry),
        Chain::Icp => read(raw, icp_rosetta::IcpHistoryEntry::history_entry),
        Chain::Kaspa => read(raw, kaspa_rest::KasHistoryEntry::history_entry),
        Chain::Decred => read(raw, insight::DcrHistoryEntry::history_entry),
        other => panic!("{other} has no history fixture"),
    }
}

fn normalize_chain_history(chain: Chain, raw: &str) -> Vec<NormalizedHistoryItem> {
    entries(chain, raw)
        .into_iter()
        .filter_map(|entry| normalize_history_entry(chain, entry))
        .collect()
}

#[test]
fn spl_history_labels_resolve_by_network_and_case_sensitive_mint() {
    let mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
    let normalize = |chain: crate::registry::Chain, mint: &str, symbol: &str| {
        normalize_chain_history(
            chain,
            &serde_json::json!([{
                "signature": "transfer", "slot": 1, "fee_lamports": 5000, "mint": mint, "symbol": symbol,
                "amount_display": "42.5", "is_incoming": true,
                "timestamp": 1700000012, "from": "sender", "to": "owner"
            }])
            .to_string(),
        )
    };
    for supplied_symbol in [mint, "FAKE"] {
        let mut rows = normalize(Chain::Solana, mint, supplied_symbol);
        assert_eq!(rows.len(), 1);
        let row = rows.remove(0);
        assert_eq!(row.symbol, "USDC");
        assert_eq!(row.asset_display_name, "USD Coin");
        assert_eq!(row.deployment_id, Some(format!("solana:spl:{mint}")));
        assert_eq!(row.amount, "42.5");
    }
    for (chain, identifier) in [
        (Chain::SolanaDevnet, mint.to_string()),
        (Chain::Solana, mint.replacen('j', "J", 1)),
    ] {
        let mut rows = normalize(chain, &identifier, "USDC");
        assert_eq!(rows.len(), 1);
        let row = rows.remove(0);
        assert_eq!(row.symbol, identifier);
        assert_eq!(row.asset_display_name, identifier);
    }
    // Folding every letter changes this mint into a 33-byte value, which
    // cannot name a Solana asset and must not produce a history row.
    assert!(normalize(Chain::Solana, &mint.to_lowercase(), "USDC").is_empty());
}

/// A token that borrows a known ticker on another contract is filed under
/// its own contract and shown as itself, not as the token it imitates.
#[test]
fn a_borrowed_ticker_does_not_borrow_an_identity() {
    let rows = normalize_chain_history(
        Chain::Tron,
        r#"[{"txid":"x","timestamp_ms":1700000013000,"from":"TFrom","to":"TTo","amount_display":"7.5","symbol":"USDT","contract":"TXYZopYRdj2D9XRtbG411XZZ3kM5VkAeBf","is_incoming":true}]"#,
    );
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(
        row.deployment_id,
        crate::catalog::tokens::deployment_id_for(
            Chain::Tron,
            Some("TXYZopYRdj2D9XRtbG411XZZ3kM5VkAeBf")
        )
    );
    assert_ne!(row.symbol, "USDT");
    assert_ne!(row.asset_display_name, "Tether USD");
}

/// One populated entry per history adapter, in the JSON its entry serializes
/// as, against the row it must normalize to. Units and timestamp scales live in
/// each adapter's `history_entry`, so this is where a wrong one shows up.
const CASES: &[(crate::registry::Chain, &str, &str)] = &[
    (
        Chain::Bitcoin,
        r#"[{"txid":"a1","confirmed":true,"block_height":800000,"block_time":1700000000,"net_sats":-12345}]"#,
        r#"[{"kind":"send","status":"confirmed","asset_display_name":"Bitcoin","symbol":"BTC","chain_id":"bitcoin","amount":"0.00012345","counterparty":"","tx_hash":"a1","block_height":800000,"timestamp":1700000000.0}]"#,
    ),
    (
        Chain::Bitcoin,
        r#"[{"txid":"a2","confirmed":false,"block_height":null,"block_time":1700000001,"net_sats":6789}]"#,
        r#"[{"kind":"receive","status":"pending","asset_display_name":"Bitcoin","symbol":"BTC","chain_id":"bitcoin","amount":"0.00006789","counterparty":"","tx_hash":"a2","block_height":null,"timestamp":1700000001.0}]"#,
    ),
    (
        Chain::Litecoin,
        r#"[{"txid":"b1","confirmed":true,"block_height":250000,"block_time":1700000002,"net_sats":-500000}]"#,
        r#"[{"kind":"send","status":"confirmed","asset_display_name":"Litecoin","symbol":"LTC","chain_id":"litecoin","amount":"0.005","counterparty":"","tx_hash":"b1","block_height":250000,"timestamp":1700000002.0}]"#,
    ),
    (
        Chain::BitcoinCash,
        r#"[{"txid":"b2","confirmed":false,"block_height":null,"block_time":1700000003,"net_sats":700000}]"#,
        r#"[{"kind":"receive","status":"pending","asset_display_name":"Bitcoin Cash","symbol":"BCH","chain_id":"bitcoin-cash","amount":"0.007","counterparty":"","tx_hash":"b2","block_height":null,"timestamp":1700000003.0}]"#,
    ),
    (
        Chain::BitcoinSV,
        r#"[{"txid":"b3","confirmed":true,"block_height":10,"block_time":1700000004,"net_sats":900000}]"#,
        r#"[{"kind":"receive","status":"confirmed","asset_display_name":"Bitcoin SV","symbol":"BSV","chain_id":"bitcoin-sv","amount":"0.009","counterparty":"","tx_hash":"b3","block_height":10,"timestamp":1700000004.0}]"#,
    ),
    (
        Chain::Dogecoin,
        r#"[{"txid":"c1","confirmed":true,"block_height":5,"block_time":1700000005,"net_sats":-123456789}]"#,
        r#"[{"kind":"send","status":"confirmed","asset_display_name":"Dogecoin","symbol":"DOGE","chain_id":"dogecoin","amount":"1.23456789","counterparty":"","tx_hash":"c1","block_height":5,"timestamp":1700000005.0}]"#,
    ),
    (
        Chain::Xrp,
        r#"[{"txid":"d1","ledger_index":1,"timestamp":1700000006,"from":"rFrom","to":"rTo","amount_drops":250000,"fee_drops":12,"is_incoming":true}]"#,
        r#"[{"kind":"receive","status":"confirmed","asset_display_name":"XRP","symbol":"XRP","chain_id":"xrp","amount":"0.25","counterparty":"rFrom","tx_hash":"d1","block_height":null,"timestamp":1700000006.0}]"#,
    ),
    (
        Chain::Stellar,
        r#"[{"txid":"e1","ledger":1,"timestamp":1700000000,"from":"GFrom","to":"GTo","amount_stroops":3000000,"fee_charged":100,"is_incoming":false}]"#,
        r#"[{"kind":"send","status":"confirmed","asset_display_name":"Stellar","symbol":"XLM","chain_id":"stellar","amount":"0.3","counterparty":"GTo","tx_hash":"e1","block_height":null,"timestamp":1700000000.0}]"#,
    ),
    (
        Chain::Stellar,
        r#"[{"txid":"e2","ledger":2,"timestamp":1700000008,"from":"GFrom","to":"GTo","amount_stroops":-4000000,"fee_charged":100,"is_incoming":true}]"#,
        r#"[{"kind":"receive","status":"confirmed","asset_display_name":"Stellar","symbol":"XLM","chain_id":"stellar","amount":"0.4","counterparty":"GFrom","tx_hash":"e2","block_height":null,"timestamp":1700000008.0}]"#,
    ),
    (
        Chain::Cardano,
        r#"[{"txid":"f1","block":"b1","block_time":1700000009,"amount_lovelace":-5000000,"fee_lovelace":170000,"is_incoming":false}]"#,
        r#"[{"kind":"send","status":"confirmed","asset_display_name":"Cardano","symbol":"ADA","chain_id":"cardano","amount":"5","counterparty":"","tx_hash":"f1","block_height":null,"timestamp":1700000009.0}]"#,
    ),
    (
        Chain::Solana,
        r#"[{"signature":"h1","slot":1,"timestamp":1700000011,"fee_lamports":5000,"is_incoming":false,"amount_display":"1.25","symbol":"SOL","mint":"","from":"sFrom","to":"sTo"}]"#,
        r#"[{"kind":"send","status":"confirmed","asset_display_name":"Solana","symbol":"SOL","chain_id":"solana","amount":"1.25","counterparty":"sTo","tx_hash":"h1","block_height":null,"timestamp":1700000011.0}]"#,
    ),
    (
        Chain::Solana,
        r#"[{"signature":"h2","slot":2,"timestamp":1700000012,"fee_lamports":5000,"is_incoming":true,"amount_display":"42.5","symbol":"USDC","mint":"EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v","from":"sFrom","to":"sTo"}]"#,
        r#"[{"kind":"receive","status":"confirmed","asset_display_name":"USD Coin","symbol":"USDC","chain_id":"solana","amount":"42.5","counterparty":"sFrom","tx_hash":"h2","block_height":null,"timestamp":1700000012.0}]"#,
    ),
    (
        Chain::Tron,
        r#"[{"txid":"i1","timestamp_ms":1700000013000,"from":"TFrom","to":"TTo","amount_display":"7.5","symbol":"USDT","contract":"TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t","is_incoming":true}]"#,
        r#"[{"kind":"receive","status":"confirmed","asset_display_name":"Tether USD","symbol":"USDT","chain_id":"tron","amount":"7.5","counterparty":"TFrom","tx_hash":"i1","block_height":null,"timestamp":1700000013.0}]"#,
    ),
    (
        Chain::Tron,
        r#"[{"txid":"i2","timestamp_ms":1700000014000,"from":"TFrom","to":"TTo","amount_display":"3.5","symbol":"TRX","is_incoming":false}]"#,
        r#"[{"kind":"send","status":"confirmed","asset_display_name":"Tron","symbol":"TRX","chain_id":"tron","amount":"3.5","counterparty":"TTo","tx_hash":"i2","block_height":null,"timestamp":1700000014.0}]"#,
    ),
    // The token catalog names a TRC-20, so every token on the chain has a
    // name.
    (
        Chain::Tron,
        r#"[{"txid":"i3","timestamp_ms":1700000014000,"from":"TFrom","to":"TTo","amount_display":"2.0","symbol":"TUSD","contract":"TUpMhErZL2fhh4sVNULAbNKLokS4GjC1F4","is_incoming":true}]"#,
        r#"[{"kind":"receive","status":"confirmed","asset_display_name":"TrueUSD","symbol":"TUSD","chain_id":"tron","amount":"2","counterparty":"TFrom","tx_hash":"i3","block_height":null,"timestamp":1700000014.0}]"#,
    ),
    // A ticker the catalog does not carry stays the ticker: a row nobody
    // can name is still a row, and inventing a name for it would be worse.
    (
        Chain::Tron,
        r#"[{"txid":"i4","timestamp_ms":1700000014000,"from":"TFrom","to":"TTo","amount_display":"1.0","symbol":"NOTATOKEN","is_incoming":true}]"#,
        r#"[{"kind":"receive","status":"confirmed","asset_display_name":"Tron","symbol":"TRX","chain_id":"tron","amount":"1","counterparty":"TFrom","tx_hash":"i4","block_height":null,"timestamp":1700000014.0}]"#,
    ),
    (
        Chain::Sui,
        r#"[{"digest":"j1","amount_mist":"800000000","timestamp_ms":1700000015000,"is_incoming":true,"from":"suiSender","to":"suiMe"}]"#,
        r#"[{"kind":"receive","status":"confirmed","asset_display_name":"Sui","symbol":"SUI","chain_id":"sui","amount":"0.8","counterparty":"suiSender","tx_hash":"j1","block_height":null,"timestamp":1700000015.0}]"#,
    ),
    (
        Chain::Ton,
        r#"[{"txid":"l1","amount_nanotons":1000000000,"fee_nanotons":0,"timestamp":1700000017,"from":"tFrom","to":"tTo","is_incoming":true,"comment":null}]"#,
        r#"[{"kind":"receive","status":"confirmed","asset_display_name":"Gram","symbol":"GRAM","chain_id":"ton","amount":"1","counterparty":"tFrom","tx_hash":"l1","block_height":null,"timestamp":1700000017.0}]"#,
    ),
    (
        Chain::Near,
        r#"[{"txid":"m1","timestamp_ns":1700000018000000000,"from":"nSigner","to":"nReceiver","amount_yocto":"1500000000000000000000000","is_incoming":false}]"#,
        r#"[{"kind":"send","status":"confirmed","asset_display_name":"NEAR","symbol":"NEAR","chain_id":"near","amount":"1.5","counterparty":"nReceiver","tx_hash":"m1","block_height":null,"timestamp":1700000018.0}]"#,
    ),
    (
        Chain::Icp,
        r#"[{"block_index":42,"amount_e8s":250000000,"fee_e8s":10000,"timestamp_ns":1700000019000000000,"from":"iFrom","to":"iTo","is_incoming":true}]"#,
        r#"[{"kind":"receive","status":"confirmed","asset_display_name":"Internet Computer","symbol":"ICP","chain_id":"internet-computer","amount":"2.5","counterparty":"iFrom","tx_hash":"42","block_height":null,"timestamp":1700000019.0}]"#,
    ),
    (
        Chain::Ethereum,
        r#"[{"txid":"p1","is_incoming":true,"status":"confirmed","value_wei":"1500000000000000000","fee_wei":"0","from":"0xFrom","to":"0xTo","block_number":18000000,"timestamp":1700000021}]"#,
        r#"[{"kind":"receive","status":"confirmed","asset_display_name":"Ethereum","symbol":"ETH","chain_id":"ethereum","amount":"1.5","counterparty":"0xFrom","tx_hash":"p1","block_height":18000000,"timestamp":1700000021.0}]"#,
    ),
    (
        Chain::Polygon,
        r#"[{"txid":"p2","is_incoming":false,"status":"confirmed","value_wei":"250000000000000000","fee_wei":"0","from":"0xFrom","to":"0xTo","block_number":49000000,"timestamp":1700000022}]"#,
        r#"[{"kind":"send","status":"confirmed","asset_display_name":"Polygon","symbol":"POL","chain_id":"polygon","amount":"0.25","counterparty":"0xTo","tx_hash":"p2","block_height":49000000,"timestamp":1700000022.0}]"#,
    ),
    (
        Chain::Kaspa,
        r#"[{"txid":"k1","block_daa_score":369401244,"timestamp":1772543921441,"amount_sompi":-39800000000,"is_incoming":false}]"#,
        r#"[{"kind":"send","status":"confirmed","asset_display_name":"Kaspa","symbol":"KAS","chain_id":"kaspa","amount":"398","counterparty":"","tx_hash":"k1","block_height":null,"timestamp":1772543921.441}]"#,
    ),
    (
        Chain::Decred,
        r#"[{"txid":"d1","block_height":900000,"timestamp":1700000025,"amount_atoms":250000000,"fee_atoms":2980,"is_incoming":true}]"#,
        r#"[{"kind":"receive","status":"confirmed","asset_display_name":"Decred","symbol":"DCR","chain_id":"decred","amount":"2.5","counterparty":"","tx_hash":"d1","block_height":900000,"timestamp":1700000025.0}]"#,
    ),
    (
        Chain::BitcoinTestnet,
        r#"[{"txid":"q1","confirmed":true,"block_height":2500000,"block_time":1700000023,"net_sats":4242}]"#,
        r#"[{"kind":"receive","status":"confirmed","asset_display_name":"Test Bitcoin","symbol":"tBTC","chain_id":"bitcoin-testnet","amount":"0.00004242","counterparty":"","tx_hash":"q1","block_height":2500000,"timestamp":1700000023.0}]"#,
    ),
    (
        Chain::LitecoinTestnet,
        r#"[{"txid":"q2","confirmed":true,"block_height":9,"block_time":1700000024,"net_sats":31337}]"#,
        r#"[{"kind":"receive","status":"confirmed","asset_display_name":"Test Litecoin","symbol":"tLTC","chain_id":"litecoin-testnet","amount":"0.00031337","counterparty":"","tx_hash":"q2","block_height":9,"timestamp":1700000024.0}]"#,
    ),
];

/// A base-unit integer is shifted, not divided as a float.
#[test]
fn base_unit_amounts_convert_exactly() {
    let rows = normalize_chain_history(
        Chain::Near,
        r#"[{"txid":"n","timestamp_ns":1,"from":"a","to":"b","amount_yocto":"100000000000000000000000","is_incoming":true}]"#,
    );
    assert_eq!(rows[0].amount, "0.1");
}

#[test]
fn every_chain_shape_normalizes_to_its_expected_row() {
    for (chain, raw, expected) in CASES {
        let rows = normalize_chain_history(*chain, raw);
        let mut actual = serde_json::to_value(&rows).unwrap();
        let sources: Vec<Value> = serde_json::from_str(raw).unwrap();
        for (row, source) in actual.as_array_mut().unwrap().iter_mut().zip(&sources) {
            let identity = row
                .as_object_mut()
                .unwrap()
                .remove("deployment_id")
                .unwrap();
            // Identity comes from the contract the row carries, or the
            // chain's own coin when it carries none — never from the ticker.
            let contract = source
                .get("mint")
                .or_else(|| source.get("contract"))
                .and_then(Value::as_str)
                .filter(|c| !c.is_empty());
            let expected = crate::catalog::tokens::deployment_id_for(*chain, contract);
            assert_eq!(identity, serde_json::to_value(expected).unwrap(), "{chain}");
        }
        assert_eq!(
            actual,
            serde_json::from_str::<Value>(expected).unwrap(),
            "{chain} normalized differently"
        );
    }
}

/// A testnet's client is its mainnet's client returning the same JSON, so
/// its history has to normalize the same way.
#[test]
fn testnets_normalize_like_their_mainnets() {
    for (testnet, mainnet) in [
        (Chain::BitcoinTestnet, Chain::Bitcoin),
        (Chain::BitcoinTestnet4, Chain::Bitcoin),
        (Chain::BitcoinSignet, Chain::Bitcoin),
        (Chain::LitecoinTestnet, Chain::Litecoin),
        (Chain::BitcoinCashTestnet, Chain::BitcoinCash),
        (Chain::DogecoinTestnet, Chain::Dogecoin),
        (Chain::EthereumSepolia, Chain::Ethereum),
    ] {
        let raw = CASES
            .iter()
            .find(|(chain, _, _)| *chain == mainnet)
            .map(|(_, raw, _)| *raw)
            .expect("mainnet fixture");
        let rows = normalize_chain_history(testnet, raw);
        assert_eq!(rows.len(), 1, "{testnet} normalized nothing");
        let mainnet_rows = normalize_chain_history(mainnet, raw);
        assert_eq!(rows[0].kind, mainnet_rows[0].kind);
        assert_eq!(rows[0].amount, mainnet_rows[0].amount);
        assert_eq!(rows[0].tx_hash, mainnet_rows[0].tx_hash);
        assert_eq!(rows[0].timestamp, mainnet_rows[0].timestamp);
    }
}

/// `amount` is a magnitude and `kind` says which way the transfer went.
/// The sign of the raw field only decides direction where the chain sends
/// no `is_incoming` flag.
#[test]
fn amounts_are_magnitudes_whatever_sign_the_chain_reports() {
    let negative = r#"[{"txid":"x","ledger":1,"amount_stroops":-60000000,"fee_charged":100,"timestamp":1,"from":"a","to":"b","is_incoming":true}]"#;
    let rows = normalize_chain_history(Chain::Stellar, negative);
    assert_eq!(rows[0].amount, "6");
    assert_eq!(rows[0].kind, TransactionKind::Receive);

    let unsigned = r#"[{"txid":"y","net_sats":-500,"confirmed":true,"block_time":1}]"#;
    let rows = normalize_chain_history(Chain::Bitcoin, unsigned);
    assert_eq!(rows[0].amount, "0.000005");
    assert_eq!(
        rows[0].kind,
        TransactionKind::Send,
        "no is_incoming flag: the sign decides"
    );
}

/// An EVM row says how its execution ended. One that says something no
/// status names is refused, not read as pending.
#[test]
fn an_evm_row_without_a_known_status_is_refused() {
    let row = |status: &str| {
        normalize_chain_history(
            Chain::Ethereum,
            &format!(
                r#"[{{"txid":"p","is_incoming":true,"status":"{status}","value_wei":"1","fee_wei":"0","from":"a","to":"b","block_number":1,"timestamp":1700000000}}]"#
            ),
        )
    };
    assert_eq!(row("failed")[0].status, TransactionStatus::Failed);
    assert!(row("reverted").is_empty(), "unknown");
}

/// A time the chain has not given yet normalizes to the unknown 0. A zero
/// is a reading gone wrong, and yields no row rather than the Unix epoch.
#[test]
fn an_undated_transaction_has_the_unknown_time() {
    let row = |time: &str| {
        normalize_chain_history(
            Chain::Litecoin,
            &format!(
                r#"[{{"txid":"t","net_sats":1,"confirmed":false,"block_height":null{time}}}]"#
            ),
        )
    };
    assert_eq!(row(r#","block_time":null"#)[0].timestamp, 0.0);
    assert_eq!(
        row(r#","block_time":1700000000"#)[0].timestamp,
        1_700_000_000.0
    );
    assert!(row(r#","block_time":0"#).is_empty(), "zero");
}

/// The entries the service builds itself, from Monero's local scan, take the
/// same path as an adapter's.
#[test]
fn a_local_monero_transfer_normalizes_like_an_adapters() {
    use crate::api::history_page::{HistoryAmount, HistoryOutcome};
    let row = normalize_history_entry(
        Chain::Monero,
        HistoryEntry {
            txid: "o1".into(),
            contract: None,
            amount: HistoryAmount::Units(1_250_000_000_000),
            incoming: false,
            outcome: HistoryOutcome::Confirmed,
            block_height: None,
            timestamp: Some(1_700_000_020.0),
            counterparty: String::new(),
        },
    )
    .unwrap();
    assert_eq!(row.amount, "1.25");
    assert_eq!(row.symbol, "XMR");
    assert_eq!(row.kind, TransactionKind::Send);
    assert_eq!(row.timestamp, 1_700_000_020.0);
}
