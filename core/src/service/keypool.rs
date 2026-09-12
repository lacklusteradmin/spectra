//! Persisted receive/change reservations and owned addresses.
use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
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
        self.write_persisted(move |service| async move {
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
            // The row goes to storage first and becomes visible second, under
            // the lock that the reservation paths also take. This used to clone
            // the whole table, mutate the copy and write it back — a shape that
            // drops any write landing in between.
            let mut tables = service.keypool.write().await;
            if let Some(db_path) = service.state_binding.path().await {
                let to_save = record.clone();
                tokio::task::spawn_blocking(move || {
                    crate::wallet_db::address_save(&db_path, &to_save)
                })
                .await
                .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))??;
            }
            tables.remember_owned(record);
            Ok(())
        })
        .await
    }

    /// The addresses this wallet owns — on one chain, or on every chain when
    /// `chain_name` is absent.
    pub async fn owned_addresses_for_wallet(
        &self,
        wallet_id: String,
        chain_name: Option<String>,
    ) -> Vec<String> {
        let tables = self.keypool.read().await;
        let rows: Box<dyn Iterator<Item = &crate::wallet_db::OwnedAddressRecord>> =
            match chain_name.as_deref() {
                Some(chain) => Box::new(tables.owned_on(chain).iter()),
                None => Box::new(tables.owned_everywhere()),
            };
        rows.filter(|r| r.wallet_id == wallet_id)
            .map(|r| r.address.clone())
            .collect()
    }

    /// The wallet's keypool for a chain, merged with the baseline and recorded.
    ///
    /// The baseline is core's own: it comes from the transactions, owned
    /// addresses and wallet addresses core already holds. A caller used to
    /// compute it and pass it in, which meant the guarantee this lock provides
    /// depended on the caller's copy of three tables being current.
    /// The keypool state for a wallet on a chain.
    ///
    /// A read. There were two of these — one that merged the baseline with the
    /// stored record and *persisted* the merge, and one that merged without
    /// persisting — returning the same value either way. The persist was a
    /// cache write of a pure function's result, and it made a read take the
    /// write lock. `reserve_*` recomputes the merge before it writes, so
    /// nothing depended on it.
    pub async fn keypool_state(
        &self,
        wallet_id: String,
        chain_name: String,
    ) -> Result<crate::wallet_db::KeypoolState, SpectraBridgeError> {
        let baseline = self.chain_keypool_baseline(&wallet_id, &chain_name).await?;
        let key = keypool_key(&wallet_id, &chain_name);
        let tables = self.keypool.read().await;
        Ok(keypool_from_record(
            &crate::store::plan_chain_keypool_state(
                baseline,
                tables.state(&key).map(record_from_keypool),
            ),
        ))
    }

    /// Reserve the next receive index, or return the one already reserved.
    pub async fn reserve_receive_index(
        &self,
        wallet_id: String,
        chain_name: String,
        minimum_index: i64,
    ) -> Result<i64, SpectraBridgeError> {
        self.write_persisted(move |service| async move {
            let baseline = service
                .chain_keypool_baseline(&wallet_id, &chain_name)
                .await?;
            let key = keypool_key(&wallet_id, &chain_name);
            let mut tables = service.keypool.write().await;
            let merged = crate::store::plan_chain_keypool_state(
                baseline,
                tables.state(&key).map(record_from_keypool),
            );
            let mut state = keypool_from_record(&merged);
            if let Some(reserved) = state.reserved_receive_index {
                // Already reserved: hand back the same index rather than burning a
                // new one every time the receive sheet opens.
                persist_keypool(
                    &service.state_binding,
                    &mut tables,
                    key,
                    &wallet_id,
                    &chain_name,
                    state,
                )
                .await?;
                return Ok(reserved);
            }
            let reserved = state.next_external_index.max(minimum_index);
            // Reserving consumes the index, so the keypool has to have a next
            // one to move to. Asking here refuses before the address is handed
            // out rather than after it has been shown.
            let after = next_keypool_index(reserved)?;
            state.reserved_receive_index = Some(reserved);
            state.next_external_index = state.next_external_index.max(after);
            persist_keypool(
                &service.state_binding,
                &mut tables,
                key,
                &wallet_id,
                &chain_name,
                state,
            )
            .await?;
            Ok(reserved)
        })
        .await
    }

    /// Reserve the next change index. Always consumes one.
    pub async fn reserve_change_index(
        &self,
        wallet_id: String,
        chain_name: String,
    ) -> Result<i64, SpectraBridgeError> {
        self.write_persisted(move |service| async move {
            let baseline = service
                .chain_keypool_baseline(&wallet_id, &chain_name)
                .await?;
            let key = keypool_key(&wallet_id, &chain_name);
            let mut tables = service.keypool.write().await;
            let merged = crate::store::plan_chain_keypool_state(
                baseline,
                tables.state(&key).map(record_from_keypool),
            );
            let mut state = keypool_from_record(&merged);
            let reserved = state.next_change_index.max(0);
            state.next_change_index = next_keypool_index(reserved)?;
            persist_keypool(
                &service.state_binding,
                &mut tables,
                key,
                &wallet_id,
                &chain_name,
                state,
            )
            .await?;
            Ok(reserved)
        })
        .await
    }

    /// Release the reserved receive index once its address has been used.
    pub async fn clear_reserved_receive_index(
        &self,
        wallet_id: String,
        chain_name: String,
    ) -> Result<(), SpectraBridgeError> {
        self.write_persisted(move |service| async move {
            let key = keypool_key(&wallet_id, &chain_name);
            let mut tables = service.keypool.write().await;
            let Some(mut state) = tables.state(&key).cloned() else {
                return Ok(());
            };
            state.reserved_receive_index = None;
            persist_keypool(
                &service.state_binding,
                &mut tables,
                key,
                &wallet_id,
                &chain_name,
                state,
            )
            .await
        })
        .await
    }
}

