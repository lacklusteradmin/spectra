//! The stored history as pages, searches and summaries read it.
use super::*;
use crate::registry::Chain;
use crate::service::{HistoryQuery, HistoryQueryFilter};

/// A database holding `wallets`, so their history is visible.
fn database_with(wallets: Vec<WalletState>) -> std::sync::Arc<WalletDatabase> {
    let db = tmp_db();
    app_state_save(
        &db,
        &ResidentState {
            wallets,
            ..ResidentState::default()
        },
    )
    .unwrap();
    db
}

/// A confirmed receive on `wallet_id`'s Ethereum account.
fn receive(id: &str, wallet_id: &str, hash: &str, created_at: f64) -> HistoryRecord {
    let mut payload = history_record_on(id, wallet_id, Chain::Ethereum).payload;
    payload.kind = crate::store::wallet_domain::TransactionKind::Receive;
    payload.status = crate::store::wallet_domain::TransactionStatus::Confirmed;
    payload.wallet_name = "Éther 测试".into();
    payload.deployment_id = Some("ethereum:native".into());
    payload.transaction_hash = Some(hash.into());
    payload.created_at_unix = created_at;
    history_record_from_payload(payload)
}

/// Every id `query` pages through, following each page's cursor.
fn paged(db: &WalletDatabase, query: HistoryQuery) -> (Vec<String>, Vec<bool>) {
    let mut ids = Vec::new();
    let mut more = Vec::new();
    let mut cursor = None;
    loop {
        let page = history_page(
            db,
            &HistoryQuery {
                cursor,
                ..query.clone()
            },
        )
        .unwrap();
        ids.extend(page.records.into_iter().map(|record| record.id));
        more.push(page.has_more);
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break (ids, more),
        }
    }
}

/// One transaction is one row, however many records name it: a pending
/// copy of a confirmed transfer neither takes a page slot nor hides the
/// confirmed row, in pages, filters and the summary alike. Search matches
/// Unicode in any case; the summary caps its recent rows; a missing id is
/// none.
#[test]
fn each_transaction_is_one_row_in_pages_searches_and_the_summary() {
    let db = database_with(vec![wallet("w1", Chain::Ethereum)]);
    let mut rows: Vec<HistoryRecord> = (0..55)
        .map(|i| {
            receive(
                &format!("tx-{i:03}"),
                "w1",
                &format!("0x{i:064x}"),
                f64::from(i + 1),
            )
        })
        .collect();
    let mut duplicate = rows[54].payload.clone();
    duplicate.id = "duplicate".into();
    duplicate.kind = crate::store::wallet_domain::TransactionKind::Send;
    duplicate.status = crate::store::wallet_domain::TransactionStatus::Pending;
    rows.push(history_record_from_payload(duplicate));
    history_upsert_batch(&db, &rows).unwrap();

    let (ids, more) = paged(&db, HistoryQuery::default());
    let expected: Vec<String> = (0..55).rev().map(|i| format!("tx-{i:03}")).collect();
    assert_eq!(ids, expected);
    assert_eq!(more, [true, true, false]);
    let page = |query: HistoryQuery| history_page(&db, &query).unwrap().records;
    assert!(
        page(HistoryQuery {
            filter: HistoryQueryFilter::Pending,
            ..Default::default()
        })
        .is_empty()
    );
    assert_eq!(
        page(HistoryQuery {
            search: "éTHER 测试".into(),
            ..Default::default()
        })
        .len(),
        20
    );
    assert!(
        page(HistoryQuery {
            search: "no-such-address".into(),
            ..Default::default()
        })
        .is_empty()
    );
    assert_eq!(
        page(HistoryQuery {
            oldest_first: true,
            limit: 1,
            ..Default::default()
        })[0]
            .id,
        "tx-000"
    );

    let summary = history_snapshot(&db, &std::sync::atomic::AtomicU64::new(0)).unwrap();
    assert_eq!(summary.total_count, 55);
    assert_eq!(summary.pending_count, 0);
    let recent: Vec<_> = summary
        .recent_and_pending
        .iter()
        .map(|record| record.id.clone())
        .collect();
    assert_eq!(recent, expected[..50]);
    assert!(summary.replaceable.is_empty());
    assert_eq!(summary.earliest[0].earliest_created_at_unix, 1.0);

    let found = history_find(&db, "TX-000").unwrap().unwrap();
    assert_eq!(
        found.status,
        crate::store::wallet_domain::TransactionStatus::Confirmed
    );
    assert!(history_find(&db, "missing").unwrap().is_none());
}

