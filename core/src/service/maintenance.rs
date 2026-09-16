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
                .find(|h| h.deployment_key() == holding_key)
        });
        let request = crate::send::SendSubmitPreflightRequest {
            wallet_found: wallet.is_some(),
            asset_found: holding.is_some(),
            destination_address,
            amount_input,
            available_balance: holding.map(|h| h.amount).unwrap_or(0.0),
            asset: holding.map(|holding| routing_input(holding, &state.token_preferences)),
            token: holding
                .and_then(|holding| send_token_identity(holding, &state.token_preferences)),
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
            .find(|h| h.deployment_key() == holding_key)?;
        Some(crate::send::route_send_asset(&routing_input(
            holding,
            &state.token_preferences,
        )))
    }
}

impl WalletService {
    /// Stamp the clock. Called once a refresh has actually run, so the next
    /// plan measures from when the work happened rather than when it was asked
    /// for.
    pub async fn record_refresh(&self, kind: RefreshKind) {
        let now = crate::store::wallet_db::now_secs() as f64;
        self.refresh_clock.write().await.record(kind, now);
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
            .find(|h| h.deployment_key() == holding_key)
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
                    .find(|h| h.deployment_key() == holding_key)
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
        self.pending_maintenance_chains()
            .await
            .is_ok_and(|chains| !chains.is_empty())
    }
}

/// The known EVM token a holding is, as `(symbol, contract)`.
///
/// `None` for a chain's own gas asset — a chain's native asset is never one of
/// its tokens — and for a token the user does not track.
fn supported_evm_token(
    holding: &crate::store::wallet_domain::AssetHolding,
    preferences: &[crate::store::wallet_domain::CoreTokenPreferenceEntry],
) -> Option<(String, String)> {
    let chain = crate::registry::Chain::from_display_name(&holding.chain_name)?;
    if !chain.is_evm() || holding.is_native() {
        return None;
    }
    preferences
        .iter()
        .find(|entry| entry.is_enabled && entry.token.matches_holding(holding))
        .map(|entry| (entry.token.symbol.clone(), entry.token.contract.clone()))
}

/// What routing needs to know about a holding, derived from core's own state.
/// The token a holding is, with the decimals the catalog gives it.
///
/// A holding whose symbol is the chain's gas asset is native and has none. A
/// token the user does not track has none either, which is what makes the
/// send refuse rather than guess a scale — every submit branch used to look
/// this up itself, and Tron's branch hard-coded six decimals for every token
/// on it.
pub(super) fn send_token_identity(
    holding: &crate::store::wallet_domain::AssetHolding,
    preferences: &[crate::store::wallet_domain::CoreTokenPreferenceEntry],
) -> Option<crate::send::SendTokenIdentity> {
    crate::registry::Chain::from_display_name(&holding.chain_name)?;
    if holding.is_native() {
        return None;
    }
    preferences
        .iter()
        .find(|entry| entry.is_enabled && entry.token.matches_holding(holding))
        .map(|entry| crate::send::SendTokenIdentity {
            contract: entry.token.contract.clone(),
            decimals: entry.token.decimals,
        })
}

