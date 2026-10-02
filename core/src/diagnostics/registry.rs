// Per-chain diagnostics registry: what the last history run and endpoint
// check found, keyed by chain id.
//
// One HashMap guarded by a single Mutex — the data is small, so contention is
// irrelevant. Process memory, like the runs it describes: every front end
// reads the same answer from core, and none keeps a copy of its own.

use std::collections::HashMap;
use std::sync::Mutex;

use super::types::*;
use crate::service::EndpointProbe;

/// What one chain's diagnostics have recorded.
#[derive(Default)]
struct ChainRecord {
    /// History rows, keyed by wallet.
    history: HashMap<String, HistoryDiagnostics>,
    history_run_at_unix: Option<f64>,
    endpoints: Vec<EndpointProbe>,
    endpoints_checked_at_unix: Option<f64>,
}

#[derive(Default)]
struct DiagnosticsRegistry {
    chains: HashMap<crate::registry::Chain, ChainRecord>,
}

fn registry() -> &'static Mutex<DiagnosticsRegistry> {
    use std::sync::OnceLock;
    static REG: OnceLock<Mutex<DiagnosticsRegistry>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(DiagnosticsRegistry::default()))
}

/// Record one wallet's history-diagnostics row for a chain.
///
/// Internal: `refresh_history` records its own rows.
pub fn diagnostics_record(chain_id: crate::registry::Chain, entry: HistoryDiagnostics) {
    registry()
        .lock()
        .unwrap()
        .chains
        .entry(chain_id)
        .or_default()
        .history
        .insert(entry.wallet_id.clone(), entry);
}

/// Stamp a chain's history run, whatever it found.
pub fn diagnostics_record_history_run(chain_id: crate::registry::Chain) {
    registry()
        .lock()
        .unwrap()
        .chains
        .entry(chain_id)
        .or_default()
        .history_run_at_unix = Some(crate::store::now_unix());
}

/// Replace a network's endpoint results with a check that just finished.
pub fn diagnostics_record_endpoints(
    chain_id: crate::registry::Chain,
    endpoints: Vec<EndpointProbe>,
) {
    let mut reg = registry().lock().unwrap();
    let record = reg.chains.entry(chain_id).or_default();
    record.endpoints = endpoints;
    record.endpoints_checked_at_unix = Some(crate::store::now_unix());
}

/// What the diagnostics registry holds for one family: history keyed by the
/// family, endpoints by the network it is on.
pub(crate) struct RecordedChainDiagnostics {
    pub history: Vec<HistoryDiagnostics>,
    pub history_run_at_unix: Option<f64>,
    pub endpoints: Vec<EndpointProbe>,
    pub endpoints_checked_at_unix: Option<f64>,
}

pub(crate) fn diagnostics_recorded(
    family: crate::registry::Chain,
    network: crate::registry::Chain,
) -> RecordedChainDiagnostics {
    let reg = registry().lock().unwrap();
    let family = reg.chains.get(&family);
    let network = reg.chains.get(&network);
    let mut history: Vec<HistoryDiagnostics> = family
        .map(|c| c.history.values().cloned().collect())
        .unwrap_or_default();
    history.sort_by(|a, b| a.wallet_id.cmp(&b.wallet_id));
    RecordedChainDiagnostics {
        history,
        history_run_at_unix: family.and_then(|c| c.history_run_at_unix),
        endpoints: network.map(|c| c.endpoints.clone()).unwrap_or_default(),
        endpoints_checked_at_unix: network.and_then(|c| c.endpoints_checked_at_unix),
    }
}

/// Drop every diagnostics row a wallet left behind, on every chain.
///
/// Internal: removing a wallet does it.
pub fn diagnostics_forget_wallet(wallet_id: String) {
    let mut reg = registry().lock().unwrap();
    for record in reg.chains.values_mut() {
        record.history.remove(&wallet_id);
    }
}

pub fn diagnostics_clear_all() {
    registry().lock().unwrap().chains.clear();
}

