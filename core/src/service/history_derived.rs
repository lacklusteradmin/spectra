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

    /// The pending sends a caller may replace by resubmitting their nonce,
    /// newest first.
    ///
    /// Swift asked this of its own transaction projection, and asked it of the
    /// chain *named* "Ethereum" — so a pending Arbitrum or Base send, which
    /// replaces exactly the way a mainnet one does, offered neither speed-up
    /// nor cancel. The family is the registry's, and the rule is here.
    pub async fn replaceable_sends(&self) -> Vec<ReplaceableSend> {
        self.fetch_all_history_records_typed()
            .await
            .unwrap_or_default()
            .iter()
            .filter_map(|record| replaceable_send(&record.payload))
            .collect()
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

/// A pending send that can still be replaced on its chain.
///
/// `can_speed_up` is the part a front end must not decide for itself. A
/// replacement re-signs the *same* transfer at the same nonce, and only a
/// native transfer can be rebuilt from a stored record — a token transfer's
/// contract is not in it. Swift composed a native transfer of the token's
/// amount to the token's recipient instead, so speeding up a 100 USDC send
/// offered to send 100 ETH. Cancelling needs none of that: it is a zero-value
/// self-transfer at the same nonce, so it is offered for every row here.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ReplaceableSend {
    pub transaction_id: String,
    pub wallet_id: String,
    /// The catalog id of the chain the pending send is on — the chain the
    /// replacement must be signed for, not whichever one the composer shows.
    pub chain_id: String,
    pub chain_name: String,
    pub symbol: String,
    pub to_address: String,
    pub amount: f64,
    pub transaction_hash: String,
    /// The nonce as recorded. A replacement still reads the live one from the
    /// chain by hash; this is what a caller can say while that is in flight.
    pub recorded_nonce: Option<i64>,
    pub can_speed_up: bool,
}

/// The rule: an EVM chain, a send, still pending, with a hash to find its
/// nonce by, belonging to a wallet.
///
/// A stored send with no status at all is not pending — `status_to_raw` reads
/// that absence as confirmed — so it is not replaceable either.
fn replaceable_send(
    record: &crate::store::persistence_models::CorePersistedTransactionRecord,
) -> Option<ReplaceableSend> {
    if record.kind != CoreTransactionKind::Send
        || record.status != Some(CoreTransactionStatus::Pending)
    {
        return None;
    }
    let wallet_id = record
        .wallet_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())?;
    let transaction_hash = record
        .transaction_hash
        .as_deref()
        .map(str::trim)
        .filter(|hash| !hash.is_empty())?;
    let chain = crate::registry::Chain::from_display_name(&record.chain_name)?;
    if !chain.is_evm() {
        return None;
    }
    Some(ReplaceableSend {
        transaction_id: record.id.clone(),
        wallet_id: wallet_id.to_owned(),
        chain_id: chain.str_id().to_owned(),
        chain_name: chain.chain_display_name().to_owned(),
        symbol: record.symbol.clone(),
        to_address: record.address.clone(),
        amount: record.amount,
        transaction_hash: transaction_hash.to_owned(),
        recorded_nonce: record.ethereum_nonce,
        can_speed_up: record.symbol == chain.coin_symbol(),
    })
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

pub(crate) fn status_string(status: Option<CoreTransactionStatus>) -> String {
    match status {
        Some(CoreTransactionStatus::Pending) | None => "pending",
        Some(CoreTransactionStatus::Confirmed) => "confirmed",
        Some(CoreTransactionStatus::Failed) => "failed",
    }
    .to_string()
}

/// The inverse of `status_string`, for reading a decision back.
pub(crate) fn parse_status(raw: &str) -> Option<CoreTransactionStatus> {
    match raw {
        "pending" => Some(CoreTransactionStatus::Pending),
        "confirmed" => Some(CoreTransactionStatus::Confirmed),
        "failed" => Some(CoreTransactionStatus::Failed),
        _ => None,
    }
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

    fn record(
        id: &str,
        wallet: &str,
        chain: &str,
        created_at_swift: f64,
    ) -> CorePersistedTransactionRecord {
        let json = format!(
            r#"{{"id":"{id}","walletId":"{wallet}","kind":"receive","walletName":"W",
                 "assetName":"Bitcoin","symbol":"BTC","chainName":"{chain}","amount":0.5,
                 "address":"bc1qreceive","createdAt":{created_at_swift}}}"#
        );
        serde_json::from_str(&json).expect("a persisted record")
    }

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
        let payload = record(
            "A1B2C3D4-E5F6-7890-ABCD-EF1234567890",
            "w1",
            "Bitcoin",
            745_200_000.0,
        );
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

#[cfg(test)]
mod replaceable_tests {
    use super::*;
    use crate::service::types::TransactionCommand;
    use crate::store::persistence_models::CorePersistedTransactionRecord;
    use serde_json::json;
    use std::sync::Arc;