impl WalletService {
    pub(super) async fn advance_receive_index_if_current(
        &self,
        wallet_id: String,
        chain_name: String,
        expected: i64,
    ) -> Result<Option<i64>, SpectraBridgeError> {
        self.write_persisted(move |service| async move {
            let baseline = service
                .chain_keypool_baseline(&wallet_id, &chain_name)
                .await?;
            let key = keypool_key(&wallet_id, &chain_name);
            let mut tables = service.keypool.write().await;
            let Some(mut state) = tables.state(&key).cloned() else {
                return Ok(None);
            };
            if state.reserved_receive_index != Some(expected) {
                return Ok(None);
            }
            state.next_external_index = state
                .next_external_index
                .max(i64::from(baseline.next_external_index));
            state.next_change_index = state
                .next_change_index
                .max(i64::from(baseline.next_change_index));
            let next = state.next_external_index.max(next_keypool_index(expected)?);
            state.next_external_index = next_keypool_index(next)?;
            state.reserved_receive_index = Some(next);
            persist_keypool(
                &service.state_binding,
                &mut tables,
                key,
                &wallet_id,
                &chain_name,
                state,
            )
            .await?;
            Ok(Some(next))
        })
        .await
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
    ) -> Result<crate::store::ChainKeypoolStateRecord, SpectraBridgeError> {
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
            return Ok(crate::store::plan_baseline_chain_keypool_state(input));
        }

        if let Some(db_path) = self.state_binding.path().await {
            let wallet = wallet_id.to_owned();
            let chain = chain_name.to_owned();
            let (external, change) = tokio::task::spawn_blocking(move || {
                crate::wallet_db::history_keypool_indices(&db_path, &wallet, &chain)
            })
            .await
            .map_err(|e| SpectraBridgeError::from(format!("keypool history task: {e}")))??;
            input.max_transaction_external_index = external;
            input.max_transaction_change_index = change;
        }

