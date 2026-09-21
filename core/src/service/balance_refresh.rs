//! Fetch and commit balances against the wallet and network that requested them.
use super::*;
#[cfg(test)]
use crate::fetch::refresh_engine::refresh_entries_for;
use crate::fetch::refresh_engine::{refresh_entry_for, RefreshEntry};
use crate::store::state::WalletState;
use crate::store::wallet_domain::AssetHolding;
use futures::{
    future::{BoxFuture, WeakShared},
    FutureExt,
};

type BalanceRead = BoxFuture<'static, Result<WalletState, String>>;

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
    pub(crate) async fn refresh_wallet_balances(
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
                .ok_or("wallet has no refreshable address")?;
            let chain = chain_for_id(&entry.chain_id)?;
            let known = state
                .token_preferences
                .iter()
                .filter(|p| {
                    p.is_enabled
                        && p.hosting_chain()
                            .is_some_and(|h| h.chain_name() == chain.chain_display_name())
                })
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
                        .map_err(|e| e.to_string())?;
                    let completed_key = entry.clone();
                    let result = service
                        .fetch_wallet_balances(entry, known)
                        .await
                        .map_err(|e| e.to_string());
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
        work.await.map_err(Into::into)
    }

    async fn fetch_wallet_balances(
        &self,
        entry: RefreshEntry,
        known: Vec<crate::store::wallet_domain::CoreTokenPreferenceEntry>,
    ) -> Result<WalletState, SpectraBridgeError> {
        let chain = chain_for_id(&entry.chain_id)?;
        let native = self
            .fetch_native_balance_summary_auto(&entry.chain_id, entry.address.clone())
            .await?;
        let mut holdings = vec![AssetHolding {
            amount: balance_amount(&native.amount_display)?,
            ..native_coin_template(&entry.chain_id).ok_or("missing native asset")?
        }];
        if !known.is_empty() {
            let descriptors = known
                .iter()
                .map(|p| {
                    Ok(TokenDescriptor {
                        contract: p.token.contract.clone(),
                        symbol: p.token.symbol.clone(),
                        decimals: u8::try_from(p.token.decimals)
                            .map_err(|_| "invalid token precision")?,
                        name: Some(p.token.name.clone()),
                    })
                })
                .collect::<Result<Vec<_>, SpectraBridgeError>>()?;
            // Failed tokens are omitted by the provider adapter; their prior balances survive.
            let balances = self
                .fetch_token_balances(entry.chain_id.clone(), entry.address.clone(), descriptors)
                .await?;
            for result in balances {
                let key = contract_key(chain.chain_display_name(), &result.contract_address);
                if let Some(p) = known
                    .iter()
                    .find(|p| contract_key(chain.chain_display_name(), &p.token.contract) == key)
                {
                    holdings.push(AssetHolding {
                        name: p.token.name.clone(),
                        symbol: p.token.symbol.clone(),
                        coingecko_id: p.token.coingecko_id.clone(),
                        chain_name: chain.chain_display_name().into(),
                        token_standard: p.token.token_standard.clone(),
                        contract_address: Some(p.token.contract.clone()),
                        amount: balance_amount(&result.balance_display)?,
                        price_usd: 0.0,
                    });
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
            if !h.amount.is_finite() || h.amount < 0.0 {
                return Err("invalid balance".into());
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
                    .ok_or("wallet removed during refresh")?;
                if refresh_entry_for(wallet).as_ref() != Some(&entry) {
                    return Ok(wallet.clone());
                }
                (index, wallet.clone())
            };
            let before = wallet.holdings.clone();
            merge_balances(&mut wallet.holdings, holdings);
            if wallet.holdings != before {
                if let Some(database) = service.state_binding.connection().await {
                    let updated = wallet.clone();
                    tokio::task::spawn_blocking(move || {
                        crate::wallet_db::wallet_upsert(&database, &updated)
                    })
                    .await
                    .map_err(|e| e.to_string())??;
                }
                service.wallet_state.write().await.wallets[index] = wallet.clone();
            }
            Ok(wallet)
        })
        .await
    }
}
fn balance_amount(raw: &str) -> Result<f64, SpectraBridgeError> {
    let value: f64 = raw.parse().map_err(|_| "invalid balance amount")?;
    if !value.is_finite() || value < 0.0 {
        return Err("invalid balance amount".into());
    }
    Ok(value)
}
fn contract_key(chain: &str, contract: &str) -> String {
    crate::tokens::normalize_token_identifier(Some(contract.into()), chain.into())
        .unwrap_or_else(|| contract.into())
}
fn balance_key(h: &AssetHolding) -> String {
    h.deployment_id()
}
fn merge_balances(stored: &mut Vec<AssetHolding>, incoming: Vec<AssetHolding>) {
    for h in incoming {
        if let Some(old) = stored
            .iter_mut()
            .find(|old| balance_key(old) == balance_key(&h))
        {
            old.amount = h.amount;
        } else if h.amount > 0.0 {
            stored.push(h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn balance_commit_preserves_metadata_and_refuses_stale_network() {
        let service = WalletService::new(vec![]).unwrap();
        let path = std::env::temp_dir().join(format!(
            "balance-owned-{}.sqlite",
            crate::store::new_event_id()
        ));
        service
            .open_state(path.to_string_lossy().into())
            .await
            .unwrap();
        let mut w = WalletState::single_address(
            "w",
            "Original",
            "Ethereum",
            "0x1111111111111111111111111111111111111111",
            None,
            false,
        );
        let mut coin = native_coin_template("ethereum").unwrap();
        coin.amount = 4.0;
        coin.price_usd = 123.0;
        w.holdings = vec![coin.clone()];
        service
            .apply_state_command(StateCommand::UpsertWallet { wallet: w })
            .await
            .unwrap();
        let entry = refresh_entries_for(&service.app_state().await).remove(0);
        coin.amount = 0.0;
        let updated = service
            .commit_balance_result(entry.clone(), vec![coin.clone()])
            .await
            .unwrap();
        assert_eq!(updated.name, "Original");
        assert_eq!(updated.holdings[0].amount, 0.0);
        assert_eq!(updated.holdings[0].price_usd, 123.0);
        let database = rusqlite::Connection::open(&path).unwrap();
        database.execute_batch("CREATE TRIGGER reject_balance BEFORE UPDATE ON wallets BEGIN SELECT RAISE(FAIL, 'balance write refused'); END;").unwrap();
        coin.amount = 7.0;
        assert!(service
            .commit_balance_result(entry.clone(), vec![coin.clone()])
            .await
            .is_err());
        assert_eq!(service.app_state().await.wallets[0].holdings[0].amount, 0.0);
        assert_eq!(
            crate::wallet_db::wallet_load(
                &crate::wallet_db::WalletDatabase::new(path.to_str().unwrap()),
                "w"
            )
            .unwrap()
            .unwrap()
            .holdings[0]
                .amount,
            0.0
        );
        database
            .execute_batch("DROP TRIGGER reject_balance")
            .unwrap();

        service
            .apply_state_command(StateCommand::SelectChainForFamily {
                chain_id: "ethereum-sepolia".into(),
            })
            .await
            .unwrap();
        coin.amount = 99.0;
        service
            .commit_balance_result(entry, vec![coin])
            .await
            .unwrap();
        let reopened = WalletService::new(vec![]).unwrap();
        assert_eq!(
            reopened
                .open_state(path.to_string_lossy().into())
                .await
                .unwrap()
                .wallets[0]
                .holdings[0]
                .amount,
            0.0
        );
        for raw in ["bad", "NaN", "inf", "-1"] {
            assert!(balance_amount(raw).is_err());
        }
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    #[tokio::test]
    async fn network_setting_and_derivation_cleanup_rollback_together() {
        let service = WalletService::new(vec![]).unwrap();
        let path = std::env::temp_dir().join(format!(
            "network-atomic-{}.sqlite",
            crate::store::new_event_id()
        ));
        service
            .open_state(path.to_string_lossy().into())
            .await
            .unwrap();
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TRIGGER reject_cleanup BEFORE DELETE ON wallet_keypool BEGIN SELECT RAISE(FAIL, 'fixture cleanup failure'); END;").unwrap();
        service
            .reserve_receive_index("w".into(), "Ethereum".into(), 0)
            .await
            .unwrap();
        assert!(service
            .apply_state_command(StateCommand::SelectChainForFamily {
                chain_id: "ethereum-sepolia".into()
            })
            .await
            .is_err());
        assert_eq!(
            service
                .app_state()
                .await
                .settings
                .selected_chain_for_family(Chain::Ethereum),
            Chain::Ethereum
        );
        db.execute_batch("DROP TRIGGER reject_cleanup;").unwrap();
        service
            .apply_state_command(StateCommand::SelectChainForFamily {
                chain_id: "ethereum-sepolia".into(),
            })
            .await
            .unwrap();
        assert!(service.keypool.read().await.is_empty());
        let reopened = WalletService::new(vec![]).unwrap();
        assert_eq!(
            reopened
                .open_state(path.to_string_lossy().into())
                .await
                .unwrap()
                .settings
                .selected_chain_for_family(Chain::Ethereum),
            Chain::EthereumSepolia
        );
    }
    #[tokio::test]
    async fn wallet_delete_removes_secrets_and_relations_and_can_retry() {
        let service = WalletService::new(vec![]).unwrap();
        let path = std::env::temp_dir().join(format!(
            "delete-owned-{}.sqlite",
            crate::store::new_event_id()
        ));
        service
            .open_state(path.to_string_lossy().into())
            .await
            .unwrap();
        let secrets = Arc::new(crate::store::secret_backends::InMemorySecretStore::new());
        crate::store::wallet_secrets::store_seed_phrase(
            &*secrets,
            "w",
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            None,
        )
        .unwrap();
        service.set_secret_store(secrets);
        service
            .apply_state_command(StateCommand::UpsertWallet {
                wallet: WalletState::single_address(
                    "w",
                    "W",
                    "Ethereum",
                    "0x1111111111111111111111111111111111111111",
                    None,
                    false,
                ),
            })
            .await
            .unwrap();
        service
            .reserve_receive_index("w".into(), "Ethereum".into(), 0)
            .await
            .unwrap();
        service
            .apply_state_command(StateCommand::RemoveWallet {
                wallet_id: "w".into(),
            })
            .await
            .unwrap();
        service
            .apply_state_command(StateCommand::RemoveWallet {
                wallet_id: "w".into(),
            })
            .await
            .unwrap();
        assert!(
            !service
                .wallet_secret_state("w".into())
                .unwrap()
                .has_signing_material
        );
        assert!(service.keypool.read().await.is_empty());
        let reopened = WalletService::new(vec![]).unwrap();
        assert!(reopened
            .open_state(path.to_string_lossy().into())
            .await
            .unwrap()
            .wallets
            .is_empty());
        assert!(reopened.keypool.read().await.is_empty());
    }
}

#[cfg(test)]
mod concurrency_tests {
    use super::*;
    use crate::service::app_refresh::AppRefreshIntent;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn requested_refresh_is_bounded_and_shares_overlapping_wallet_reads() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (sent, mut requests) = tokio::sync::mpsc::unbounded_channel();
        let permits = Arc::new(tokio::sync::Semaphore::new(0));
        let release = permits.clone();
        let server = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let sent = sent.clone();
                let release = release.clone();
                connections.spawn(async move {
                    let mut request = Vec::new();
                    let mut buf = [0; 2048];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        let n = socket.read(&mut buf).await.unwrap();
                        if n == 0 { return; }
                        request.extend_from_slice(&buf[..n]);
                    }
                    let request = String::from_utf8(request).unwrap();
                    let body = if request.lines().next().unwrap().contains("/payments") {
                        r#"{"_embedded":{"records":[]}}"#
                    } else {
                        sent.send(()).unwrap();
                        release.acquire().await.unwrap().forget();
                        r#"{"sequence":"1","balances":[{"asset_type":"native","balance":"2.0000000"}]}"#
                    };
                    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();
                });
            }
        });
        let service = WalletService::new(vec![ChainEndpoints {
            chain_id: "stellar".into(),
            endpoints: vec![endpoint],
        }])
        .unwrap();
        let path = std::env::temp_dir().join(format!(
            "balance-concurrency-{}.sqlite",
            crate::store::new_event_id()
        ));
        service
            .open_state(path.to_string_lossy().into())
            .await
            .unwrap();
        for i in 0..10 {
            service
                .apply_state_command(StateCommand::UpsertWallet {
                    wallet: WalletState::single_address(
                        format!("w{i}"),
                        "Concurrent",
                        "Stellar",
                        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
                        None,
                        true,
                    ),
                })
                .await
                .unwrap();
        }
        let requested = service.clone();
        let refresh = tokio::spawn(async move {
            requested
                .refresh_app(
                    AppRefreshIntent::AfterSend {
                        chain_id: "stellar".into(),
                    },
                    crate::fetch::refresh_policy::DeviceConditions {
                        app_is_active: true,
                        is_network_reachable: true,
                        is_constrained_network: false,
                        is_expensive_network: false,
                        is_low_power_mode: false,
                        battery_level: 1.0,
                        wants_price_refresh: false,
                    },
                )
                .await
        });
        for _ in 0..8 {
            tokio::time::timeout(Duration::from_secs(10), requests.recv())
                .await
                .unwrap()
                .unwrap();
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(50), requests.recv())
                .await
                .is_err()
        );
        // Poll an overlapping reader into the in-flight work before releasing the node.
        let overlapping = service.refresh_wallet_balances("w0".into());
        tokio::pin!(overlapping);
        assert!(futures::poll!(&mut overlapping).is_pending());
        permits.add_permits(10);
        let result = tokio::time::timeout(Duration::from_secs(10), refresh)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(result.failures.is_empty(), "{:?}", result.failures);
        assert_eq!(
            requests.len(),
            2,
            "one native request per wallet, including the overlapping read"
        );
        // Even an unpolled waiter must not turn a completed read into a cache.
        permits.add_permits(1);
        assert_eq!(
            service
                .refresh_wallet_balances("w0".into())
                .await
                .unwrap()
                .holdings[0]
                .amount,
            2.0
        );
        assert_eq!(requests.len(), 3);
        assert_eq!(overlapping.await.unwrap().holdings[0].amount, 2.0);
        let reopened = WalletService::new(vec![]).unwrap();
        let state = reopened
            .open_state(path.to_string_lossy().into())
            .await
            .unwrap();
        assert!(state.wallets.iter().all(|w| w.holdings[0].amount == 2.0));
        server.abort();
    }
}
