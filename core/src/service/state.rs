//! Resident app-state projections and the serialized persistence writer.
//! Keypool, discovery, transactions, imports and events live in sibling modules.

use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Load the JSON state blob stored under `key` in the SQLite database at
    /// `db_path`. Returns an empty JSON object `"{}"` when no value has been
    /// saved yet. Thread-safe: rusqlite is called in `spawn_blocking`.
    pub async fn load_state(&self, key: String) -> Result<String, SpectraBridgeError> {
        let db_path = self.bound_state_db_path().await?;
        tokio::task::spawn_blocking(move || sqlite_load(&db_path, &key))
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
            .map_err(Into::into)
    }

    /// Persist the JSON state blob under `key` in the SQLite database at
    /// `db_path`. Creates the file (and the `state` table) on first use.
    pub async fn save_state(
        &self,
        key: String,
        state_json: String,
    ) -> Result<(), SpectraBridgeError> {
        let db_path = self.bound_state_db_path().await?;
        tokio::task::spawn_blocking(move || sqlite_save(&db_path, &key, &state_json))
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
            .map_err(Into::into)
    }

    /// Remove all relational wallet state (keypool + addresses) for a deleted wallet.
    /// This is the single call to make when a wallet is removed.
    pub async fn delete_wallet_relational_data(
        &self,
        wallet_id: String,
    ) -> Result<(), SpectraBridgeError> {
        self.write_persisted(move |service| async move {
            let db_path = service.bound_state_db_path().await?;
            let to_delete = wallet_id.clone();
            tokio::task::spawn_blocking(move || {
                crate::wallet_db::delete_wallet_data(&db_path, &to_delete)
            })
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
            .map_err(SpectraBridgeError::from)?;
            // Clear the in-memory rows too. Leaving them means the keypool
            // baseline still counts a deleted wallet's addresses.
            service
                .keypool
                .write()
                .await
                .retain(|key, _| key.split_once('|').is_none_or(|(id, _)| id != wallet_id));
            for rows in service.owned_addresses.write().await.values_mut() {
                rows.retain(|row| row.wallet_id != wallet_id);
            }
            Ok(())
        })
        .await
    }

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
    pub async fn open_state(&self, db_path: String) -> Result<CoreAppState, SpectraBridgeError> {
        self.write_persisted(move |service| async move {
            // Opening is idempotent. A second call with the same database returns
            // what is already held rather than re-reading — a late `open_state`
            // (the app's launch reload racing a user action) would otherwise
            // replace the in-memory state with a snapshot taken before the newer
            // command, silently reverting it.
            if service.state_db_path.read().await.as_deref() == Some(db_path.as_str()) {
                return Ok(service.wallet_state.read().await.clone());
            }

            let database = crate::wallet_db::WalletDatabase::acquire(&db_path);
            let loaded = {
                let path = db_path.clone();
                tokio::task::spawn_blocking(move || crate::wallet_db::app_state_load(&path))
                    .await
                    .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))??
            };
            let keypool = {
                let path = db_path.clone();
                tokio::task::spawn_blocking(move || crate::wallet_db::keypool_load_all(&path))
                    .await
                    .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))??
            };
            let keypool = keypool
                .into_iter()
                .flat_map(|(chain, per_wallet)| {
                    per_wallet
                        .into_iter()
                        .map(move |(wallet, state)| (keypool_key(&wallet, &chain), state))
                })
                .collect();

            let owned = {
                let path = db_path.clone();
                tokio::task::spawn_blocking(move || {
                    crate::wallet_db::address_load_all_chains(&path)
                })
                .await
                .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))??
            };
            let mut by_chain: HashMap<String, Vec<crate::wallet_db::OwnedAddressRecord>> =
                HashMap::new();
            for record in owned {
                by_chain
                    .entry(record.chain_name.clone())
                    .or_default()
                    .push(record);
            }

            let events = {
                let path = db_path.clone();
                tokio::task::spawn_blocking(move || sqlite_load(&path, OPERATIONAL_EVENTS_KEY))
                    .await
                    .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))??
            };
            let events = serde_json::from_str(&events)?;
            *service.keypool.write().await = keypool;
            *service.owned_addresses.write().await = by_chain;
            *service.operational_events.write().await = events;

            *service.state_database.write().await = Some(database);
            let db_path_for_seed = db_path.clone();
            *service.state_db_path.write().await = Some(db_path);
            // The token list is the catalog plus whatever the user added, so
            // opening seeds it. A caller that forgot to ask for the merge
            // otherwise held a list with no built-ins in it at all — which is
            // what `spectra token track` had nothing to turn on — while the
            // app happened to ask and so never saw it. Seeding here rather
            // than on each read keeps "what was loaded" and "what is stored"
            // the same document.
            let mut state = service.wallet_state.write().await;
            *state = loaded.clone();
            let merged = reduce_state_in_place(&mut state, StateCommand::MergeBuiltInTokens);
            if !merged.is_empty() {
                let changes = crate::wallet_db::AppStateChanges::between(Some(&loaded), &state)?;
                let path = db_path_for_seed.clone();
                tokio::task::spawn_blocking(move || changes.save(&path))
                    .await
                    .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))??;
            }
            crate::tor::apply_policy(state.settings.tor_enabled, state.settings.tor_kill_switch);
            Ok(state.clone())
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
        command: StateCommand,
    ) -> Result<StateTransition, SpectraBridgeError> {
        self.mutate_persisted_state(move |state| reduce_state_in_place(state, command))
            .await
    }

    // ── Operational events ────────────────────────────────────────────────

    /// The dashboard's asset rows: holdings grouped across chains, ordered,
    /// with the pinned ones first.
    ///
    /// Live prices are the only input core does not have — everything else
    /// (which holdings count toward the total, which tokens are tracked, which
    /// symbols are pinned, which networks are unpriced, how a chain identifies
    /// an asset) is core's already. `prices` is keyed the way core keys an
    /// asset: `"<network title>|<symbol>"`.
    pub async fn dashboard_asset_groups(
        &self,
        prices: HashMap<String, f64>,
    ) -> Result<Vec<crate::store::wallet_domain::CoreDashboardAssetGroup>, SpectraBridgeError> {
        use crate::store::wallet_domain::{CoreDashboardAssetGroup, CoreDashboardAssetHolding};

        let settings = self.wallet_state.read().await.settings.clone();
        let derived = self.wallet_derived_state(Vec::new(), Vec::new()).await?;
        let pinned = settings.pinned_dashboard_assets();

        let network_title = |chain_name: &str| -> String {
            crate::registry::Chain::from_display_name(chain_name)
                .map(|chain| {
                    settings
                        .network_chain(chain)
                        .chain_display_name()
                        .to_string()
                })
                .unwrap_or_else(|| chain_name.to_string())
        };
        // Unpriced on a testnet, then the live quote, then the amount the
        // holding was last stored with. Same order the shell applied.
        let value_of = |coin: &crate::store::wallet_domain::AssetHolding| -> Option<f64> {
            let title = network_title(&coin.chain_name);
            if crate::registry::Chain::from_display_name(&coin.chain_name)
                .is_some_and(|chain| settings.network_chain(chain).is_testnet())
            {
                return None;
            }
            let price = prices
                .get(&format!("{title}|{}", coin.symbol))
                .copied()
                .filter(|p| *p > 0.0)
                .or(Some(coin.price_usd).filter(|p| *p > 0.0))?;
            Some(coin.amount * price)
        };

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
            .filter(|c| c.amount > 0.0)
        {
            let contract = crate::tokens::normalize_token_identifier(
                coin.contract_address.clone(),
                coin.chain_name.clone(),
            )
            .unwrap_or_else(|| "native".to_string());
            let key = crate::formatting::dashboard_asset_grouping_key(
                &coin.coin_gecko_id,
                &network_title(&coin.chain_name),
                &contract,
            );
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
                    coin.chain_name.clone(),
                )
                .unwrap_or_else(|| "native".to_string());
                let place = format!(
                    "{}|{}|{contract}",
                    network_title(&coin.chain_name).to_lowercase(),
                    coin.token_standard.to_lowercase()
                );
                match by_place.get_mut(&place) {
                    Some(existing) => {
                        existing.amount += coin.amount;
                        existing.price_usd = coin.price_usd;
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
                    value_usd: value_of(coin),
                    coin: coin.clone(),
                })
                .collect();
            // Largest value first, so the row is presented as the place most of
            // it is. Ties break on chain name so the order does not wander.
            holdings.sort_by(|lhs, rhs| {
                let (l, r) = (lhs.value_usd.unwrap_or(-1.0), rhs.value_usd.unwrap_or(-1.0));
                if (l - r).abs() > 0.000_001 {
                    return r.total_cmp(&l);
                }
                lhs.coin
                    .chain_name
                    .to_lowercase()
                    .cmp(&rhs.coin.chain_name.to_lowercase())
            });
            let Some(first) = holdings.first() else {
                continue;
            };
            groups.push(CoreDashboardAssetGroup {
                is_pinned: pinned.contains(&first.coin.symbol.to_uppercase()),
                holdings,
                id: key,
            });
        }

        // A pinned symbol the user holds none of still gets a row.
        // The row is presented as its first holding, so that is where its
        // symbol comes from.
        let row_symbol = |g: &CoreDashboardAssetGroup| -> String {
            g.holdings
                .first()
                .map(|h| h.coin.symbol.to_uppercase())
                .unwrap_or_default()
        };
        let row_value = |g: &CoreDashboardAssetGroup| -> Option<f64> {
            g.holdings
                .iter()
                .map(|h| h.value_usd)
                .try_fold(0.0, |sum, v| v.map(|v| sum + v))
        };
        let present: std::collections::HashSet<String> = groups.iter().map(row_symbol).collect();
        for symbol in pinned.iter().filter(|s| !present.contains(*s)) {
            let Some(prototype) = self.pinned_prototype(symbol, &derived).await else {
                continue;
            };
            groups.push(CoreDashboardAssetGroup {
                id: format!("pinned:{}", symbol.to_lowercase()),
                holdings: vec![CoreDashboardAssetHolding {
                    coin: prototype,
                    value_usd: Some(0.0),
                }],
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
                        .get(row_symbol(lhs).as_str())
                        .copied()
                        .unwrap_or(usize::MAX);
                    let r = pin_order
                        .get(row_symbol(rhs).as_str())
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

    /// Fold this build's built-in token catalog into the stored preferences
    /// and keep the result.
    ///
    /// A user's `is_enabled` survives; tokens the build
    /// added appear; tokens the user added stay. The caller used to fetch the
    /// catalog from core, reshape it, send both lists back for merging and
    /// assign the answer — core owns both sides, so it does all of it.
    pub async fn merge_built_in_token_preferences(
        &self,
    ) -> Result<CoreAppState, SpectraBridgeError> {
        Ok(self
            .apply_state_command(StateCommand::MergeBuiltInTokens)
            .await?
            .state)
    }

    /// Evaluate the stored price alerts against live prices, record what
    /// changed, and return only what the platform has to act on.
    ///
    /// Core owns the alerts, so it owns the verdict too. The planner this
    /// replaces took the list as an argument and returned `has_triggered`
    /// updates for the caller to write back — a caller that forgot to, or
    /// wrote them to its own copy, silently re-notified on every price tick.
    pub async fn evaluate_price_alerts(
        &self,
        prices: Vec<crate::store::PriceAlertEvaluationPrice>,
    ) -> Result<Vec<crate::store::PriceAlertNotification>, SpectraBridgeError> {
        let alerts = self.wallet_state.read().await.price_alerts.clone();
        if alerts.is_empty() {
            return Ok(Vec::new());
        }
        let plan = crate::store::plan_price_alert_evaluation(alerts.clone(), prices);
        if plan.updates.is_empty() {
            return Ok(plan.notifications);
        }
        let triggered: HashMap<&str, bool> = plan
            .updates
            .iter()
            .map(|u| (u.id.as_str(), u.has_triggered))
            .collect();
        let next = alerts
            .into_iter()
            .map(|mut alert| {
                if let Some(has_triggered) = triggered.get(alert.id.as_str()) {
                    alert.has_triggered = *has_triggered;
                }
                alert
            })
            .collect();
        self.apply_state_command(StateCommand::SetPriceAlerts { alerts: next })
            .await?;
        Ok(plan.notifications)
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
    /// Replaces `core_plan_store_derived_state` + `core_plan_transfer_availability`,
    /// which returned holding *indices* that the caller resolved back into
    /// coins against its own copy of the wallets. Core holds the wallets, so it
    /// resolves them itself.
    ///
    /// The two inputs are the things core genuinely cannot know: which wallets
    /// have signing material and which have a private key. Both are the
    /// platform keystore's answer.
    pub async fn wallet_derived_state(
        &self,
        signing_material_wallet_ids: Vec<String>,
        private_key_backed_wallet_ids: Vec<String>,
    ) -> Result<WalletDerivedState, SpectraBridgeError> {
        use std::collections::{BTreeMap, HashSet};

        let wallets = self.wallets_for_display().await?;
        let (token_preferences, settings) = {
            let state = self.wallet_state.read().await;
            (state.token_preferences.clone(), state.settings.clone())
        };
        // The network the user picked for a holding's family, and whether that
        // network is quoted at all.
        let network_of = |chain_name: &str| -> Option<crate::registry::Chain> {
            crate::registry::Chain::from_display_name(chain_name)
                .map(|chain| settings.network_chain(chain))
        };
        let signing: HashSet<&str> = signing_material_wallet_ids
            .iter()
            .map(String::as_str)
            .collect();

        let mut included_portfolio_holdings = Vec::new();
        let mut unique_price_request_coins = Vec::new();
        let mut seen_price_keys = HashSet::new();
        let mut grouped_order: Vec<String> = Vec::new();
        let mut grouped_totals: BTreeMap<String, f64> = BTreeMap::new();
        let mut grouped_representative: BTreeMap<
            String,
            crate::store::wallet_domain::AssetHolding,
        > = BTreeMap::new();

        let mut send_coins_by_wallet_id = HashMap::new();
        let mut receive_coins_by_wallet_id = HashMap::new();
        let mut send_enabled_wallet_ids = Vec::new();
        let mut receive_enabled_wallet_ids = Vec::new();

        for wallet in &wallets {
            let has_signing_material = signing.contains(wallet.id.as_str());
            let mut send_coins = Vec::new();
            let mut receive_coins = Vec::new();

            for holding in &wallet.holdings {
                let network = network_of(&holding.chain_name);
                // Identity is per *network*: testnet BTC groups separately from
                // mainnet BTC and is quoted separately (which is to say, not).
                let title = network
                    .map(|chain| chain.chain_display_name().to_string())
                    .unwrap_or_else(|| holding.chain_name.clone());
                let identity_key = format!("{}|{}", title, holding.symbol);
                // `chain_backends()` was a 78-row table beside `chains.toml`,
                // with the same 78 names and `Live` on every one — so
                // "has a backend", "supports send", "supports receive" and "is
                // a live chain" were four spellings of "the registry knows this
                // chain". Verified identical before it was deleted.
                let chain_is_known =
                    crate::registry::Chain::from_display_name(&holding.chain_name).is_some();

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
                    *grouped_totals.entry(identity_key).or_default() += holding.amount;
                }

                if crate::send::transfer::can_send_coin(
                    holding,
                    has_signing_material,
                    chain_is_known,
                    chain_is_known,
                    &token_preferences,
                ) {
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
                representative.amount = grouped_totals.get(&key).copied().unwrap_or(0.0);
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
            refreshable_chain_names: wallets
                .iter()
                .map(|w| w.selected_chain.clone())
                .collect::<HashSet<_>>()
                .into_iter()
                .collect(),
            signing_material_wallet_ids,
            private_key_backed_wallet_ids,
        })
    }

    /// The wallets core holds, as the shape the iOS app renders.
    ///
    /// A view model built from the authoritative `WalletSummary` list, with the
    /// derivation-path table filled from the catalog defaults for the wallet's
    /// preset.
    pub async fn wallets_for_display(
        &self,
    ) -> Result<Vec<crate::store::wallet_domain::CoreImportedWallet>, SpectraBridgeError> {
        let wallets = self.wallet_state.read().await.wallets.clone();
        let mut rendered = Vec::with_capacity(wallets.len());
        for wallet in wallets {
            let account = match wallet.derivation_preset.as_str() {
                "account1" => 1,
                "account2" => 2,
                _ => 0,
            };
            let defaults = crate::app_core_derivation_paths_for_preset(account)?;
            rendered.push(wallet.to_imported_wallet(&defaults));
        }
        Ok(rendered)
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
    pub(crate) async fn store_fiat_rates(
        &self,
        rates: std::collections::HashMap<String, f64>,
    ) -> Result<(), SpectraBridgeError> {
        self.mutate_persisted_state(move |state| {
            if state.fiat_rates_from_usd == rates {
                return Vec::new();
            }
            state.fiat_rates_from_usd = rates;
            vec![crate::store::state::StateEvent {
                kind: "fiatRatesChanged".to_string(),
                subject_id: None,
            }]
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
    async fn mutate_persisted_state<F>(
        &self,
        mutate: F,
    ) -> Result<StateTransition, SpectraBridgeError>
    where
        F: FnOnce(&mut CoreAppState) -> Vec<crate::store::state::StateEvent> + Send + 'static,
    {
        self.write_persisted(move |service| async move {
            let path = service.state_db_path.read().await.clone();
            let (snapshot, events, changes) = {
                let before = service.wallet_state.read().await;
                let mut state = before.clone();
                let events = mutate(&mut state);
                let changes = if path.is_some() && !events.is_empty() {
                    Some(crate::wallet_db::AppStateChanges::between(
                        Some(&before),
                        &state,
                    )?)
                } else {
                    None
                };
                (state, events, changes)
            };

            if let (Some(path), Some(changes)) = (path, changes) {
                tokio::task::spawn_blocking(move || changes.save(&path))
                    .await
                    .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))??;
            }
            if events.is_empty() {
                return Ok(StateTransition {
                    state: snapshot,
                    events,
                });
            }

            *service.wallet_state.write().await = snapshot.clone();
            // The HTTP layer reads the Tor policy per request rather than the
            // store, so a change to either flag is pushed as it lands.
            crate::tor::apply_policy(
                snapshot.settings.tor_enabled,
                snapshot.settings.tor_kill_switch,
            );
            Ok(StateTransition {
                state: snapshot,
                events,
            })
        })
        .await
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

    /// Fiat currency the user has chosen, as an ISO 4217 code.
    pub async fn fiat_currency_code(&self) -> String {
        self.wallet_state
            .read()
            .await
            .settings
            .fiat_currency_code
            .clone()
    }

    /// A stand-in coin for a pinned symbol the user holds none of: a holding
    /// if one exists at zero, else a known token, else nothing.
    async fn pinned_prototype(
        &self,
        symbol: &str,
        derived: &WalletDerivedState,
    ) -> Option<crate::store::wallet_domain::AssetHolding> {
        if let Some(coin) = derived
            .included_portfolio_holdings
            .iter()
            .find(|c| c.symbol.eq_ignore_ascii_case(symbol))
        {
            return Some(coin.clone());
        }
        let preferences = self.wallet_state.read().await.token_preferences.clone();
        let entry = preferences
            .iter()
            .find(|e| e.token.symbol.eq_ignore_ascii_case(symbol))?;
        Some(crate::store::wallet_domain::AssetHolding {
            name: entry.token.name.clone(),
            symbol: entry.token.symbol.clone(),
            coin_gecko_id: entry.token.coingecko_id.clone(),
            chain_name: entry.token.chain.clone(),
            token_standard: entry.token.token_standard.clone(),
            contract_address: Some(entry.token.contract.clone()).filter(|c| !c.is_empty()),
            amount: 0.0,
            // No quote. A pinned asset the wallet does not hold has no price
            // until the feed answers for it, and inventing one puts a number
            // the user cannot tell from a real quote next to their funds.
            price_usd: 0.0,
        })
    }

    /// The bound state database, or an error naming what the caller skipped.
    pub(super) async fn bound_state_db_path(&self) -> Result<String, SpectraBridgeError> {
        self.state_db_path.read().await.clone().ok_or_else(|| {
            SpectraBridgeError::from(
                "transaction store not opened: call open_state first".to_string(),
            )
        })
    }
}

#[cfg(test)]
mod pruning_reads_cores_own_tables {
    use crate::registry::{Chain, PendingStatusPoll};

    /// A chain that stops at the first confirmation must not keep a tracker for
    /// a confirmed transaction.
    ///
    /// The filter this replaced lived in Swift and kept `pending` **or**
    /// `confirmed` for every chain, without asking the chain's poll shape — so
    /// on the chains that stop at one confirmation, every confirmed send held a
    /// tracker nothing would ever poll again.
    #[test]
    fn only_chains_that_count_depth_keep_confirmed_transactions() {
        let keeps_confirmed = |chain: Chain| {
            matches!(
                chain.pending_status_poll(),
                PendingStatusPoll::Utxo {
                    tracks_finality: true,
                    ..
                }
            )
        };
        assert!(keeps_confirmed(Chain::Dogecoin), "Dogecoin shows a depth");
        assert!(!keeps_confirmed(Chain::Litecoin));
        assert!(!keeps_confirmed(Chain::Bitcoin));

        // And a chain with no UTXO poll at all keeps nothing.
        assert!(!keeps_confirmed(Chain::Ethereum));
        assert!(!keeps_confirmed(Chain::Solana));
    }

    /// Pruning takes the stricter side when it cannot see the transactions.
    #[tokio::test]
    async fn pruning_refuses_rather_than_guessing() {
        let service = crate::service::WalletService::new_typed(Vec::new()).expect("service");
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
        let service = crate::service::WalletService::new_typed(Vec::new()).expect("service");
        let evm = Chain::Ethereum.str_id().to_string();
        assert!(service
            .discover_utxo_addresses("w".into(), evm.clone())
            .await
            .expect("ok")
            .is_empty());
        assert!(service
            .known_utxo_addresses("w".into(), evm.clone())
            .await
            .expect("ok")
            .is_empty());
        assert!(service
            .utxo_receive_address("w".into(), evm.clone(), false)
            .await
            .expect("ok")
            .is_none());
        service
            .advance_used_utxo_reservations(evm)
            .await
            .expect("a chain without the walk is a no-op, not an error");
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod performance_tests;
