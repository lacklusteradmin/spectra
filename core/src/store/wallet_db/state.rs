use super::*;

// ── App state (wallets + settings) ────────────────────────────────────────────
//
// This is the persistence layer for `store::state::CoreAppState` — the
// chain-agnostic wallet model. Before it existed, `CoreAppState` had no home in
// Rust at all: iOS persisted its own Swift-side model and the CLI wrote its own
// `wallets.json`, so "the wallet list" had two incompatible on-disk shapes and
// neither belonged to the core.
//
// Storage follows the `history_records` house style: identity and query columns
// are promoted, the rest of the record rides along as JSON in `payload`. Wallet
// order is explicit in `sort_index` because `CoreAppState.wallets` is a `Vec`
// and its order is user-visible.

pub(super) const META_SCHEMA_VERSION: &str = "schema_version";
pub(super) const META_SELECTED_WALLET_ID: &str = "selected_wallet_id";
pub(super) const META_SETTINGS: &str = "settings";
/// Known tokens. A separate meta row rather than a field inside `settings`,
/// because `AppSettings` is the "every front end must agree" bag and this is a
/// list the user edits.
pub(super) const META_TOKEN_PREFERENCES: &str = "token_preferences";
pub(super) const META_PRICE_ALERTS: &str = "price_alerts";
/// USD → display-currency cross rates. Its own row for the same reason as the
/// two above: it is written by a refresh rather than by a settings edit, so
/// folding it into `settings` would make every rate refresh rewrite the bag
/// every front end agrees on.
pub(super) const META_FIAT_RATES: &str = "fiat_rates_from_usd";

/// Serialized write set, prepared against the last committed state while the
/// service writer is held. Unchanged collections are neither encoded nor written.
pub(crate) struct AppStateChanges {
    replace: bool,
    reset_chains: Vec<String>,
    wallets: Vec<(usize, WalletSummary, String)>,
    removed_wallets: Vec<String>,
    addresses: Vec<(usize, AddressBookEntry, String)>,
    removed_addresses: Vec<String>,
    meta: Vec<(&'static str, Option<String>)>,
}

impl AppStateChanges {
    pub(crate) fn between(
        before: Option<&CoreAppState>,
        after: &CoreAppState,
    ) -> Result<Self, String> {
        let old_wallets: std::collections::HashMap<_, _> = before
            .into_iter()
            .flat_map(|state| state.wallets.iter().enumerate())
            .map(|(index, wallet)| (wallet.id.as_str(), (index, wallet)))
            .collect();
        let old_addresses: std::collections::HashMap<_, _> = before
            .into_iter()
            .flat_map(|state| state.address_book.iter().enumerate())
            .map(|(index, entry)| (entry.id.as_str(), (index, entry)))
            .collect();
        let mut changes = Self {
            replace: before.is_none(),
            reset_chains: before
                .map(|b| changed_network_chains(b, after))
                .unwrap_or_default(),
            wallets: vec![],
            removed_wallets: vec![],
            addresses: vec![],
            removed_addresses: vec![],
            meta: vec![],
        };
        let mut remaining_wallets = old_wallets;
        for (index, wallet) in after.wallets.iter().enumerate() {
            if remaining_wallets.remove(wallet.id.as_str()) != Some((index, wallet)) {
                changes.wallets.push((
                    index,
                    wallet.clone(),
                    serde_json::to_string(wallet).map_err(|e| e.to_string())?,
                ));
            }
        }
        changes
            .removed_wallets
            .extend(remaining_wallets.into_keys().map(str::to_owned));
        let mut remaining_addresses = old_addresses;
        for (index, entry) in after.address_book.iter().enumerate() {
            if remaining_addresses.remove(entry.id.as_str()) != Some((index, entry)) {
                changes.addresses.push((
                    index,
                    entry.clone(),
                    serde_json::to_string(entry).map_err(|e| e.to_string())?,
                ));
            }
        }
        changes
            .removed_addresses
            .extend(remaining_addresses.into_keys().map(str::to_owned));
        macro_rules! json_field {
            ($field:ident, $key:expr) => {
                if before.map(|state| &state.$field) != Some(&after.$field) {
                    changes.meta.push((
                        $key,
                        Some(serde_json::to_string(&after.$field).map_err(|e| e.to_string())?),
                    ));
                }
            };
        }
        json_field!(diagnostics, "diagnostics");
        json_field!(schema_version, META_SCHEMA_VERSION);
        json_field!(settings, META_SETTINGS);
        json_field!(token_preferences, META_TOKEN_PREFERENCES);
        json_field!(price_alerts, META_PRICE_ALERTS);
        json_field!(fiat_rates_from_usd, META_FIAT_RATES);
        json_field!(quotes, "quotes");
        if before.map(|state| &state.selected_wallet_id) != Some(&after.selected_wallet_id) {
            changes
                .meta
                .push((META_SELECTED_WALLET_ID, after.selected_wallet_id.clone()));
        }
        Ok(changes)
    }

