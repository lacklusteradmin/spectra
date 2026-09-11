//! Esplora pagination shared by a single address and an HD account.
//!
//! Provider pages and UI pages are different sizes. The opaque core cursor
//! retains unconsumed rows and each address's provider cursor. Complete block
//! cohorts are merged before being displayed, so an HD transfer split across
//! addresses or provider pages is counted exactly once.
use super::chains::bitcoin::BitcoinHistoryEntry;
use crate::history::CoreBitcoinHistorySnapshot;
use futures::{stream, StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::future::Future;

pub(crate) struct HistoryPage<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct AddressCursor {
    address: String,
    after: Option<String>,
    exhausted: bool,
    rows: VecDeque<BitcoinHistoryEntry>,
    visited: HashSet<String>,
}

#[derive(Serialize, Deserialize)]
struct Cursor {
    network: String,
    sources: Vec<AddressCursor>,
    ready: VecDeque<CoreBitcoinHistorySnapshot>,
}

fn height(row: &BitcoinHistoryEntry) -> u64 {
    if row.confirmed {
        row.block_height.unwrap_or(0)
    } else {
        u64::MAX
    }
}

async fn refill<F, Fut>(source: &mut AddressCursor, fetch: &F) -> Result<(), String>
where
    F: Fn(String, Option<String>) -> Fut,
    Fut: Future<Output = Result<Vec<BitcoinHistoryEntry>, String>>,
{
    let rows = fetch(source.address.clone(), source.after.clone()).await?;
    accept_page(source, rows)
}

fn accept_page(source: &mut AddressCursor, rows: Vec<BitcoinHistoryEntry>) -> Result<(), String> {
    if rows
        .iter()
        .any(|r| r.txid.is_empty() || (r.confirmed && r.block_height.is_none()))
    {
        return Err("bitcoin history: provider returned an incomplete transaction".into());
    }
    if rows
        .windows(2)
        .any(|pair| height(&pair[0]) < height(&pair[1]))
    {
        return Err("bitcoin history: provider page is not in descending block order".into());
    }
    let confirmed = rows.iter().filter(|r| r.confirmed).count();
    if let Some(last) = rows.iter().rev().find(|r| r.confirmed) {
        if !source.visited.insert(last.txid.clone()) {
            return Err("bitcoin history: provider repeated a pagination cursor".into());
        }
        source.after = Some(last.txid.clone());
    }
    // Esplora returns up to 25 confirmed transactions, plus mempool rows on
    // the initial request. An exactly full page needs another request.
    source.exhausted = confirmed < 25;
    source.rows.extend(rows);
    Ok(())
}

pub(crate) async fn page<F, Fut>(
    network: &str,
    addresses: &[String],
    previous: Option<&str>,
    limit: usize,
    fetch: F,
) -> Result<HistoryPage<CoreBitcoinHistorySnapshot>, String>
where
    F: Fn(String, Option<String>) -> Fut,
    Fut: Future<Output = Result<Vec<BitcoinHistoryEntry>, String>>,
{
    if addresses.is_empty() || limit == 0 {
        return Err("bitcoin history: empty scope or page size".into());
    }
    let mut cursor = match previous {
        Some(raw) => {
            let saved: Cursor =
                serde_json::from_str(raw).map_err(|e| format!("bitcoin history cursor: {e}"))?;
            if saved.network != network
                || !saved
                    .sources
                    .iter()
                    .map(|s| &s.address)
                    .eq(addresses.iter())
            {
                return Err("bitcoin history scope changed; refresh before loading more".into());
            }
            saved
        }
        None => Cursor {
            network: network.to_string(),
            ready: VecDeque::new(),
            sources: addresses
                .iter()
                .map(|address| AddressCursor {
                    address: address.clone(),
                    after: None,
                    exhausted: false,
                    rows: VecDeque::new(),
                    visited: HashSet::new(),
                })
                .collect(),
        },
    };
    let mut items = Vec::new();
    while items.len() < limit {
        if let Some(item) = cursor.ready.pop_front() {
            items.push(item);
            continue;
        }
        // Keep the HD fan-out bounded without paying one network round trip
        // per address in sequence. Each future exclusively owns its cursor.
        let mut requests = Vec::new();
        for (index, source) in cursor.sources.iter().enumerate() {
            if source.rows.is_empty() && !source.exhausted {
                requests.push((index, fetch(source.address.clone(), source.after.clone())));
            }
        }
        let pages = stream::iter(requests)
            .map(|(index, request)| async move { request.await.map(|rows| (index, rows)) })
            .buffer_unordered(4)
            .try_collect::<Vec<_>>()
            .await?;
        for (index, rows) in pages {
            accept_page(&mut cursor.sources[index], rows)?;
        }
        let Some(top) = cursor
            .sources
            .iter()
            .filter_map(|s| s.rows.front().map(height))
            .max()
        else {
            break;
        };
        let mut cohort = Vec::new();
        for source in &mut cursor.sources {
            loop {
                while source.rows.front().is_some_and(|r| height(r) == top) {
                    cohort.push(source.rows.pop_front().unwrap());
                }
                if source.rows.is_empty() && !source.exhausted {
                    refill(source, &fetch).await?;
                } else {
                    break;
                }
            }
        }
        // Sum in satoshis before conversion; floats must not decide whether
        // an internal transfer has a zero net change.
        let mut grouped = std::collections::BTreeMap::<String, (i128, BitcoinHistoryEntry)>::new();
        for row in cohort {
            let entry = grouped.entry(row.txid.clone()).or_insert((0, row.clone()));
            entry.0 += i128::from(row.net_sats);
        }
        let mut merged = grouped
            .into_values()
            .filter(|(net, _)| *net != 0)
            .map(|(net, row)| CoreBitcoinHistorySnapshot {
                txid: row.txid,
                amount_btc: net.unsigned_abs() as f64 / 100_000_000.0,
                kind: if net > 0 { "receive" } else { "send" }.into(),
                status: if row.confirmed {
                    "confirmed"
                } else {
                    "pending"
                }
                .into(),
                counterparty_address: String::new(),
                block_height: row.block_height.map(|h| h as i64),
                created_at_unix: row.block_time.unwrap_or(0) as f64,
            })
            .collect::<Vec<_>>();
        merged.sort_by(|a, b| {
            b.created_at_unix
                .total_cmp(&a.created_at_unix)
                .then_with(|| a.txid.cmp(&b.txid))
        });
        cursor.ready.extend(merged);
    }
    let more = !cursor.ready.is_empty()
        || cursor
            .sources
            .iter()
            .any(|s| !s.exhausted || !s.rows.is_empty());
    let next_cursor = more
        .then(|| serde_json::to_string(&cursor))
        .transpose()
        .map_err(|e| e.to_string())?;
    Ok(HistoryPage { items, next_cursor })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rows(count: u64) -> Vec<BitcoinHistoryEntry> {
        (1..=count)
            .rev()
            .map(|i| BitcoinHistoryEntry {
                txid: format!("{i:064x}"),
                confirmed: true,
                block_height: Some(i),
                block_time: Some(i),
                net_sats: 100,
                fee_sats: None,
            })
            .collect()
    }
    fn provider(all: Vec<BitcoinHistoryEntry>, after: Option<String>) -> Vec<BitcoinHistoryEntry> {
        let start = after
            .map(|id| all.iter().position(|r| r.txid == id).unwrap() + 1)
            .unwrap_or(0);
        all.into_iter().skip(start).take(25).collect()
    }
    #[tokio::test]
    async fn pages_do_not_repeat_or_drop_rows_at_provider_boundaries() {
        for count in [0, 10, 20, 25, 26, 50, 61] {
            let mut cursor = None;
            let mut seen = Vec::new();
            let calls = std::sync::Mutex::new(Vec::new());
            for _ in 0..10 {
                let result = page(
                    "bitcoin",
                    &["a".into()],
                    cursor.as_deref(),
                    10,
                    |_, after| {
                        calls.lock().unwrap().push(after.clone());
                        std::future::ready(Ok(provider(rows(count), after)))
                    },
                )
                .await
                .unwrap();
                seen.extend(result.items.into_iter().map(|i| i.txid));
                cursor = result.next_cursor;
                if cursor.is_none() {
                    break;
                }
            }
            assert!(cursor.is_none());
            assert_eq!(
                seen,
                rows(count).into_iter().map(|r| r.txid).collect::<Vec<_>>()
            );
            let calls = calls.into_inner().unwrap();
            assert_eq!(calls.iter().filter(|c| c.is_none()).count(), 1);
            assert_eq!(calls.len(), count as usize / 25 + 1);
        }
    }
    #[tokio::test]
    async fn hd_cohort_crossing_a_provider_page_is_merged_before_ui_pagination() {
        let mut a = rows(30);
        for r in &mut a {
            r.block_height = Some(100);
        }
        let mut b = a.clone();
        for r in &mut b {
            r.net_sats = -99;
        }
        // The same block may arrive in different transaction order per address.
        b.reverse();
        let mut cursor = None;
        let mut items = Vec::new();
        loop {
            let p = page(
                "bitcoin",
                &["a".into(), "b".into()],
                cursor.as_deref(),
                10,
                |address, after| {
                    std::future::ready(Ok(provider(
                        if address == "a" { a.clone() } else { b.clone() },
                        after,
                    )))
                },
            )
            .await
            .unwrap();
            items.extend(p.items);
            cursor = p.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(items.len(), 30);
        assert!(items
            .iter()
            .all(|r| r.amount_btc == 0.00000001 && r.kind == "receive"));
        assert_eq!(
            items.iter().map(|r| &r.txid).collect::<HashSet<_>>().len(),
            30
        );
    }
    #[tokio::test]
    async fn failures_and_repeated_cursors_do_not_claim_exhaustion() {
        let p = page("bitcoin", &["a".into()], None, 10, |_, after| {
            std::future::ready(Ok(provider(rows(50), after)))
        })
        .await
        .unwrap();
        assert!(page(
            "bitcoin-testnet-4",
            &["a".into()],
            p.next_cursor.as_deref(),
            10,
            |_, _| std::future::ready(Ok(vec![]))
        )
        .await
        .is_err());
        assert!(page(
            "bitcoin",
            &["a".into()],
            p.next_cursor.as_deref(),
            100,
            |_, _| std::future::ready(Err("offline".into()))
        )
        .await
        .is_err());
        assert!(page("bitcoin", &["a".into()], None, 100, |_, _| {
            std::future::ready(Ok(rows(25)))
        })
        .await
        .is_err());
    }
}
