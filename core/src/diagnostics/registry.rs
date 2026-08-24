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
    ($field:ident, $ty:ident, $all:ident, $record:ident) => {
        /// The whole map for a chain. Internal: the exporter builds a document
        /// from it, and nothing outside the crate has a use for every row.
        pub fn $all(chain_name: String) -> HashMap<String, $ty> {
            registry()
                .lock()
                .unwrap()
                .$field
                .get(&chain_name)
                .cloned()
                .unwrap_or_default()
        }

        /// Record one wallet's result.
        ///
        /// Internal now: `diagnostics_record` is the one entry point, and the
        /// shape it dispatches on is the entry's own variant.
        pub fn $record(chain_name: String, wallet_id: String, entry: $ty) {
            registry()
                .lock()
                .unwrap()
                .$field
                .entry(chain_name)
                .or_default()
                .insert(wallet_id, entry);
        }
    };
}

chain_keyed_registry!(
    utxo,
    BitcoinHistoryDiagnostics,
    diagnostics_all_utxo,
    diagnostics_record_utxo
);
chain_keyed_registry!(
    evm,
    EthereumTokenTransferHistoryDiagnostics,
    diagnostics_all_evm,
    diagnostics_record_evm
);
chain_keyed_registry!(
    simple,
    SimpleHistoryDiagnostics,
    diagnostics_all_simple,
    diagnostics_record_simple
);

/// A history-diagnostics row, in whichever shape its chain reports.
///
/// The five variants are the five record shapes, which genuinely differ —
/// `DiagnosticsShape` on the registry is the same distinction. What did not
/// need to differ is the *call*: there were five exported writers, one per
/// shape, and every Swift call site picked between them by knowing which shape
/// its chain used. The entry carries its own shape now.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum HistoryDiagnosticsEntry {
    Utxo { entry: BitcoinHistoryDiagnostics },
    Evm { entry: EthereumTokenTransferHistoryDiagnostics },
    Simple { entry: SimpleHistoryDiagnostics },
    Tron { entry: TronHistoryDiagnostics },
    Solana { entry: SolanaHistoryDiagnostics },
}

/// Record one wallet's history-diagnostics row for a chain.
#[uniffi::export]
pub fn diagnostics_record(chain_name: String, wallet_id: String, entry: HistoryDiagnosticsEntry) {
    match entry {
        HistoryDiagnosticsEntry::Utxo { entry } => {
            diagnostics_record_utxo(chain_name, wallet_id, entry)
        }
        HistoryDiagnosticsEntry::Evm { entry } => {
            diagnostics_record_evm(chain_name, wallet_id, entry)
        }
        HistoryDiagnosticsEntry::Simple { entry } => {
            diagnostics_record_simple(chain_name, wallet_id, entry)
        }
        // Tron and Solana have one chain each, so their maps are keyed by
        // wallet alone and the chain name has nowhere to go.
        HistoryDiagnosticsEntry::Tron { entry } => diagnostics_record_tron(wallet_id, entry),
        HistoryDiagnosticsEntry::Solana { entry } => diagnostics_record_solana(wallet_id, entry),
    }
}

/// One chain each, so a chain argument could only ever hold one value.
pub fn diagnostics_all_tron() -> HashMap<String, TronHistoryDiagnostics> {
    registry().lock().unwrap().tron.clone()
}

pub fn diagnostics_record_tron(wallet_id: String, entry: TronHistoryDiagnostics) {
    registry().lock().unwrap().tron.insert(wallet_id, entry);
}

pub fn diagnostics_all_solana() -> HashMap<String, SolanaHistoryDiagnostics> {
    registry().lock().unwrap().solana.clone()
}

pub fn diagnostics_record_solana(wallet_id: String, entry: SolanaHistoryDiagnostics) {
    registry().lock().unwrap().solana.insert(wallet_id, entry);
}

/// What a chain's diagnostics screen shows about its history run: how many
/// wallets reported, and which source each used.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DiagnosticsRunSummary {
    pub wallet_count: u32,
    /// One entry per wallet, in no particular order — the caller groups them.
    pub sources: Vec<String>,
}

