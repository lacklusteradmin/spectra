// Per-wallet diagnostics registry, keyed by wallet id.
//
// Exposes get/set/remove/list/clear via UniFFI, one trio per chain.
//
// The in-memory shape is one typed HashMap per chain, guarded by a single
// Mutex — simple, and the dict-sized data is trivial so contention is
// irrelevant.

use std::collections::HashMap;
use std::sync::Mutex;

use super::types::*;

/// Per-wallet history diagnostics, keyed by chain then wallet.
///
/// One registry per record shape. This was a macro stamping out five exports
/// per chain over one hash map — 120 FFI functions, 72 of which had no caller
/// on either side.
#[derive(Default)]
struct DiagnosticsRegistry {
    utxo: HashMap<String, HashMap<String, BitcoinHistoryDiagnostics>>,
    evm: HashMap<String, HashMap<String, EthereumTokenTransferHistoryDiagnostics>>,
    simple: HashMap<String, HashMap<String, SimpleHistoryDiagnostics>>,
    tron: HashMap<String, TronHistoryDiagnostics>,
    solana: HashMap<String, SolanaHistoryDiagnostics>,
}

impl DiagnosticsRegistry {
    fn clear(&mut self) {
        self.utxo.clear();
        self.evm.clear();
        self.simple.clear();
        self.tron.clear();
        self.solana.clear();
    }
}

fn registry() -> &'static Mutex<DiagnosticsRegistry> {
    use std::sync::OnceLock;
    static REG: OnceLock<Mutex<DiagnosticsRegistry>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(DiagnosticsRegistry::default()))
}

macro_rules! chain_keyed_registry {
    ($field:ident, $ty:ident, $all:ident, $replace:ident) => {
        #[uniffi::export]
        pub fn $all(chain_name: String) -> HashMap<String, $ty> {
            registry()
                .lock()
                .unwrap()
                .$field
                .get(&chain_name)
                .cloned()
                .unwrap_or_default()
        }

        #[uniffi::export]
        pub fn $replace(chain_name: String, entries: HashMap<String, $ty>) {
            registry()
                .lock()
                .unwrap()
                .$field
                .insert(chain_name, entries);
        }
    };
}

chain_keyed_registry!(
    utxo,
    BitcoinHistoryDiagnostics,
    diagnostics_all_utxo,
    diagnostics_replace_utxo
);
chain_keyed_registry!(
    evm,
    EthereumTokenTransferHistoryDiagnostics,
    diagnostics_all_evm,
    diagnostics_replace_evm
);
chain_keyed_registry!(
    simple,
    SimpleHistoryDiagnostics,
    diagnostics_all_simple,
    diagnostics_replace_simple
);

/// One chain each, so a chain argument could only ever hold one value.
#[uniffi::export]
pub fn diagnostics_all_tron() -> HashMap<String, TronHistoryDiagnostics> {
    registry().lock().unwrap().tron.clone()
}

#[uniffi::export]
pub fn diagnostics_replace_tron(entries: HashMap<String, TronHistoryDiagnostics>) {
    registry().lock().unwrap().tron = entries;
}

#[uniffi::export]
pub fn diagnostics_all_solana() -> HashMap<String, SolanaHistoryDiagnostics> {
    registry().lock().unwrap().solana.clone()
}

#[uniffi::export]
pub fn diagnostics_replace_solana(entries: HashMap<String, SolanaHistoryDiagnostics>) {
    registry().lock().unwrap().solana = entries;
}

#[uniffi::export]
pub fn diagnostics_clear_all() {
    registry().lock().unwrap().clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    // Registry is a shared global; serialize tests to avoid cross-test races.
    fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::{Mutex, OnceLock};
        static L: OnceLock<Mutex<()>> = OnceLock::new();
        L.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn sample_bitcoin(id: &str) -> BitcoinHistoryDiagnostics {
        BitcoinHistoryDiagnostics {
            wallet_id: id.to_string(),
            identifier: "addr".into(),
            source_used: "rust".into(),
            transaction_count: 1,
            next_cursor: None,
            error: None,
        }
    }

    #[test]
    fn replace_and_read_back_by_chain() {
        let _g = test_lock();
        diagnostics_clear_all();
        assert!(diagnostics_all_utxo("Bitcoin".into()).is_empty());

        let mut entries = HashMap::new();
        entries.insert("w1".to_string(), sample_bitcoin("w1"));
        entries.insert("w2".to_string(), sample_bitcoin("w2"));
        diagnostics_replace_utxo("Bitcoin".into(), entries);
        assert_eq!(diagnostics_all_utxo("Bitcoin".into()).len(), 2);

        // Replace is a replace, not a merge.
        let mut only_w3 = HashMap::new();
        only_w3.insert("w3".to_string(), sample_bitcoin("w3"));
        diagnostics_replace_utxo("Bitcoin".into(), only_w3);
        let stored = diagnostics_all_utxo("Bitcoin".into());
        assert_eq!(stored.len(), 1);
        assert!(stored.contains_key("w3"));

        diagnostics_clear_all();
        assert!(diagnostics_all_utxo("Bitcoin".into()).is_empty());
    }

    /// Chains sharing a record shape must not share a bucket. The old macro got
    /// this from having a separate field per chain; keyed storage has to be
    /// asked.
    #[test]
    fn chains_of_the_same_shape_keep_separate_buckets() {
        let _g = test_lock();
        diagnostics_clear_all();
        let mut entries = HashMap::new();
        entries.insert("w".to_string(), sample_bitcoin("w"));
        diagnostics_replace_utxo("Bitcoin".into(), entries);

        assert_eq!(diagnostics_all_utxo("Bitcoin".into()).len(), 1);
        assert!(diagnostics_all_utxo("Litecoin".into()).is_empty());
        assert!(diagnostics_all_utxo("Bitcoin Cash".into()).is_empty());
        // A different shape entirely is untouched.
        assert!(diagnostics_all_simple("XRP Ledger".into()).is_empty());
    }

    #[test]
    fn an_unknown_chain_reads_empty_rather_than_trapping() {
        let _g = test_lock();
        diagnostics_clear_all();
        assert!(diagnostics_all_utxo("Nope".into()).is_empty());
        assert!(diagnostics_all_simple("Nope".into()).is_empty());
        assert!(diagnostics_all_evm("Nope".into()).is_empty());
    }
}
