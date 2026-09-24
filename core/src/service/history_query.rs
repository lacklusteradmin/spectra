//! Bounded history queries and the small summary used outside the history screen.
use super::*;
use crate::store::persistence_models::CorePersistedTransactionRecord;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum HistoryQueryFilter {
    #[default]
    All,
    Send,
    Receive,
    Pending,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct HistoryQuery {
    pub wallet_id: Option<String>,
    pub filter: HistoryQueryFilter,
    pub search: String,
    pub oldest_first: bool,
    pub cursor: Option<String>,
    pub limit: u32,
}
impl Default for HistoryQuery {
    fn default() -> Self {
        Self {
            wallet_id: None,
            filter: HistoryQueryFilter::All,
            search: String::new(),
            oldest_first: false,
            cursor: None,
            limit: 20,
        }
    }
}
#[derive(Debug, Clone, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub records: Vec<CorePersistedTransactionRecord>,
    pub has_more: bool,
    pub next_cursor: Option<String>,
}
#[derive(Debug, Clone, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TransactionSnapshot {
    pub revision: u64,
    pub recent_and_pending: Vec<CorePersistedTransactionRecord>,
    pub replaceable: Vec<super::history_derived::ReplaceableSend>,
    pub earliest: Vec<crate::store::WalletEarliestTransactionDate>,
    pub total_count: u64,
}

/// One end of a stored transfer, and whether it is the wallet's own address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TransactionEndpoint {
    pub address: String,
    pub is_mine: bool,
}

/// The two ends of a stored transfer as the detail sheet shows them. An end
/// the record does not name, or that would repeat the other end, is absent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TransactionEndpoints {
    pub from: Option<TransactionEndpoint>,
    pub to: Option<TransactionEndpoint>,
}

/// Which ends of `record` to show, judged against the addresses the wallet is
/// known to hold. Addresses compare in the chain's own normal form.
pub(crate) fn transaction_endpoints_for(
    record: &CorePersistedTransactionRecord,
    owned: &[String],
) -> TransactionEndpoints {
    let normalize = |value: &str| crate::send::flow::normalize_address(&record.chain_id, value);
    let owned: std::collections::HashSet<String> = owned.iter().map(|a| normalize(a)).collect();
    let named = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    let is_mine = |value: &str| owned.contains(&normalize(value));
    let same = |a: &Option<String>, b: &Option<String>| match (a, b) {
        (Some(a), Some(b)) => normalize(a) == normalize(b),
        (None, None) => true,
        _ => false,
    };
    let source = named(&record.source_address);
    let counterparty = named(&Some(record.address.clone()));
    let wallet_side = source
        .clone()
        .filter(|a| is_mine(a))
        .or_else(|| counterparty.clone().filter(|a| is_mine(a)));
    let (from, to) = match record.kind {
        crate::store::wallet_domain::CoreTransactionKind::Send => {
            let to = counterparty.filter(|c| !same(&Some(c.clone()), &source));
            (source, to)
        }
        crate::store::wallet_domain::CoreTransactionKind::Receive => {
            let from = counterparty.filter(|c| !same(&Some(c.clone()), &wallet_side));
            (from, wallet_side)
        }
    };
    let endpoint = |address: String| TransactionEndpoint {
        is_mine: is_mine(&address),
        address,
    };
    TransactionEndpoints {
        from: from.map(endpoint),
        to: to.map(endpoint),
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// The ends of a stored transfer and which of them belong to the wallet.
    pub async fn transaction_endpoints(
        &self,
        transaction_id: String,
    ) -> Result<Option<TransactionEndpoints>, SpectraBridgeError> {
        let Some(record) = self.transaction(transaction_id).await? else {
            return Ok(None);
        };
        let owned = match &record.wallet_id {
            Some(wallet_id) => self.known_wallet_addresses(wallet_id.clone()).await?,
            None => Vec::new(),
        };
        Ok(Some(transaction_endpoints_for(&record, &owned)))
    }

    pub async fn history_page(
        &self,
        query: HistoryQuery,
    ) -> Result<HistoryPage, SpectraBridgeError> {
        if query.limit == 0 || query.limit > 200 {
            return Err("history query limit must be 1...200".into());
        }
        let database = self.bound_database().await?;
        tokio::task::spawn_blocking(move || crate::wallet_db::history_page(&database, &query))
            .await
            .map_err(|e| SpectraBridgeError::from(e.to_string()))?
            .map(|mut page| {
                page.records = page
                    .records
                    .into_iter()
                    .map(CorePersistedTransactionRecord::with_actions)
                    .collect();
                page
            })
            .map_err(Into::into)
    }
    pub async fn transaction_snapshot(&self) -> Result<TransactionSnapshot, SpectraBridgeError> {
        let database = self.bound_database().await?;
        let sequence = self.projection_sequence.clone();
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::history_snapshot(&database, &sequence)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(e.to_string()))?
        .map(|mut snapshot| {
            snapshot.recent_and_pending = snapshot
                .recent_and_pending
                .into_iter()
                .map(CorePersistedTransactionRecord::with_actions)
                .collect();
            snapshot
        })
        .map_err(Into::into)
    }
    pub async fn transaction(
        &self,
        id: String,
    ) -> Result<Option<CorePersistedTransactionRecord>, SpectraBridgeError> {
        let database = self.bound_database().await?;
        tokio::task::spawn_blocking(move || crate::wallet_db::history_find(&database, &id))
            .await
            .map_err(|e| SpectraBridgeError::from(e.to_string()))?
            .map(|record| record.map(CorePersistedTransactionRecord::with_actions))
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod endpoint_tests {
    use super::*;

    fn record(kind: &str, address: &str, source: Option<&str>) -> CorePersistedTransactionRecord {
        serde_json::from_value(serde_json::json!({
            "id": "t", "walletId": "w", "kind": kind, "status": "confirmed",
            "walletName": "W", "assetDisplayName": "Ether", "symbol": "ETH",
            "chainId": "ethereum", "amount": "1", "address": address,
            "sourceAddress": source, "createdAtUnix": 1.0
        }))
        .unwrap()
    }

    const MINE: &str = "0x1111111111111111111111111111111111111111";
    const THEIRS: &str = "0x2222222222222222222222222222222222222222";

    #[test]
    fn a_send_runs_from_the_source_to_the_counterparty() {
        let ends = transaction_endpoints_for(&record("send", THEIRS, Some(MINE)), &[MINE.into()]);
        assert_eq!(
            ends.from,
            Some(TransactionEndpoint {
                address: MINE.into(),
                is_mine: true
            })
        );
        assert_eq!(
            ends.to,
            Some(TransactionEndpoint {
                address: THEIRS.into(),
                is_mine: false
            })
        );
    }

    /// Ownership compares normal forms: a checksummed EVM address is the same
    /// address as its lowercase spelling.
    #[test]
    fn a_receive_ends_at_the_wallet_in_any_case() {
        let ends = transaction_endpoints_for(
            &record(
                "receive",
                &MINE.to_uppercase().replacen("0X", "0x", 1),
                None,
            ),
            &[MINE.into()],
        );
        assert_eq!(ends.from, None, "the counterparty is the wallet itself");
        assert!(ends.to.is_some_and(|to| to.is_mine));
    }

    #[test]
    fn a_self_send_names_its_address_once() {
        let ends = transaction_endpoints_for(&record("send", MINE, Some(MINE)), &[MINE.into()]);
        assert!(ends.from.is_some());
        assert_eq!(ends.to, None);
    }
}