        let tables = self.keypool.read().await;
        {
            let for_wallet = tables
                .owned_on(chain_name)
                .iter()
                .filter(|r| r.wallet_id == wallet_id);
            let (mut external, mut change): (Option<i64>, Option<i64>) = (None, None);
            for row in for_wallet {
                let Some(index) = row.branch_index else {
                    continue;
                };
                match row.branch.as_deref() {
                    Some("external") => {
                        external = Some(external.map_or(index, |old| old.max(index)))
                    }
                    Some("change") => change = Some(change.map_or(index, |old| old.max(index))),
                    _ => {}
                }
            }
            input.max_owned_external_index = external
                .map(i32::try_from)
                .transpose()
                .map_err(|_| SpectraBridgeError::from("owned external index out of range"))?;
            input.max_owned_change_index = change
                .map(i32::try_from)
                .transpose()
                .map_err(|_| SpectraBridgeError::from("owned change index out of range"))?;
        }

        for index in [
            input.max_transaction_external_index,
            input.max_transaction_change_index,
            input.max_owned_external_index,
            input.max_owned_change_index,
        ]
        .into_iter()
        .flatten()
        {
            if index < 0 || index == i32::MAX {
                return Err(SpectraBridgeError::from(
                    "keypool index has no valid successor",
                ));
            }
        }
        Ok(crate::store::plan_baseline_chain_keypool_state(input))
    }
}

/// Keypool map key. A wallet has one keypool per chain.
pub(super) fn keypool_key(wallet_id: &str, chain_name: &str) -> String {
    format!("{wallet_id}|{chain_name}")
}

/// The addresses a wallet has handed out, and the next index on each chain.
///
/// One type because the two maps are one fact. `owned` is where a chain's
/// baseline comes from — the highest index already issued — and `indices` is
/// where the next one is taken from, so a reader of one that cannot see the
/// other can hand out an address somebody already holds.
///
/// They were two `pub(crate)` fields on `WalletService` behind two locks,
/// taken separately by three files: `open_state` replaced both, wallet
/// deletion retained one and then the other, and registering an owned address
/// cloned the table, mutated the copy and wrote it back. Every one of those is
/// safe, and none of them is safe *by itself* — what serializes them is the
/// `state_writer` mutex two layers up, which nothing here can see. One lock
/// over both tables is the same guarantee where it can be checked.
#[derive(Default)]
pub(crate) struct Keypool {
    tables: AsyncRwLock<KeypoolTables>,
}

/// The two tables, together.
#[derive(Default)]
pub(crate) struct KeypoolTables {
    /// Keypool indices, keyed by `wallet_id|chain_name`.
    indices: HashMap<String, crate::wallet_db::KeypoolState>,
    /// Addresses this wallet is known to own, keyed by chain name.
    owned: HashMap<String, Vec<crate::wallet_db::OwnedAddressRecord>>,
}

impl Keypool {
    pub(crate) async fn read(&self) -> tokio::sync::RwLockReadGuard<'_, KeypoolTables> {
        self.tables.read().await
    }

    pub(crate) async fn write(&self) -> tokio::sync::RwLockWriteGuard<'_, KeypoolTables> {
        self.tables.write().await
    }
}

impl KeypoolTables {
    pub(crate) fn state(&self, key: &str) -> Option<&crate::wallet_db::KeypoolState> {
        self.indices.get(key)
    }

    pub(crate) fn set_state(&mut self, key: String, state: crate::wallet_db::KeypoolState) {
        self.indices.insert(key, state);
    }

    pub(crate) fn owned_on(&self, chain_name: &str) -> &[crate::wallet_db::OwnedAddressRecord] {
        self.owned.get(chain_name).map_or(&[], Vec::as_slice)
    }

    pub(crate) fn owned_everywhere(
        &self,
    ) -> impl Iterator<Item = &crate::wallet_db::OwnedAddressRecord> {
        self.owned.values().flatten()
    }

