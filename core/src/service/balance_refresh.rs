//! Fetch and commit balances against the wallet and network that requested them.
use super::*;
use crate::service::refresh_engine::{RefreshEntry, refresh_entry_for};
use crate::store::state::WalletState;
use crate::store::wallet_domain::AssetHolding;
use futures::{
    FutureExt,
    future::{BoxFuture, WeakShared},
};

type BalanceRead = BoxFuture<'static, Result<WalletState, SpectraBridgeError>>;

pub(super) struct BalanceRefreshes {
    active: parking_lot::Mutex<HashMap<RefreshEntry, WeakShared<BalanceRead>>>,
    permits: tokio::sync::Semaphore,
}
impl Default for BalanceRefreshes {
    fn default() -> Self {
        Self {
            active: Default::default(),
            permits: tokio::sync::Semaphore::new(8),
        }
    }
}

impl WalletService {
    /// Fetch one wallet's balances and store them. The app's sweeps and the
    /// CLI's live portfolio both come through here.
    pub async fn refresh_wallet_balances(
        &self,
        wallet_id: String,
    ) -> Result<WalletState, SpectraBridgeError> {
        let (entry, known) = {
            let state = self.wallet_state.read().await;
            let entry = state
                .wallets
                .iter()
                .find(|w| w.id == wallet_id)
                .and_then(refresh_entry_for)
                .ok_or_else(|| SpectraBridgeError::failure("wallet has no refreshable address"))?;
            let chain = entry.chain_id;
            let known = state
                .token_preferences
                .iter()
                .filter(|p| p.hosting_chain() == Some(chain))
                .cloned()
                .collect::<Vec<_>>();
            (entry, known)
        };
        // Both automatic and requested refreshes await the same in-flight read.
        // Weak handles retain neither a completed result nor a cancelled service.
        let work = {
            let mut active = self.balance_refreshes.active.lock();
            active.retain(|_, work| work.upgrade().is_some());
            if let Some(work) = active.get(&entry).and_then(WeakShared::upgrade) {
                work
            } else {
                let service = self.clone();
                let key = entry.clone();
                let work = async move {
                    let _permit = service
                        .balance_refreshes
                        .permits
                        .acquire()
                        .await
                        .map_err(SpectraBridgeError::failure)?;
                    let completed_key = entry.clone();
                    let result = service.fetch_wallet_balances(entry, known).await;
                    service
                        .balance_refreshes
                        .active
                        .lock()
                        .remove(&completed_key);
                    result
                }
                .boxed()
                .shared();
                active.insert(key, work.downgrade().expect("unpolled shared read"));
                work
            }
        };
        work.await
    }

    async fn fetch_wallet_balances(
        &self,
        entry: RefreshEntry,
        known: Vec<crate::store::wallet_domain::TokenPreferenceEntry>,
    ) -> Result<WalletState, SpectraBridgeError> {
        let chain = entry.chain_id;
        let native = if chain.uses_account_utxo() {
            self.discover_account_once(&entry.wallet_id, chain).await?;
            self.account_utxo_wallet_balance(&entry.wallet_id, chain)
                .await?
        } else {
            super::network_balance::fetch_native_balance_summary(&entry.address, chain, self)
                .await?
        };
        let mut native_amount = balance_amount(&native.amount_display)?;
        // A Zcash wallet's shielded funds are ZEC too, and a Litecoin
        // wallet's MWEB funds LTC: what its last scan found, read from this
        // device.
        let scanned = match chain.mainnet_counterpart() {
            Chain::Zcash => self.zcash_shielded_total(&entry.wallet_id).await?,
            Chain::Litecoin => self.litecoin_mweb_total(&entry.wallet_id).await?,
            _ => None,
        };
        if let Some(scanned) = scanned {
            native_amount = crate::amount::decimal::add(&native_amount, &scanned)
                .ok_or_else(|| SpectraBridgeError::failure("invalid balance amount"))?;
        }
        let mut holdings = vec![
            AssetHolding {
                amount: native_amount,
                ..entry.chain_id.native_holding_template()
            }
            .identified(),
        ];
        if !known.is_empty() {
            let descriptors = known
                .iter()
                .filter(|p| {
                    p.token
                        .token_standard
                        .is_some_and(|standard| chain.allows_token_standard(standard))
                })
                .map(|p| {
                    Ok(TokenDescriptor {
                        standard: p.token.token_standard,
                        contract: p.token.contract.clone(),
                        symbol: p.token.symbol.clone(),
                        decimals: u8::try_from(p.token.decimals)
                            .map_err(|_| SpectraBridgeError::failure("invalid token precision"))?,
                        name: Some(p.token.name.clone()),
                    })
                })
                .collect::<Result<Vec<_>, SpectraBridgeError>>()?;
            // Failed tokens are omitted by the provider adapter; their prior balances survive.
            let balances = self
                .known_token_balances(entry.chain_id, entry.address.clone(), descriptors)
                .await?;
            for result in balances {
                let key = contract_key(chain, &result.contract_address);
                if let Some(p) = known.iter().find(|p| {
                    p.token.token_standard == result.standard
                        && contract_key(chain, &p.token.contract) == key
                }) {
                    holdings.push(
                        AssetHolding {
                            amount: balance_amount(&result.balance_display)?,
                            ..p.token.holding_template()
                        }
                        .identified(),
                    );
                }
            }
        }
        self.commit_balance_result(entry, holdings).await
    }

