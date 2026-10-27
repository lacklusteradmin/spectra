// Per-chain diagnostics: what the last history run and endpoint check
// found, keyed by chain id.
//
// Process memory, like the runs it describes, held by the service that ran
// them: every front end reads the same answer from core, and none keeps a
// copy of its own. One map behind one lock; the data is small, so contention
// is irrelevant.

use std::collections::HashMap;

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

/// What the service's history runs and endpoint checks found, per chain.
#[derive(Default)]
pub(crate) struct DiagnosticRuns {
    chains: parking_lot::Mutex<HashMap<crate::registry::Chain, ChainRecord>>,
}

/// What the runs hold for one family: history keyed by the family,
/// endpoints by the network it is on.
pub(crate) struct RecordedChainDiagnostics {
    pub history: Vec<HistoryDiagnostics>,
    pub history_run_at_unix: Option<f64>,
    pub endpoints: Vec<EndpointProbe>,
    pub endpoints_checked_at_unix: Option<f64>,
}

impl DiagnosticRuns {
    /// Record one wallet's history-diagnostics row for a chain.
    pub(crate) fn record(&self, chain_id: crate::registry::Chain, entry: HistoryDiagnostics) {
        self.chains
            .lock()
            .entry(chain_id)
            .or_default()
            .history
            .insert(entry.wallet_id.clone(), entry);
    }

    /// Stamp a chain's history run, whatever it found.
    pub(crate) fn record_history_run(&self, chain_id: crate::registry::Chain) {
        self.chains
            .lock()
            .entry(chain_id)
            .or_default()
            .history_run_at_unix = Some(crate::store::now_unix());
    }

    /// Replace a network's endpoint results with a check that just finished.
    pub(crate) fn record_endpoints(
        &self,
        chain_id: crate::registry::Chain,
        endpoints: Vec<EndpointProbe>,
    ) {
        let mut chains = self.chains.lock();
        let record = chains.entry(chain_id).or_default();
        record.endpoints = endpoints;
        record.endpoints_checked_at_unix = Some(crate::store::now_unix());
    }

    pub(crate) fn recorded(
        &self,
        family: crate::registry::Chain,
        network: crate::registry::Chain,
    ) -> RecordedChainDiagnostics {
        let chains = self.chains.lock();
        let family = chains.get(&family);
        let network = chains.get(&network);
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

    /// Drop every row a wallet left behind, on every chain: removing a
    /// wallet does it.
    pub(crate) fn forget_wallet(&self, wallet_id: &str) {
        for record in self.chains.lock().values_mut() {
            record.history.remove(wallet_id);
        }
    }

    pub(crate) fn clear(&self) {
        self.chains.lock().clear();
    }

    /// A chain's history rows, keyed by wallet.
    #[cfg(test)]
    pub(crate) fn all(
        &self,
        chain_id: crate::registry::Chain,
    ) -> HashMap<String, HistoryDiagnostics> {
        self.recorded(chain_id, chain_id)
            .history
            .into_iter()
            .map(|row| (row.wallet_id.clone(), row))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let runs = DiagnosticRuns::default();
        assert!(runs.all(crate::registry::Chain::Bitcoin).is_empty());

        runs.record(crate::registry::Chain::Bitcoin, sample("w1"));
        runs.record(crate::registry::Chain::Bitcoin, sample("w2"));
        assert_eq!(runs.all(crate::registry::Chain::Bitcoin).len(), 2);

        runs.record(crate::registry::Chain::Bitcoin, sample("w3"));
        let stored = runs.all(crate::registry::Chain::Bitcoin);
        assert_eq!(
            stored.len(),
            3,
            "recording a third wallet dropped the first two"
        );
        assert!(stored.contains_key("w1") && stored.contains_key("w3"));

        // And a wallet that goes away takes its rows with it, on every chain.
        runs.forget_wallet("w1");
        let stored = runs.all(crate::registry::Chain::Bitcoin);
        assert_eq!(stored.len(), 2);
        assert!(!stored.contains_key("w1"));

        runs.clear();
        assert!(runs.all(crate::registry::Chain::Bitcoin).is_empty());
    }

    /// History is the family's, endpoints the named network's, and each
    /// carries the time its run finished.
    #[test]
    fn recorded_diagnostics_join_family_history_with_network_endpoints() {
        let runs = DiagnosticRuns::default();
        let empty = runs.recorded(
            crate::registry::Chain::Bitcoin,
            crate::registry::Chain::BitcoinTestnet,
        );
        assert!(empty.history.is_empty() && empty.endpoints.is_empty());
        assert!(empty.history_run_at_unix.is_none() && empty.endpoints_checked_at_unix.is_none());

        runs.record(crate::registry::Chain::Bitcoin, sample("w2"));
        runs.record(crate::registry::Chain::Bitcoin, sample("w1"));
        runs.record_history_run(crate::registry::Chain::Bitcoin);
        let probe = |endpoint: &str| EndpointProbe {
            api: crate::api::endpoint::EndpointApi::Esplora,
            chain_id: crate::registry::Chain::Bitcoin,
            endpoint: endpoint.into(),
            capabilities: Vec::new(),
            checked: true,
            reachable: true,
            detail: String::new(),
        };
        runs.record_endpoints(crate::registry::Chain::Bitcoin, vec![probe("https://main")]);
        runs.record_endpoints(
            crate::registry::Chain::BitcoinTestnet,
            vec![probe("https://test")],
        );

        let recorded = runs.recorded(
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
        runs.clear();
    }

    /// One map for every chain, so the keying is the only thing keeping them
    /// apart. The five-map version got this for free and could not have got it
    /// wrong; this one has to be asked.
    #[test]
    fn chains_keep_separate_buckets() {
        let runs = DiagnosticRuns::default();
        runs.record(crate::registry::Chain::Bitcoin, sample("w"));

        assert_eq!(runs.all(crate::registry::Chain::Bitcoin).len(), 1);
        assert!(runs.all(crate::registry::Chain::Litecoin).is_empty());
        assert!(runs.all(crate::registry::Chain::BitcoinCash).is_empty());
        assert!(runs.all(crate::registry::Chain::Ethereum).is_empty());
        assert!(runs.all(crate::registry::Chain::Tron).is_empty());
        runs.clear();
    }

    /// A wallet on two chains keeps a row on each.
    #[test]
    fn one_wallet_on_two_chains_keeps_a_row_on_each() {
        let runs = DiagnosticRuns::default();
        runs.record(crate::registry::Chain::Bitcoin, sample("w"));
        runs.record(crate::registry::Chain::Litecoin, sample("w"));
        assert_eq!(runs.all(crate::registry::Chain::Bitcoin).len(), 1);
        assert_eq!(runs.all(crate::registry::Chain::Litecoin).len(), 1);

        runs.forget_wallet("w");
        assert!(runs.all(crate::registry::Chain::Bitcoin).is_empty());
        assert!(runs.all(crate::registry::Chain::Litecoin).is_empty());
    }
}