/// The registry is a shared global; tests that clear or read it whole hold
/// this so they do not race each other.
#[cfg(test)]
pub(crate) fn diagnostics_test_lock() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// A chain's history rows, keyed by wallet.
#[cfg(test)]
pub fn diagnostics_all(chain_id: crate::registry::Chain) -> HashMap<String, HistoryDiagnostics> {
    diagnostics_recorded(chain_id, chain_id)
        .history
        .into_iter()
        .map(|row| (row.wallet_id.clone(), row))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        diagnostics_test_lock()
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
        assert!(diagnostics_all(crate::registry::Chain::Bitcoin).is_empty());

        diagnostics_record(crate::registry::Chain::Bitcoin, sample("w1"));
        diagnostics_record(crate::registry::Chain::Bitcoin, sample("w2"));
        assert_eq!(diagnostics_all(crate::registry::Chain::Bitcoin).len(), 2);

        diagnostics_record(crate::registry::Chain::Bitcoin, sample("w3"));
        let stored = diagnostics_all(crate::registry::Chain::Bitcoin);
        assert_eq!(
            stored.len(),
            3,
            "recording a third wallet dropped the first two"
        );
        assert!(stored.contains_key("w1") && stored.contains_key("w3"));

        // And a wallet that goes away takes its rows with it, on every chain.
        diagnostics_forget_wallet("w1".into());
        let stored = diagnostics_all(crate::registry::Chain::Bitcoin);
        assert_eq!(stored.len(), 2);
        assert!(!stored.contains_key("w1"));

        diagnostics_clear_all();
        assert!(diagnostics_all(crate::registry::Chain::Bitcoin).is_empty());
    }

    /// History is the family's, endpoints the named network's, and each
    /// carries the time its run finished.
    #[test]
    fn recorded_diagnostics_join_family_history_with_network_endpoints() {
        let _g = test_lock();
        diagnostics_clear_all();
        let empty = diagnostics_recorded(
            crate::registry::Chain::Bitcoin,
            crate::registry::Chain::BitcoinTestnet,
        );
        assert!(empty.history.is_empty() && empty.endpoints.is_empty());
        assert!(empty.history_run_at_unix.is_none() && empty.endpoints_checked_at_unix.is_none());

        diagnostics_record(crate::registry::Chain::Bitcoin, sample("w2"));
        diagnostics_record(crate::registry::Chain::Bitcoin, sample("w1"));
        diagnostics_record_history_run(crate::registry::Chain::Bitcoin);
        let probe = |endpoint: &str| EndpointProbe {
            api: crate::EndpointApi::Esplora,
            chain_id: crate::registry::Chain::Bitcoin,
            endpoint: endpoint.into(),
            capabilities: Vec::new(),
            checked: true,
            reachable: true,
            detail: String::new(),
        };
        diagnostics_record_endpoints(crate::registry::Chain::Bitcoin, vec![probe("https://main")]);
        diagnostics_record_endpoints(
            crate::registry::Chain::BitcoinTestnet,
            vec![probe("https://test")],
        );

        let recorded = diagnostics_recorded(
            crate::registry::Chain::Bitcoin,
            crate::registry::Chain::BitcoinTestnet,
        );
        let wallets: Vec<_> = recorded
            .history
            .iter()
            .map(|h| h.wallet_id.as_str())
            .collect();
        assert_eq!(wallets, ["w1", "w2"], "rows are ordered, not hash-ordered");
        assert!(recorded.history_run_at_unix.is_some());
        assert_eq!(recorded.endpoints.len(), 1);
        assert_eq!(recorded.endpoints[0].endpoint, "https://test");
        assert!(recorded.endpoints_checked_at_unix.is_some());
        diagnostics_clear_all();
    }

    /// One map for every chain, so the keying is the only thing keeping them
    /// apart. The five-map version got this for free and could not have got it
    /// wrong; this one has to be asked.
    #[test]
    fn chains_keep_separate_buckets() {
        let _g = test_lock();
        diagnostics_clear_all();
        diagnostics_record(crate::registry::Chain::Bitcoin, sample("w"));

        assert_eq!(diagnostics_all(crate::registry::Chain::Bitcoin).len(), 1);
        assert!(diagnostics_all(crate::registry::Chain::Litecoin).is_empty());
        assert!(diagnostics_all(crate::registry::Chain::BitcoinCash).is_empty());
        assert!(diagnostics_all(crate::registry::Chain::Ethereum).is_empty());
        assert!(diagnostics_all(crate::registry::Chain::Tron).is_empty());
        diagnostics_clear_all();
    }

    /// A wallet on two chains keeps a row on each.
    #[test]
    fn one_wallet_on_two_chains_keeps_a_row_on_each() {
        let _g = test_lock();
        diagnostics_clear_all();
        diagnostics_record(crate::registry::Chain::Bitcoin, sample("w"));
        diagnostics_record(crate::registry::Chain::Litecoin, sample("w"));
        assert_eq!(diagnostics_all(crate::registry::Chain::Bitcoin).len(), 1);
        assert_eq!(diagnostics_all(crate::registry::Chain::Litecoin).len(), 1);

        diagnostics_forget_wallet("w".into());
        assert!(diagnostics_all(crate::registry::Chain::Bitcoin).is_empty());
        assert!(diagnostics_all(crate::registry::Chain::Litecoin).is_empty());
    }
}