    /// Record one owned address, replacing the row for the same wallet and
    /// address if there is one.
    ///
    /// A method rather than a clone-mutate-write-back at the call site: that
    /// shape loses any write that landed in between, and only the serializing
    /// mutex upstream made it safe.
    pub(crate) fn remember_owned(&mut self, record: crate::wallet_db::OwnedAddressRecord) {
        let rows = self.owned.entry(record.chain_name.clone()).or_default();
        match rows.iter_mut().find(|existing| {
            existing.wallet_id == record.wallet_id && existing.address == record.address
        }) {
            Some(existing) => *existing = record,
            None => rows.push(record),
        }
    }

    /// Seed both tables from storage. Only `open_state` does this.
    pub(crate) fn load(
        &mut self,
        indices: HashMap<String, crate::wallet_db::KeypoolState>,
        owned: HashMap<String, Vec<crate::wallet_db::OwnedAddressRecord>>,
    ) {
        self.indices = indices;
        self.owned = owned;
    }

    /// Drop everything belonging to deleted wallets or reset chains.
    ///
    /// Both tables in one call, because forgetting an index without forgetting
    /// the addresses it issued — or the reverse — is how the same address gets
    /// handed out twice. The caller used to do this as two `retain`s under two
    /// separately acquired locks.
    pub(crate) fn forget(&mut self, removed_wallets: &[String], reset_chains: &[String]) {
        self.indices.retain(|key, _| {
            key.split_once('|').is_none_or(|(wallet_id, chain_name)| {
                !removed_wallets.iter().any(|r| r == wallet_id)
                    && !reset_chains.iter().any(|c| c == chain_name)
            })
        });
        self.owned
            .retain(|chain_name, _| !reset_chains.contains(chain_name));
        for rows in self.owned.values_mut() {
            rows.retain(|row| !removed_wallets.contains(&row.wallet_id));
        }
    }

    /// Whether either table holds anything. Test affordance.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.indices.is_empty() && self.owned.is_empty()
    }

    /// The index table, for the one test that compares whole snapshots.
    #[cfg(test)]
    pub(crate) fn indices(&self) -> &HashMap<String, crate::wallet_db::KeypoolState> {
        &self.indices
    }
}

/// Largest index the keypool can hand out: the last non-hardened BIP-32 child.
///
/// It is also exactly `i32::MAX`, which is what makes the narrowing below
/// lossless — the keypool carries indices as `i64` in SQLite and as `i32`
/// across the FFI, and the two used to be bridged with a plain `as`.
const MAX_KEYPOOL_INDEX: i64 = (crate::derivation::primitives::HARDENED_OFFSET - 1) as i64;

/// The index after `index`, or an error at the end of the keypool.
///
/// A chain's non-hardened range is finite, and running off the end of it is a
/// refusal rather than a wrap: past this point there is no child key to derive,
/// so an index that got there would name an address nobody can spend from.
fn next_keypool_index(index: i64) -> Result<i64, SpectraBridgeError> {
    match index.checked_add(1) {
        Some(next) if next <= MAX_KEYPOOL_INDEX => Ok(next),
        _ => Err(SpectraBridgeError::from(
            "keypool exhausted: no non-hardened child index left on this chain",
        )),
    }
}

/// Narrow a keypool index to the FFI record's width.
///
/// Lossless by construction — every producer above refuses an index outside
/// `0..=MAX_KEYPOOL_INDEX`, which is `i32`'s non-negative range exactly. The
/// clamp is what happens if that ever stops being true, and it clamps rather
/// than wraps on purpose: an index at the ceiling fails at derivation, where a
/// negative one derives at a path nobody asked for.
fn narrow_keypool_index(index: i64) -> i32 {
    index.clamp(0, MAX_KEYPOOL_INDEX) as i32
}