fn routing_input(
    holding: &crate::store::wallet_domain::AssetHolding,
    preferences: &[crate::store::wallet_domain::CoreTokenPreferenceEntry],
) -> crate::send::SendAssetRoutingInput {
    crate::send::SendAssetRoutingInput {
        is_native: holding.is_native(),
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
    holding: &crate::store::wallet_domain::AssetHolding,
    preferences: &[crate::store::wallet_domain::CoreTokenPreferenceEntry],
) -> bool {
    use crate::store::wallet_domain::CoreTokenHostingChain;
    let chain = crate::registry::Chain::Solana;
    if holding.chain_name != chain.chain_display_name() {
        return false;
    }
    if holding.is_native() {
        return true;
    }
    if holding.token_standard != token_standard_for(CoreTokenHostingChain::Solana) {
        return false;
    }
    let Some(mint) = holding.contract_address.clone().filter(|c| !c.is_empty()) else {
        return false;
    };
    preferences.iter().any(|entry| {
        entry.hosting_chain() == Some(CoreTokenHostingChain::Solana) && entry.token.contract == mint
    })
}

/// Whether a NEAR holding is a token this build can send. NEAR itself is not:
/// the native path handles it.
fn supports_near_token_send(
    holding: &crate::store::wallet_domain::AssetHolding,
    preferences: &[crate::store::wallet_domain::CoreTokenPreferenceEntry],
) -> bool {
    use crate::store::wallet_domain::CoreTokenHostingChain;
    let chain = crate::registry::Chain::Near;
    if holding.chain_name != chain.chain_display_name() || holding.is_native() {
        return false;
    }
    if holding.token_standard != token_standard_for(CoreTokenHostingChain::Near) {
        return false;
    }
    let Some(contract) = holding
        .contract_address
        .as_deref()
        .filter(|c| !c.is_empty())
    else {
        return false;
    };
    preferences.iter().any(|entry| {
        entry.hosting_chain() == Some(CoreTokenHostingChain::Near)
            && entry.token.contract.eq_ignore_ascii_case(contract)
    })
}

/// The catalog's token standard for a chain, e.g. `SPL Token` for Solana.
fn token_standard_for(chain: crate::store::wallet_domain::CoreTokenHostingChain) -> String {
    chain.token_standard()
}

#[cfg(test)]
mod preflight_tests {
    use super::*;
    use crate::store::state::WalletState;
    use crate::store::wallet_domain::AssetHolding;
    use crate::store::wallet_domain::{
        CoreTokenHostingChain, CoreTokenPreferenceCategory, CoreTokenPreferenceEntry,
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

    fn known(chain: CoreTokenHostingChain, contract: &str) -> CoreTokenPreferenceEntry {
        CoreTokenPreferenceEntry {
            category: CoreTokenPreferenceCategory::Stablecoin,
            is_built_in: false,
            is_enabled: true,
            token: crate::tokens::TokenEntry {
                id: "fixture:token".into(),
                token_id: "fixture:token".into(),
                kind: crate::tokens::TokenKind::Protocol {
                    standard: "fixture".into(),
                    identifier: "fixture".into(),
                },
                chain: chain.chain_name().to_string(),
                name: "Token".into(),
                symbol: "TOK".into(),
                token_standard: String::new(),
                contract: contract.to_string(),
                coingecko_id: String::new(),
                decimals: 6,
                tags: Vec::new(),
                color: None,
                artwork_name: String::new(),
                enabled: true,
            },
        }
    }

    /// The two send-support rules, against core's own token list.
    ///
    /// They were iOS predicates whose answers were passed *in* to the
    /// preflight — so on the funds path core took a caller's word about which
    /// assets core itself knows how to send.
    #[test]
    fn solana_sends_sol_always_and_a_token_only_when_known() {
        let sol = holding("Solana", "SOL", "Native", None);
        assert!(supports_solana_send(&sol, &[]));

        let standard = token_standard_for(CoreTokenHostingChain::Solana);
        let mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        let usdc = holding("Solana", "USDC", &standard, Some(mint));
        assert!(!supports_solana_send(&usdc, &[]), "unknown mint");
        assert!(supports_solana_send(
            &usdc,
            &[known(CoreTokenHostingChain::Solana, mint)]
        ));

        let wrong_standard = holding("Solana", "USDC", "ERC-20", Some(mint));
        assert!(!supports_solana_send(
            &wrong_standard,
            &[known(CoreTokenHostingChain::Solana, mint)]
        ));
    }

    #[test]
    fn near_sends_known_tokens_but_not_near_itself() {
        let standard = token_standard_for(CoreTokenHostingChain::Near);
        let native = holding("NEAR", "NEAR", &standard, Some("wrap.near"));
        assert!(
            !supports_near_token_send(&native, &[]),
            "native is not a token send"
        );

        let token = holding("NEAR", "USDC", &standard, Some("usdc.near"));
        assert!(!supports_near_token_send(&token, &[]));
        assert!(
            supports_near_token_send(&token, &[known(CoreTokenHostingChain::Near, "USDC.NEAR")]),
            "contract matching is case-insensitive"
        );
    }

    /// Knowing a mint is what makes a Solana token routable — through the
    /// service, from core's own token list.
    ///
    /// The preview path and the submit path both asked this on their own side
    /// before, from a Swift copy of the rule. Three answers to one question is
    /// three chances to disagree about whether a send can be made.
    #[tokio::test]
    async fn routing_follows_the_token_list_core_holds() {
        let service = WalletService::new(Vec::new()).expect("service");
        let mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        let standard = token_standard_for(CoreTokenHostingChain::Solana);
        {
            let mut state = service.wallet_state.write().await;
            let mut wallet =
                WalletState::single_address("w1", "W", "Solana", "SoLaddr", None, false);
            wallet.holdings = vec![holding("Solana", "USDC", &standard, Some(mint))];
            state.wallets.push(wallet);
        }

        let untracked = service
            .send_asset_routing(
                "w1".into(),
                "solana:spl:EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".into(),
            )
            .await
            .expect("the holding is there");
        assert_eq!(
            untracked.submit_kind, None,
            "an untracked mint is not sendable"
        );

        service.wallet_state.write().await.token_preferences =
            vec![known(CoreTokenHostingChain::Solana, mint)];
        let known_now = service
            .send_asset_routing(
                "w1".into(),
                "solana:spl:EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".into(),
            )
            .await
            .expect("the holding is there");
        assert_eq!(known_now.submit_kind.as_deref(), Some("solana"));
        assert_eq!(
            known_now.preview_kind, known_now.submit_kind,
            "the preview and the submit are one decision"
        );
    }

    /// A wallet or holding core cannot find is refused, not guessed at.
    #[tokio::test]
    async fn a_send_for_an_unknown_wallet_is_refused() {
        let service = WalletService::new(Vec::new()).expect("service");
        let err = service
            .send_submit_preflight(
                "nope".into(),
                "bitcoin:native".into(),
                "bc1qexample".into(),
                "1".into(),
            )
            .await;
        assert!(err.is_err(), "no wallet, no send");
    }

    #[tokio::test]
    async fn a_send_resolves_its_holding_by_chain_and_symbol() {
        let service = WalletService::new(Vec::new()).expect("service");
        {
            let mut state = service.wallet_state.write().await;
            let mut wallet =
                WalletState::single_address("w1", "W", "Bitcoin", "bc1qowner", None, false);
            wallet.holdings = vec![holding("Bitcoin", "BTC", "Native", None)];
            state.wallets.push(wallet);
        }
        let plan = service
            .send_submit_preflight(
                "w1".into(),
                "bitcoin:native".into(),
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

#[cfg(test)]
mod send_token_identity_tests {
    use super::send_token_identity;
    use crate::store::wallet_domain::{
        AssetHolding, CoreTokenPreferenceCategory, CoreTokenPreferenceEntry,
    };

    fn entry(chain: &str, symbol: &str, contract: &str, decimals: u32) -> CoreTokenPreferenceEntry {
        CoreTokenPreferenceEntry {
            token: crate::tokens::TokenEntry {
                id: "fixture:token".into(),
                token_id: "fixture:token".into(),
                kind: crate::tokens::TokenKind::Protocol {
                    standard: "fixture".into(),
                    identifier: "fixture".into(),
                },
                chain: chain.to_string(),
                name: symbol.to_string(),
                symbol: symbol.to_string(),
                token_standard: "trc20".to_string(),
                contract: contract.to_string(),
                coingecko_id: String::new(),
                decimals,
                tags: Vec::new(),
                color: None,
                artwork_name: String::new(),
                enabled: true,
            },
            category: CoreTokenPreferenceCategory::Stablecoin,
            is_built_in: true,
            is_enabled: true,
        }
    }

    fn holding(chain: &str, symbol: &str, contract: Option<&str>) -> AssetHolding {
        AssetHolding {
            name: symbol.to_string(),
            symbol: symbol.to_string(),
            coin_gecko_id: String::new(),
            chain_name: chain.to_string(),
            token_standard: "trc20".to_string(),
            contract_address: contract.map(str::to_string),
            amount: 1.0,
            price_usd: 0.0,
        }
    }

    /// A token carries its own decimals, and a native asset carries none.
    ///
    /// Every submit branch resolved this itself. Tron's hard-coded six for
    /// every token on it — right for USDT, wrong for the four
    /// eighteen-decimal ones — and NEAR's fell back to six for a token it
    /// could not find, which is a scale guessed on the funds path.
    #[test]
    fn a_token_carries_its_own_decimals_and_a_native_asset_none() {
        let preferences = vec![
            entry("tron", "USDT", "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t", 6),
            entry("tron", "USDD", "TPYmHEhy5n8TCEfYGqW2rPxsghSfzghPDn", 18),
        ];

        // The native asset is not a token.
        assert!(send_token_identity(&holding("Tron", "TRX", None), &preferences).is_none());

        // Each token's own scale, matched by contract.
        let usdd = send_token_identity(
            &holding("Tron", "USDD", Some("TPYmHEhy5n8TCEfYGqW2rPxsghSfzghPDn")),
            &preferences,
        )
        .expect("USDD is tracked");
        assert_eq!(usdd.decimals, 18);
        let usdt = send_token_identity(
            &holding("Tron", "USDT", Some("TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t")),
            &preferences,
        )
        .expect("USDT is tracked");
        assert_eq!(usdt.decimals, 6);

        // A ticker is not sufficient identity for a protocol token.
        assert!(send_token_identity(&holding("Tron", "USDD", None), &preferences).is_none());

        // A token nothing tracks has no identity, so the send refuses rather
        // than guessing a scale.
        assert!(send_token_identity(&holding("Tron", "NOPE", None), &preferences).is_none());
        assert!(send_token_identity(
            &holding("Tron", "USDT", Some("TSomeOtherContractAddressEntirely")),
            &preferences
        )
        .is_none());
        // And a chain that hosts no known tokens has none either.
        assert!(send_token_identity(&holding("Monero", "XMR", None), &preferences).is_none());
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
        let eth = crate::fetch::refresh::policy::HistoryRefreshKey::new("w", "ethereum");
        let btc = crate::fetch::refresh::policy::HistoryRefreshKey::new("w", "bitcoin");
        service.record_history_refresh(eth.clone()).await;
        let due = service
            .history_refresh_plans(vec![eth, btc.clone()], 120.0)
            .await;
        assert_eq!(due, vec![btc]);
    }
}

impl WalletService {
    pub(crate) async fn history_refresh_plans(
        &self,
        keys: Vec<crate::fetch::refresh::policy::HistoryRefreshKey>,
        interval_secs: f64,
    ) -> Vec<crate::fetch::refresh::policy::HistoryRefreshKey> {
        let clock = self.refresh_clock.read().await;
        crate::fetch::refresh::policy::history_plans(
            &clock,
            keys,
            interval_secs,
            crate::store::wallet_db::now_secs() as f64,
        )
    }

    pub(crate) async fn record_history_refresh(
        &self,
        key: crate::fetch::refresh::policy::HistoryRefreshKey,
    ) {
        let now = crate::store::wallet_db::now_secs() as f64;
        self.refresh_clock.write().await.record_history(key, now);
    }
}