    pub(crate) fn save(self, db_path: &str) -> Result<(), String> {
        with_conn(db_path, |conn| {
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            let updated_at = now_secs();
            if self.replace {
                tx.execute("DELETE FROM wallets", [])
                    .map_err(|e| e.to_string())?;
                tx.execute("DELETE FROM address_book", [])
                    .map_err(|e| e.to_string())?;
            }
            for chain in self.reset_chains {
                for table in ["wallet_keypool", "wallet_owned_addresses"] {
                    tx.execute(
                        &format!("DELETE FROM {table} WHERE chain_name = ?1"),
                        params![chain],
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
            for id in self.removed_wallets {
                for table in ["wallet_keypool", "wallet_owned_addresses"] {
                    tx.execute(
                        &format!("DELETE FROM {table} WHERE wallet_id = ?1"),
                        params![id],
                    )
                    .map_err(|e| e.to_string())?;
                }
                tx.execute(
                    "DELETE FROM history_records WHERE lower(wallet_id) = lower(?1)",
                    params![id],
                )
                .map_err(|e| e.to_string())?;
                tx.execute("DELETE FROM wallets WHERE id = ?1", params![id])
                    .map_err(|e| e.to_string())?;
            }
            for id in self.removed_addresses {
                tx.execute("DELETE FROM address_book WHERE id = ?1", params![id])
                    .map_err(|e| e.to_string())?;
            }
            for (index, wallet, payload) in self.wallets {
                tx.execute("INSERT INTO wallets
                    (id, name, chain_name, is_watch_only, include_in_portfolio_total, sort_index, payload, updated_at)
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                    ON CONFLICT(id) DO UPDATE SET name=excluded.name, chain_name=excluded.chain_name,
                    is_watch_only=excluded.is_watch_only, include_in_portfolio_total=excluded.include_in_portfolio_total,
                    sort_index=excluded.sort_index, payload=excluded.payload, updated_at=excluded.updated_at",
                    params![wallet.id, wallet.name, wallet.chain_name, wallet.is_watch_only,
                        wallet.include_in_portfolio_total, index as i64, payload, updated_at])
                    .map_err(|e| format!("app_state_save wallet: {e}"))?;
            }
            for (index, entry, payload) in self.addresses {
                tx.execute("INSERT INTO address_book (id, chain_name, address, sort_index, payload, updated_at)
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                    ON CONFLICT(id) DO UPDATE SET chain_name=excluded.chain_name, address=excluded.address,
                    sort_index=excluded.sort_index, payload=excluded.payload, updated_at=excluded.updated_at",
                    params![entry.id, entry.chain_name, entry.address, index as i64, payload, updated_at])
                    .map_err(|e| format!("app_state_save address: {e}"))?;
            }
            for (key, value) in self.meta {
                if let Some(value) = value {
                    tx.execute(
                        "INSERT INTO app_state_meta (key, value) VALUES (?1, ?2)
                        ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                        params![key, value],
                    )
                    .map_err(|e| format!("app_state_save {key}: {e}"))?;
                } else {
                    tx.execute("DELETE FROM app_state_meta WHERE key = ?1", params![key])
                        .map_err(|e| format!("app_state_save clear {key}: {e}"))?;
                }
            }
            tx.commit()
                .map_err(|e| format!("app_state_save commit: {e}"))
        })
    }
}

/// Explicit snapshot replacement (imports and standalone store callers).
/// Service commands use a delta against their serialized committed state.
pub fn app_state_save(db_path: &str, state: &CoreAppState) -> Result<(), String> {
    AppStateChanges::between(None, state)?.save(db_path)
}

/// Load every saved recipient, in the stored display order.
pub fn address_book_load_all(db_path: &str) -> Result<Vec<AddressBookEntry>, String> {
    with_conn(db_path, |conn| {
        let mut stmt = conn
            .prepare("SELECT id, payload FROM address_book ORDER BY sort_index ASC")
            .map_err(|e| format!("address_book_load_all prepare: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("address_book_load_all query: {e}"))?;
        let mut entries = Vec::new();
        for row in rows {
            let (id, payload) = row.map_err(|e| format!("address_book_load_all row: {e}"))?;
            entries.push(
                serde_json::from_str(&payload)
                    .map_err(|e| format!("address_book_load_all decode {id}: {e}"))?,
            );
        }
        Ok(entries)
    })
}

/// Load the persisted [`CoreAppState`].
///
/// An untouched database loads as `CoreAppState::default()`, so first run needs
/// no special-casing at the call site.
/// Decode a rebuildable metadata row, or say so and start it over.
///
/// Separate from the `?` the wallet rows use: what is lost here is a cache,
/// and what a hard failure would lose with it is not.
fn drop_unreadable<T: Default + serde::de::DeserializeOwned>(value: &str, key: &str) -> T {
    match serde_json::from_str(value) {
        Ok(parsed) => parsed,
        Err(error) => {
            tracing::warn!(%key, %error, "unreadable state row; rebuilding it");
            T::default()
        }
    }
}

pub fn app_state_load(db_path: &str) -> Result<CoreAppState, String> {
    let wallets = wallet_load_all(db_path)?;
    let address_book = address_book_load_all(db_path)?;
    with_conn(db_path, |conn| {
        let mut stmt = conn
            .prepare("SELECT key, value FROM app_state_meta")
            .map_err(|e| format!("app_state_load prepare: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("app_state_load query: {e}"))?;

        let mut state = CoreAppState {
            wallets,
            address_book,
            ..CoreAppState::default()
        };
        for row in rows {
            let (key, value) = row.map_err(|e| format!("app_state_load row: {e}"))?;
            match key.as_str() {
                "diagnostics" => {
                    state.diagnostics = serde_json::from_str(&value)
                        .map_err(|e| format!("invalid diagnostics: {e}"))?
                }
                META_SCHEMA_VERSION => {
                    state.schema_version = value
                        .parse()
                        .map_err(|e| format!("app_state_load schema_version {value:?}: {e}"))?;
                }
                META_SELECTED_WALLET_ID => state.selected_wallet_id = Some(value),
                META_SETTINGS => {
                    state.settings = serde_json::from_str(&value)
                        .map_err(|e| format!("app_state_load settings: {e}"))?;
                }
                // A row this build cannot read is dropped, not fatal. These
                // three are rebuilt — token preferences and alerts from the
                // catalog on the next evaluation, fiat rates on the next
                // refresh — so losing one costs a rebuild, where failing the
                // whole load loses the wallet list, which cannot be rebuilt
                // from anything. `settings` and the wallet rows above stay
                // fatal for exactly that reason.
                //
                // The row itself is left on disk untouched, so a build that
                // can read it still will.
                META_TOKEN_PREFERENCES => {
                    state.token_preferences = drop_unreadable(&value, "token_preferences");
                }
                META_PRICE_ALERTS => {
                    state.price_alerts = drop_unreadable(&value, "price_alerts");
                }
                "quotes" => state.quotes = drop_unreadable(&value, "quotes"),
                META_FIAT_RATES => {
                    state.fiat_rates_from_usd = drop_unreadable(&value, "fiat_rates_from_usd");
                }
                // Forward compatibility: a newer build's extra meta keys are
                // ignored rather than treated as corruption.
                _ => {}
            }
        }
        Ok(state)
    })
}

/// All members of a changed family are invalidated in the settings transaction.
pub(crate) fn changed_network_chains(before: &CoreAppState, after: &CoreAppState) -> Vec<String> {
    crate::registry::Chain::mainnets()
        .filter(|c| before.settings.network_chain(*c) != after.settings.network_chain(*c))
        .flat_map(|c| {
            c.network_choices()
                .iter()
                .map(|n| n.chain_display_name().to_string())
                .collect::<Vec<_>>()
        })
        .collect()
}