    async fn commit_balance_result(
        &self,
        entry: RefreshEntry,
        holdings: Vec<AssetHolding>,
    ) -> Result<WalletState, SpectraBridgeError> {
        for h in &holdings {
            if crate::amount::decimal::canonical(&h.amount).as_ref() != Some(&h.amount) {
                return Err(SpectraBridgeError::failure("invalid balance"));
            }
        }
        self.write_persisted(move |service| async move {
            // The shared writer keeps this wallet index stable through persistence.
            // Clone and save just the changed wallet, not the entire application state.
            let (index, mut wallet) = {
                let state = service.wallet_state.read().await;
                let (index, wallet) = state
                    .wallets
                    .iter()
                    .enumerate()
                    .find(|(_, w)| w.id == entry.wallet_id)
                    .ok_or_else(|| SpectraBridgeError::failure("wallet removed during refresh"))?;
                if refresh_entry_for(wallet).as_ref() != Some(&entry) {
                    return Ok(wallet.clone());
                }
                (index, wallet.clone())
            };
            let before = wallet.holdings.clone();
            merge_balances(&mut wallet.holdings, holdings);
            // The first read replaces the import's placeholders, even with
            // the same zeros: from here on they are balances.
            let first_read = wallet.balances_read_at.is_none();
            if first_read {
                wallet.balances_read_at = Some(crate::store::now_unix());
            }
            if first_read || wallet.holdings != before {
                if let Some(database) = service.state_binding.connection().await {
                    let updated = wallet.clone();
                    tokio::task::spawn_blocking(move || {
                        crate::wallet_db::wallet_upsert(&database, &updated)
                    })
                    .await??;
                }
                let mut state = service.wallet_state.write().await;
                state.wallets[index] = wallet.clone();
                state.revision += 1;
            }
            Ok(wallet)
        })
        .await
    }
}
/// A provider's balance text as an exact decimal. Refused rather than
/// rounded: a balance is what the wallet shows and what a send spends.
fn balance_amount(raw: &str) -> Result<String, SpectraBridgeError> {
    crate::amount::decimal::canonical(raw)
        .ok_or_else(|| SpectraBridgeError::failure("invalid balance amount"))
}
fn contract_key(chain: Chain, contract: &str) -> String {
    crate::catalog::tokens::normalize_token_identifier(Some(contract.into()), chain)
        .unwrap_or_else(|| contract.into())
}
fn balance_key(h: &AssetHolding) -> (Chain, bool, Option<String>) {
    (
        h.chain_id,
        h.is_native(),
        crate::catalog::tokens::normalize_token_identifier(h.contract_address.clone(), h.chain_id),
    )
}
fn merge_balances(stored: &mut Vec<AssetHolding>, incoming: Vec<AssetHolding>) {
    for h in incoming {
        let key = balance_key(&h);
        let mut matched = false;
        stored.retain_mut(|old| {
            if balance_key(old) != key {
                return true;
            }
            if matched {
                return false;
            }
            // A successful read owns this identity and balance. A changed
            // protocol label must replace its old alias, never duplicate funds.
            *old = h.clone();
            matched = true;
            true
        });
        if !matched && !crate::amount::decimal::is_zero(&h.amount) {
            stored.push(h);
        }
    }
}

#[cfg(test)]
#[path = "tests/balance_refresh.rs"]
mod tests;
