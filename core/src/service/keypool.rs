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
            let mut table = service.owned_addresses.read().await.clone();
            let rows = table.entry(chain_name).or_default();
            match rows.iter_mut().find(|existing| {
                existing.wallet_id == record.wallet_id && existing.address == record.address
            }) {
                Some(existing) => *existing = record.clone(),
                None => rows.push(record.clone()),
            }
            if let Some(db_path) = service.state_db_path.read().await.clone() {
                tokio::task::spawn_blocking(move || {
                    crate::wallet_db::address_save(&db_path, &record)
                })
                .await
                .map_err(|e| SpectraBridgeError::from(format!("spawn_blocking: {e}")))??;
            }
            *service.owned_addresses.write().await = table;
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
        let keypool = self.keypool.read().await;
        Ok(keypool_from_record(
            &crate::store::plan_chain_keypool_state(
                baseline,
                keypool.get(&key).map(record_from_keypool),
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
            let mut keypool = service.keypool.write().await;
            let merged = crate::store::plan_chain_keypool_state(
                baseline,
                keypool.get(&key).map(record_from_keypool),
            );
            let mut state = keypool_from_record(&merged);
            if let Some(reserved) = state.reserved_receive_index {
                // Already reserved: hand back the same index rather than burning a
                // new one every time the receive sheet opens.
                persist_keypool(
                    &service.state_db_path,
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
                &service.state_db_path,
                &mut keypool,
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
            let mut keypool = service.keypool.write().await;
            let merged = crate::store::plan_chain_keypool_state(
                baseline,
                keypool.get(&key).map(record_from_keypool),
            );
            let mut state = keypool_from_record(&merged);
            let reserved = state.next_change_index.max(0);
            state.next_change_index = reserved + 1;
            persist_keypool(
                &service.state_db_path,
                &mut keypool,
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
            let mut keypool = service.keypool.write().await;
            let Some(mut state) = keypool.get(&key).cloned() else {
                return Ok(());
            };
            state.reserved_receive_index = None;
            persist_keypool(
                &service.state_db_path,
                &mut keypool,
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
            let mut keypool = service.keypool.write().await;
            let Some(mut state) = keypool.get(&key).cloned() else {
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
            let next = state.next_external_index.max(
                expected
                    .checked_add(1)
                    .ok_or_else(|| SpectraBridgeError::from("receive index overflow"))?,
            );
            state.next_external_index = next
                .checked_add(1)
                .ok_or_else(|| SpectraBridgeError::from("receive index overflow"))?;
            state.reserved_receive_index = Some(next);
            persist_keypool(
                &service.state_db_path,
                &mut keypool,
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

        if let Some(db_path) = self.state_db_path.read().await.clone() {
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

        let owned = self.owned_addresses.read().await;
        if let Some(rows) = owned.get(chain_name) {
            let for_wallet = rows.iter().filter(|r| r.wallet_id == wallet_id);
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
    state_db_path: &Arc<AsyncRwLock<Option<String>>>,
    keypool: &mut HashMap<String, crate::wallet_db::KeypoolState>,
    key: String,
    wallet_id: &str,
    chain_name: &str,
    state: crate::wallet_db::KeypoolState,
) -> Result<(), SpectraBridgeError> {
    if keypool.get(&key) == Some(&state) {
        return Ok(());
    }
    // Without a bound database the service runs in memory only — the shape
    // tests and short-lived tools. Nothing to write.
    let Some(db_path) = state_db_path.read().await.clone() else {
        keypool.insert(key, state);
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
    keypool.insert(key, state);
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
