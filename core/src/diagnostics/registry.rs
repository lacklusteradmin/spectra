// Per-wallet diagnostics registry, keyed by wallet id.
//
// One HashMap keyed by chain then wallet, guarded by a single Mutex — the
// dict-sized data is trivial, so contention is irrelevant.

use std::collections::HashMap;
use std::sync::Mutex;

use super::types::*;

/// Per-wallet history diagnostics, keyed by chain then wallet.
///
/// One map. There were five, one per record shape, plus a `DiagnosticsShape`
/// enum to pick between them and five exported writers for callers to pick
/// between by hand.
#[derive(Default)]
struct DiagnosticsRegistry {
    history: HashMap<String, HashMap<String, HistoryDiagnostics>>,
}

fn registry() -> &'static Mutex<DiagnosticsRegistry> {
    use std::sync::OnceLock;
    static REG: OnceLock<Mutex<DiagnosticsRegistry>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(DiagnosticsRegistry::default()))
}

/// Every row recorded for a chain, keyed by wallet. Internal: the exporter
/// builds a document from it, and nothing outside the crate wants every row.
pub fn diagnostics_all(chain_name: String) -> HashMap<String, HistoryDiagnostics> {
    registry()
        .lock()
        .unwrap()
        .history
        .get(&chain_name)
        .cloned()
        .unwrap_or_default()
}

/// Record one wallet's history-diagnostics row for a chain.
#[uniffi::export]
pub fn diagnostics_record(chain_name: String, entry: HistoryDiagnostics) {
    registry()
        .lock()
        .unwrap()
        .history
        .entry(chain_name)
        .or_default()
        .insert(entry.wallet_id.clone(), entry);
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
    let sources: Vec<String> = registry()
        .lock()
        .unwrap()
        .history
        .get(&chain_name)
        .map(|rows| rows.values().map(|d| d.source_used.clone()).collect())
        .unwrap_or_default();
    DiagnosticsRunSummary {
        wallet_count: sources.len() as u32,
        sources,
    }
}

/// Drop every diagnostics row a wallet left behind, on every chain.
#[uniffi::export]
pub fn diagnostics_forget_wallet(wallet_id: String) {
    let mut reg = registry().lock().unwrap();
    for by_chain in reg.history.values_mut() {
        by_chain.remove(&wallet_id);
    }
}

#[uniffi::export]
pub fn diagnostics_clear_all() {
    registry().lock().unwrap().history.clear();
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

    fn sample(id: &str) -> HistoryDiagnostics {
        HistoryDiagnostics {
            wallet_id: id.to_string(),
            identifier: "addr".into(),
            source_used: "rust".into(),
            transaction_count: 1,
            scanned_count: None,
            next_cursor: None,
            error: None,
            per_source: Vec::new(),
        }
    }

    /// Recording is per wallet, and one wallet's row does not disturb another's.
    #[test]
    fn recording_one_wallet_leaves_the_others_alone() {
        let _g = test_lock();
        diagnostics_clear_all();
        assert!(diagnostics_all("Bitcoin".into()).is_empty());

        diagnostics_record("Bitcoin".into(), sample("w1"));
        diagnostics_record("Bitcoin".into(), sample("w2"));
        assert_eq!(diagnostics_all("Bitcoin".into()).len(), 2);

        diagnostics_record("Bitcoin".into(), sample("w3"));
        let stored = diagnostics_all("Bitcoin".into());
        assert_eq!(
            stored.len(),
            3,
            "recording a third wallet dropped the first two"
        );
        assert!(stored.contains_key("w1") && stored.contains_key("w3"));

        // And a wallet that goes away takes its rows with it, on every chain.
        diagnostics_forget_wallet("w1".into());
        let stored = diagnostics_all("Bitcoin".into());
        assert_eq!(stored.len(), 2);
        assert!(!stored.contains_key("w1"));

        diagnostics_clear_all();
        assert!(diagnostics_all("Bitcoin".into()).is_empty());
    }

    /// The screen's two numbers come from core.
    #[test]
    fn the_run_summary_counts_wallets_and_lists_their_sources() {
        let _g = test_lock();
        diagnostics_clear_all();
        assert_eq!(diagnostics_run_summary("Bitcoin".into()).wallet_count, 0);

        diagnostics_record("Bitcoin".into(), sample("w1"));
        diagnostics_record("Bitcoin".into(), sample("w2"));
        let summary = diagnostics_run_summary("Bitcoin".into());
        assert_eq!(summary.wallet_count, 2);
        assert_eq!(summary.sources.len(), 2);

        // Another chain reads its own rows, not this one's.
        assert_eq!(diagnostics_run_summary("XRP Ledger".into()).wallet_count, 0);
        // And a name no chain has is empty rather than a panic.
        assert_eq!(diagnostics_run_summary("Nope".into()).wallet_count, 0);
        diagnostics_clear_all();
    }

    /// One map for every chain, so the keying is the only thing keeping them
    /// apart. The five-map version got this for free and could not have got it
    /// wrong; this one has to be asked.
    #[test]
    fn chains_keep_separate_buckets() {
        let _g = test_lock();
        diagnostics_clear_all();
        diagnostics_record("Bitcoin".into(), sample("w"));

        assert_eq!(diagnostics_all("Bitcoin".into()).len(), 1);
        assert!(diagnostics_all("Litecoin".into()).is_empty());
        assert!(diagnostics_all("Bitcoin Cash".into()).is_empty());
        assert!(diagnostics_all("Ethereum".into()).is_empty());
        assert!(diagnostics_all("Tron".into()).is_empty());
        diagnostics_clear_all();
    }

    /// Two chains that used to live in the same typed map now live in the same
    /// map full stop, and a wallet on both keeps a row on each.
    #[test]
    fn one_wallet_on_two_chains_keeps_a_row_on_each() {
        let _g = test_lock();
        diagnostics_clear_all();
        diagnostics_record("Bitcoin".into(), sample("w"));
        diagnostics_record("Litecoin".into(), sample("w"));
        assert_eq!(diagnostics_all("Bitcoin".into()).len(), 1);
        assert_eq!(diagnostics_all("Litecoin".into()).len(), 1);

        diagnostics_forget_wallet("w".into());
        assert!(diagnostics_all("Bitcoin".into()).is_empty());
        assert!(diagnostics_all("Litecoin".into()).is_empty());
    }
}