/// A hash is the network's spelling: two Solana signatures that differ only
/// in case are two transactions, and so are two records of one hash whose
/// asset is unknown, which share nothing to call them one.
#[test]
fn identities_keep_case_sensitive_hashes_and_unknown_assets_apart() {
    let db = database_with(vec![wallet("w1", Chain::Solana)]);
    let rows: Vec<_> = [
        ("case-upper", "A".repeat(88), Some("solana:native")),
        ("case-lower", "a".repeat(88), Some("solana:native")),
        ("unknown-one", "B".repeat(88), None),
        ("unknown-two", "B".repeat(88), None),
    ]
    .into_iter()
    .map(|(id, hash, deployment)| {
        let mut payload = history_record_on(id, "w1", Chain::Solana).payload;
        payload.status = crate::store::wallet_domain::TransactionStatus::Confirmed;
        payload.transaction_hash = Some(hash);
        payload.deployment_id = deployment.map(str::to_string);
        payload.created_at_unix = 1.0;
        history_record_from_payload(payload)
    })
    .collect();
    history_upsert_batch(&db, &rows).unwrap();
    let mut ids = paged(&db, HistoryQuery::default()).0;
    ids.sort();
    assert_eq!(
        ids,
        ["case-lower", "case-upper", "unknown-one", "unknown-two"]
    );
}

/// Rows at one time are ordered by id in both directions; a cursor carries
/// on past its own row after that row is deleted and newer rows arrive; and
/// a cursor continues only the query it came from.
#[test]
fn a_cursor_seeks_by_time_then_id_and_continues_only_its_own_query() {
    let db = database_with(vec![wallet("w1", Chain::Ethereum)]);
    let rows: Vec<_> = (0..9)
        .map(|i| {
            receive(
                &format!("tie-{i}"),
                "w1",
                &format!("tie-{i}"),
                f64::from(i / 3 + 1),
            )
        })
        .collect();
    history_upsert_batch(&db, &rows).unwrap();
    let pages = |oldest_first| {
        paged(
            &db,
            HistoryQuery {
                oldest_first,
                limit: 2,
                ..Default::default()
            },
        )
        .0
    };
    let ties = |order: [usize; 9]| order.map(|i| format!("tie-{i}"));
    assert_eq!(pages(false), ties([6, 7, 8, 3, 4, 5, 0, 1, 2]));
    assert_eq!(pages(true), ties([0, 1, 2, 3, 4, 5, 6, 7, 8]));

    let first = history_page(
        &db,
        &HistoryQuery {
            limit: 2,
            ..Default::default()
        },
    )
    .unwrap();
    let ids: Vec<_> = first.records.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["tie-6", "tie-7"]);
    let cursor = first.next_cursor.unwrap();
    with_conn(&db, |conn| {
        conn.execute("DELETE FROM history_records WHERE id = 'tie-7'", [])
            .map_err(DbError::from)
    })
    .unwrap();
    history_upsert_batch(&db, &[receive("newest", "w1", "newest", 10.0)]).unwrap();
    let next = |query: HistoryQuery| {
        history_page(
            &db,
            &HistoryQuery {
                cursor: Some(cursor.clone()),
                limit: 2,
                ..query
            },
        )
    };
    let second: Vec<_> = next(HistoryQuery::default())
        .unwrap()
        .records
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(second, ["tie-8", "tie-3"]);
    assert_eq!(
        history_page(&db, &HistoryQuery::default()).unwrap().records[0].id,
        "newest"
    );

    for other in [
        HistoryQuery {
            oldest_first: true,
            ..Default::default()
        },
        HistoryQuery {
            filter: HistoryQueryFilter::Pending,
            ..Default::default()
        },
        HistoryQuery {
            search: "tie".into(),
            ..Default::default()
        },
    ] {
        assert_eq!(
            next(other).unwrap_err().to_string(),
            DbError::Invalid("History cursor does not match this query; restart pagination".into())
                .to_string()
        );
    }
    let broken = history_page(
        &db,
        &HistoryQuery {
            cursor: Some("broken".into()),
            ..Default::default()
        },
    );
    assert_eq!(
        broken.unwrap_err().to_string(),
        DbError::Invalid("Invalid history cursor".into()).to_string()
    );
}
