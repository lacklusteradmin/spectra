// Rust-owned canonical storage for AppState's three core `@Published` state
// collections: `wallets`, `transactions`, and `addressBook`.
//
// Previously these were plain Swift arrays on `AppState`. Swift now keeps
// `@Published` mirrors *only* for SwiftUI observation; the canonical data lives
// here, behind a `Mutex<Vec<_>>` per collection with UniFFI get/replace/upsert/
// remove/clear. Every Swift mutation helper (`setWallets`, `appendWallet`,
// `removeWallet(id:)`, etc. — see `AppState.swift`) funnels through these
// functions, then updates the mirror. Direct assignments to `self.wallets` /
// `self.transactions` / `self.addressBook` are forbidden outside those helpers.
//
// Transactions and address-book entries are stored as their
// `CorePersisted*` UniFFI variants (the same shape used by the Swift
// persistence layer), since Swift's native `TransactionRecord` /
// `AddressBookEntry` structs carry UUID+Date and aren't themselves UniFFI
// records. The Swift helpers convert via the existing `persistedSnapshot` /
// `init(snapshot:)` bridges.

use std::sync::{Mutex, OnceLock};

use crate::app_state::events::{publish, AppStateEvent};
use crate::persistence::models::{
    CorePersistedAddressBookEntry, CorePersistedTransactionRecord,
};
use crate::wallet_domain::CoreImportedWallet;

fn publish_wallets(g: &StoreRegistry) {
    publish(AppStateEvent::WalletsChanged(g.wallets.clone()));
}
fn publish_transactions(g: &StoreRegistry) {
    publish(AppStateEvent::TransactionsChanged(g.transactions.clone()));
}
fn publish_address_book(g: &StoreRegistry) {
    publish(AppStateEvent::AddressBookChanged(g.address_book.clone()));
}

#[derive(Default)]
struct StoreRegistry {
    wallets: Vec<CoreImportedWallet>,
    transactions: Vec<CorePersistedTransactionRecord>,
    address_book: Vec<CorePersistedAddressBookEntry>,
}

fn registry() -> &'static Mutex<StoreRegistry> {
    static REG: OnceLock<Mutex<StoreRegistry>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(StoreRegistry::default()))
}

// ── Wallets ─────────────────────────────────────────────────────────────
#[uniffi::export]
pub fn store_wallets_get_all() -> Vec<CoreImportedWallet> {
    registry().lock().unwrap().wallets.clone()
}

#[uniffi::export]
pub fn store_wallets_replace_all(wallets: Vec<CoreImportedWallet>) {
    let mut g = registry().lock().unwrap();
    g.wallets = wallets;
    publish_wallets(&g);
}

#[uniffi::export]
pub fn store_wallets_append(wallet: CoreImportedWallet) {
    let mut g = registry().lock().unwrap();
    g.wallets.push(wallet);
    publish_wallets(&g);
}

#[uniffi::export]
pub fn store_wallets_append_many(wallets: Vec<CoreImportedWallet>) {
    let mut g = registry().lock().unwrap();
    g.wallets.extend(wallets);
    publish_wallets(&g);
}

/// Insert-or-replace by `id`. Preserves position on update; appends on insert.
#[uniffi::export]
pub fn store_wallets_upsert(wallet: CoreImportedWallet) {
    let mut guard = registry().lock().unwrap();
    if let Some(idx) = guard.wallets.iter().position(|w| w.id == wallet.id) {
        guard.wallets[idx] = wallet;
    } else {
        guard.wallets.push(wallet);
    }
    publish_wallets(&guard);
}

#[uniffi::export]
pub fn store_wallets_remove(id: String) {
    let mut g = registry().lock().unwrap();
    g.wallets.retain(|w| w.id != id);
    publish_wallets(&g);
}

#[uniffi::export]
pub fn store_wallets_clear() {
    let mut g = registry().lock().unwrap();
    g.wallets.clear();
    publish_wallets(&g);
}

// ── Transactions ────────────────────────────────────────────────────────
#[uniffi::export]
pub fn store_transactions_get_all() -> Vec<CorePersistedTransactionRecord> {
    registry().lock().unwrap().transactions.clone()
}

#[uniffi::export]
pub fn store_transactions_replace_all(transactions: Vec<CorePersistedTransactionRecord>) {
    let mut g = registry().lock().unwrap();
    g.transactions = transactions;
    publish_transactions(&g);
}

#[uniffi::export]
pub fn store_transactions_prepend(transaction: CorePersistedTransactionRecord) {
    let mut g = registry().lock().unwrap();
    g.transactions.insert(0, transaction);
    publish_transactions(&g);
}

#[uniffi::export]
pub fn store_transactions_remove_for_wallet(wallet_id: String) {
    let mut g = registry().lock().unwrap();
    g.transactions
        .retain(|t| t.wallet_id.as_deref() != Some(wallet_id.as_str()));
    publish_transactions(&g);
}

#[uniffi::export]
pub fn store_transactions_clear() {
    let mut g = registry().lock().unwrap();
    g.transactions.clear();
    publish_transactions(&g);
}

// ── Address book ───────────────────────────────────────────────────────
#[uniffi::export]
pub fn store_address_book_get_all() -> Vec<CorePersistedAddressBookEntry> {
    registry().lock().unwrap().address_book.clone()
}

#[uniffi::export]
pub fn store_address_book_replace_all(entries: Vec<CorePersistedAddressBookEntry>) {
    let mut g = registry().lock().unwrap();
    g.address_book = entries;
    publish_address_book(&g);
}

#[uniffi::export]
pub fn store_address_book_prepend(entry: CorePersistedAddressBookEntry) {
    let mut g = registry().lock().unwrap();
    g.address_book.insert(0, entry);
    publish_address_book(&g);
}

