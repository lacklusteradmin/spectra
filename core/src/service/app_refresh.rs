//! Refresh intentions are platform inputs; scope, cadence and work belong here.
use super::*;
use crate::fetch::refresh_policy::{DeviceConditions, RefreshKind};

#[derive(Debug, Clone, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum AppRefreshIntent {
    Scheduled,
    Foreground,
    BalancesUpdated,
    User,
    Chain { chain_id: String },
    AfterSend { chain_id: String },
    DeepRescan { chain_id: String },
}
#[derive(Debug, Clone, serde::Serialize, uniffi::Record)]
pub struct AppRefreshResult {
    pub state: CoreAppState,
    pub pending: Option<PendingMaintenanceResult>,
    pub failures: Vec<String>,
    pub poll_seconds: u64,
    /// Price alerts this refresh crossed, to notify about.
    pub price_alerts: Vec<crate::store::PriceAlertNotification>,
    /// A large portfolio movement this refresh revealed, to notify about.
    pub movement: Option<super::standalone::LargeMovementEvaluation>,
    /// Stored transactions changed: a status or receipt, or new history. A
    /// front end re-reads its transaction projection only when this is set.
    pub transactions_changed: bool,
    /// The diagnostic state changed: a log line, or a chain's health. A front
    /// end re-reads diagnostics only when this is set.
    pub diagnostics_changed: bool,
}
#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Run one refresh for `intent` and record what went wrong in the
    /// operational log, so every caller — scheduled tick, user, CLI — logs alike.
    pub async fn refresh_app(
        &self,
        intent: AppRefreshIntent,
        conditions: DeviceConditions,
    ) -> Result<AppRefreshResult, SpectraBridgeError> {
        let rescanned = match &intent {
            AppRefreshIntent::DeepRescan { chain_id } => Some(chain_id.clone()),
            _ => None,
        };
        let diagnostics_before = self.diagnostics_fingerprint().await;
        let mut result = self.run_app_refresh(intent, conditions).await;
        match &result {
            Ok(result) => self.record_refresh_outcome(result, rescanned).await,
            Err(error) => {
                if let Some(chain_id) = rescanned {
                    self.record_event(
                        DiagnosticLogLevel::Error,
                        "Rescan",
                        format!("Deep rescan failed: {error}"),
                        Some(chain_id),
                        None,
                    )
                    .await;
                }
            }
        }
        if let Ok(result) = result.as_mut() {
            result.diagnostics_changed = self.diagnostics_fingerprint().await != diagnostics_before;
        }
        result
    }
}

impl WalletService {
    /// Enough of the diagnostic state to tell whether a refresh changed it:
    /// the newest log line and every chain's health.
    async fn diagnostics_fingerprint(&self) -> String {
        let state = self.wallet_state.read().await;
        let diagnostics = &state.diagnostics;
        let mut health: Vec<_> = diagnostics
            .degraded
            .iter()
            .map(|(chain, reason)| format!("{chain}:{reason:?}"))
            .chain(
                diagnostics
                    .last_good_unix
                    .iter()
                    .map(|(chain, at)| format!("{chain}@{at}")),
            )
            .collect();
        health.sort();
        let newest = diagnostics.logs.first().map(|log| log.id.as_str());
        format!("{newest:?}|{}", health.join(","))
    }

    async fn record_refresh_outcome(&self, result: &AppRefreshResult, rescanned: Option<String>) {
        for failure in &result.failures {
            self.record_event(
                DiagnosticLogLevel::Error,
                "Refresh",
                failure.clone(),
                rescanned.clone(),
                None,
            )
            .await;
        }
        for failure in result.pending.iter().flat_map(|p| &p.failures) {
            self.record_event(
                DiagnosticLogLevel::Error,
                "Pending Transactions",
                failure.message.clone(),
                Some(failure.chain_id.clone()),
                None,
            )
            .await;
        }
        if let Some(chain_id) = rescanned {
            let (level, message) = if result.failures.is_empty() {
                (
                    DiagnosticLogLevel::Info,
                    "Deep rescan completed.".to_string(),
                )
            } else {
                (
                    DiagnosticLogLevel::Warning,
                    format!(
                        "Deep rescan completed with {} failure(s).",
                        result.failures.len()
                    ),
                )
            };
            self.record_event(level, "Rescan", message, Some(chain_id), None)
                .await;
        }
    }

