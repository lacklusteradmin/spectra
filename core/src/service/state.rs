//! The resident domain state and its persistence: `CoreAppState` and the
//! commands that reduce it, the transaction store, the confirmation-poll
//! trackers, the keypool, and wallet import.
//!
//! A command persists before it returns, so no caller can forget to save —
//! that is how two copies of the truth start diverging.

use super::*;

/// Where the operational log is stored in the state key/value table.
const OPERATIONAL_EVENTS_KEY: &str = "operationalEvents.byChain.v1";

/// Keypool map key. A wallet has one keypool per chain.
fn keypool_key(wallet_id: &str, chain_name: &str) -> String {
    format!("{wallet_id}|{chain_name}")
}

fn record_from_keypool(
    state: &crate::wallet_db::KeypoolState,
) -> crate::store::ChainKeypoolStateRecord {
    crate::store::ChainKeypoolStateRecord {
        next_external_index: state.next_external_index as i32,
        next_change_index: state.next_change_index as i32,
        reserved_receive_index: state.reserved_receive_index.map(|i| i as i32),
    }
}

/// Store one keypool entry in memory and in SQLite. Caller holds the lock, so
/// the read-modify-write around this call stays atomic.
async fn persist_keypool(
    state_db_path: &Arc<RwLock<Option<String>>>,
    keypool: &mut HashMap<String, crate::wallet_db::KeypoolState>,
    key: String,
    wallet_id: &str,
    chain_name: &str,
    state: crate::wallet_db::KeypoolState,
) -> Result<(), SpectraBridgeError> {
    keypool.insert(key, state.clone());
    // Without a bound database the service runs in memory only — the shape
    // tests and short-lived tools. Nothing to write.
    let Some(db_path) = state_db_path.read().await.clone() else {
        return Ok(());
    };
    let (wallet_id, chain_name) = (wallet_id.to_string(), chain_name.to_string());
    tokio::task::spawn_blocking(move || {
        crate::wallet_db::keypool_save(&db_path, &wallet_id, &chain_name, &state)
    })
    .await
    .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
    .map_err(Into::into)
}