#[uniffi::export]
pub fn diagnostics_run_summary(chain_name: String) -> DiagnosticsRunSummary {
    use crate::registry::{Chain, DiagnosticsShape};
    let reg = registry().lock().unwrap();
    let sources: Vec<String> = match Chain::from_display_name(&chain_name).map(Chain::diagnostics_shape) {
        Some(DiagnosticsShape::Utxo) => reg
            .utxo
            .get(&chain_name)
            .map(|m| m.values().map(|d| d.source_used.clone()).collect())
            .unwrap_or_default(),
        Some(DiagnosticsShape::Evm) => reg
            .evm
            .get(&chain_name)
            .map(|m| m.values().map(|d| d.source_used.clone()).collect())
            .unwrap_or_default(),
        Some(DiagnosticsShape::Tron) => reg.tron.values().map(|d| d.source_used.clone()).collect(),
        Some(DiagnosticsShape::Solana) => reg.solana.values().map(|d| d.source_used.clone()).collect(),
        Some(DiagnosticsShape::Simple) => reg
            .simple
            .get(&chain_name)
            .map(|m| m.values().map(|d| d.source_used.clone()).collect())
            .unwrap_or_default(),
        None => Vec::new(),
    };
    DiagnosticsRunSummary {
        wallet_count: sources.len() as u32,
        sources,
    }
}

/// Drop every diagnostics row a wallet left behind, on every chain.
#[uniffi::export]
pub fn diagnostics_forget_wallet(wallet_id: String) {
    let mut reg = registry().lock().unwrap();
    for by_chain in reg.utxo.values_mut() {
        by_chain.remove(&wallet_id);
    }
    for by_chain in reg.evm.values_mut() {
        by_chain.remove(&wallet_id);
    }
    for by_chain in reg.simple.values_mut() {
        by_chain.remove(&wallet_id);
    }
    reg.tron.remove(&wallet_id);
    reg.solana.remove(&wallet_id);
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

    /// Recording is per wallet, and one wallet's row does not disturb another's.
    ///
    /// The pair this replaces was read-everything / write-everything, so this
    /// test asserted that "replace is a replace, not a merge" — the property
    /// that made two concurrent writers lose one of the two rows.
    #[test]
    fn recording_one_wallet_leaves_the_others_alone() {
        let _g = test_lock();
        diagnostics_clear_all();
        assert!(diagnostics_all_utxo("Bitcoin".into()).is_empty());

        diagnostics_record_utxo("Bitcoin".into(), "w1".into(), sample_bitcoin("w1"));
        diagnostics_record_utxo("Bitcoin".into(), "w2".into(), sample_bitcoin("w2"));
        assert_eq!(diagnostics_all_utxo("Bitcoin".into()).len(), 2);

        diagnostics_record_utxo("Bitcoin".into(), "w3".into(), sample_bitcoin("w3"));
        let stored = diagnostics_all_utxo("Bitcoin".into());
        assert_eq!(stored.len(), 3, "recording a third wallet dropped the first two");
        assert!(stored.contains_key("w1") && stored.contains_key("w3"));

        // And a wallet that goes away takes its rows with it, on every chain.
        diagnostics_forget_wallet("w1".into());
        let stored = diagnostics_all_utxo("Bitcoin".into());
        assert_eq!(stored.len(), 2);
        assert!(!stored.contains_key("w1"));

        diagnostics_clear_all();
        assert!(diagnostics_all_utxo("Bitcoin".into()).is_empty());
    }

    /// The screen's two numbers come from core, whatever the record shape.
    #[test]
    fn the_run_summary_counts_wallets_and_lists_their_sources() {
        let _g = test_lock();
        diagnostics_clear_all();
        assert_eq!(diagnostics_run_summary("Bitcoin".into()).wallet_count, 0);

        diagnostics_record_utxo("Bitcoin".into(), "w1".into(), sample_bitcoin("w1"));
        diagnostics_record_utxo("Bitcoin".into(), "w2".into(), sample_bitcoin("w2"));
        let summary = diagnostics_run_summary("Bitcoin".into());
        assert_eq!(summary.wallet_count, 2);
        assert_eq!(summary.sources.len(), 2);

        // A chain of another shape reads its own registry, not this one.
        assert_eq!(diagnostics_run_summary("XRP Ledger".into()).wallet_count, 0);
        // And a name no chain has is empty rather than a panic.
        assert_eq!(diagnostics_run_summary("Nope".into()).wallet_count, 0);
        diagnostics_clear_all();
    }

    /// Chains sharing a record shape must not share a bucket. The old macro got
    /// this from having a separate field per chain; keyed storage has to be
    /// asked.
    #[test]
    fn chains_of_the_same_shape_keep_separate_buckets() {
        let _g = test_lock();
        diagnostics_clear_all();
        diagnostics_record_utxo("Bitcoin".into(), "w".into(), sample_bitcoin("w"));

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
