//! Resident app-state projections and the serialized persistence writer.
//! Keypool, discovery, transactions, imports and events live in sibling modules.

use super::*;

/// The service owns its database handle; storage operations clone this handle.
#[derive(Default)]
pub struct StateBinding {
    bound: AsyncRwLock<Option<Arc<crate::wallet_db::WalletDatabase>>>,
}
impl StateBinding {
    pub(crate) async fn bind(&self, connection: Arc<crate::wallet_db::WalletDatabase>) {
        *self.bound.write().await = Some(connection);
    }
    pub(crate) async fn connection(&self) -> Option<Arc<crate::wallet_db::WalletDatabase>> {
        self.bound.read().await.clone()
    }
    pub(crate) async fn is_bound_to(&self, path: &str) -> bool {
        self.bound
            .read()
            .await
            .as_ref()
            .is_some_and(|db| db.path() == path)
    }
    pub(crate) async fn required_connection(
        &self,
    ) -> Result<Arc<crate::wallet_db::WalletDatabase>, SpectraBridgeError> {
        self.connection()
            .await
            .ok_or_else(|| "transaction store not opened: call open_state first".into())
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    // ── Owned application state ───────────────────────────────────────────
    //
    // `CoreAppState` is the domain state, and this service owns it. Front ends
    // send a `StateCommand` and receive the resulting state; they do not keep
    // their own copy and mutate it.
    //
    // `open_state` binds a database path, after which every accepted command is
    // persisted before it returns. Callers therefore cannot forget to save,
    // which is how two copies of the truth start diverging.

    /// Bind the service to its state database and load what is stored there.
    ///
    /// An untouched database yields `CoreAppState::default()`. Call once at
    /// startup; the returned state is the caller's initial snapshot.
    pub async fn open_state(
        &self,
        database_path: String,
    ) -> Result<CoreAppState, SpectraBridgeError> {
        self.write_persisted(move |service| async move {
            // Opening is idempotent. A second call with the same database returns
            // what is already held rather than re-reading — a late `open_state`
            // (the app's launch reload racing a user action) would otherwise
            // replace the in-memory state with a snapshot taken before the newer
            // command, silently reverting it.
            if service.state_binding.is_bound_to(&database_path).await {
                return Ok(service.wallet_state.read().await.clone());
            }

            let database = crate::wallet_db::WalletDatabase::new(&database_path);
            let source = database.clone();
            let (loaded, keypool, owned) = tokio::task::spawn_blocking(move || {
                let loaded = crate::wallet_db::app_state_load(&source)?;
                let keypool = crate::wallet_db::keypool_load_all(&source)?;
                let owned = crate::wallet_db::address_load_all_chains(&source)?;
                let is_new = source.with_connection(|conn| {
                    conn.query_row(
                        "SELECT NOT EXISTS(SELECT 1 FROM app_state_meta)",
                        [],
                        |row| row.get::<_, bool>(0),
                    )
                    .map_err(|e| e.to_string())
                })?;
                if is_new {
                    crate::wallet_db::app_state_save(&source, &loaded)?;
                }
                Ok::<_, String>((loaded, keypool, owned))
            })
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))??;
            let keypool = keypool
                .into_iter()
                .flat_map(|(chain, per_wallet)| {
                    per_wallet
                        .into_iter()
                        .map(move |(wallet, state)| (keypool_key(&wallet, &chain), state))
                })
                .collect();

            let mut by_chain: HashMap<String, Vec<crate::wallet_db::OwnedAddressRecord>> =
                HashMap::new();
            for record in owned {
                by_chain
                    .entry(record.chain_id.clone())
                    .or_default()
                    .push(record);
            }

            let mut state = loaded.clone();
            let merged = reduce_state_in_place(&mut state, StateCommand::MergeBuiltInTokens);
            if !merged.is_empty() {
                let changes = crate::wallet_db::AppStateChanges::between(Some(&loaded), &state)?;
                let target = database.clone();
                tokio::task::spawn_blocking(move || changes.save(&target))
                    .await
                    .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))??;
            }
            // Publish only after every fallible initialization step succeeds.
            service.keypool.write().await.load(keypool, by_chain);
            let state = service.publish_state(state).await;
            service.state_binding.bind(database).await;
            service.reconcile_transport(&state.settings, false);
            Ok(state)
        })
        .await
    }

    /// Apply a command to the owned state, persist it, and return the result.
    ///
    /// The returned `StateTransition` carries the new state and the events the
    /// reducer produced, so a front end can both re-render and react without a
    /// second call. When no command applied — setting a value to what it
    /// already is — `events` is empty and nothing is written.
    pub async fn apply_state_command(
        &self,
        mut command: StateCommand,
    ) -> Result<StateTransition, SpectraBridgeError> {
        let validate =
            |wallet: &mut crate::store::state::WalletState| -> Result<(), SpectraBridgeError> {
                crate::registry::Chain::from_str_id(&wallet.chain_id)
                    .ok_or("unknown wallet network")?;
                for holding in &mut wallet.holdings {
                    holding.canonicalize()?;
                }
                Ok(())
            };
        match &mut command {
            StateCommand::SetPinnedDashboardAssets { token_ids } => {
                let options = self.dashboard_pin_options().await?;
                for id in token_ids
                    .iter()
                    .map(|id| id.trim())
                    .filter(|id| !id.is_empty())
                {
                    if !options.iter().any(|option| option.token_id == id) {
                        return Err(SpectraBridgeError::InvalidInput {
                            message: format!("unknown or unpinnable token ID: {id}"),
                        });
                    }
                }
            }
            StateCommand::SetDashboardAssetPinned {
                token_id,
                is_pinned: true,
            } => {
                let options = self.dashboard_pin_options().await?;
                let id = token_id.trim();
                if !options.iter().any(|option| option.token_id == id) {
                    return Err(SpectraBridgeError::InvalidInput {
                        message: format!("unknown or unpinnable token ID: {id}"),
                    });
                }
            }
            StateCommand::UpsertWallet { wallet }
            | StateCommand::UpdateWalletIfPresent { wallet } => validate(wallet)?,
            StateCommand::ReplaceState { state } => {
                for wallet in &mut state.wallets {
                    validate(wallet)?;
                }
            }
            _ => {}
        }
        self.mutate_persisted_state(move |state| reduce_state_in_place(state, command))
            .await
    }

    // ── Operational events ────────────────────────────────────────────────

    /// Pin candidates, with pinned assets first and each group ordered by symbol.
    pub async fn dashboard_pin_options(
        &self,
    ) -> Result<Vec<crate::store::wallet_domain::CoreDashboardPinOption>, SpectraBridgeError> {
        dashboard_pin_options_from(&self.app_state().await)
    }

    /// Evaluate and update alerts against core-owned quotes under the state writer.
    pub async fn evaluate_price_alerts(
        &self,
    ) -> Result<Vec<crate::store::PriceAlertNotification>, SpectraBridgeError> {
        let notifications = Arc::new(std::sync::Mutex::new(Vec::new()));
        let output = notifications.clone();
        self.mutate_persisted_state(move |state| {
            if !state.settings.use_price_alerts {
                return Vec::new();
            }
            let prices = state
                .quotes
                .prices
                .iter()
                .filter(|(_, p)| p.is_finite() && **p > 0.0)
                .map(|(key, p)| crate::store::PriceAlertEvaluationPrice {
                    holding_key: key.clone(),
                    live_price: *p,
                })
                .collect();
            let evaluation = crate::store::evaluate_price_alerts(
                state
                    .price_alerts
                    .iter()
                    .filter(|alert| {
                        crate::tokens::deployment(&alert.holding_key)
                            .is_some_and(|t| !t.coingecko_id.is_empty())
                    })
                    .cloned()
                    .collect(),
                prices,
            );
            for update in &evaluation.updates {
                if let Some(alert) = state.price_alerts.iter_mut().find(|a| a.id == update.id) {
                    alert.has_triggered = update.has_triggered;
                }
            }
            // Priced in the display currency when its rate is known.
            let display = |usd: f64| super::valuation::to_display(state, usd);
            let notifications = evaluation
                .notifications
                .into_iter()
                .map(|mut n| {
                    if let (Some(target), Some(live)) =
                        (display(n.target_price), display(n.live_price))
                    {
                        n.target_price = target;
                        n.live_price = live;
                        n.currency = state.settings.fiat_currency;
                    }
                    n
                })
                .collect();
            *output.lock().expect("alert result lock") = notifications;
            if evaluation.updates.is_empty() {
                Vec::new()
            } else {
                vec![crate::store::state::StateEvent::PriceAlertsEvaluated]
            }
        })
        .await?;
        let result = notifications.lock().expect("alert result lock").clone();
        Ok(result)
    }

    // ── Owned transaction store ───────────────────────────────────────────
    //
    // Transactions are core-owned like everything else in this section, but
    // they deliberately do *not* live in `CoreAppState`. History is unbounded,
    // and `apply_state_command` returns the whole state — putting them there
    // would clone every transaction on every unrelated command.
    //
    // So the store is SQLite (`history_records`), and a command reports *what
    // changed by id* rather than handing back the list. Core computes that
    // delta itself, which is the part a caller can get wrong: whether a record
    // is new or an update is a property of the store, not of the caller.

    // Reserving an index is read-modify-write. Doing that across an FFI round
    // trip is a race — two callers read the same index and both hand it out,
    // which on a UTXO chain means the same receive address given to two
    // people. Every mutation below holds the lock for the whole operation and
    // writes through to SQLite before returning.

    /// Everything the wallet list implies, rendered.
    ///
    /// Resolves holdings and transfer availability from core-owned wallets.
    ///
    /// Signing availability is read through the registered SecretStore.
    pub async fn wallet_derived_state(&self) -> Result<WalletDerivedState, SpectraBridgeError> {
        let state = self.app_state().await;
        self.derive_wallet_projection(&state)
    }

    /// Current snapshot of the owned state.
    pub async fn app_state(&self) -> CoreAppState {
        self.wallet_state.read().await.clone()
    }

    // ── History pagination cursor methods live in `service/history_cursor.rs` ──
    // (split out to keep this file navigable; UniFFI merges the impl blocks).
}