fn record_from_keypool(
    state: &crate::wallet_db::KeypoolState,
) -> crate::store::ChainKeypoolStateRecord {
    crate::store::ChainKeypoolStateRecord {
        next_external_index: narrow_keypool_index(state.next_external_index),
        next_change_index: narrow_keypool_index(state.next_change_index),
        reserved_receive_index: state.reserved_receive_index.map(narrow_keypool_index),
    }
}

/// Store one keypool entry in memory and in SQLite. Caller holds the lock, so
/// the read-modify-write around this call stays atomic.
async fn persist_keypool(
    binding: &crate::service::state::StateBinding,
    tables: &mut KeypoolTables,
    key: String,
    wallet_id: &str,
    chain_name: &str,
    state: crate::wallet_db::KeypoolState,
) -> Result<(), SpectraBridgeError> {
    if tables.state(&key) == Some(&state) {
        return Ok(());
    }
    // Without a bound database the service runs in memory only — the shape
    // tests and short-lived tools. Nothing to write.
    let Some(db_path) = binding.path().await else {
        tables.set_state(key, state);
        return Ok(());
    };
    let (wallet_id, chain_name) = (wallet_id.to_string(), chain_name.to_string());
    let to_save = state.clone();
    tokio::task::spawn_blocking(move || {
        crate::wallet_db::keypool_save(&db_path, &wallet_id, &chain_name, &to_save)
    })
    .await
    .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))?
    .map_err(SpectraBridgeError::from)?;
    tables.set_state(key, state);
    Ok(())
}

pub(super) fn keypool_from_record(
    record: &crate::store::ChainKeypoolStateRecord,
) -> crate::wallet_db::KeypoolState {
    crate::wallet_db::KeypoolState {
        next_external_index: record.next_external_index as i64,
        next_change_index: record.next_change_index as i64,
        reserved_receive_index: record.reserved_receive_index.map(|i| i as i64),
    }
}

/// A keypool index has to name a derivable child key.
#[cfg(test)]
mod the_keypool_stays_inside_the_non_hardened_range {
    use super::*;

    /// The ceiling is BIP-32's, and it is also exactly what the FFI record can
    /// hold — which is why narrowing to it is lossless rather than lucky.
    #[test]
    fn the_ceiling_is_the_last_non_hardened_child() {
        assert_eq!(
            MAX_KEYPOOL_INDEX,
            i64::from(crate::derivation::primitives::HARDENED_OFFSET - 1)
        );
        assert_eq!(MAX_KEYPOOL_INDEX, i64::from(i32::MAX));
    }

    /// Running off the end is a refusal. Past the last non-hardened child
    /// there is no key to derive, so an index that got there would name an
    /// address nobody can spend from.
    #[test]
    fn the_end_of_the_keypool_is_an_error_and_not_a_wrap() {
        assert_eq!(next_keypool_index(0).unwrap(), 1);
        assert_eq!(
            next_keypool_index(MAX_KEYPOOL_INDEX - 1).unwrap(),
            MAX_KEYPOOL_INDEX
        );
        assert!(next_keypool_index(MAX_KEYPOOL_INDEX).is_err());
        assert!(next_keypool_index(i64::MAX).is_err());
    }

    /// `as i32` turned anything past the ceiling into a negative index, which
    /// derivation would then take as a path nobody asked for. Clamping keeps
    /// the failure at the ceiling, where it is an exhausted keypool.
    #[test]
    fn narrowing_clamps_rather_than_wrapping_to_a_negative_index() {
        for (index, expected) in [
            (0i64, 0i32),
            (42, 42),
            (MAX_KEYPOOL_INDEX, i32::MAX),
            (MAX_KEYPOOL_INDEX + 1, i32::MAX),
            (i64::MAX, i32::MAX),
            (-1, 0),
        ] {
            let narrowed = narrow_keypool_index(index);
            assert_eq!(narrowed, expected, "{index}");
            assert!(narrowed >= 0, "{index} narrowed to a negative index");
        }
        // The cast this replaces did the opposite on both ends.
        assert!(((MAX_KEYPOOL_INDEX + 1) as i32) < 0);
    }
}

