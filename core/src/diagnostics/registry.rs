// Rust-owned per-wallet diagnostics registry.
//
// Swift previously held 24 `[String: T]` dictionaries (keyed by wallet id)
// on `WalletChainDiagnosticsState`. The state lives here and
// exposes get/set/remove/list/clear via UniFFI, one trio per chain.
//
// The in-memory shape is one typed HashMap per chain, guarded by a single
// Mutex — simple, and the dict-sized data is trivial so contention is
// irrelevant.

use std::collections::HashMap;
use std::sync::Mutex;

use super::types::*;

macro_rules! per_chain_registries {
    (
        $(
            ($field:ident, $ty:ident, $get:ident, $set:ident, $rm:ident, $all:ident, $replace:ident)
        ),* $(,)?
    ) => {
        #[derive(Default)]
        struct DiagnosticsRegistry {
            $( $field: HashMap<String, $ty>, )*
        }

        impl DiagnosticsRegistry {
            fn clear(&mut self) {
                $( self.$field.clear(); )*
            }
        }

        fn registry() -> &'static Mutex<DiagnosticsRegistry> {
            use std::sync::OnceLock;
            static REG: OnceLock<Mutex<DiagnosticsRegistry>> = OnceLock::new();
            REG.get_or_init(|| Mutex::new(DiagnosticsRegistry::default()))
        }

        $(
            #[uniffi::export]
            pub fn $get(wallet_id: String) -> Option<$ty> {
                registry().lock().unwrap().$field.get(&wallet_id).cloned()
            }

            #[uniffi::export]
            pub fn $set(wallet_id: String, value: $ty) {
                registry().lock().unwrap().$field.insert(wallet_id, value);
            }

            #[uniffi::export]
            pub fn $rm(wallet_id: String) {
                registry().lock().unwrap().$field.remove(&wallet_id);
            }

            #[uniffi::export]
            pub fn $all() -> HashMap<String, $ty> {
                registry().lock().unwrap().$field.clone()
            }

            /// Replace the entire per-chain dict in one call. Used by the Swift
            /// compatibility setter that presents a `[String: T]` API.
            #[uniffi::export]
            pub fn $replace(entries: HashMap<String, $ty>) {
                registry().lock().unwrap().$field = entries;
            }
        )*

        #[uniffi::export]
        pub fn diagnostics_clear_all() {
            registry().lock().unwrap().clear();
        }
    };
}

