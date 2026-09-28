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
    chains: HashMap<String, ChainRecord>,
}

fn registry() -> &'static Mutex<DiagnosticsRegistry> {
    use std::sync::OnceLock;
    static REG: OnceLock<Mutex<DiagnosticsRegistry>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(DiagnosticsRegistry::default()))
}

/// Record one wallet's history-diagnostics row for a chain.
///
/// Internal: `refresh_history` records its own rows.
pub fn diagnostics_record(chain_id: String, entry: HistoryDiagnostics) {
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
pub fn diagnostics_record_history_run(chain_id: String) {
    registry()
        .lock()
        .unwrap()
        .chains
        .entry(chain_id)
        .or_default()
        .history_run_at_unix = Some(crate::store::now_unix());
}

/// Replace a network's endpoint results with a check that just finished.
pub fn diagnostics_record_endpoints(chain_id: String, endpoints: Vec<EndpointProbe>) {
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

pub(crate) fn diagnostics_recorded(family_id: &str, network_id: &str) -> RecordedChainDiagnostics {
    let reg = registry().lock().unwrap();
    let family = reg.chains.get(family_id);
    let network = reg.chains.get(network_id);
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
pub fn diagnostics_all(chain_id: String) -> HashMap<String, HistoryDiagnostics> {
    diagnostics_recorded(&chain_id, &chain_id)
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
        assert!(diagnostics_all("bitcoin".into()).is_empty());

        diagnostics_record("bitcoin".into(), sample("w1"));
        diagnostics_record("bitcoin".into(), sample("w2"));
        assert_eq!(diagnostics_all("bitcoin".into()).len(), 2);

        diagnostics_record("bitcoin".into(), sample("w3"));
        let stored = diagnostics_all("bitcoin".into());
        assert_eq!(
            stored.len(),
            3,
            "recording a third wallet dropped the first two"
        );
        assert!(stored.contains_key("w1") && stored.contains_key("w3"));

        // And a wallet that goes away takes its rows with it, on every chain.
        diagnostics_forget_wallet("w1".into());
        let stored = diagnostics_all("bitcoin".into());
        assert_eq!(stored.len(), 2);
        assert!(!stored.contains_key("w1"));

        diagnostics_clear_all();
        assert!(diagnostics_all("bitcoin".into()).is_empty());
    }

    /// History is the family's, endpoints the selected network's, and each
    /// carries the time its run finished.
    #[test]
    fn recorded_diagnostics_join_family_history_with_network_endpoints() {
        let _g = test_lock();
        diagnostics_clear_all();
        let empty = diagnostics_recorded("bitcoin", "bitcoin-testnet");
        assert!(empty.history.is_empty() && empty.endpoints.is_empty());
        assert!(empty.history_run_at_unix.is_none() && empty.endpoints_checked_at_unix.is_none());

        diagnostics_record("bitcoin".into(), sample("w2"));
        diagnostics_record("bitcoin".into(), sample("w1"));
        diagnostics_record_history_run("bitcoin".into());
        let probe = |endpoint: &str| EndpointProbe {
            api: crate::EndpointApi::Esplora,
            chain_id: String::new(),
            endpoint: endpoint.into(),
            capabilities: Vec::new(),
            checked: true,
            reachable: true,
            detail: String::new(),
        };
        diagnostics_record_endpoints("bitcoin".into(), vec![probe("https://main")]);
        diagnostics_record_endpoints("bitcoin-testnet".into(), vec![probe("https://test")]);

        let recorded = diagnostics_recorded("bitcoin", "bitcoin-testnet");
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
        diagnostics_record("bitcoin".into(), sample("w"));

        assert_eq!(diagnostics_all("bitcoin".into()).len(), 1);
        assert!(diagnostics_all("litecoin".into()).is_empty());
        assert!(diagnostics_all("bitcoin-cash".into()).is_empty());
        assert!(diagnostics_all("ethereum".into()).is_empty());
        assert!(diagnostics_all("tron".into()).is_empty());
        diagnostics_clear_all();
    }

    /// A wallet on two chains keeps a row on each.
    #[test]
    fn one_wallet_on_two_chains_keeps_a_row_on_each() {
        let _g = test_lock();
        diagnostics_clear_all();
        diagnostics_record("bitcoin".into(), sample("w"));
        diagnostics_record("litecoin".into(), sample("w"));
        assert_eq!(diagnostics_all("bitcoin".into()).len(), 1);
        assert_eq!(diagnostics_all("litecoin".into()).len(), 1);

        diagnostics_forget_wallet("w".into());
        assert!(diagnostics_all("bitcoin".into()).is_empty());
        assert!(diagnostics_all("litecoin".into()).is_empty());
    }
}