    async fn run_app_refresh(
        &self,
        intent: AppRefreshIntent,
        conditions: DeviceConditions,
    ) -> Result<AppRefreshResult, SpectraBridgeError> {
        let _guard = self.app_refresh_lock.lock().await;
        let plan = self.maintenance_plan(conditions.clone()).await;
        let chain = match &intent {
            AppRefreshIntent::Chain { chain_id }
            | AppRefreshIntent::AfterSend { chain_id }
            | AppRefreshIntent::DeepRescan { chain_id } => Some(chain_for_id(chain_id)?),
            _ => None,
        };
        let deep_rescan = matches!(intent, AppRefreshIntent::DeepRescan { .. });
        if deep_rescan && !chain.is_some_and(|c| c.supports_deep_utxo_discovery()) {
            return Err("Chain does not support deep UTXO discovery".into());
        }
        let mut result = AppRefreshResult {
            state: self.app_state().await,
            pending: None,
            failures: vec![],
            poll_seconds: plan.poll_seconds,
            price_alerts: vec![],
            movement: None,
            transactions_changed: false,
            diagnostics_changed: false,
        };
        if !conditions.is_network_reachable {
            if deep_rescan {
                result
                    .failures
                    .push("Deep rescan requires a network connection".into());
            }
            return Ok(result);
        }
        if matches!(intent, AppRefreshIntent::Foreground) {
            let last = self.refresh_clock.read().await.full_refresh_at;
            if last.is_some_and(|at| crate::wallet_db::now_secs() as f64 - at < 120.0) {
                return Ok(result);
            }
        }
        if deep_rescan {
            let id = chain.unwrap().str_id().to_string();
            match self.discover_chain_addresses(id.clone()).await {
                Ok(rows) => {
                    for row in rows {
                        if let Some(error) = row.error {
                            result.failures.push(format!("{}: {error}", row.wallet_id));
                        }
                    }
                }
                Err(error) => result.failures.push(error.to_string()),
            }
            if let Err(error) = self.advance_used_utxo_reservations(id).await {
                result.failures.push(error.to_string());
            }
        }
        let balances_updated = matches!(intent, AppRefreshIntent::BalancesUpdated);
        let scheduled = matches!(intent, AppRefreshIntent::Scheduled);
        if scheduled && !conditions.app_is_active && !plan.run_background_tick {
            return Ok(result);
        }
        let after_send = matches!(intent, AppRefreshIntent::AfterSend { .. });
        let heavy = !balances_updated
            && (!scheduled || (!conditions.app_is_active && plan.allow_heavy_background_work));
        let poll = !balances_updated
            && (!scheduled || plan.refresh_pending_transactions || plan.run_background_tick);
        if poll {
            match self.refresh_pending_transactions().await {
                Ok(pending) => {
                    result.transactions_changed |= !pending.changes.is_empty();
                    if pending.failures.is_empty() {
                        self.record_refresh(RefreshKind::PendingTransactions).await;
                    }
                    result.pending = Some(pending);
                }
                Err(e) => result.failures.push(e.to_string()),
            }
        }
        if heavy {
            use futures::{StreamExt, stream};
            let entries = {
                let state = self.wallet_state.read().await;
                crate::fetch::refresh_engine::refresh_entries_for(&state)
            };
            let entries = entries.into_iter().filter(|entry| {
                chain.is_none_or(|c| {
                    entry.chain_id == c.str_id()
                        || (deep_rescan
                            && !c.is_testnet()
                            && chain_for_id(&entry.chain_id)
                                .is_ok_and(|network| network.mainnet_counterpart() == c))
                })
            });
            let outcomes = stream::iter(entries)
                .map(|entry| self.refresh_wallet_balances(entry.wallet_id))
                .buffer_unordered(8)
                .collect::<Vec<_>>()
                .await;
            for outcome in outcomes {
                if let Err(error) = outcome {
                    result.failures.push(error.to_string());
                }
            }
            // History remains useful on receipt-polling chains too: it supplies
            // incoming transfers and complete transaction details after a send.
            let scope = match chain {
                Some(c) => HistoryRefreshScope::Chains {
                    chain_ids: vec![c.str_id().into()],
                },
                None => HistoryRefreshScope::All,
            };
            let interval = if scheduled { 300.0 } else { 0.0 };
            match self.refresh_history(scope, false, None, interval).await {
                Ok(rows) => {
                    for row in rows {
                        if let Some(error) = row.error {
                            result.failures.push(error);
                        }
                        if let Some(outcome) = row.outcome {
                            result.transactions_changed |= outcome.added > 0 || outcome.updated > 0;
                            if outcome.wallets_failed > 0 {
                                result.failures.push(format!(
                                    "{}: {} history reads failed",
                                    row.chain_id, outcome.wallets_failed
                                ));
                            }
                        }
                    }
                }
                Err(e) => result.failures.push(e.to_string()),
            }
        }
        if !after_send && !deep_rescan {
            if !scheduled
                || plan.refresh_live_prices
                || (plan.run_background_tick && conditions.wants_price_refresh)
            {
                match self.refresh_owned_prices(false).await {
                    Ok(state) => match state.quotes.prices_error {
                        Some(error) => result.failures.push(error),
                        None => self.record_refresh(RefreshKind::LivePrices).await,
                    },
                    Err(e) => result.failures.push(e.to_string()),
                }
            }
            match self.refresh_owned_fiat_rates(false).await {
                Ok(state) => {
                    if let Some(error) = state.quotes.fiat_error {
                        result.failures.push(error);
                    }
                }
                Err(e) => result.failures.push(e.to_string()),
            }
        }
        if scheduled
            && plan.run_background_tick
            && result.failures.is_empty()
            && result
                .pending
                .as_ref()
                .is_none_or(|p| p.failures.is_empty())
        {
            self.record_refresh(RefreshKind::BackgroundTick).await;
        }
        if heavy
            && chain.is_none()
            && result.failures.is_empty()
            && result
                .pending
                .as_ref()
                .is_none_or(|p| p.failures.is_empty())
        {
            self.refresh_clock.write().await.full_refresh_at =
                Some(crate::wallet_db::now_secs() as f64);
        }
        // What the refresh changed is judged here, once, after every write:
        // a front end that evaluated alerts itself had to order the calls.
        match self.evaluate_price_alerts().await {
            Ok(notifications) => result.price_alerts = notifications,
            Err(e) => result.failures.push(e.to_string()),
        }
        match self
            .evaluate_portfolio_movement(conditions.app_is_active)
            .await
        {
            Ok(movement) => result.movement = movement,
            Err(e) => result.failures.push(e.to_string()),
        }
        result.state = self.app_state().await;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn offline_intents_do_no_work_and_invalid_networks_refuse() {
        let service = WalletService::new(vec![]).unwrap();
        let conditions = DeviceConditions {
            app_is_active: true,
            is_network_reachable: false,
            is_constrained_network: false,
            is_expensive_network: false,
            is_low_power_mode: false,
            battery_level: 1.0,
            wants_price_refresh: true,
        };
        for intent in [
            AppRefreshIntent::User,
            AppRefreshIntent::Scheduled,
            AppRefreshIntent::Chain {
                chain_id: "ethereum".into(),
            },
            AppRefreshIntent::AfterSend {
                chain_id: "bitcoin".into(),
            },
        ] {
            let result = service
                .refresh_app(intent, conditions.clone())
                .await
                .unwrap();
            assert!(result.pending.is_none());
            assert!(result.failures.is_empty());
            assert!(result.state.quotes.prices_attempt_at.is_none());
            assert!(result.state.quotes.fiat_attempt_at.is_none());
        }
        let result = service
            .refresh_app(
                AppRefreshIntent::DeepRescan {
                    chain_id: "bitcoin".into(),
                },
                conditions.clone(),
            )
            .await
            .unwrap();
        assert!(!result.failures.is_empty());
        assert!(result.pending.is_none());
        assert!(
            service
                .refresh_app(
                    AppRefreshIntent::DeepRescan {
                        chain_id: "ethereum".into()
                    },
                    conditions.clone()
                )
                .await
                .is_err()
        );
        assert!(
            service
                .refresh_app(
                    AppRefreshIntent::AfterSend {
                        chain_id: "missing".into()
                    },
                    conditions
                )
                .await
                .is_err()
        );
    }
}