/// The two tables are one fact, and `forget` is where that shows.
#[cfg(test)]
mod the_keypool_forgets_indices_and_addresses_together {
    use super::*;

    fn owned(
        wallet_id: &str,
        chain_name: &str,
        index: i64,
    ) -> crate::wallet_db::OwnedAddressRecord {
        crate::wallet_db::OwnedAddressRecord {
            wallet_id: wallet_id.to_string(),
            chain_name: chain_name.to_string(),
            address: format!("{wallet_id}-{chain_name}-{index}"),
            derivation_path: None,
            branch: Some("external".to_string()),
            branch_index: Some(index),
        }
    }

    fn populated() -> KeypoolTables {
        let mut tables = KeypoolTables::default();
        for (wallet, chain) in [("w1", "Bitcoin"), ("w1", "Litecoin"), ("w2", "Bitcoin")] {
            tables.set_state(
                keypool_key(wallet, chain),
                crate::wallet_db::KeypoolState {
                    next_external_index: 5,
                    next_change_index: 2,
                    reserved_receive_index: None,
                },
            );
            tables.remember_owned(owned(wallet, chain, 4));
        }
        tables
    }

    /// Deleting a wallet drops its indices *and* the addresses issued from
    /// them. Keeping either half is how the same address is handed out twice:
    /// the baseline is computed from the addresses, and the next index from
    /// the index table.
    #[test]
    fn a_deleted_wallet_leaves_neither_table_holding_it() {
        let mut tables = populated();
        tables.forget(&["w1".to_string()], &[]);

        assert!(tables.state(&keypool_key("w1", "Bitcoin")).is_none());
        assert!(tables.state(&keypool_key("w1", "Litecoin")).is_none());
        assert!(tables.state(&keypool_key("w2", "Bitcoin")).is_some());

        let left: Vec<_> = tables.owned_everywhere().map(|r| &r.wallet_id).collect();
        assert_eq!(left, vec!["w2"], "w1 kept addresses after its indices went");
    }

    /// Resetting a chain is the same rule along the other axis.
    #[test]
    fn a_reset_chain_leaves_neither_table_holding_it() {
        let mut tables = populated();
        tables.forget(&[], &["Bitcoin".to_string()]);

        assert!(tables.state(&keypool_key("w1", "Bitcoin")).is_none());
        assert!(tables.state(&keypool_key("w2", "Bitcoin")).is_none());
        assert!(tables.state(&keypool_key("w1", "Litecoin")).is_some());

        assert!(tables.owned_on("Bitcoin").is_empty());
        assert_eq!(tables.owned_on("Litecoin").len(), 1);
    }

    /// Registering the same address twice updates the row rather than issuing
    /// a duplicate. The call site used to clone the table, mutate the copy and
    /// write it back, which drops any write that landed in between.
    #[test]
    fn remembering_an_address_twice_replaces_rather_than_duplicates() {
        let mut tables = KeypoolTables::default();
        tables.remember_owned(owned("w1", "Bitcoin", 4));
        let mut revised = owned("w1", "Bitcoin", 4);
        revised.derivation_path = Some("m/84'/0'/0'/0/4".to_string());
        tables.remember_owned(revised);

        assert_eq!(tables.owned_on("Bitcoin").len(), 1);
        assert_eq!(
            tables.owned_on("Bitcoin")[0].derivation_path.as_deref(),
            Some("m/84'/0'/0'/0/4")
        );
    }

    /// A chain nobody has an address on reads as empty, not as a missing key
    /// the caller has to handle.
    #[test]
    fn an_untouched_chain_reads_as_no_addresses() {
        assert!(KeypoolTables::default().owned_on("Bitcoin").is_empty());
    }
}