fn keypool_from_record(
    record: &crate::store::ChainKeypoolStateRecord,
) -> crate::wallet_db::KeypoolState {
    crate::wallet_db::KeypoolState {
        next_external_index: record.next_external_index as i64,
        next_change_index: record.next_change_index as i64,
        reserved_receive_index: record.reserved_receive_index.map(|i| i as i64),
    }
}
/// What one confirmation poll found.
///
/// Replaces a pair of methods and, inside the success arm, a pair of booleans:
/// `resolved_status_confirmed` and `resolved_status_pending` encoded three
/// states in two flags, so "confirmed and pending" type-checked and meant
/// nothing.
#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum StatusPollOutcome {
    /// The provider reported the transaction confirmed.
    Confirmed { confirmations: Option<u32> },
    /// The provider reported it still pending.
    Pending,
    /// The provider answered without resolving it either way.
    Unresolved,
    /// The poll itself failed — a network or provider error, not a verdict.
    Failed,
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Load the JSON state blob stored under `key` in the SQLite database at
    /// `db_path`. Returns an empty JSON object `"{}"` when no value has been
    /// saved yet. Thread-safe: rusqlite is called in `spawn_blocking`.
    pub async fn load_state(
        &self,
        key: String,
    ) -> Result<String, SpectraBridgeError> {
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



    /// Remove all keypool state for a wallet (called when a wallet is deleted).
    pub async fn delete_keypool_for_wallet(
        &self,
        wallet_id: String,
    ) -> Result<(), SpectraBridgeError> {
        let db_path = self.bound_state_db_path().await?;
        // Drop the in-memory rows too, or a reserve after this would still see
        // the deleted wallet's indices.
        {
            let suffix_owner = wallet_id.clone();
            let mut keypool = self.keypool.write().await;
            keypool.retain(|key, _| !key.starts_with(&format!("{suffix_owner}|")));
        }
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::keypool_delete_for_wallet(&db_path, &wallet_id)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(Into::into)
    }

    /// Remove all keypool state for a chain (called when the user switches network modes,
    /// triggering a rescan).
    pub async fn delete_keypool_for_chain(
        &self,
        chain_name: String,
    ) -> Result<(), SpectraBridgeError> {
        let db_path = self.bound_state_db_path().await?;
        // Switching network mode invalidates every index on the chain; the
        // in-memory copy has to go with the stored one.
        {
            let suffix = format!("|{chain_name}");
            let mut keypool = self.keypool.write().await;
            keypool.retain(|key, _| !key.ends_with(&suffix));
        }
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::keypool_delete_for_chain(&db_path, &chain_name)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(Into::into)
    }

    /// Record an address this wallet owns.
    ///
    /// Core holds the table rather than mirroring a caller's: the keypool
    /// baseline is derived from it, and a baseline computed from a stale copy
    /// reissues an address that was already handed out.
    pub async fn register_owned_address(
        &self,
        wallet_id: String,
        chain_name: String,
        address: String,
        derivation_path: Option<String>,
        branch: Option<String>,
        branch_index: Option<i64>,
    ) -> Result<(), SpectraBridgeError> {
        let address = address.trim().to_string();
        if address.is_empty() || wallet_id.is_empty() {
            return Ok(());
        }
        let record = crate::wallet_db::OwnedAddressRecord {
            wallet_id,
            chain_name: chain_name.clone(),
            address,
            derivation_path,
            branch,
            branch_index,
        };
        let mut table = self.owned_addresses.write().await;
        let rows = table.entry(chain_name).or_default();
        match rows.iter_mut().find(|existing| {
            existing.wallet_id == record.wallet_id && existing.address == record.address
        }) {
            Some(existing) => *existing = record.clone(),
            None => rows.push(record.clone()),
        }
        let Some(db_path) = self.state_db_path.read().await.clone() else {
            return Ok(());
        };
        tokio::task::spawn_blocking(move || crate::wallet_db::address_save(&db_path, &record))
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
            .map_err(Into::into)
    }

    /// The addresses this wallet owns — on one chain, or on every chain when
    /// `chain_name` is absent.
    pub async fn owned_addresses_for_wallet(
        &self,
        wallet_id: String,
        chain_name: Option<String>,
    ) -> Vec<String> {
        let table = self.owned_addresses.read().await;
        let rows: Box<dyn Iterator<Item = &crate::wallet_db::OwnedAddressRecord>> =
            match chain_name.as_deref() {
                Some(chain) => match table.get(chain) {
                    Some(rows) => Box::new(rows.iter()),
                    None => return Vec::new(),
                },
                None => Box::new(table.values().flatten()),
            };
        rows.filter(|r| r.wallet_id == wallet_id)
            .map(|r| r.address.clone())
            .collect()
    }

    /// Remove every owned address for a chain (a network-mode switch, or a
    /// full rescan). Clears the in-memory rows too, not just the stored ones.
    pub async fn delete_owned_addresses_for_chain(
        &self,
        chain_name: String,
    ) -> Result<(), SpectraBridgeError> {
        self.owned_addresses.write().await.remove(&chain_name);
        let Some(db_path) = self.state_db_path.read().await.clone() else {
            return Ok(());
        };
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::address_delete_for_chain(&db_path, &chain_name)
        })
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
        let db_path = self.bound_state_db_path().await?;
        // Clear the in-memory rows too. Leaving them means the keypool
        // baseline still counts a deleted wallet's addresses.
        self.keypool
            .write()
            .await
            .retain(|key, _| key.split_once('|').is_none_or(|(id, _)| id != wallet_id));
        for rows in self.owned_addresses.write().await.values_mut() {
            rows.retain(|row| row.wallet_id != wallet_id);
        }
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::delete_wallet_data(&db_path, &wallet_id)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(Into::into)
    }

    /// Upsert a batch of transaction history records. `records[*].payload`
    /// is the typed `CorePersistedTransactionRecord`; Rust serializes to JSON
    /// for the SQLite TEXT column internally — no JSON crosses the FFI.
    pub async fn upsert_history_records(
        &self,
        records: Vec<crate::wallet_db::HistoryRecord>,
    ) -> Result<(), SpectraBridgeError> {
        let db_path = self.bound_state_db_path().await?;
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::history_upsert_batch(&db_path, &records)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(Into::into)
    }

    pub async fn fetch_all_history_records_typed(
        &self,
    ) -> Result<Vec<crate::wallet_db::HistoryRecord>, SpectraBridgeError> {
        let db_path = self.bound_state_db_path().await?;
        tokio::task::spawn_blocking(move || crate::wallet_db::history_fetch_all(&db_path))
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
            .map_err(Into::into)
    }

    /// Delete history records by ID.
    pub async fn delete_history_records(
        &self,
        ids: Vec<String>,
    ) -> Result<(), SpectraBridgeError> {
        let db_path = self.bound_state_db_path().await?;
        tokio::task::spawn_blocking(move || crate::wallet_db::history_delete(&db_path, &ids))
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
            .map_err(Into::into)
    }

    /// Atomically replace ALL history records with the provided batch.
    /// Replace the whole history table.
    ///
    /// `clear_all_history_records()` was a second export for this with an empty
    /// slice: `history_replace_all` deletes every row before inserting, so the
    /// two ran the same `DELETE` and differed only in what followed it.
    pub async fn replace_all_history_records(
        &self,
        records: Vec<crate::wallet_db::HistoryRecord>,
    ) -> Result<(), SpectraBridgeError> {
        let db_path = self.bound_state_db_path().await?;
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::history_replace_all(&db_path, &records)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(Into::into)
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
        // Opening is idempotent. A second call with the same database returns
        // what is already held rather than re-reading — a late `open_state`
        // (the app's launch reload racing a user action) would otherwise
        // replace the in-memory state with a snapshot taken before the newer
        // command, silently reverting it.
        if self.state_db_path.read().await.as_deref() == Some(db_path.as_str()) {
            return Ok(self.wallet_state.read().await.clone());
        }

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
        *self.keypool.write().await = keypool
            .into_iter()
            .flat_map(|(chain, per_wallet)| {
                per_wallet
                    .into_iter()
                    .map(move |(wallet, state)| (keypool_key(&wallet, &chain), state))
            })
            .collect();

        let owned = {
            let path = db_path.clone();
            tokio::task::spawn_blocking(move || crate::wallet_db::address_load_all_chains(&path))
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
        *self.owned_addresses.write().await = by_chain;

        let events = {
            let path = db_path.clone();
            tokio::task::spawn_blocking(move || sqlite_load(&path, OPERATIONAL_EVENTS_KEY))
                .await
                .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))??
        };
        *self.operational_events.write().await = serde_json::from_str(&events).unwrap_or_default();

        *self.state_db_path.write().await = Some(db_path);
        let mut state = self.wallet_state.write().await;
        *state = loaded;
        Ok(state.clone())
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
        let (snapshot, events) = {
            let mut state = self.wallet_state.write().await;
            let events = reduce_state_in_place(&mut state, command);
            (state.clone(), events)
        };

        if !events.is_empty() {
            if let Some(path) = self.state_db_path.read().await.clone() {
                let to_save = snapshot.clone();
                tokio::task::spawn_blocking(move || {
                    crate::wallet_db::app_state_save(&path, &to_save)
                })
                .await
                .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))??;
            }
        }

        Ok(StateTransition {
            state: snapshot,
            events,
        })
    }

    // ── Operational events ────────────────────────────────────────────────

    /// Record something that happened on a chain — a broadcast accepted, a
    /// verification warning, a send failure.
    ///
    /// Core stamps the id and the time and applies the cap. The planner this
    /// replaces took the existing list in and handed a capped one back, so the
    /// list, its ordering and its bound were only as correct as whichever
    /// caller wrote the answer down.
    pub async fn append_chain_operational_event(
        &self,
        chain_name: String,
        level: crate::store::ChainOperationalEventLevel,
        message: String,
        transaction_hash: Option<String>,
    ) -> Result<(), SpectraBridgeError> {
        let event = crate::store::ChainOperationalEventRecord {
            id: crate::store::new_event_id(),
            timestamp_unix: crate::store::now_unix(),
            chain_name: chain_name.clone(),
            level,
            message,
            transaction_hash: transaction_hash.filter(|hash| !hash.trim().is_empty()),
        };
        let snapshot = {
            let mut table = self.operational_events.write().await;
            let existing = table.remove(&chain_name).unwrap_or_default();
            table.insert(
                chain_name,
                crate::store::plan_append_chain_operational_event(existing, event),
            );
            serde_json::to_string(&*table)?
        };
        self.persist_operational_events(snapshot).await
    }

    /// This chain's events, newest first.
    pub async fn operational_events(
        &self,
        chain_name: String,
    ) -> Vec<crate::store::ChainOperationalEventRecord> {
        self.operational_events
            .read()
            .await
            .get(&chain_name)
            .cloned()
            .unwrap_or_default()
    }

    /// Drop one chain's events, or every chain's when `chain_name` is absent.
    pub async fn clear_operational_events(
        &self,
        chain_name: Option<String>,
    ) -> Result<(), SpectraBridgeError> {
        let snapshot = {
            let mut table = self.operational_events.write().await;
            match chain_name {
                Some(chain) => {
                    table.remove(&chain);
                }
                None => table.clear(),
            }
            serde_json::to_string(&*table)?
        };
        self.persist_operational_events(snapshot).await
    }

    /// The dashboard's asset rows: holdings grouped across chains, ordered,
    /// with the pinned ones first.
    ///
    /// Live prices are the only input core does not have — everything else
    /// (which holdings count toward the total, which tokens are tracked, which
    /// symbols are pinned, which networks are unpriced, how a chain identifies
    /// an asset) is core's already. `prices` is keyed the way core keys an
    /// asset: `"<network title>|<symbol>"`.
    ///
    /// This was ~100 lines of grouping and sorting in the shell, reading five
    /// Swift caches. Grouping the same asset across chains and deciding row
    /// order are domain rules; the CLI could not reach either.
    pub async fn dashboard_asset_groups(
        &self,
        prices: HashMap<String, f64>,
    ) -> Result<Vec<crate::store::wallet_domain::CoreDashboardAssetGroup>, SpectraBridgeError> {
        use crate::store::wallet_domain::{CoreDashboardAssetChainEntry, CoreDashboardAssetGroup};

        let settings = self.wallet_state.read().await.settings.clone();
        let derived = self.wallet_derived_state(Vec::new(), Vec::new()).await?;
        let pinned = &settings.pinned_dashboard_asset_symbols;

        let network_title = |chain_name: &str| -> String {
            crate::registry::Chain::from_display_name(chain_name)
                .map(|chain| settings.network_chain(chain).chain_display_name().to_string())
                .unwrap_or_else(|| chain_name.to_string())
        };
        // Unpriced on a testnet, then the live quote, then the amount the
        // holding was last stored with. Same order the shell applied.
        let value_of = |coin: &crate::store::wallet_domain::CoreCoin| -> Option<f64> {
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

        let mut order: Vec<String> = Vec::new();
        let mut grouped: HashMap<String, Vec<crate::store::wallet_domain::CoreCoin>> =
            HashMap::new();
        for coin in derived
            .included_portfolio_holdings
            .iter()
            .filter(|c| c.amount > 0.0)
        {
            let key = crate::formatting::dashboard_asset_grouping_key(
                &network_title(&coin.chain_name),
                &coin.coin_gecko_id,
                &coin.symbol,
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
            // One row per (chain, standard, contract): the same token held on
            // two wallets is one entry with the amounts summed.
            let mut chain_order: Vec<String> = Vec::new();
            let mut by_chain: HashMap<String, crate::store::wallet_domain::CoreCoin> =
                HashMap::new();
            for coin in coins {
                let contract = crate::tokens::normalize_token_identifier(
                    coin.contract_address.clone(),
                    coin.chain_name.clone(),
                )
                .unwrap_or_else(|| "native".to_string());
                let chain_key = format!(
                    "{}|{}|{contract}",
                    network_title(&coin.chain_name).to_lowercase(),
                    coin.token_standard.to_lowercase()
                );
                match by_chain.get_mut(&chain_key) {
                    Some(existing) => {
                        existing.amount += coin.amount;
                        existing.price_usd = coin.price_usd;
                    }
                    None => {
                        chain_order.push(chain_key.clone());
                        by_chain.insert(chain_key, coin.clone());
                    }
                }
            }
            let mut chain_entries: Vec<CoreDashboardAssetChainEntry> = chain_order
                .iter()
                .filter_map(|k| by_chain.get(k))
                .map(|coin| CoreDashboardAssetChainEntry {
                    value_usd: value_of(coin),
                    coin: coin.clone(),
                })
                .collect();
            chain_entries.sort_by(|lhs, rhs| {
                let (l, r) = (lhs.value_usd.unwrap_or(-1.0), rhs.value_usd.unwrap_or(-1.0));
                if (l - r).abs() > 0.000_001 {
                    return r.total_cmp(&l);
                }
                lhs.coin
                    .chain_name
                    .to_lowercase()
                    .cmp(&rhs.coin.chain_name.to_lowercase())
            });
            let Some(representative) = chain_entries.first().map(|e| e.coin.clone()) else {
                continue;
            };
            let total_value_usd = chain_entries
                .iter()
                .map(|e| e.value_usd)
                .try_fold(0.0, |sum, v| v.map(|v| sum + v));
            groups.push(CoreDashboardAssetGroup {
                id: key,
                is_pinned: pinned.contains(&representative.symbol.to_uppercase()),
                total_amount: coins.iter().map(|c| c.amount).sum(),
                total_value_usd,
                chain_entries,
                representative_coin: representative,
            });
        }

        // A pinned symbol the user holds none of still gets a row.
        let present: std::collections::HashSet<String> = groups
            .iter()
            .map(|g| g.representative_coin.symbol.to_uppercase())
            .collect();
        for symbol in pinned.iter().filter(|s| !present.contains(*s)) {
            let Some(prototype) = self.pinned_prototype(symbol, &derived).await else {
                continue;
            };
            groups.push(CoreDashboardAssetGroup {
                id: format!("pinned:{}", symbol.to_lowercase()),
                representative_coin: prototype,
                total_amount: 0.0,
                total_value_usd: Some(0.0),
                chain_entries: Vec::new(),
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
                        .get(lhs.representative_coin.symbol.to_uppercase().as_str())
                        .copied()
                        .unwrap_or(usize::MAX);
                    let r = pin_order
                        .get(rhs.representative_coin.symbol.to_uppercase().as_str())
                        .copied()
                        .unwrap_or(usize::MAX);
                    return l.cmp(&r);
                }
                (false, false) => {}
            }
            let (l, r) = (
                lhs.total_value_usd.unwrap_or(-1.0),
                rhs.total_value_usd.unwrap_or(-1.0),
            );
            if (l - r).abs() > 0.000_001 {
                return r.total_cmp(&l);
            }
            lhs.representative_coin
                .symbol
                .to_lowercase()
                .cmp(&rhs.representative_coin.symbol.to_lowercase())
        });
        Ok(groups)
    }

    /// Fold this build's built-in token catalog into the stored preferences
    /// and keep the result.
    ///
    /// A user's `is_enabled` and `display_decimals` survive; tokens the build
    /// added appear; tokens the user added stay. The caller used to fetch the
    /// catalog from core, reshape it, send both lists back for merging and
    /// assign the answer — core owns both sides, so it does all of it.
    pub async fn merge_built_in_token_preferences(
        &self,
    ) -> Result<CoreAppState, SpectraBridgeError> {
        let persisted = self.wallet_state.read().await.token_preferences.clone();
        let merged = crate::store::plan_merge_built_in_token_preferences(
            crate::store::built_in_token_preferences(),
            persisted,
        );
        Ok(self
            .apply_state_command(StateCommand::SetTokenPreferences { entries: merged })
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

    /// Change a command made to the transaction store. Ids, not records —
    /// callers re-read only what they need.
    pub async fn apply_transaction_command(
        &self,
        command: TransactionCommand,
    ) -> Result<TransactionChange, SpectraBridgeError> {
        let db_path = self.bound_state_db_path().await?;

        tokio::task::spawn_blocking(move || -> Result<TransactionChange, String> {
            match command {
                TransactionCommand::Upsert { records } => {
                    if records.is_empty() {
                        return Ok(TransactionChange::default());
                    }
                    let rows: Vec<crate::wallet_db::HistoryRecord> = records
                        .into_iter()
                        .map(crate::wallet_db::history_record_from_payload)
                        .collect();
                    let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
                    let existing = crate::wallet_db::history_existing_ids(&db_path, &ids)?;
                    crate::wallet_db::history_upsert_batch(&db_path, &rows)?;
                    let existing: std::collections::HashSet<String> =
                        existing.into_iter().collect();
                    let (updated, added): (Vec<String>, Vec<String>) =
                        ids.into_iter().partition(|id| existing.contains(id));
                    Ok(TransactionChange {
                        added,
                        updated,
                        removed: Vec::new(),
                    })
                }
                TransactionCommand::Merge {
                    incoming,
                    chain_name,
                    preserve_created_at_sentinel_unix,
                } => {
                    let chain = Chain::from_display_name(&chain_name)
                        .ok_or_else(|| format!("merge: unknown chain {chain_name:?}"))?;
                    let strategy = chain.transaction_merge_strategy();
                    let include_symbol_in_identity = chain.merge_identity_includes_symbol();
                    let existing: Vec<crate::fetch::transactions::CoreTransactionRecord> =
                        crate::wallet_db::history_fetch_all(&db_path)?
                            .into_iter()
                            .map(|row| row.payload.into())
                            .collect();
                    let before: std::collections::HashMap<String, String> = existing
                        .iter()
                        .map(|record| (record.id.to_lowercase(), fingerprint(record)))
                        .collect();

                    let merged = crate::fetch::transactions::merge_transactions(
                        crate::fetch::transactions::TransactionMergeRequest {
                            existing_transactions: existing,
                            incoming_transactions: incoming,
                            strategy,
                            chain_name,
                            include_symbol_in_identity,
                            preserve_created_at_sentinel_unix,
                        },
                    );

                    // Only records the merge actually altered are written — a
                    // history refresh mostly returns what is already stored.
                    let mut added = Vec::new();
                    let mut updated = Vec::new();
                    let mut rows = Vec::new();
                    for record in merged {
                        let id = record.id.to_lowercase();
                        match before.get(&id) {
                            Some(previous) if *previous == fingerprint(&record) => continue,
                            Some(_) => updated.push(id),
                            None => added.push(id),
                        }
                        rows.push(crate::wallet_db::history_record_from_payload(record.into()));
                    }
                    if !rows.is_empty() {
                        crate::wallet_db::history_upsert_batch(&db_path, &rows)?;
                    }
                    Ok(TransactionChange {
                        added,
                        updated,
                        removed: Vec::new(),
                    })
                }
                TransactionCommand::Remove { ids } => {
                    if ids.is_empty() {
                        return Ok(TransactionChange::default());
                    }
                    let ids: Vec<String> = ids.iter().map(|id| id.to_lowercase()).collect();
                    let removed = crate::wallet_db::history_existing_ids(&db_path, &ids)?;
                    crate::wallet_db::history_delete(&db_path, &ids)?;
                    Ok(TransactionChange {
                        removed,
                        ..TransactionChange::default()
                    })
                }
                TransactionCommand::RemoveForWallet { wallet_id } => {
                    let removed: Vec<String> =
                        crate::wallet_db::history_fetch_for_wallet(&db_path, &wallet_id)?
                            .into_iter()
                            .map(|record| record.id)
                            .collect();
                    crate::wallet_db::history_delete_for_wallet(&db_path, &wallet_id)?;
                    Ok(TransactionChange {
                        removed,
                        ..TransactionChange::default()
                    })
                }
                TransactionCommand::Clear => {
                    let removed: Vec<String> = crate::wallet_db::history_fetch_all(&db_path)?
                        .into_iter()
                        .map(|record| record.id)
                        .collect();
                    crate::wallet_db::history_clear(&db_path)?;
                    Ok(TransactionChange {
                        removed,
                        ..TransactionChange::default()
                    })
                }
            }
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(Into::into)
    }

    /// Every stored transaction, newest first.
    pub async fn transactions(
        &self,
    ) -> Result<
        Vec<crate::store::persistence_models::CorePersistedTransactionRecord>,
        SpectraBridgeError,
    > {
        let db_path = self.bound_state_db_path().await?;
        tokio::task::spawn_blocking(move || crate::wallet_db::history_fetch_all(&db_path))
            .await
            .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
            .map(|rows| rows.into_iter().map(|row| row.payload).collect())
            .map_err(Into::into)
    }

    /// Which of `transaction_ids` are due for a confirmation poll now.
    ///
    /// An untracked transaction is always due — that is what makes a fresh
    /// launch re-poll everything pending.
    pub async fn transactions_due_for_status_poll(
        &self,
        transaction_ids: Vec<String>,
    ) -> Vec<String> {
        let now_unix = crate::store::wallet_db::now_secs() as f64;
        let trackers = self.status_trackers.read().await;
        transaction_ids
            .into_iter()
            .filter(|id| {
                crate::store::plan_transaction_status_should_poll(
                    trackers.get(id).cloned(),
                    now_unix,
                )
            })
            .collect()
    }

    /// Record the outcome of one confirmation poll.
    ///
    /// Two methods before, and the success arm took `resolved_status_confirmed`
    /// and `resolved_status_pending` as separate booleans — a three-state
    /// written as two, so "confirmed and pending" was representable and had no
    /// meaning. The outcome is the outcome.
    pub async fn record_status_poll(&self, transaction_id: String, outcome: StatusPollOutcome) {
        let now_unix = crate::store::wallet_db::now_secs() as f64;
        let mut trackers = self.status_trackers.write().await;
        let previous = trackers.get(&transaction_id).cloned();
        let next = match outcome {
            StatusPollOutcome::Failed => crate::store::plan_transaction_status_poll_failure(
                previous,
                now_unix,
                TransactionStatusPollConfig::default(),
            ),
            StatusPollOutcome::Confirmed { confirmations } => {
                crate::store::plan_transaction_status_poll_success(
                    previous,
                    true,
                    false,
                    confirmations,
                    now_unix,
                    TransactionStatusPollConfig::default(),
                )
            }
            StatusPollOutcome::Pending => crate::store::plan_transaction_status_poll_success(
                previous,
                false,
                true,
                None,
                now_unix,
                TransactionStatusPollConfig::default(),
            ),
            StatusPollOutcome::Unresolved => crate::store::plan_transaction_status_poll_success(
                previous,
                false,
                false,
                None,
                now_unix,
                TransactionStatusPollConfig::default(),
            ),
        };
        trackers.insert(transaction_id, next);
    }

    /// Force `transaction_id` to be polled on the next sweep.
    ///
    /// `clear_finality` re-opens a transaction that had already been treated as
    /// final — the UTXO chains do this when a reorg is suspected.
    pub async fn reset_status_tracker(
        &self,
        transaction_id: String,
        clear_finality: bool,
    ) {
        let now_unix = crate::store::wallet_db::now_secs() as f64;
        let mut trackers = self.status_trackers.write().await;
        let entry = trackers
            .entry(transaction_id)
            .or_insert_with(|| TransactionStatusTrackerState::initial(now_unix));
        entry.next_check_at_unix = f64::NEG_INFINITY;
        if clear_finality {
            entry.reached_finality = false;
        }
    }

    /// Drop trackers for transactions that no longer exist.
    /// Keep only these trackers and forget the rest.
    ///
    /// `clear_status_trackers()` was a second name for this with an empty list,
    /// and one call site already spelled it that way.
    pub async fn retain_status_trackers(&self, transaction_ids: Vec<String>) {
        let keep: std::collections::HashSet<String> = transaction_ids.into_iter().collect();
        self.status_trackers
            .write()
            .await
            .retain(|id, _| keep.contains(id));
    }


    /// Pending transactions old enough, and failing often enough, to be treated
    /// as failed. Failure counts come from core's own trackers.
    pub async fn stale_pending_failure_ids(
        &self,
        transactions: Vec<crate::store::StalePendingFailureTransactionInput>,
    ) -> Vec<String> {
        let failures: HashMap<String, u32> = self
            .status_trackers
            .read()
            .await
            .iter()
            .map(|(id, tracker)| (id.clone(), tracker.consecutive_failures))
            .collect();
        crate::store::plan_stale_pending_failure_ids(
            transactions,
            &failures,
            crate::store::wallet_db::now_secs() as f64,
            TransactionStatusPollConfig::default(),
        )
    }

    /// Decide what each resolved pending transaction becomes, advancing the
    /// confirmation trackers as a side effect.
    pub async fn apply_resolved_pending_transaction_statuses(
        &self,
        inputs: Vec<crate::store::ResolvedPendingTransactionInput>,
    ) -> Vec<crate::store::ResolvedPendingTransactionDecision> {
        let now_unix = crate::store::wallet_db::now_secs() as f64;
        let mut trackers = self.status_trackers.write().await;
        crate::store::plan_apply_resolved_pending_transaction_statuses(
            inputs,
            &mut trackers,
            now_unix,
            TransactionStatusPollConfig::default(),
        )
    }

    /// Import wallets: plan them, build them, and store them.
    ///
    /// Replaces the old `core_plan_wallet_import` round trip, where core
    /// decided what to create and the caller constructed and stored it.
    /// Secrets are not touched here — `secret_instructions` in the outcome
    /// tells the platform what to write to its own keystore.
    pub async fn import_wallets(
        &self,
        commit: crate::derivation::import::WalletImportCommit,
    ) -> Result<crate::derivation::import::WalletImportOutcome, SpectraBridgeError> {
        // One validation rule for every chain, applied before planning so a
        // malformed address cannot reach storage. Both inputs carry addresses:
        // `resolved_addresses` for a signing import, `watch_only_entries` for a
        // watch-only one. Validating only the first covered the path whose
        // address core derived itself and skipped the path where the user
        // typed it.
        let mut commit = commit;
        // The two inputs carry addresses of different provenance, so they are
        // judged against different networks.
        //
        // `resolved_addresses` holds what the caller *derived*, and derivation
        // runs against the mainnet chain whatever network mode is selected — a
        // testnet wallet stores a mainnet-format address and re-derives the
        // testnet one for display. Judging it by the selected mode would drop
        // every address on a testnet import.
        //
        // `watch_only_entries` holds what the user *typed*, for the network
        // they are on, and `ImportDraft` has no testnet row to put it in — so
        // a testnet address arrives in the mainnet slot and only the mode says
        // how to read it.
        let typed_networks = crate::derivation::import::ImportNetworks {
            by_family: commit.network_chain_by_family.clone(),
        };
        let (validated, mut rejected_addresses) = crate::derivation::import::validated_addresses(
            &commit.request.resolved_addresses,
            &crate::derivation::import::ImportNetworks::default(),
        );
        commit.request.resolved_addresses = validated;
        let (validated_watch_only, rejected_watch_only) =
            crate::derivation::import::validated_watch_only_entries(
                &commit.request.watch_only_entries,
                &typed_networks,
            );
        commit.request.watch_only_entries = validated_watch_only;
        rejected_addresses.extend(rejected_watch_only);

        // A plan that fails *because* validation emptied the input is a refusal
        // of what the caller supplied, not an internal failure — say which
        // address was refused, and classify it so a caller can tell the two
        // apart without reading the message.
        let plan = match crate::derivation::import::plan_wallet_import(commit.request.clone()) {
            Ok(plan) => plan,
            Err(message) if !rejected_addresses.is_empty() => {
                return Err(SpectraBridgeError::InvalidInput {
                    message: format!("{message} Rejected: {}", rejected_addresses.join(", ")),
                })
            }
            Err(message) => return Err(SpectraBridgeError::from(message)),
        };
        let wallets = crate::derivation::import::wallets_for_import(&commit, &plan);
        let is_watch_only = commit.request.is_watch_only_import;
        for wallet in &wallets {
            self.apply_state_command(StateCommand::UpsertWallet {
                wallet: wallet.to_summary(is_watch_only),
            })
            .await?;
        }
        Ok(crate::derivation::import::WalletImportOutcome {
            secret_kind: plan.secret_kind,
            secret_instructions: plan.secret_instructions,
            wallets,
            rejected_addresses,
        })
    }

    // ── Keypool ───────────────────────────────────────────────────────────
    //
    // Reserving an index is read-modify-write. Doing that across an FFI round
    // trip is a race — two callers read the same index and both hand it out,
    // which on a UTXO chain means the same receive address given to two
    // people. Every mutation below holds the lock for the whole operation and
    // writes through to SQLite before returning.

    /// The wallet's keypool for a chain, merged with the baseline and recorded.
    ///
    /// The baseline is core's own: it comes from the transactions, owned
    /// addresses and wallet addresses core already holds. A caller used to
    /// compute it and pass it in, which meant the guarantee this lock provides
    /// depended on the caller's copy of three tables being current.
    pub async fn keypool_state(
        &self,
        wallet_id: String,
        chain_name: String,
    ) -> Result<crate::wallet_db::KeypoolState, SpectraBridgeError> {
        let baseline = self.chain_keypool_baseline(&wallet_id, &chain_name).await;
        let key = keypool_key(&wallet_id, &chain_name);
        let mut keypool = self.keypool.write().await;
        let merged = crate::store::plan_chain_keypool_state(
            baseline,
            keypool.get(&key).map(record_from_keypool),
        );
        let next = keypool_from_record(&merged);
        persist_keypool(
            &self.state_db_path,
            &mut keypool,
            key,
            &wallet_id,
            &chain_name,
            next.clone(),
        )
        .await?;
        Ok(next)
    }

    /// Reserve the next receive index, or return the one already reserved.
    pub async fn reserve_receive_index(
        &self,
        wallet_id: String,
        chain_name: String,
        minimum_index: i64,
    ) -> Result<i64, SpectraBridgeError> {
        let baseline = self.chain_keypool_baseline(&wallet_id, &chain_name).await;
        let key = keypool_key(&wallet_id, &chain_name);
        let mut keypool = self.keypool.write().await;
        let merged = crate::store::plan_chain_keypool_state(
            baseline,
            keypool.get(&key).map(record_from_keypool),
        );
        let mut state = keypool_from_record(&merged);
        if let Some(reserved) = state.reserved_receive_index {
            // Already reserved: hand back the same index rather than burning a
            // new one every time the receive sheet opens.
            persist_keypool(
                &self.state_db_path,
                &mut keypool,
                key,
                &wallet_id,
                &chain_name,
                state,
            )
            .await?;
            return Ok(reserved);
        }
        let reserved = state.next_external_index.max(minimum_index);
        state.reserved_receive_index = Some(reserved);
        state.next_external_index = state.next_external_index.max(reserved + 1);
        persist_keypool(
            &self.state_db_path,
            &mut keypool,
            key,
            &wallet_id,
            &chain_name,
            state,
        )
        .await?;
        Ok(reserved)
    }

    /// Reserve the next change index. Always consumes one.
    pub async fn reserve_change_index(
        &self,
        wallet_id: String,
        chain_name: String,
    ) -> Result<i64, SpectraBridgeError> {
        let baseline = self.chain_keypool_baseline(&wallet_id, &chain_name).await;
        let key = keypool_key(&wallet_id, &chain_name);
        let mut keypool = self.keypool.write().await;
        let merged = crate::store::plan_chain_keypool_state(
            baseline,
            keypool.get(&key).map(record_from_keypool),
        );
        let mut state = keypool_from_record(&merged);
        let reserved = state.next_change_index.max(0);
        state.next_change_index = reserved + 1;
        persist_keypool(
            &self.state_db_path,
            &mut keypool,
            key,
            &wallet_id,
            &chain_name,
            state,
        )
        .await?;
        Ok(reserved)
    }

    /// Release the reserved receive index once its address has been used.
    pub async fn clear_reserved_receive_index(
        &self,
        wallet_id: String,
        chain_name: String,
    ) -> Result<(), SpectraBridgeError> {
        let key = keypool_key(&wallet_id, &chain_name);
        let mut keypool = self.keypool.write().await;
        let Some(mut state) = keypool.get(&key).cloned() else {
            return Ok(());
        };
        state.reserved_receive_index = None;
        persist_keypool(
            &self.state_db_path,
            &mut keypool,
            key,
            &wallet_id,
            &chain_name,
            state,
        )
        .await
    }

    /// The keypool a wallet would get, recording nothing.
    ///
    /// The mutating `keypool_state` writes, and a SwiftUI `body` that calls it
    /// to report state ends up changing what it reports.
    pub async fn keypool_state_for_display(
        &self,
        wallet_id: String,
        chain_name: String,
    ) -> crate::wallet_db::KeypoolState {
        let baseline = self.chain_keypool_baseline(&wallet_id, &chain_name).await;
        let key = keypool_key(&wallet_id, &chain_name);
        let keypool = self.keypool.read().await;
        keypool_from_record(&crate::store::plan_chain_keypool_state(
            baseline,
            keypool.get(&key).map(record_from_keypool),
        ))
    }


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
    ///
    /// The selected network used to be a third input, passed as a
    /// `NetworkModes` record of three chain names. It is a core-owned setting
    /// now, so core reads it — which also means a testnet on *any* family is
    /// unpriced, where the record's rule listed two families by hand and quoted
    /// Dogecoin testnet at mainnet prices.
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
        let mut grouped_representative: BTreeMap<String, crate::store::wallet_domain::CoreCoin> =
            BTreeMap::new();

        let mut send_coins_by_wallet_id = HashMap::new();
        let mut receive_coins_by_wallet_id = HashMap::new();
        let mut receive_chains_by_wallet_id = HashMap::new();
        let mut send_enabled_wallet_ids = Vec::new();
        let mut receive_enabled_wallet_ids = Vec::new();

        for wallet in &wallets {
            let has_signing_material = signing.contains(wallet.id.as_str());
            let mut send_coins = Vec::new();
            let mut receive_coins = Vec::new();
            let mut receive_chains: Vec<String> = Vec::new();

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
                    if !receive_chains.contains(&holding.chain_name) {
                        receive_chains.push(holding.chain_name.clone());
                    }
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
            receive_chains_by_wallet_id.insert(wallet.id.clone(), receive_chains);
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
            receive_chains_by_wallet_id,
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

/// Cheap content signature, to tell an unchanged merge result from a real one.
/// Serialization is enough: these records are flat and compare by value.
fn fingerprint(record: &crate::fetch::transactions::CoreTransactionRecord) -> String {
    serde_json::to_string(record).unwrap_or_default()
}

impl WalletService {
    // ── Not exported ──────────────────────────────────────────────────────
    //
    // Reachable from Rust — the CLI, or core itself — and from nothing across
    // the boundary. A method in the block above is an entry point whether or
    // not a platform uses it, and these were entry points nobody had taken.

    /// Stored transactions for one wallet, newest first.
    pub async fn transactions_for_wallet(
        &self,
        wallet_id: String,
    ) -> Result<
        Vec<crate::store::persistence_models::CorePersistedTransactionRecord>,
        SpectraBridgeError,
    > {
        let db_path = self.bound_state_db_path().await?;
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::history_fetch_for_wallet(&db_path, &wallet_id)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map(|rows| rows.into_iter().map(|row| row.payload).collect())
        .map_err(Into::into)
    }

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
    /// if one exists at zero, else a tracked token, else nothing.
    async fn pinned_prototype(
        &self,
        symbol: &str,
        derived: &WalletDerivedState,
    ) -> Option<crate::store::wallet_domain::CoreCoin> {
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
            .find(|e| e.symbol.eq_ignore_ascii_case(symbol))?;
        Some(crate::store::wallet_domain::CoreCoin {
            id: format!("tracked:{}:{}", entry.chain.chain_name(), entry.symbol),
            name: entry.name.clone(),
            symbol: entry.symbol.clone(),
            coin_gecko_id: entry.coin_gecko_id.clone(),
            chain_name: entry.chain.chain_name().to_string(),
            token_standard: entry.token_standard.clone(),
            contract_address: Some(entry.contract_address.clone())
                .filter(|c| !c.is_empty()),
            amount: 0.0,
            price_usd: crate::formatting::stablecoin_fallback_price_usd(&entry.symbol),
        })
    }

    async fn persist_operational_events(&self, snapshot: String) -> Result<(), SpectraBridgeError> {
        let Some(db_path) = self.state_db_path.read().await.clone() else {
            return Ok(());
        };
        tokio::task::spawn_blocking(move || {
            sqlite_save(&db_path, OPERATIONAL_EVENTS_KEY, &snapshot)
        })
        .await
        .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
        .map_err(Into::into)
    }

    /// The floor a wallet's keypool must respect on a chain, from what core
    /// already knows was handed out.
    ///
    /// Deep-UTXO chains take the highest index seen in a transaction's source
    /// or change path and the highest recorded owned-address index; everything
    /// else has one address, so the only question is whether the wallet has it
    /// yet.
    pub(crate) async fn chain_keypool_baseline(
        &self,
        wallet_id: &str,
        chain_name: &str,
    ) -> crate::store::ChainKeypoolStateRecord {
        let supports_deep = crate::registry::Chain::from_display_name(chain_name)
            .is_some_and(|chain| chain.supports_deep_utxo_discovery());

        let mut input = crate::store::ChainKeypoolBaselineInput {
            supports_deep_utxo_discovery: supports_deep,
            max_transaction_external_index: None,
            max_transaction_change_index: None,
            max_owned_external_index: None,
            max_owned_change_index: None,
            has_resolved_address: false,
        };

        if !supports_deep {
            if let Some(chain) = crate::registry::Chain::from_display_name(chain_name) {
                let state = self.wallet_state.read().await;
                input.has_resolved_address = state
                    .wallets
                    .iter()
                    .find(|w| w.id == wallet_id)
                    .and_then(|w| w.address_on(chain))
                    .is_some_and(|address| !address.trim().is_empty());
            }
            return crate::store::plan_baseline_chain_keypool_state(input);
        }

        let records = self
            .transactions_for_wallet(wallet_id.to_string())
            .await
            .unwrap_or_default();
        let index_of = |path: &Option<String>, branch: u32| -> Option<u32> {
            path.as_deref()
                .and_then(|p| crate::app_core::utxo_discovery_index(p, chain_name, branch))
        };
        input.max_transaction_external_index = records
            .iter()
            .filter(|r| r.chain_name == chain_name)
            .filter_map(|r| index_of(&r.source_derivation_path, 0))
            .max()
            .map(|v| v as i32);
        input.max_transaction_change_index = records
            .iter()
            .filter(|r| r.chain_name == chain_name)
            .filter_map(|r| index_of(&r.change_derivation_path, 1))
            .max()
            .map(|v| v as i32);

        let owned = self.owned_addresses.read().await;
        if let Some(rows) = owned.get(chain_name) {
            let for_wallet = rows.iter().filter(|r| r.wallet_id == wallet_id);
            let (mut external, mut change) = (Vec::new(), Vec::new());
            for row in for_wallet {
                let Some(index) = row.branch_index else {
                    continue;
                };
                match row.branch.as_deref() {
                    Some("external") => external.push(index),
                    Some("change") => change.push(index),
                    _ => {}
                }
            }
            input.max_owned_external_index = external.into_iter().max().map(|v| v as i32);
            input.max_owned_change_index = change.into_iter().max().map(|v| v as i32);
        }

        crate::store::plan_baseline_chain_keypool_state(input)
    }

    /// The bound state database, or an error naming what the caller skipped.
    async fn bound_state_db_path(&self) -> Result<String, SpectraBridgeError> {
        self.state_db_path.read().await.clone().ok_or_else(|| {
            SpectraBridgeError::from(
                "transaction store not opened: call open_state first".to_string(),
            )
        })
    }
}
