//! When the app should refresh, and how hard.
//!
//! Core holds the clock now. It used to be five `Date?` properties and two
//! dictionaries on `AppState`, handed back as arguments on every question —
//! `core_active_maintenance_plan`, `core_should_run_background_maintenance`,
//! `evaluate_heavy_refresh_gate`, `compute_background_maintenance_interval`
//! and `active_pending_refresh_interval_for_profile` were five exports that
//! together answered one: what should happen this tick. The intervals they
//! needed are settings core owns, and the only inputs core genuinely lacks are
//! the device's — reachability, power, and whether a screen showing prices is
//! in front of the user.

use crate::fetch::refresh::policy::{DeviceConditions, MaintenancePlan, RefreshKind};
use crate::service::WalletService;
use crate::SpectraBridgeError;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// What to do this tick, and how long to wait for the next one.
    pub async fn maintenance_plan(&self, conditions: DeviceConditions) -> MaintenancePlan {
        let settings = self.wallet_state.read().await.settings.clone();
        let has_pending_work = self.has_pending_transaction_work().await;
        let clock = self.refresh_clock.read().await.clone();
        crate::fetch::refresh::policy::maintenance_plan(
            &clock,
            &settings,
            &conditions,
            has_pending_work,
            crate::store::wallet_db::now_secs() as f64,
        )
    }

    /// Stamp the clock. Called once a refresh has actually run, so the next
    /// plan measures from when the work happened rather than when it was asked
    /// for.
    pub async fn record_refresh(&self, kind: RefreshKind) {
        let now = crate::store::wallet_db::now_secs() as f64;
        self.refresh_clock.write().await.record(kind, now);
    }

    /// Which of these chains are due a history refresh.
    ///
    /// The caller used to pass its own copy of "when did each chain's history
    /// last refresh"; core keeps it, so the answer cannot be stale by the age
    /// of the caller's dictionary.
    pub async fn history_refresh_plans(
        &self,
        chain_ids: Vec<String>,
        interval_secs: f64,
    ) -> Vec<String> {
        let clock = self.refresh_clock.read().await;
        crate::fetch::refresh::policy::history_plans(
            &clock,
            chain_ids,
            interval_secs,
            crate::store::wallet_db::now_secs() as f64,
        )
    }

    /// Stamp one chain's history clock.
    pub async fn record_history_refresh(&self, chain_id: String) {
        let now = crate::store::wallet_db::now_secs() as f64;
        self.refresh_clock
            .write()
            .await
            .record_history(chain_id, now);
    }

    /// Can this send be made, and how should it be routed?
    pub async fn send_submit_preflight(
        &self,
        wallet_id: String,
        holding_key: String,
        destination_address: String,
        amount_input: String,
    ) -> Result<crate::send::SendSubmitPreflightPlan, SpectraBridgeError> {
        let state = self.wallet_state.read().await;
        let wallet = state.wallets.iter().find(|w| w.id == wallet_id);
        let holding = wallet.and_then(|wallet| {
            wallet
                .holdings
                .iter()
                .find(|h| format!("{}|{}", h.chain_name, h.symbol) == holding_key)
        });
        let request = crate::send::SendSubmitPreflightRequest {
            wallet_found: wallet.is_some(),
            asset_found: holding.is_some(),
            destination_address,
            amount_input,
            available_balance: holding.map(|h| h.amount).unwrap_or(0.0),
            asset: holding.map(|holding| routing_input(holding, &state.token_preferences)),
        };
        Ok(crate::send::plan_send_submit_preflight(request)?)
    }

    /// How a holding's send and preview are routed.
    ///
    /// The same derivation the preflight does, for the callers that only want
    /// the routing: which preview to refresh, and which submit branch to take.
    /// Both used to be decided again on the caller's side — the preview from a
    /// `SendAssetRoutingInput` it assembled, the Solana submit branch from its
    /// own copy of the send-support rule — so three places could disagree
    /// about whether an asset is sendable.
    pub async fn send_asset_routing(
        &self,
        wallet_id: String,
        holding_key: String,
    ) -> Option<crate::send::SendAssetRoutingPlan> {
        let state = self.wallet_state.read().await;
        let holding = state
            .wallets
            .iter()
            .find(|w| w.id == wallet_id)?
            .holdings
            .iter()
            .find(|h| format!("{}|{}", h.chain_name, h.symbol) == holding_key)?;
        Some(crate::send::route_send_asset(&routing_input(
            holding,
            &state.token_preferences,
        )))
    }

    /// Reasons this send looks risky, as codes the platform localizes.
    pub async fn high_risk_send_reasons(
        &self,
        wallet_id: String,
        holding_key: String,
        amount: f64,
        destination_address: String,
        destination_input: String,
        used_ens_resolution: bool,
    ) -> Vec<crate::send::flow::HighRiskSendWarning> {
        let state = self.wallet_state.read().await;
        let Some(wallet) = state.wallets.iter().find(|w| w.id == wallet_id) else {
            return Vec::new();
        };
        let Some(holding) = wallet
            .holdings
            .iter()
            .find(|h| format!("{}|{}", h.chain_name, h.symbol) == holding_key)
        else {
            return Vec::new();
        };
        let chain_name = holding.chain_name.clone();
        let symbol = holding.symbol.clone();
        let holding_amount = holding.amount;
        let wallet_selected_chain = wallet.chain_name.clone();
        let address_book_entries: Vec<_> = state
            .address_book
            .iter()
            .map(|entry| crate::send::flow::HighRiskChainAddress {
                chain_name: entry.chain_name.clone(),
                address: entry.address.clone(),
            })
            .collect();
        drop(state);

        // Addresses this wallet has already sent to on this chain. A first-time
        // destination is one of the risk signals, so reading a caller's copy of
        // the history meant the signal was only as complete as that copy.
        let mut seen: std::collections::BTreeSet<String> = Default::default();
        if let Ok(rows) = self.fetch_all_history_records_typed().await {
            for row in rows {
                if row.payload.chain_name == chain_name {
                    seen.insert(row.payload.address.clone());
                }
            }
        }

        crate::send::flow::core_evaluate_high_risk_send_reasons(
            crate::send::flow::HighRiskSendRequest {
                chain_name: chain_name.clone(),
                symbol,
                amount,
                holding_amount,
                destination_address,
                destination_input,
                used_ens_resolution,
                wallet_selected_chain,
                address_book_entries,
                tx_addresses: seen
                    .into_iter()
                    .map(|address| crate::send::flow::HighRiskChainAddress {
                        chain_name: chain_name.clone(),
                        address,
                    })
                    .collect(),
            },
        )
    }

    /// Warnings about an EVM recipient, as codes the platform localizes.
    ///
    /// Core makes the two contract-code probes itself. They were already its
    /// own network calls — the caller made them, caught their errors, worked
    /// out which token the holding is from core's token list, and handed the
    /// three answers back for core to turn into warnings.
    pub async fn evm_recipient_preflight(
        &self,
        wallet_id: String,
        holding_key: String,
        destination_address: String,
    ) -> Vec<crate::store::EvmRecipientPreflightWarning> {
        let state = self.wallet_state.read().await;
        let Some(holding) = state
            .wallets
            .iter()
            .find(|w| w.id == wallet_id)
            .and_then(|wallet| {
                wallet
                    .holdings
                    .iter()
                    .find(|h| format!("{}|{}", h.chain_name, h.symbol) == holding_key)
            })
        else {
            return Vec::new();
        };
        let Some(chain) = crate::registry::Chain::from_display_name(&holding.chain_name)
            .filter(|chain| chain.is_evm())
        else {
            return Vec::new();
        };
        let chain_name = holding.chain_name.clone();
        let holding_symbol = holding.symbol.clone();
        let token = supported_evm_token(holding, &state.token_preferences);
        drop(state);

        // A probe that fails is `None`, not `false`: "we could not check" and
        // "it is not a contract" are different answers, and the evaluator
        // treats them differently.
        let chain_id = chain.str_id().to_string();
        let recipient_has_code = self
            .fetch_evm_has_contract_code(chain_id.clone(), destination_address)
            .await
            .ok();
        let token_has_code = match &token {
            Some((_, contract)) => self
                .fetch_evm_has_contract_code(chain_id, contract.clone())
                .await
                .ok(),
            None => None,
        };
        crate::store::core_evm_recipient_preflight_warnings(
            crate::store::EvmRecipientPreflightRequest {
                chain_name,
                holding_symbol,
                token_symbol: token.map(|(symbol, _)| symbol),
                recipient_has_code,
                token_has_code,
            },
        )
    }
}

