//! Awaited domain reset. Platform caches and authentication stay in the shell.
use super::*;
#[derive(Debug, Clone, uniffi::Record)]
pub struct ResetOutcome {
    pub state: CoreAppState,
    pub plan: crate::store::CoreResetPlan,
}
#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Complete selected domain cleanup before returning. Steps are idempotent;
    /// failure is explicit, so a partially completed reset can be retried.
    pub async fn reset_data(
        &self,
        scopes: Vec<String>,
    ) -> Result<ResetOutcome, SpectraBridgeError> {
        self.bound_database().await?;
        for scope in &scopes {
            if !matches!(
                scope.as_str(),
                "walletsAndSecrets"
                    | "historyAndCache"
                    | "alertsAndContacts"
                    | "settingsAndEndpoints"
                    | "dashboardCustomization"
                    | "providerState"
            ) {
                return Err(SpectraBridgeError::InvalidInput {
                    message: format!("unknown reset scope {scope}"),
                });
            }
        }
        let plan = crate::store::core_reset_dispatch(scopes);
        let mutation = plan.clone();
        self.mutate_persisted_state(move |state| {
            if mutation.reset_wallets_and_secrets {
                state.wallets.clear();
                state.selected_wallet_id = None;
            }
            if mutation.reset_alerts_and_contacts {
                state.price_alerts.clear();
                state.address_book.clear();
            }
            if mutation.reset_settings_and_endpoints {
                reduce_state_in_place(state, StateCommand::ResetAppSettings);
                reduce_state_in_place(state, StateCommand::ResetTokenPreferences);
            }
            if mutation.reset_dashboard_customization {
                state.settings.pinned_dashboard_token_ids.clear();
            }
            if mutation.reset_history_and_cache {
                state.diagnostics = Default::default();
                state.quotes = Default::default();
                state.movement_baseline = None;
                state.fiat_rates_from_usd.clear();
            }
            vec![crate::store::state::StateEvent {
                kind: "dataReset".into(),
                subject_id: None,
            }]
        })
        .await?;
        if plan.reset_history_and_cache {
            self.apply_transaction_command(crate::service::types::TransactionCommand::Clear)
                .await?;
            self.clear_operational_events(None).await?;
            self.reset_history(crate::service::history_cursor::HistoryScope::All);
            self.status_trackers.write().await.clear();
            *self.refresh_clock.write().await = Default::default();
            crate::diagnostics::diagnostics_clear_all();
        }
        Ok(ResetOutcome {
            state: self.app_state().await,
            plan,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn owned_reset_refuses_invalid_scope_and_waits_for_secret_cleanup() {
        let service = WalletService::new_typed(vec![]).unwrap();
        let path = std::env::temp_dir()
            .join(format!("reset-{}.sqlite", crate::store::new_event_id()))
            .to_string_lossy()
            .into_owned();
        service.open_state(path.clone()).await.unwrap();
        let wallet = crate::store::state::WalletSummary {
            id: "w".into(),
            name: "W".into(),
            is_watch_only: false,
            chain_name: "Ethereum".into(),
            include_in_portfolio_total: true,
            network_id: "ethereum".into(),
            xpub: None,
            derivation_preset: "standard".into(),
            derivation_path: None,
            derivation_overrides: Default::default(),
            holdings: vec![],
            addresses: vec![],
        };
        service
            .apply_state_command(StateCommand::UpsertWallet {
                wallet: wallet.clone(),
            })
            .await
            .unwrap();
        assert!(service.reset_data(vec!["typo".into()]).await.is_err());
        assert!(service
            .reset_data(vec!["walletsAndSecrets".into()])
            .await
            .is_err());
        assert_eq!(service.app_state().await.wallets.len(), 1);
        service
            .reset_data(vec!["settingsAndEndpoints".into()])
            .await
            .unwrap();
        assert_eq!(service.app_state().await.wallets.len(), 1);
        let mut watch = wallet;
        watch.is_watch_only = true;
        service
            .apply_state_command(StateCommand::UpsertWallet { wallet: watch })
            .await
            .unwrap();
        service
            .record_status_poll("orphan".into(), StatusPollOutcome::Failed)
            .await;
        let key = crate::fetch::refresh::policy::HistoryRefreshKey::new("w", "ethereum");
        service.record_history_refresh(key.clone()).await;
        let result = service
            .reset_data(vec!["walletsAndSecrets".into(), "alertsAndContacts".into()])
            .await
            .unwrap();
        assert!(result.plan.reset_history_and_cache);
        assert!(result.state.wallets.is_empty());
        assert_eq!(
            service
                .history_refresh_plans(vec![key.clone()], 3600.0)
                .await,
            vec![key]
        );
        assert!(service.status_trackers.read().await.is_empty());
        let reopened = WalletService::new_typed(vec![]).unwrap();
        assert!(reopened.open_state(path).await.unwrap().wallets.is_empty());
    }
}
