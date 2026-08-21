//! Views of the transaction store, derived where the store is.
//!
//! Three exports used to take the transaction list as an argument —
//! `core_normalize_history`, `core_earliest_transaction_dates`,
//! `core_active_wallet_transaction_ids` — so a caller converted its projection
//! of core's own records into three different FFI input shapes and handed them
//! back for core to reduce. A fourth, `core_normalized_history_signature`,
//! existed only to let that caller decide whether the round trip was worth
//! making; core decides that where the data is, so it is gone.

use crate::service::WalletService;
use crate::store::wallet_domain::{CoreTransactionKind, CoreTransactionStatus};

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// The history list, deduplicated and ready to render.
    ///
    /// `unknown_label` is the platform's word for a wallet it cannot name —
    /// the one genuinely localizable string in the result, so it comes in
    /// rather than being invented here.
    pub async fn normalized_history(
        &self,
        unknown_label: String,
    ) -> Vec<crate::fetch::history::CoreNormalizedHistoryEntry> {
        let (records, wallets) = self.history_and_wallets().await;
        crate::fetch::history::normalize_history(crate::fetch::history::NormalizeHistoryRequest {
            wallets,
            transactions: records
                .iter()
                .map(|record| crate::fetch::history::HistoryTransaction {
                    id: record.payload.id.to_lowercase(),
                    wallet_id: record.payload.wallet_id.as_deref().map(str::to_lowercase),
                    kind: kind_string(record.payload.kind),
                    status: status_string(record.payload.status),
                    wallet_name: record.payload.wallet_name.clone(),
                    asset_name: record.payload.asset_name.clone(),
                    symbol: record.payload.symbol.clone(),
                    chain_name: record.payload.chain_name.clone(),
                    address: record.payload.address.clone(),
                    transaction_hash: record.payload.transaction_hash.clone(),
                    transaction_history_source: record.payload.transaction_history_source.clone(),
                    // The row's timestamp, which is Unix. The payload's is in
                    // Swift reference time, and reading the wrong one shifts
                    // every entry by thirty-one years.
                    created_at_unix: record.created_at,
                })
                .collect(),
            unknown_label,
        })
    }

    /// The earliest recorded activity per wallet, in unix seconds.
    pub async fn earliest_transaction_dates(
        &self,
    ) -> Vec<crate::store::WalletEarliestTransactionDate> {
        let (records, _) = self.history_and_wallets().await;
        crate::store::core_earliest_transaction_dates(
            records
                .iter()
                .map(|record| crate::store::TransactionEarliestInput {
                    wallet_id: record.payload.wallet_id.clone(),
                    created_at_unix: record.created_at,
                })
                .collect(),
        )
    }

    /// Transaction ids whose wallet is still active, for a caller pruning the
    /// ones whose wallet is gone.
    pub async fn active_wallet_transaction_ids(&self) -> Vec<String> {
        let (records, wallets) = self.history_and_wallets().await;
        crate::store::core_active_wallet_transaction_ids(
            records
                .iter()
                .map(|record| crate::store::TransactionActivityInput {
                    id: record.payload.id.clone(),
                    wallet_id: record.payload.wallet_id.clone(),
                    chain_name: record.payload.chain_name.clone(),
                })
                .collect(),
            wallets
                .iter()
                .map(|wallet| crate::store::WalletChainInput {
                    wallet_id: wallet.wallet_id.clone(),
                    selected_chain: wallet.selected_chain.clone(),
                })
                .collect(),
        )
    }
}

impl WalletService {
    /// The store's records and the wallets they belong to, read together.
    async fn history_and_wallets(
        &self,
    ) -> (
        Vec<crate::wallet_db::HistoryRecord>,
        Vec<crate::fetch::history::HistoryWallet>,
    ) {
        let wallets = self
            .wallet_state
            .read()
            .await
            .wallets
            .iter()
            .map(|wallet| crate::fetch::history::HistoryWallet {
                wallet_id: wallet.id.to_lowercase(),
                selected_chain: wallet.chain_name.clone(),
            })
            .collect();
        // The store knows where it is; an unopened one simply has no records.
        let records = self
            .fetch_all_history_records_typed()
            .await
            .unwrap_or_default();
        (records, wallets)
    }
}

