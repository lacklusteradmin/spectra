//! Core owns history protocol selection, scope, and successful refresh clocks.
use super::{HistoryRefreshOutcome, WalletService};
use crate::fetch::refresh::policy::HistoryRefreshKey;
use crate::{registry::Chain, SpectraBridgeError};

#[derive(Debug, Clone, uniffi::Enum)]
pub enum HistoryRefreshScope {
    All,
    Chains { chain_ids: Vec<String> },
    Wallets { wallet_ids: Vec<String> },
}
#[derive(Debug, Clone, uniffi::Record)]
pub struct ChainHistoryRefresh {
    pub chain_id: String,
    pub outcome: Option<HistoryRefreshOutcome>,
    pub error: Option<String>,
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Refresh or page the requested stored history. Empty explicit scopes mean
    /// no work, never every wallet. Failed work does not consume its cooldown.
    pub async fn refresh_history(
        &self,
        scope: HistoryRefreshScope,
        load_more: bool,
        limit: Option<u32>,
        interval_secs: f64,
    ) -> Result<Vec<ChainHistoryRefresh>, SpectraBridgeError> {
        self.bound_database().await?;
        if !interval_secs.is_finite() || interval_secs < 0.0 {
            return Err(SpectraBridgeError::InvalidInput {
                message: "history interval must be finite and nonnegative".into(),
            });
        }
        let state = self.app_state().await;
        let chains = match &scope {
            HistoryRefreshScope::Chains { chain_ids } => Some(
                chain_ids
                    .iter()
                    .map(|id| {
                        Chain::from_str_id(id).ok_or_else(|| SpectraBridgeError::InvalidInput {
                            message: format!("unknown chain {id}"),
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            _ => None,
        };
        let mut groups = std::collections::BTreeMap::<String, Vec<HistoryRefreshKey>>::new();
        for wallet in &state.wallets {
            let Some(chain) = Chain::from_display_name(&wallet.chain_name) else {
                continue;
            };
            let selected = match &scope {
                HistoryRefreshScope::All => true,
                HistoryRefreshScope::Wallets { wallet_ids } => wallet_ids
                    .iter()
                    .any(|id| id.eq_ignore_ascii_case(&wallet.id)),
                HistoryRefreshScope::Chains { .. } => chains
                    .as_ref()
                    .unwrap()
                    .iter()
                    .any(|c| *c == chain || Some(*c) == wallet.chain()),
            };
            if selected {
                groups
                    .entry(chain.str_id().into())
                    .or_default()
                    .push(HistoryRefreshKey::new(&wallet.id, &wallet.chain_id));
            }
        }
        drop(state);
        let mut results = Vec::new();
        for (chain_id, keys) in groups {
            let keys = if load_more {
                keys
            } else {
                self.history_refresh_plans(keys, interval_secs).await
            };
            let mut ids: Vec<_> = keys.iter().map(|key| key.wallet_id.clone()).collect();
            if load_more {
                ids.retain(|id| {
                    !self
                        .history_cursor(chain_id.clone(), id.clone())
                        .is_exhausted
                });
            }
            if ids.is_empty() {
                continue;
            }
            let chain = Chain::from_str_id(&chain_id).unwrap();
            let result = match chain.history_refresh_kind() {
                crate::registry::HistoryRefreshKind::Bitcoin => {
                    self.refresh_bitcoin_history(ids, load_more, limit).await
                }
                crate::registry::HistoryRefreshKind::Evm => {
                    self.refresh_evm_chain_history(chain_id.clone(), ids, load_more, limit)
                        .await
                }
                crate::registry::HistoryRefreshKind::Utxo => {
                    self.refresh_utxo_chain_history(chain_id.clone(), ids, load_more)
                        .await
                }
                crate::registry::HistoryRefreshKind::Normalized => {
                    self.refresh_chain_history(chain_id.clone(), ids).await
                }
            };
            self.record_history_run(chain, &result).await;
            match result {
                Ok(outcome) => {
                    if !load_more && outcome.wallets_failed == 0 && outcome.wallets_refreshed > 0 {
                        // Only a fully successful batch consumes its clocks. Partial
                        // failures remain immediately retryable.
                        for key in keys {
                            self.record_history_refresh(key).await;
                        }
                    }
                    results.push(ChainHistoryRefresh {
                        chain_id,
                        outcome: Some(outcome),
                        error: None,
                    });
                }
                Err(error) => results.push(ChainHistoryRefresh {
                    chain_id,
                    outcome: None,
                    error: Some(error.to_string()),
                }),
            }
        }
        Ok(results)
    }
}

impl WalletService {
    /// Write what a history run found where the diagnostics screen reads it:
    /// one row per wallet, and whether the chain is degraded or healthy.
    ///
    /// The app did this from the result of the call — copying each row back
    /// through `diagnostics_record` and deciding health itself — on the two
    /// paths it drove, and nothing did it on the scheduled refresh core runs.
    /// A diagnostics write failing does not fail the refresh it describes.
    pub(crate) async fn record_history_run(
        &self,
        chain: Chain,
        result: &Result<HistoryRefreshOutcome, SpectraBridgeError>,
    ) {
        use crate::service::DiagnosticCommand;
        let chain_name = chain.chain_display_name().to_string();
        let command = match result {
            Ok(outcome) => {
                for row in &outcome.diagnostics {
                    crate::diagnostics::diagnostics_record(
                        chain_name.clone(),
                        crate::diagnostics::HistoryDiagnostics {
                            wallet_id: row.wallet_id.clone(),
                            identifier: row.identifier.clone(),
                            source_used: row.source_used.clone(),
                            transaction_count: i32::try_from(row.transaction_count)
                                .unwrap_or(i32::MAX),
                            scanned_count: None,
                            next_cursor: row.next_cursor.clone(),
                            error: row.error.clone(),
                            per_source: Vec::new(),
                        },
                    );
                }
                // English on purpose: this is stored, and the templates are
                // what the diagnostics screen localizes when it shows them.
                if outcome.wallets_failed > 0 {
                    let detail = if outcome.wallets_refreshed == 0 {
                        format!("{chain_name} history refresh failed. Using cached history.")
                    } else {
                        format!("{chain_name} history loaded with partial provider failures.")
                    };
                    Some(DiagnosticCommand::Degraded { chain_name, detail })
                } else if outcome.wallets_refreshed > 0 {
                    Some(DiagnosticCommand::Healthy { chain_name })
                } else {
                    None
                }
            }
            Err(error) => Some(DiagnosticCommand::Degraded {
                chain_name,
                detail: error.to_string(),
            }),
        };
        if let Some(command) = command {
            if let Err(error) = self.apply_diagnostic_command(command).await {
                tracing::warn!(%error, "history diagnostics were not recorded");
            }
        }
    }
}
