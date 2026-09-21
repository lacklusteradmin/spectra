//! Refresh scheduling from core-owned clocks, settings and pending transactions.
use crate::fetch::refresh_policy::{DeviceConditions, MaintenancePlan, RefreshKind};
use crate::service::WalletService;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// What to do this tick, and how long to wait for the next one.
    pub async fn maintenance_plan(&self, conditions: DeviceConditions) -> MaintenancePlan {
        let settings = self.wallet_state.read().await.settings.clone();
        let has_pending_work = self.has_pending_transaction_work().await;
        let clock = self.refresh_clock.read().await.clone();
        crate::fetch::refresh_policy::maintenance_plan(
            &clock,
            &settings,
            &conditions,
            has_pending_work,
            crate::wallet_db::now_secs() as f64,
        )
    }
}

impl WalletService {
    /// Stamp the clock. Called once a refresh has actually run, so the next
    /// plan measures from when the work happened rather than when it was asked
    /// for.
    pub async fn record_refresh(&self, kind: RefreshKind) {
        let now = crate::wallet_db::now_secs() as f64;
        self.refresh_clock.write().await.record(kind, now);
    }

    /// Whether any recorded send is still worth polling for confirmation.
    ///
    /// Read from core's own store. iOS derived this from its transaction
    /// projection and passed the answer in, which is the shape the migration
    /// removes: core has the transactions.
    async fn has_pending_transaction_work(&self) -> bool {
        self.pending_maintenance_chains()
            .await
            .is_ok_and(|chains| !chains.is_empty())
    }
}

impl WalletService {
    pub(crate) async fn history_refresh_plans(
        &self,
        keys: Vec<crate::fetch::refresh_policy::HistoryRefreshKey>,
        interval_secs: f64,
    ) -> Vec<crate::fetch::refresh_policy::HistoryRefreshKey> {
        let clock = self.refresh_clock.read().await;
        crate::fetch::refresh_policy::history_plans(
            &clock,
            keys,
            interval_secs,
            crate::wallet_db::now_secs() as f64,
        )
    }

    pub(crate) async fn record_history_refresh(
        &self,
        key: crate::fetch::refresh_policy::HistoryRefreshKey,
    ) {
        let now = crate::wallet_db::now_secs() as f64;
        self.refresh_clock.write().await.record_history(key, now);
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    #[tokio::test]
    async fn owned_clock_coalesces_refreshes_and_partitions_history() {
        let service = WalletService::new(vec![]).unwrap();
        let conditions = DeviceConditions {
            app_is_active: true,
            is_network_reachable: true,
            is_constrained_network: false,
            is_expensive_network: false,
            is_low_power_mode: false,
            battery_level: 1.0,
            wants_price_refresh: true,
        };
        assert!(
            service
                .maintenance_plan(conditions.clone())
                .await
                .refresh_live_prices
        );
        service.record_refresh(RefreshKind::LivePrices).await;
        assert!(
            !service
                .maintenance_plan(conditions)
                .await
                .refresh_live_prices
        );
        let eth = crate::fetch::refresh_policy::HistoryRefreshKey::new("w", "ethereum");
        let btc = crate::fetch::refresh_policy::HistoryRefreshKey::new("w", "bitcoin");
        service.record_history_refresh(eth.clone()).await;
        let due = service
            .history_refresh_plans(vec![eth, btc.clone()], 120.0)
            .await;
        assert_eq!(due, vec![btc]);
    }
}