impl WalletService {
    /// Store freshly fetched fiat cross-rates.
    ///
    /// Not a `StateCommand`: the rates are a fetch result, not an intent, so
    /// no front end gets a way to write arbitrary ones.
    #[cfg(test)]
    pub(crate) async fn store_fiat_rates(
        &self,
        rates: std::collections::HashMap<String, f64>,
    ) -> Result<(), SpectraBridgeError> {
        self.mutate_persisted_state(move |state| {
            if state.fiat_rates_from_usd == rates {
                return Vec::new();
            }
            state.fiat_rates_from_usd = rates;
            vec![crate::store::state::StateEvent::FiatRatesChanged]
        })
        .await
        .map(|_| ())
    }

    /// Apply one mutation to the resident state, persist what it changed, and
    /// publish the result.
    ///
    /// `apply_state_command` is this with the reducer as the mutation. Core's
    /// own writes use it directly — fiat rates come from a fetch rather than
    /// from an intent, so they take the same writer, the same incremental diff
    /// and the same publish order without becoming a command a front end could
    /// send arbitrary values through.
    pub(super) async fn mutate_persisted_state<F>(
        &self,
        mutate: F,
    ) -> Result<StateTransition, SpectraBridgeError>
    where
        F: FnOnce(&mut CoreAppState) -> Vec<crate::store::state::StateEvent> + Send + 'static,
    {
        self.write_persisted(move |service| async move {
            let database = service.state_binding.connection().await;
            let (snapshot, events, changes, removed, reset_chains, esplora_changed) = {
                let before = service.wallet_state.read().await;
                let mut state = before.clone();
                let events = mutate(&mut state);
                for old in &before.wallets {
                    if !state.wallets.iter().any(|w| w.id == old.id) {
                        state.diagnostics.forget_wallet(&old.id);
                    }
                }
                let changes = if database.is_some() && !events.is_empty() {
                    Some(crate::wallet_db::AppStateChanges::between(
                        Some(&before),
                        &state,
                    )?)
                } else {
                    None
                };
                let removed: Vec<String> = before
                    .wallets
                    .iter()
                    .filter(|w| !state.wallets.iter().any(|next| next.id == w.id))
                    .map(|w| w.id.clone())
                    .collect();
                let reset_chains = crate::wallet_db::changed_selected_chains(&before, &state);
                let esplora_changed =
                    before.settings.custom_endpoints != state.settings.custom_endpoints;
                (
                    state,
                    events,
                    changes,
                    removed,
                    reset_chains,
                    esplora_changed,
                )
            };

            // Secret deletion is idempotent. A backend failure leaves the wallet
            // present so the same intent can be retried. SQLite changes commit together.
            if !removed.is_empty() {
                let store = service
                    .secret_store
                    .read()
                    .map_err(|_| "secret store lock poisoned")?
                    .clone();
                if store.is_none()
                    && database.is_some()
                    && service
                        .wallet_state
                        .read()
                        .await
                        .wallets
                        .iter()
                        .any(|w| removed.contains(&w.id) && !w.is_watch_only())
                {
                    return Err(
                        "secret store must be registered before deleting a signing wallet".into(),
                    );
                }
                if let Some(store) = store {
                    for id in &removed {
                        crate::store::wallet_secrets::delete(&*store, id)
                            .map_err(|e| SpectraBridgeError::from(e.to_string()))?;
                    }
                }
            }
            if let (Some(database), Some(changes)) = (database, changes) {
                tokio::task::spawn_blocking(move || changes.save(&database))
                    .await
                    .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))??;
            }
            if events.is_empty() {
                return Ok(StateTransition {
                    state: snapshot,
                    events,
                });
            }

            // Both tables under one lock and in one call: forgetting an index
            // without forgetting the addresses it issued — or the reverse — is
            // how the same address gets handed out twice.
            service
                .keypool
                .write()
                .await
                .forget(&removed, &reset_chains);
            // History pagination and the diagnostics rows describe what was
            // fetched, so they go with what they were fetched for: a removed
            // wallet, a family whose network changed, and Bitcoin when its
            // Esplora source did. The app issued these resets itself after
            // each of those commands, and nothing else did.
            for id in &removed {
                service.history_pagination.reset_all_for_wallet(id);
                crate::diagnostics::diagnostics_forget_wallet(id.clone());
            }
            for name in &reset_chains {
                if let Some(chain) = crate::registry::Chain::from_str_id(name) {
                    service
                        .history_pagination
                        .reset_chain(chain.mainnet_counterpart().str_id());
                }
            }
            if esplora_changed {
                service
                    .history_pagination
                    .reset_chain(crate::registry::Chain::Bitcoin.str_id());
            }
            let snapshot = service.publish_state(snapshot).await;
            // The HTTP layer reads the Tor policy per request rather than the
            // store, so a change to either flag is pushed as it lands.
            service.reconcile_transport(&snapshot.settings, false);
            Ok(StateTransition {
                state: snapshot,
                events,
            })
        })
        .await
    }

    /// Publish only after persistence succeeds, while holding the state writer.
    pub(super) async fn publish_state(&self, mut state: CoreAppState) -> CoreAppState {
        let mut current = self.wallet_state.write().await;
        state.revision = current.revision + 1;
        *current = state.clone();
        state
    }

    /// Once submitted, a persistent mutation finishes even if the caller cancels.
    /// The worker owns the serialization guard through both commit and publication.
    /// Do not call this recursively from another persistent mutation.
    pub(super) async fn write_persisted<T, F, Fut>(
        &self,
        operation: F,
    ) -> Result<T, SpectraBridgeError>
    where
        T: Send + 'static,
        F: FnOnce(Self) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<T, SpectraBridgeError>> + Send,
    {
        let service = self.clone();
        tokio::spawn(async move {
            let writer = service.state_writer.clone();
            let _guard = writer.lock().await;
            operation(service).await
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("state writer: {e}")))?
    }

    // ── Not exported ──────────────────────────────────────────────────────
    //
    // Reachable from Rust — the CLI, or core itself — and from nothing across
    // the boundary. A method in the block above is an entry point whether or
    // not a platform uses it, and these were entry points nobody had taken.

    /// Resolve a pinned token by identity, including native tokens without a balance.
    /// The bound state database, or an error naming what the caller skipped.
    ///
    /// Kept as a method on the service because twelve call sites read it and
    /// `self.state_binding.required_connection()` at each of them reaches through the
    /// service to say the same thing.
    pub(super) async fn bound_database(
        &self,
    ) -> Result<Arc<crate::wallet_db::WalletDatabase>, SpectraBridgeError> {
        self.state_binding.required_connection().await
    }
}