#[uniffi::export]
pub fn store_address_book_remove(id: String) {
    let mut g = registry().lock().unwrap();
    g.address_book.retain(|e| e.id != id);
    publish_address_book(&g);
}

#[uniffi::export]
pub fn store_address_book_clear() {
    let mut g = registry().lock().unwrap();
    g.address_book.clear();
    publish_address_book(&g);
}

#[uniffi::export]
pub fn store_clear_all() {
    let mut g = registry().lock().unwrap();
    g.wallets.clear();
    g.transactions.clear();
    g.address_book.clear();
    publish_wallets(&g);
    publish_transactions(&g);
    publish_address_book(&g);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet_domain::{CoreTransactionKind, CoreTransactionStatus};

    fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        static L: OnceLock<Mutex<()>> = OnceLock::new();
        L.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn sample_wallet(id: &str) -> CoreImportedWallet {
        let mut w = CoreImportedWallet::default();
        w.id = id.to_string();
        w.name = format!("wallet-{id}");
        w
    }

    fn sample_tx(id: &str, wallet_id: Option<&str>) -> CorePersistedTransactionRecord {
        CorePersistedTransactionRecord {
            id: id.to_string(),
            wallet_id: wallet_id.map(|s| s.to_string()),
            kind: CoreTransactionKind::Send,
            status: Some(CoreTransactionStatus::Confirmed),
            wallet_name: "W".into(),
            asset_name: "Bitcoin".into(),
            symbol: "BTC".into(),
            chain_name: "Bitcoin".into(),
            amount: 1.0,
            address: "addr".into(),
            transaction_hash: None,
            ethereum_nonce: None,
            receipt_block_number: None,
            receipt_gas_used: None,
            receipt_effective_gas_price_gwei: None,
            receipt_network_fee_eth: None,
            fee_priority_raw: None,
            fee_rate_description: None,
            confirmation_count: None,
            dogecoin_confirmed_network_fee_doge: None,
            dogecoin_confirmations: None,
            dogecoin_fee_priority_raw: None,
            dogecoin_estimated_fee_rate_doge_per_kb: None,
            used_change_output: None,
            dogecoin_used_change_output: None,
            source_derivation_path: None,
            change_derivation_path: None,
            source_address: None,
            change_address: None,
            dogecoin_raw_transaction_hex: None,
            signed_transaction_payload: None,
            signed_transaction_payload_format: None,
            failure_reason: None,
            transaction_history_source: None,
            created_at: 0.0,
        }
    }

    fn sample_entry(id: &str) -> CorePersistedAddressBookEntry {
        CorePersistedAddressBookEntry {
            id: id.to_string(),
            name: format!("entry-{id}"),
            chain_name: "Bitcoin".into(),
            address: "addr".into(),
            note: "".into(),
        }
    }

    #[test]
    fn wallets_roundtrip() {
        let _g = test_lock();
        store_clear_all();
        assert!(store_wallets_get_all().is_empty());

        store_wallets_replace_all(vec![sample_wallet("a"), sample_wallet("b")]);
        assert_eq!(store_wallets_get_all().len(), 2);

        store_wallets_append(sample_wallet("c"));
        assert_eq!(store_wallets_get_all().len(), 3);

        // Upsert existing by id replaces in place.
        let mut updated = sample_wallet("b");
        updated.name = "renamed".into();
        store_wallets_upsert(updated);
        let all = store_wallets_get_all();
        assert_eq!(all.len(), 3);
        assert_eq!(all.iter().find(|w| w.id == "b").unwrap().name, "renamed");

        // Upsert new id appends.
        store_wallets_upsert(sample_wallet("d"));
        assert_eq!(store_wallets_get_all().len(), 4);

        store_wallets_remove("b".into());
        assert!(store_wallets_get_all().iter().all(|w| w.id != "b"));

        store_wallets_append_many(vec![sample_wallet("e"), sample_wallet("f")]);
        assert_eq!(store_wallets_get_all().len(), 5);

        store_wallets_clear();
        assert!(store_wallets_get_all().is_empty());
    }

    #[test]
    fn transactions_roundtrip() {
        let _g = test_lock();
        store_clear_all();
        store_transactions_replace_all(vec![sample_tx("t1", Some("w1"))]);
        assert_eq!(store_transactions_get_all().len(), 1);

        store_transactions_prepend(sample_tx("t2", Some("w2")));
        assert_eq!(store_transactions_get_all()[0].id, "t2");

        store_transactions_prepend(sample_tx("t3", Some("w1")));
        store_transactions_remove_for_wallet("w1".into());
        let remaining = store_transactions_get_all();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "t2");

        store_transactions_clear();
        assert!(store_transactions_get_all().is_empty());
    }

    #[test]
    fn address_book_roundtrip() {
        let _g = test_lock();
        store_clear_all();
        store_address_book_replace_all(vec![sample_entry("1"), sample_entry("2")]);
        assert_eq!(store_address_book_get_all().len(), 2);

        store_address_book_prepend(sample_entry("3"));
        assert_eq!(store_address_book_get_all()[0].id, "3");

        store_address_book_remove("2".into());
        assert_eq!(store_address_book_get_all().len(), 2);
        assert!(store_address_book_get_all().iter().all(|e| e.id != "2"));

        store_address_book_clear();
        assert!(store_address_book_get_all().is_empty());
    }

    #[test]
    fn clear_all_resets_every_collection() {
        let _g = test_lock();
        store_wallets_append(sample_wallet("x"));
        store_transactions_prepend(sample_tx("x", None));
        store_address_book_prepend(sample_entry("x"));
        store_clear_all();
        assert!(store_wallets_get_all().is_empty());
        assert!(store_transactions_get_all().is_empty());
        assert!(store_address_book_get_all().is_empty());
    }
}