// Plain `impl`, not exported. The block above is the boundary; a private
// helper written into it becomes an entry point, which is how
// `pinned_prototype` shipped as one.
impl WalletService {
    /// Whether any recorded send is still worth polling for confirmation.
    ///
    /// Read from core's own store. iOS derived this from its transaction
    /// projection and passed the answer in, which is the shape the migration
    /// removes: core has the transactions.
    async fn has_pending_transaction_work(&self) -> bool {
        let Ok(rows) = self.fetch_all_history_records_typed().await else {
            return false;
        };
        rows.iter().any(|row| {
            let payload = &row.payload;
            matches!(payload.kind, crate::store::wallet_domain::CoreTransactionKind::Send)
                && payload.transaction_hash.is_some()
                && matches!(
                    payload.status,
                    Some(crate::store::wallet_domain::CoreTransactionStatus::Pending)
                        | Some(crate::store::wallet_domain::CoreTransactionStatus::Confirmed)
                )
        })
    }
}

/// The tracked EVM token a holding is, as `(symbol, contract)`.
///
/// `None` for a chain's own gas asset — a chain's native asset is never one of
/// its tokens — and for a token the user does not track.
fn supported_evm_token(
    holding: &crate::store::state::AssetHolding,
    preferences: &[crate::store::wallet_domain::CoreTokenPreferenceEntry],
) -> Option<(String, String)> {
    use crate::store::wallet_domain::CoreTokenTrackingChain;
    let chain = crate::registry::Chain::from_display_name(&holding.chain_name)?;
    if !chain.is_evm() || chain.entry().gas_token_symbol == holding.symbol {
        return None;
    }
    let tracking = CoreTokenTrackingChain::from_chain_name(&holding.chain_name)?;
    let contract = holding.contract_address.as_deref().map(str::to_lowercase);
    preferences
        .iter()
        .find(|entry| {
            entry.chain == tracking
                && entry.is_enabled
                && entry.symbol == holding.symbol
                && contract
                    .as_deref()
                    .is_none_or(|c| entry.contract_address.to_lowercase() == c)
        })
        .map(|entry| (entry.symbol.clone(), entry.contract_address.clone()))
}