per_chain_registries! {
    (bitcoin, BitcoinHistoryDiagnostics,
        diagnostics_get_bitcoin, diagnostics_set_bitcoin,
        diagnostics_remove_bitcoin, diagnostics_all_bitcoin,
        diagnostics_replace_bitcoin),
    (bitcoin_cash, BitcoinHistoryDiagnostics,
        diagnostics_get_bitcoin_cash, diagnostics_set_bitcoin_cash,
        diagnostics_remove_bitcoin_cash, diagnostics_all_bitcoin_cash,
        diagnostics_replace_bitcoin_cash),
    (bitcoin_sv, BitcoinHistoryDiagnostics,
        diagnostics_get_bitcoin_sv, diagnostics_set_bitcoin_sv,
        diagnostics_remove_bitcoin_sv, diagnostics_all_bitcoin_sv,
        diagnostics_replace_bitcoin_sv),
    (litecoin, BitcoinHistoryDiagnostics,
        diagnostics_get_litecoin, diagnostics_set_litecoin,
        diagnostics_remove_litecoin, diagnostics_all_litecoin,
        diagnostics_replace_litecoin),
    (dogecoin, BitcoinHistoryDiagnostics,
        diagnostics_get_dogecoin, diagnostics_set_dogecoin,
        diagnostics_remove_dogecoin, diagnostics_all_dogecoin,
        diagnostics_replace_dogecoin),

    (ethereum, EthereumTokenTransferHistoryDiagnostics,
        diagnostics_get_ethereum, diagnostics_set_ethereum,
        diagnostics_remove_ethereum, diagnostics_all_ethereum,
        diagnostics_replace_ethereum),
    (etc, EthereumTokenTransferHistoryDiagnostics,
        diagnostics_get_etc, diagnostics_set_etc,
        diagnostics_remove_etc, diagnostics_all_etc,
        diagnostics_replace_etc),
    (arbitrum, EthereumTokenTransferHistoryDiagnostics,
        diagnostics_get_arbitrum, diagnostics_set_arbitrum,
        diagnostics_remove_arbitrum, diagnostics_all_arbitrum,
        diagnostics_replace_arbitrum),
    (optimism, EthereumTokenTransferHistoryDiagnostics,
        diagnostics_get_optimism, diagnostics_set_optimism,
        diagnostics_remove_optimism, diagnostics_all_optimism,
        diagnostics_replace_optimism),
    (bnb, EthereumTokenTransferHistoryDiagnostics,
        diagnostics_get_bnb, diagnostics_set_bnb,
        diagnostics_remove_bnb, diagnostics_all_bnb,
        diagnostics_replace_bnb),
    (avalanche, EthereumTokenTransferHistoryDiagnostics,
        diagnostics_get_avalanche, diagnostics_set_avalanche,
        diagnostics_remove_avalanche, diagnostics_all_avalanche,
        diagnostics_replace_avalanche),
    (hyperliquid, EthereumTokenTransferHistoryDiagnostics,
        diagnostics_get_hyperliquid, diagnostics_set_hyperliquid,
        diagnostics_remove_hyperliquid, diagnostics_all_hyperliquid,
        diagnostics_replace_hyperliquid),

    (tron, TronHistoryDiagnostics,
        diagnostics_get_tron, diagnostics_set_tron,
        diagnostics_remove_tron, diagnostics_all_tron,
        diagnostics_replace_tron),
    (solana, SolanaHistoryDiagnostics,
        diagnostics_get_solana, diagnostics_set_solana,
        diagnostics_remove_solana, diagnostics_all_solana,
        diagnostics_replace_solana),
    (xrp, XRPHistoryDiagnostics,
        diagnostics_get_xrp, diagnostics_set_xrp,
        diagnostics_remove_xrp, diagnostics_all_xrp,
        diagnostics_replace_xrp),
    (stellar, StellarHistoryDiagnostics,
        diagnostics_get_stellar, diagnostics_set_stellar,
        diagnostics_remove_stellar, diagnostics_all_stellar,
        diagnostics_replace_stellar),
    (monero, MoneroHistoryDiagnostics,
        diagnostics_get_monero, diagnostics_set_monero,
        diagnostics_remove_monero, diagnostics_all_monero,
        diagnostics_replace_monero),
    (sui, SuiHistoryDiagnostics,
        diagnostics_get_sui, diagnostics_set_sui,
        diagnostics_remove_sui, diagnostics_all_sui,
        diagnostics_replace_sui),
    (aptos, AptosHistoryDiagnostics,
        diagnostics_get_aptos, diagnostics_set_aptos,
        diagnostics_remove_aptos, diagnostics_all_aptos,
        diagnostics_replace_aptos),
    (ton, TONHistoryDiagnostics,
        diagnostics_get_ton, diagnostics_set_ton,
        diagnostics_remove_ton, diagnostics_all_ton,
        diagnostics_replace_ton),
    (icp, ICPHistoryDiagnostics,
        diagnostics_get_icp, diagnostics_set_icp,
        diagnostics_remove_icp, diagnostics_all_icp,
        diagnostics_replace_icp),
    (near, NearHistoryDiagnostics,
        diagnostics_get_near, diagnostics_set_near,
        diagnostics_remove_near, diagnostics_all_near,
        diagnostics_replace_near),
    (polkadot, PolkadotHistoryDiagnostics,
        diagnostics_get_polkadot, diagnostics_set_polkadot,
        diagnostics_remove_polkadot, diagnostics_all_polkadot,
        diagnostics_replace_polkadot),
    (cardano, CardanoHistoryDiagnostics,
        diagnostics_get_cardano, diagnostics_set_cardano,
        diagnostics_remove_cardano, diagnostics_all_cardano,
        diagnostics_replace_cardano),
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
    fn set_get_remove_list_clear() {
        let _g = test_lock();
        diagnostics_clear_all();
        assert!(diagnostics_get_bitcoin("w1".into()).is_none());

        diagnostics_set_bitcoin("w1".into(), sample_bitcoin("w1"));
        diagnostics_set_bitcoin("w2".into(), sample_bitcoin("w2"));
        assert_eq!(diagnostics_all_bitcoin().len(), 2);
        assert_eq!(
            diagnostics_get_bitcoin("w1".into()).unwrap().wallet_id,
            "w1"
        );

        diagnostics_remove_bitcoin("w1".into());
        assert!(diagnostics_get_bitcoin("w1".into()).is_none());
        assert_eq!(diagnostics_all_bitcoin().len(), 1);

        // Replace-all
        let mut replacement = HashMap::new();
        replacement.insert("w3".into(), sample_bitcoin("w3"));
        diagnostics_replace_bitcoin(replacement);
        assert_eq!(diagnostics_all_bitcoin().len(), 1);
        assert!(diagnostics_get_bitcoin("w3".into()).is_some());
        assert!(diagnostics_get_bitcoin("w2".into()).is_none());

        diagnostics_clear_all();
        assert!(diagnostics_all_bitcoin().is_empty());
    }

    #[test]
    fn independent_chain_buckets() {
        let _g = test_lock();
        diagnostics_clear_all();
        diagnostics_set_bitcoin("w".into(), sample_bitcoin("w"));
        assert!(diagnostics_all_litecoin().is_empty());
        assert_eq!(diagnostics_all_bitcoin().len(), 1);
    }
}