#[cfg(test)]
mod pruning_reads_cores_own_tables {
    /// Pruning takes the stricter side when it cannot see the transactions.
    #[tokio::test]
    async fn pruning_refuses_rather_than_guessing() {
        let service = crate::service::WalletService::new(Vec::new()).expect("service");
        assert!(service.prune_status_trackers().await.is_err());
    }
}

#[cfg(test)]
mod utxo_discovery_is_the_registrys_chain_set {
    use crate::registry::Chain;

    /// The Swift table this replaced named five chains; the registry answers
    /// for twelve. `deriveUTXOAddress` required both, so the table won and
    /// discovery was dead on every UTXO testnet — the shape of bug rule 2
    /// exists to stop.
    #[test]
    fn the_testnets_are_in_the_set_their_mainnets_are_in() {
        for (mainnet, testnet) in [
            (Chain::Bitcoin, Chain::BitcoinTestnet),
            (Chain::BitcoinCash, Chain::BitcoinCashTestnet),
            (Chain::BitcoinSV, Chain::BitcoinSVTestnet),
            (Chain::Litecoin, Chain::LitecoinTestnet),
            (Chain::Dogecoin, Chain::DogecoinTestnet),
        ] {
            assert!(mainnet.supports_deep_utxo_discovery(), "{mainnet:?}");
            assert!(
                testnet.supports_deep_utxo_discovery(),
                "{testnet:?} walks the same addresses its mainnet does"
            );
        }
        assert_eq!(
            Chain::all()
                .filter(|c| c.supports_deep_utxo_discovery())
                .count(),
            12,
            "five mainnets and seven testnets"
        );
    }