/// What routing needs to know about a holding, derived from core's own state.
fn routing_input(
    holding: &crate::store::state::AssetHolding,
    preferences: &[crate::store::wallet_domain::CoreTokenPreferenceEntry],
) -> crate::send::SendAssetRoutingInput {
    crate::send::SendAssetRoutingInput {
        chain_name: holding.chain_name.clone(),
        symbol: holding.symbol.clone(),
        is_evm_chain: crate::registry::Chain::from_display_name(&holding.chain_name)
            .is_some_and(|chain| chain.is_evm()),
        supports_solana_send_coin: supports_solana_send(holding, preferences),
        supports_near_token_send: supports_near_token_send(holding, preferences),
    }
}

/// Whether a Solana holding is one this build can send.
///
/// SOL always; a token only if the user tracks its mint. The mint comes from
/// the holding's contract, or from the catalog by symbol when the holding does
/// not carry one.
fn supports_solana_send(
    holding: &crate::store::state::AssetHolding,
    preferences: &[crate::store::wallet_domain::CoreTokenPreferenceEntry],
) -> bool {
    use crate::store::wallet_domain::CoreTokenTrackingChain;
    if holding.chain_name != "Solana" {
        return false;
    }
    if holding.symbol == "SOL" {
        return true;
    }
    if holding.token_standard != token_standard_for(CoreTokenTrackingChain::Solana) {
        return false;
    }
    let Some(mint) = holding
        .contract_address
        .clone()
        .filter(|c| !c.is_empty())
        .or_else(|| {
            crate::tokens::catalog()
                .iter()
                .find(|token| {
                    token.chain == "solana" && token.symbol.eq_ignore_ascii_case(&holding.symbol)
                })
                .map(|token| token.contract.clone())
        })
    else {
        return false;
    };
    preferences
        .iter()
        .any(|entry| entry.chain == CoreTokenTrackingChain::Solana && entry.contract_address == mint)
}