    /// Built from the stored JSON shape so a test says only what it is about.
    /// `overrides` are merged over the base object.
    fn record(
        id: &str,
        chain: &str,
        symbol: &str,
        overrides: serde_json::Value,
    ) -> CorePersistedTransactionRecord {
        let mut value = json!({
            "id": id,
            "walletId": "wallet-1",
            "kind": "send",
            "status": "pending",
            "walletName": "Main",
            "assetName": chain,
            "symbol": symbol,
            "chainName": chain,
            "amount": 1.5,
            "address": "0x1111111111111111111111111111111111111111",
            "transactionHash": "0xabc",
            "createdAt": 745_200_000.0,
        });
        for (key, patch) in overrides.as_object().expect("overrides object") {
            match patch {
                serde_json::Value::Null => {
                    value.as_object_mut().unwrap().remove(key);
                }
                _ => {
                    value[key] = patch.clone();
                }
            }
        }
        serde_json::from_value(value).expect("stored transaction shape")
    }

    /// The chain named "Ethereum" was the whole rule on the Swift side.
    #[test]
    fn every_evm_chain_offers_replacement_and_nothing_else_does() {
        let ethereum =
            replaceable_send(&record("a", "Ethereum", "ETH", json!({}))).expect("ethereum");
        assert_eq!(ethereum.chain_id, "ethereum");
        assert!(ethereum.can_speed_up);

        let arbitrum =
            replaceable_send(&record("b", "Arbitrum", "ETH", json!({"ethereumNonce": 7})))
                .expect("arbitrum");
        assert_eq!(arbitrum.chain_id, "arbitrum");
        assert_eq!(arbitrum.recorded_nonce, Some(7));
        assert!(arbitrum.can_speed_up);

        for chain in ["Bitcoin", "Solana", "Dogecoin", "Monero"] {
            assert!(
                replaceable_send(&record("c", chain, "BTC", json!({}))).is_none(),
                "{chain}"
            );
        }
        assert!(replaceable_send(&record("d", "Not A Chain", "ETH", json!({}))).is_none());
    }

    /// A token transfer cannot be rebuilt from the record, so it may be
    /// cancelled but not sped up. `ARB` is Arbitrum's own ticker while its gas
    /// is `ETH`, which is exactly the pair that has to come apart here.
    #[test]
    fn only_a_native_transfer_can_be_sped_up() {
        let token =
            replaceable_send(&record("a", "Arbitrum", "ARB", json!({}))).expect("token send");
        assert!(!token.can_speed_up);
        assert_eq!(token.symbol, "ARB");
        assert!(
            !replaceable_send(&record("b", "Ethereum", "USDC", json!({})))
                .expect("erc20")
                .can_speed_up
        );
    }

    #[test]
    fn a_row_without_a_pending_send_to_replace_is_not_offered() {
        for overrides in [
            json!({"kind": "receive"}),
            json!({"status": "confirmed"}),
            json!({"status": "failed"}),
            // No status at all reads as confirmed for a send.
            json!({"status": null}),
            json!({"transactionHash": null}),
            json!({"transactionHash": "  "}),
            json!({"walletId": null}),
            json!({"walletId": " "}),
        ] {
            assert!(
                replaceable_send(&record("a", "Ethereum", "ETH", overrides.clone())).is_none(),
                "{overrides}"
            );
        }
    }

    fn database() -> String {
        std::env::temp_dir()
            .join(format!(
                "spectra-replaceable-{}.sqlite",
                crate::store::new_event_id()
            ))
            .to_string_lossy()
            .into_owned()
    }

    /// The list is the store's, newest first, and survives reopening it.
    #[tokio::test]
    async fn the_store_answers_and_a_reopened_service_answers_the_same() {
        let service = Arc::new(WalletService::new_typed(vec![]).unwrap());
        let db = database();
        service.open_state(db.clone()).await.unwrap();
        service
            .apply_transaction_command(TransactionCommand::Upsert {
                records: vec![
                    record(
                        "11111111-1111-1111-1111-111111111111",
                        "Base",
                        "ETH",
                        json!({"createdAt": 1.0}),
                    ),
                    record(
                        "22222222-2222-2222-2222-222222222222",
                        "Optimism",
                        "ETH",
                        json!({"createdAt": 2.0}),
                    ),
                    record(
                        "33333333-3333-3333-3333-333333333333",
                        "Bitcoin",
                        "BTC",
                        json!({"createdAt": 3.0}),
                    ),
                    record(
                        "44444444-4444-4444-4444-444444444444",
                        "Ethereum",
                        "ETH",
                        json!({"status": "confirmed"}),
                    ),
                ],
            })
            .await
            .unwrap();

        let expected = vec!["optimism".to_string(), "base".to_string()];
        let chains: Vec<String> = service
            .replaceable_sends()
            .await
            .into_iter()
            .map(|send| send.chain_id)
            .collect();
        assert_eq!(chains, expected);

        let reopened = Arc::new(WalletService::new_typed(vec![]).unwrap());
        reopened.open_state(db).await.unwrap();
        assert_eq!(
            reopened.replaceable_sends().await,
            service.replaceable_sends().await
        );
    }

    /// An unopened store has nothing to replace rather than failing.
    #[tokio::test]
    async fn an_unopened_store_is_empty() {
        let service = WalletService::new_typed(vec![]).unwrap();
        assert!(service.replaceable_sends().await.is_empty());
    }
}