/// The strings the normalizer keys on. They are the Swift raw values, which is
/// what the persisted records have always carried.
fn kind_string(kind: CoreTransactionKind) -> String {
    match kind {
        CoreTransactionKind::Send => "send",
        CoreTransactionKind::Receive => "receive",
    }
    .to_string()
}

fn status_string(status: Option<CoreTransactionStatus>) -> String {
    match status {
        Some(CoreTransactionStatus::Pending) | None => "pending",
        Some(CoreTransactionStatus::Confirmed) => "confirmed",
        Some(CoreTransactionStatus::Failed) => "failed",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::persistence_models::CorePersistedTransactionRecord;

    fn temp_db(label: &str) -> String {
        let path = std::env::temp_dir().join(format!(
            "history_derived_{label}_{}_{:?}.sqlite",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);
        path.to_string_lossy().into_owned()
    }

    fn record(id: &str, wallet: &str, chain: &str, created_at_swift: f64) -> CorePersistedTransactionRecord {
        let json = format!(
            r#"{{"id":"{id}","walletId":"{wallet}","kind":"receive","walletName":"W",
                 "assetName":"Bitcoin","symbol":"BTC","chainName":"{chain}","amount":0.5,
                 "address":"bc1qreceive","createdAt":{created_at_swift}}}"#
        );
        serde_json::from_str(&json).expect("a persisted record")
    }

    /// Writes land in the database the service was opened on.
    ///
    /// Twelve exported methods used to take a `db_path` the service already
    /// held. Nothing checked it against the binding, so a caller passing a
    /// different path wrote to a different file — and read back from the one
    /// core thought it was on, which reads as data silently disappearing.
    #[tokio::test]
    async fn records_land_in_the_database_the_service_was_opened_on() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        assert!(
            service.fetch_all_history_records_typed().await.is_err(),
            "an unopened store has no database to read"
        );

        let db = temp_db("bound");
        service.open_state(db.clone()).await.expect("open");
        service
            .upsert_history_records(vec![crate::wallet_db::history_record_from_payload(record(
                "B1B2C3D4-E5F6-7890-ABCD-EF1234567890",
                "w1",
                "Bitcoin",
                745_200_000.0,
            ))])
            .await
            .expect("upsert");

        // Read back through a second service opened on the same file: the
        // record is there, and it is there because the path came from the
        // binding rather than from the call.
        let reopened = WalletService::new_typed(Vec::new()).expect("service");
        reopened.open_state(db).await.expect("open");
        assert_eq!(
            reopened
                .fetch_all_history_records_typed()
                .await
                .expect("read")
                .len(),
            1
        );
    }

    /// The derived views read the store's timestamp, not the payload's.
    ///
    /// `HistoryRecord::created_at` is Unix; the payload's `created_at` is in
    /// Swift reference time. They differ by thirty-one years, and both are in
    /// scope at the point this code reads one — the wrong one dates every
    /// history entry to 1970 and orders the list by it.
    #[tokio::test]
    async fn derived_views_use_the_rows_unix_timestamp() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let db = temp_db("timestamps");
        service.open_state(db.clone()).await.expect("open");
        let payload = record("A1B2C3D4-E5F6-7890-ABCD-EF1234567890", "w1", "Bitcoin", 745_200_000.0);
        service
            .upsert_history_records(vec![crate::wallet_db::history_record_from_payload(payload)])
            .await
            .expect("upsert");

        let earliest = service.earliest_transaction_dates().await;
        assert_eq!(earliest.len(), 1);
        // 745200000 Swift-reference seconds is 2024-08-12 in Unix terms. Read
        // as Unix it would be 1993.
        assert!(
            earliest[0].earliest_created_at_unix > 1_700_000_000.0,
            "read the payload's Swift-reference timestamp as Unix: got {}",
            earliest[0].earliest_created_at_unix
        );
    }
}