/// Whether a NEAR holding is a token this build can send. NEAR itself is not:
/// the native path handles it.
fn supports_near_token_send(
    holding: &crate::store::state::AssetHolding,
    preferences: &[crate::store::wallet_domain::CoreTokenPreferenceEntry],
) -> bool {
    use crate::store::wallet_domain::CoreTokenTrackingChain;
    if holding.chain_name != "NEAR" || holding.symbol == "NEAR" {
        return false;
    }
    if holding.token_standard != token_standard_for(CoreTokenTrackingChain::Near) {
        return false;
    }
    let Some(contract) = holding.contract_address.as_deref().filter(|c| !c.is_empty()) else {
        return false;
    };
    preferences.iter().any(|entry| {
        entry.chain == CoreTokenTrackingChain::Near
            && entry.contract_address.eq_ignore_ascii_case(contract)
    })
}

/// The catalog's token standard for a chain, e.g. `SPL Token` for Solana.
fn token_standard_for(chain: crate::store::wallet_domain::CoreTokenTrackingChain) -> String {
    let name = chain.chain_name();
    crate::registry::Chain::from_display_name(name)
        .map(|chain| chain.entry().token_standard.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod preflight_tests {
    use super::*;
    use crate::store::state::{AssetHolding, WalletSummary};
    use crate::store::wallet_domain::{
        CoreTokenPreferenceCategory, CoreTokenPreferenceEntry, CoreTokenTrackingChain,
    };

    fn holding(chain: &str, symbol: &str, standard: &str, contract: Option<&str>) -> AssetHolding {
        AssetHolding {
            name: symbol.to_string(),
            symbol: symbol.to_string(),
            coin_gecko_id: String::new(),
            chain_name: chain.to_string(),
            token_standard: standard.to_string(),
            contract_address: contract.map(str::to_string),
            amount: 10.0,
            price_usd: 1.0,
        }
    }

    fn tracked(chain: CoreTokenTrackingChain, contract: &str) -> CoreTokenPreferenceEntry {
        CoreTokenPreferenceEntry {
            id: format!("t:{contract}"),
            chain,
            name: "Token".into(),
            symbol: "TOK".into(),
            token_standard: String::new(),
            contract_address: contract.to_string(),
            coin_gecko_id: String::new(),
            decimals: 6,
            display_decimals: None,
            category: CoreTokenPreferenceCategory::Stablecoin,
            is_built_in: false,
            is_enabled: true,
        }
    }

    /// The two send-support rules, against core's own token list.
    ///
    /// They were iOS predicates whose answers were passed *in* to the
    /// preflight — so on the funds path core took a caller's word about which
    /// assets core itself knows how to send.
    #[test]
    fn solana_sends_sol_always_and_a_token_only_when_tracked() {
        let sol = holding("Solana", "SOL", "", None);
        assert!(supports_solana_send(&sol, &[]));

        let standard = token_standard_for(CoreTokenTrackingChain::Solana);
        let mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        let usdc = holding("Solana", "USDC", &standard, Some(mint));
        assert!(!supports_solana_send(&usdc, &[]), "untracked mint");
        assert!(supports_solana_send(
            &usdc,
            &[tracked(CoreTokenTrackingChain::Solana, mint)]
        ));

        let wrong_standard = holding("Solana", "USDC", "ERC-20", Some(mint));
        assert!(!supports_solana_send(
            &wrong_standard,
            &[tracked(CoreTokenTrackingChain::Solana, mint)]
        ));
    }

    #[test]
    fn near_sends_tracked_tokens_but_not_near_itself() {
        let standard = token_standard_for(CoreTokenTrackingChain::Near);
        let native = holding("NEAR", "NEAR", &standard, Some("wrap.near"));
        assert!(!supports_near_token_send(&native, &[]), "native is not a token send");

        let token = holding("NEAR", "USDC", &standard, Some("usdc.near"));
        assert!(!supports_near_token_send(&token, &[]));
        assert!(supports_near_token_send(
            &token,
            &[tracked(CoreTokenTrackingChain::Near, "USDC.NEAR")]
        ), "contract matching is case-insensitive");
    }

    /// Tracking a mint is what makes a Solana token routable — through the
    /// service, from core's own token list.
    ///
    /// The preview path and the submit path both asked this on their own side
    /// before, from a Swift copy of the rule. Three answers to one question is
    /// three chances to disagree about whether a send can be made.
    #[tokio::test]
    async fn routing_follows_the_token_list_core_holds() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        let standard = token_standard_for(CoreTokenTrackingChain::Solana);
        {
            let mut state = service.wallet_state.write().await;
            let mut wallet =
                WalletSummary::single_address("w1", "W", "Solana", "SoLaddr", None, false);
            wallet.holdings = vec![holding("Solana", "USDC", &standard, Some(mint))];
            state.wallets.push(wallet);
        }

        let untracked = service
            .send_asset_routing("w1".into(), "Solana|USDC".into())
            .await
            .expect("the holding is there");
        assert_eq!(untracked.submit_kind, None, "an untracked mint is not sendable");

        service.wallet_state.write().await.token_preferences =
            vec![tracked(CoreTokenTrackingChain::Solana, mint)];
        let tracked_now = service
            .send_asset_routing("w1".into(), "Solana|USDC".into())
            .await
            .expect("the holding is there");
        assert_eq!(tracked_now.submit_kind.as_deref(), Some("solana"));
        assert_eq!(
            tracked_now.preview_kind, tracked_now.submit_kind,
            "the preview and the submit are one decision"
        );
    }

    /// A wallet or holding core cannot find is refused, not guessed at.
    #[tokio::test]
    async fn a_send_for_an_unknown_wallet_is_refused() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let err = service
            .send_submit_preflight(
                "nope".into(),
                "Bitcoin|BTC".into(),
                "bc1qexample".into(),
                "1".into(),
            )
            .await;
        assert!(err.is_err(), "no wallet, no send");
    }

    #[tokio::test]
    async fn a_send_resolves_its_holding_by_chain_and_symbol() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        {
            let mut state = service.wallet_state.write().await;
            let mut wallet =
                WalletSummary::single_address("w1", "W", "Bitcoin", "bc1qowner", None, false);
            wallet.holdings = vec![holding("Bitcoin", "BTC", "", None)];
            state.wallets.push(wallet);
        }
        let plan = service
            .send_submit_preflight(
                "w1".into(),
                "Bitcoin|BTC".into(),
                "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4".into(),
                "1".into(),
            )
            .await
            .expect("a known wallet and holding");
        assert_eq!(plan.chain_name, "Bitcoin");
        assert_eq!(plan.symbol, "BTC");
        assert_eq!(plan.amount, 1.0);
    }
}