    /// Every entry point answers empty for a chain without the walk rather
    /// than failing: the refresh loop asks for every chain a wallet is on.
    #[tokio::test]
    async fn a_chain_without_the_walk_does_nothing() {
        let service = crate::service::WalletService::new(Vec::new()).expect("service");
        let evm = Chain::Ethereum.str_id().to_string();
        assert!(
            service
                .discover_utxo_addresses("w".into(), evm.clone())
                .await
                .expect("ok")
                .is_empty()
        );
        assert!(
            service
                .known_utxo_addresses("w".into(), evm.clone())
                .await
                .expect("ok")
                .is_empty()
        );
        assert!(
            service
                .utxo_receive_address("w".into(), evm.clone(), false)
                .await
                .expect("ok")
                .is_none()
        );
        service
            .advance_used_utxo_reservations(evm)
            .await
            .expect("a chain without the walk is a no-op, not an error");
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "state_performance_tests.rs"]
mod performance_tests;

fn wallets_for_display(
    state: &CoreAppState,
) -> Result<Vec<crate::store::wallet_domain::WalletView>, SpectraBridgeError> {
    let wallets = &state.wallets;
    let mut rendered = Vec::with_capacity(wallets.len());
    for wallet in wallets {
        let defaults = crate::derivation_paths_for_preset(wallet.derivation_preset)?;
        rendered.push(wallet.to_wallet_view(&defaults));
    }
    Ok(rendered)
}

fn derive_wallet_state(
    state: &CoreAppState,
    signing_material_wallet_ids: Vec<String>,
) -> Result<WalletDerivedState, SpectraBridgeError> {
    use std::collections::{BTreeMap, HashSet};
    let wallets = &state.wallets;
    let token_preferences = &state.token_preferences;
    // The network the user picked for a holding's family, and whether that
    // network is quoted at all.
    let network_of = |chain_id: &str| -> Option<crate::registry::Chain> {
        crate::registry::Chain::from_str_id(chain_id)
    };
    let signing: HashSet<&str> = signing_material_wallet_ids
        .iter()
        .map(String::as_str)
        .collect();

    let mut included_portfolio_holdings = Vec::new();
    let mut unique_price_request_coins = Vec::new();
    let mut seen_price_keys = HashSet::new();
    let mut grouped_order: Vec<String> = Vec::new();
    let mut grouped_totals: BTreeMap<String, String> = BTreeMap::new();
    let mut grouped_representative: BTreeMap<String, crate::store::wallet_domain::AssetHolding> =
        BTreeMap::new();

    let mut send_coins_by_wallet_id = HashMap::new();
    let mut receive_coins_by_wallet_id = HashMap::new();
    let mut send_enabled_wallet_ids = Vec::new();
    let mut receive_enabled_wallet_ids = Vec::new();

    for wallet in wallets {
        let has_signing_material = signing.contains(wallet.id.as_str());
        let mut send_coins = Vec::new();
        let mut receive_coins = Vec::new();

        for holding in wallet
            .holdings
            .iter()
            .cloned()
            .map(AssetHolding::identified)
        {
            let holding = &holding;
            let network = network_of(&holding.chain_id);
            // Identity is per *network*: testnet BTC groups separately from
            // mainnet BTC and is quoted separately (which is to say, not).
            let identity_key = holding.deployment_id();
            // `chain_backends()` was a 78-row table beside `chains.toml`,
            // with the same 78 names and `Live` on every one — so
            // "has a backend", "supports send", "supports receive" and "is
            // a live chain" were four spellings of "the registry knows this
            // chain". Verified identical before it was deleted.
            let chain_is_known = crate::registry::Chain::from_str_id(&holding.chain_id).is_some();

            if network.is_none_or(|chain| !chain.is_testnet())
                && seen_price_keys.insert(identity_key.clone())
            {
                unique_price_request_coins.push(holding.clone());
            }

            if wallet.include_in_portfolio_total {
                included_portfolio_holdings.push(holding.clone());
                if !grouped_totals.contains_key(&identity_key) {
                    grouped_order.push(identity_key.clone());
                    grouped_representative.insert(identity_key.clone(), holding.clone());
                }
                let total = grouped_totals
                    .entry(identity_key)
                    .or_insert_with(|| "0".to_string());
                *total = crate::decimal::add(total, &holding.amount)
                    .ok_or("portfolio total out of range")?;
            }

            let selected_network = wallet.chain();
            let on_selected_network = match (holding.chain(), selected_network) {
                (Some(asset), Some(selected))
                    if asset.mainnet_counterpart() == selected.mainnet_counterpart() =>
                {
                    asset == selected
                }
                _ => true,
            };
            if on_selected_network
                && crate::send::transfer::can_send_coin(
                    holding,
                    has_signing_material,
                    chain_is_known,
                    chain_is_known,
                    token_preferences,
                )
            {
                send_coins.push(holding.clone());
            }
            if chain_is_known {
                receive_coins.push(holding.clone());
            }
        }

        if !send_coins.is_empty() {
            send_enabled_wallet_ids.push(wallet.id.clone());
        }
        if !receive_coins.is_empty() {
            receive_enabled_wallet_ids.push(wallet.id.clone());
        }
        send_coins_by_wallet_id.insert(wallet.id.clone(), send_coins);
        receive_coins_by_wallet_id.insert(wallet.id.clone(), receive_coins);
    }

    let portfolio = grouped_order
        .into_iter()
        .filter_map(|key| {
            let mut representative = grouped_representative.remove(&key)?;
            representative.amount = grouped_totals
                .remove(&key)
                .unwrap_or_else(|| "0".to_string());
            Some(representative)
        })
        .collect();

    Ok(WalletDerivedState {
        included_portfolio_holdings,
        unique_price_request_coins,
        portfolio,
        send_coins_by_wallet_id,
        receive_coins_by_wallet_id,
        send_enabled_wallet_ids,
        receive_enabled_wallet_ids,
    })
}

fn dashboard_pin_options_from(
    state: &CoreAppState,
) -> Result<Vec<crate::store::wallet_domain::CoreDashboardPinOption>, SpectraBridgeError> {
    use crate::store::wallet_domain::CoreDashboardPinOption;
    let pinned = state.settings.pinned_dashboard_assets();
    let catalog = crate::tokens::list_token_deployments(String::new());
    let coins = catalog
        .iter()
        .chain(state.token_preferences.iter().map(|e| &e.token))
        .map(|t| t.holding_template())
        .chain(state.wallets.iter().flat_map(|w| w.holdings.clone()));
    let mut options = std::collections::BTreeMap::<String, CoreDashboardPinOption>::new();
    for coin in coins {
        if coin.chain().is_none_or(|n| n.is_testnet()) {
            continue;
        }
        let token_id = coin.token_identity();
        options
            .entry(token_id.clone())
            .or_insert_with(|| CoreDashboardPinOption {
                token_id: token_id.clone(),
                deployment_id: coin.deployment_id(),
                symbol: coin.symbol.clone(),
                name: coin.name.clone(),
                subtitle: if token_id.starts_with("custom:") {
                    format!(
                        "{} · {}",
                        coin.chain().unwrap().chain_display_name(),
                        coin.contract_address.as_deref().unwrap_or("")
                    )
                } else {
                    coin.chain().unwrap().chain_display_name().to_string()
                },
                artwork_name: Some(crate::store::deployment_artwork_name(Some(
                    coin.deployment_id(),
                ))),
                is_pinned: pinned.contains(&token_id),
            });
    }
    let mut options: Vec<_> = options.into_values().collect();
    options.sort_by(|a, b| {
        b.is_pinned
            .cmp(&a.is_pinned)
            .then(a.symbol.cmp(&b.symbol))
            .then(a.token_id.cmp(&b.token_id))
    });
    Ok(options)
}

fn dashboard_groups_from(
    state: &CoreAppState,
    derived: &WalletDerivedState,
) -> Result<Vec<crate::store::wallet_domain::CoreDashboardAssetGroup>, SpectraBridgeError> {
    use crate::store::wallet_domain::{CoreDashboardAssetGroup, CoreDashboardAssetHolding};

    let settings = &state.settings;
    let pinned = settings.pinned_dashboard_assets();

    // Ordering uses USD, which needs no exchange rate; what the row shows is
    // the display currency.
    let usd_of = |coin: &crate::store::wallet_domain::AssetHolding| valuation::value(state, coin);
    let display_of = |usd: Option<f64>| usd.and_then(|usd| valuation::to_display(state, usd));

    // One row per asset, wherever it is held. The same asset on two
    // chains, or on one chain across two wallets, is one row.
    //
    // Two passes: group holdings by asset, then split each group by
    // (network, standard, contract) so the row can show where it lives.
    let mut order: Vec<String> = Vec::new();
    let mut grouped: HashMap<String, Vec<crate::store::wallet_domain::AssetHolding>> =
        HashMap::new();
    for coin in derived
        .included_portfolio_holdings
        .iter()
        .filter(|c| !crate::decimal::is_zero(&c.amount))
    {
        let key = coin.token_identity();
        if !grouped.contains_key(&key) {
            order.push(key.clone());
        }
        grouped.entry(key).or_default().push(coin.clone());
    }

    let mut groups: Vec<CoreDashboardAssetGroup> = Vec::new();
    for key in order {
        let Some(coins) = grouped.get(&key) else {
            continue;
        };
        // Within a row, one entry per place: the same asset held on one
        // chain by two wallets is one entry with the amounts summed.
        let mut place_order: Vec<String> = Vec::new();
        let mut by_place: HashMap<String, crate::store::wallet_domain::AssetHolding> =
            HashMap::new();
        for coin in coins {
            let contract = crate::tokens::normalize_token_identifier(
                coin.contract_address.clone(),
                coin.chain_id.clone(),
            )
            .unwrap_or_else(|| "native".to_string());
            let place = format!(
                "{}|{}|{contract}",
                coin.chain_id,
                coin.token_standard.to_lowercase()
            );
            match by_place.get_mut(&place) {
                Some(existing) => {
                    existing.amount = crate::decimal::add(&existing.amount, &coin.amount)
                        .ok_or("asset total out of range")?;
                }
                None => {
                    place_order.push(place.clone());
                    by_place.insert(place, coin.clone());
                }
            }
        }
        let mut holdings: Vec<CoreDashboardAssetHolding> = place_order
            .iter()
            .filter_map(|p| by_place.get(p))
            .map(|coin| CoreDashboardAssetHolding {
                value: display_of(usd_of(coin)),
                coin: coin.clone(),
            })
            .collect();
        // Largest value first, so the row is presented as the place most of
        // it is. Ties break on chain id so the order does not wander.
        holdings.sort_by(|lhs, rhs| {
            let (l, r) = (
                usd_of(&lhs.coin).unwrap_or(-1.0),
                usd_of(&rhs.coin).unwrap_or(-1.0),
            );
            if (l - r).abs() > 0.000_001 {
                return r.total_cmp(&l);
            }
            lhs.coin
                .chain_id
                .to_lowercase()
                .cmp(&rhs.coin.chain_id.to_lowercase())
        });
        let Some(largest) = holdings.first() else {
            continue;
        };
        let total_usd = holdings.iter().try_fold(0.0, |sum, holding| {
            usd_of(&holding.coin).and_then(|value| {
                let total = sum + value;
                total.is_finite().then_some(total)
            })
        });
        let total_amount = holdings
            .iter()
            .try_fold("0".to_string(), |sum, h| {
                crate::decimal::add(&sum, &h.coin.amount)
            })
            .ok_or("asset total out of range")?;
        groups.push(CoreDashboardAssetGroup {
            total_amount,
            total_value: display_of(total_usd),
            price: valuation::display_price(state, &largest.coin),
            is_pinned: pinned.contains(&key),
            identity: largest.coin.clone(),
            holdings,
            id: key,
        });
    }

    // A pinned token the user holds none of still gets a row, named by the
    // catalog and holding nothing.
    let row_symbol = |g: &CoreDashboardAssetGroup| -> String { g.identity.symbol.to_uppercase() };
    let row_value = |g: &CoreDashboardAssetGroup| {
        g.holdings.iter().try_fold(0.0, |sum, holding| {
            usd_of(&holding.coin).map(|value| sum + value)
        })
    };
    let present: std::collections::HashSet<String> = groups.iter().map(|g| g.id.clone()).collect();
    for symbol in pinned.iter().filter(|s| !present.contains(*s)) {
        let Some(prototype) = pinned_prototype(state, symbol, derived) else {
            continue;
        };
        groups.push(CoreDashboardAssetGroup {
            total_amount: "0".to_string(),
            total_value: display_of(Some(0.0)),
            price: valuation::display_price(state, &prototype),
            id: symbol.clone(),
            identity: prototype,
            holdings: Vec::new(),
            is_pinned: true,
        });
    }

    let pin_order: HashMap<&str, usize> = pinned
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), i))
        .collect();
    groups.sort_by(|lhs, rhs| {
        match (lhs.is_pinned, rhs.is_pinned) {
            (true, false) => return std::cmp::Ordering::Less,
            (false, true) => return std::cmp::Ordering::Greater,
            (true, true) => {
                let l = pin_order
                    .get(lhs.id.as_str())
                    .copied()
                    .unwrap_or(usize::MAX);
                let r = pin_order
                    .get(rhs.id.as_str())
                    .copied()
                    .unwrap_or(usize::MAX);
                return l.cmp(&r);
            }
            (false, false) => {}
        }
        let (l, r) = (
            row_value(lhs).unwrap_or(-1.0),
            row_value(rhs).unwrap_or(-1.0),
        );
        if (l - r).abs() > 0.000_001 {
            return r.total_cmp(&l);
        }
        row_symbol(lhs).cmp(&row_symbol(rhs))
    });
    Ok(groups)
}
fn pinned_prototype(
    state: &CoreAppState,
    token_id: &str,
    derived: &WalletDerivedState,
) -> Option<crate::store::wallet_domain::AssetHolding> {
    if let Some(coin) = derived
        .included_portfolio_holdings
        .iter()
        .find(|c| c.token_identity() == token_id)
    {
        let mut coin = coin.clone();
        coin.amount = "0".to_string();
        return Some(coin);
    }
    let tokens = crate::tokens::list_token_deployments(String::new());
    tokens
        .iter()
        .chain(state.token_preferences.iter().map(|e| &e.token))
        .find(|token| token.token_id == token_id)
        .map(|token| token.holding_template())
}

