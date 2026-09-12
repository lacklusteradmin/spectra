//! Fetch and commit balances against the wallet and network that requested them.
use super::*;
use crate::fetch::refresh::engine::{refresh_entries_for, RefreshEntry};
use crate::store::state::WalletSummary;
use crate::store::wallet_domain::AssetHolding;

impl WalletService {
    pub(crate) async fn refresh_wallet_balances(
        &self,
        wallet_id: String,
    ) -> Result<WalletSummary, SpectraBridgeError> {
        let state = self.app_state().await;
        let entry = refresh_entries_for(&state)
            .into_iter()
            .find(|e| e.wallet_id == wallet_id)
            .ok_or("wallet has no refreshable address")?;
        let chain = chain_for_id(&entry.chain_id)?;
        let native = self
            .fetch_native_balance_summary_auto(&entry.network_chain_id, entry.address.clone())
            .await?;
        let mut holdings = vec![AssetHolding {
            amount: balance_amount(&native.amount_display)?,
            ..native_coin_template(&entry.chain_id).ok_or("missing native asset")?
        }];
        let known: Vec<_> = state
            .token_preferences
            .iter()
            .filter(|p| {
                p.is_enabled
                    && p.hosting_chain()
                        .is_some_and(|h| h.chain_name() == chain.chain_display_name())
            })
            .cloned()
            .collect();
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
                .fetch_token_balances(
                    entry.network_chain_id.clone(),
                    entry.address.clone(),
                    descriptors,
                )
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
                        coin_gecko_id: p.token.coingecko_id.clone(),
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
    ) -> Result<WalletSummary, SpectraBridgeError> {
        for h in &holdings {
            if !h.amount.is_finite() || h.amount < 0.0 {
                return Err("invalid balance".into());
            }
        }
        let wallet_id = entry.wallet_id.clone();
        let transition = self
            .mutate_persisted_state(move |state| {
                let current = refresh_entries_for(state)
                    .into_iter()
                    .find(|e| e.wallet_id == entry.wallet_id);
                if !current.is_some_and(|e| {
                    e.address == entry.address
                        && e.network_chain_id == entry.network_chain_id
                        && e.chain_id == entry.chain_id
                }) {
                    return vec![];
                }
                let wallet = state
                    .wallets
                    .iter_mut()
                    .find(|w| w.id == entry.wallet_id)
                    .unwrap();
                let before = wallet.holdings.clone();
                merge_balances(&mut wallet.holdings, holdings);
                if wallet.holdings == before {
                    vec![]
                } else {
                    vec![crate::store::state::StateEvent {
                        kind: "walletsChanged".into(),
                        subject_id: Some(wallet.id.clone()),
                    }]
                }
            })
            .await?;
        transition
            .state
            .wallets
            .into_iter()
            .find(|w| w.id == wallet_id)
            .ok_or_else(|| "wallet removed during refresh".into())
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
fn balance_key(h: &AssetHolding) -> (String, String) {
    (
        h.chain_name.clone(),
        h.contract_address
            .as_ref()
            .map(|c| contract_key(&h.chain_name, c))
            .unwrap_or_else(|| h.symbol.clone()),
    )
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
        let service = WalletService::new_typed(vec![]).unwrap();
        let path = std::env::temp_dir().join(format!(
            "balance-owned-{}.sqlite",
            crate::store::new_event_id()
        ));
        service
            .open_state(path.to_string_lossy().into())
            .await
            .unwrap();
        let mut w = WalletSummary::single_address(
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
        service
            .apply_state_command(StateCommand::SelectNetworkChain {
                chain_id: "ethereum-sepolia".into(),
            })
            .await
            .unwrap();
        coin.amount = 99.0;
        service
            .commit_balance_result(entry, vec![coin])
            .await
            .unwrap();
        let reopened = WalletService::new_typed(vec![]).unwrap();
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
        let service = WalletService::new_typed(vec![]).unwrap();
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
            .apply_state_command(StateCommand::SelectNetworkChain {
                chain_id: "ethereum-sepolia".into()
            })
            .await
            .is_err());
        assert_eq!(
            service
                .app_state()
                .await
                .settings
                .network_chain(Chain::Ethereum),
            Chain::Ethereum
        );
        db.execute_batch("DROP TRIGGER reject_cleanup;").unwrap();
        service
            .apply_state_command(StateCommand::SelectNetworkChain {
                chain_id: "ethereum-sepolia".into(),
            })
            .await
            .unwrap();
        assert!(service.keypool.read().await.is_empty());
        let reopened = WalletService::new_typed(vec![]).unwrap();
        assert_eq!(
            reopened
                .open_state(path.to_string_lossy().into())
                .await
                .unwrap()
                .settings
                .network_chain(Chain::Ethereum),
            Chain::EthereumSepolia
        );
    }
    #[tokio::test]
    async fn wallet_delete_removes_secrets_and_relations_and_can_retry() {
        let service = WalletService::new_typed(vec![]).unwrap();
        let path = std::env::temp_dir().join(format!(
            "delete-owned-{}.sqlite",
            crate::store::new_event_id()
        ));
        service
            .open_state(path.to_string_lossy().into())
            .await
            .unwrap();
        service.set_secret_store(Arc::new(
            crate::store::secret_backends::InMemorySecretStore::new(),
        ));
        service.store_wallet_seed_phrase("w".into(), "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into(), None).unwrap();
        service
            .apply_state_command(StateCommand::UpsertWallet {
                wallet: WalletSummary::single_address(
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
        assert!(!service.wallet_secret_state("w".into()).has_signing_material);
        assert!(service.keypool.read().await.is_empty());
        let reopened = WalletService::new_typed(vec![]).unwrap();
        assert!(reopened
            .open_state(path.to_string_lossy().into())
            .await
            .unwrap()
            .wallets
            .is_empty());
        assert!(reopened.keypool.read().await.is_empty());
    }
}