impl WalletService {
    fn derive_wallet_projection(
        &self,
        state: &CoreAppState,
    ) -> Result<WalletDerivedState, SpectraBridgeError> {
        // What a wallet signs with is recorded on it, so the projection reads
        // no secret store.
        let signing_material_wallet_ids: Vec<String> = state
            .wallets
            .iter()
            .filter(|wallet| !wallet.is_watch_only())
            .map(|wallet| wallet.id.clone())
            .collect();
        derive_wallet_state(state, signing_material_wallet_ids)
    }
}

/// A coherent read of the portfolio. Revision orders snapshots within this service session.
#[derive(Debug, Clone, serde::Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioSnapshot {
    pub revision: u64,
    pub state: CoreAppState,
    pub wallets: Vec<crate::store::wallet_domain::WalletView>,
    pub derived: WalletDerivedState,
    pub groups: Vec<crate::store::wallet_domain::CoreDashboardAssetGroup>,
    pub pin_options: Vec<crate::store::wallet_domain::CoreDashboardPinOption>,
    pub valuation: super::valuation::PortfolioValuation,
    pub asset_precision: crate::formatting::AssetPrecisionCatalog,
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    pub async fn portfolio_snapshot(&self) -> Result<PortfolioSnapshot, SpectraBridgeError> {
        let _guard = self.state_writer.lock().await;
        let state = self.wallet_state.read().await.clone();
        let revision = self
            .projection_sequence
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        let derived = self.derive_wallet_projection(&state)?;
        Ok(PortfolioSnapshot {
            revision,
            wallets: wallets_for_display(&state)?,
            groups: dashboard_groups_from(&state, &derived)?,
            pin_options: dashboard_pin_options_from(&state)?,
            valuation: valuation::portfolio_valuation(&state),
            asset_precision: crate::formatting::asset_precision_catalog(&state),
            derived,
            state,
        })
    }
}

impl WalletService {
    pub(super) async fn pinned_prototype(
        &self,
        token_id: &str,
        derived: &WalletDerivedState,
    ) -> Option<AssetHolding> {
        pinned_prototype(&self.app_state().await, token_id, derived)
    }
}
