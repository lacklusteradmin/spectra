//! Bitcoin single-address and HD history orchestration.
use super::history_refresh::{
    HistoryRefreshOutcome, HistoryWalletDiagnostics, SENTINEL_CREATED_AT_UNIX,
};
use super::*;

/// How many records a Bitcoin page asks for when the caller names no limit.
const DEFAULT_BITCOIN_LIMIT: u32 = 20;
const MIN_BITCOIN_LIMIT: u32 = 10;
const MAX_BITCOIN_LIMIT: u32 = 100;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Fetch and merge Bitcoin history for its wallets.
    ///
    /// Bitcoin is the one chain with an account xpub, so its history is the HD
    /// range's rather than one address's: with a seed to hand, the account
    /// xpub is derived and the whole range walked; failing that a stored xpub
    /// is walked, and failing that the single stored address is fetched. The
    /// front end held all three arms — it read the seed out of the Keychain,
    /// cut the account path out of the wallet's derivation path by string
    /// surgery, derived the xpub, chose between the results and built the
    /// records. All of it is here, over the seed and paths core already holds.
    ///
    /// A wallet whose seed is behind a password derives no xpub — the same as
    /// before, where the Keychain read simply returned nothing — and falls
    /// through to its stored address.
    pub async fn refresh_bitcoin_history(
        &self,
        wallet_ids: Vec<String>,
        load_more: bool,
        limit: Option<u32>,
    ) -> Result<HistoryRefreshOutcome, SpectraBridgeError> {
        let chain = Chain::Bitcoin;
        let chain_id = chain.str_id().to_string();
        let limit = limit
            .unwrap_or(DEFAULT_BITCOIN_LIMIT)
            .clamp(MIN_BITCOIN_LIMIT, MAX_BITCOIN_LIMIT);
        let (wallets, settings): (Vec<crate::store::state::WalletSummary>, _) = {
            let state = self.app_state().await;
            let wallets = state
                .wallets
                .iter()
                .filter(|wallet| wallet.chain_name == chain.chain_display_name())
                .filter(|wallet| {
                    wallet_ids.is_empty()
                        || wallet_ids
                            .iter()
                            .any(|id| id.eq_ignore_ascii_case(&wallet.id))
                })
                .cloned()
                .collect();
            (wallets, state.settings.clone())
        };
        if wallets.is_empty() {
            return Ok(HistoryRefreshOutcome::nothing());
        }

        let mut incoming = Vec::new();
        let mut diagnostics = Vec::new();
        let mut wallets_refreshed = 0;
        let mut wallets_failed = 0;
        let mut exhausted = true;
        let mut cursor_updates = Vec::new();
        for mut wallet in wallets {
            // The clone carries the wallet's derivation secrets in the clear.
            // Taking them into the guard wipes them at the end of the
            // iteration rather than leaving them in a dropped `WalletSummary`.
            let overrides = crate::store::wallet_domain::SensitiveOverrides::take_from(&mut wallet);
            if load_more {
                if self
                    .history_cursor(chain_id.clone(), wallet.id.clone())
                    .is_exhausted
                {
                    continue;
                }
            } else {
                self.reset_history(
                    crate::service::history_cursor::HistoryScope::ChainAndWallet {
                        chain_id: chain_id.clone(),
                        wallet_id: wallet.id.clone(),
                    },
                );
            }
            let cursor = self
                .history_cursor(chain_id.clone(), wallet.id.clone())
                .next_cursor;

            let network = wallet
                .network_chain(&settings)
                .filter(|network| network.mainnet_counterpart() == chain)
                .unwrap_or(chain);
            match self
                .bitcoin_history_page(&wallet, &overrides, network, limit, cursor.as_deref())
                .await
            {
                Ok(page) => {
                    wallets_refreshed += 1;
                    exhausted = exhausted && page.next_cursor.is_none();
                    cursor_updates.push((wallet.id.clone(), page.next_cursor.clone()));
                    diagnostics.push(HistoryWalletDiagnostics {
                        wallet_id: wallet.id.clone(),
                        identifier: page.identifier.clone(),
                        source_used: page.source_used.clone(),
                        transaction_count: page.snapshots.len() as u32,
                        next_cursor: page.next_cursor.clone(),
                        error: None,
                    });
                    incoming.extend(page.snapshots.into_iter().map(|snapshot| {
                        bitcoin_record(&wallet, chain, &page.source_used, snapshot)
                    }));
                }
                Err(error) => {
                    wallets_failed += 1;
                    exhausted = false;
                    // The cursor is left where it was. `advance_history_cursor`
                    // with `None` means "the chain says there is no more",
                    // which a fetch that failed did not say: it marked the
                    // wallet exhausted, so this outcome reported more to load
                    // while the wallet's own cursor refused to load it, and
                    // "load more" did nothing until a pull-to-refresh reset it.
                    diagnostics.push(HistoryWalletDiagnostics {
                        wallet_id: wallet.id.clone(),
                        identifier: bitcoin_identifier(&wallet).unwrap_or_default(),
                        source_used: "none".to_string(),
                        transaction_count: 0,
                        next_cursor: None,
                        error: Some(error.to_string()),
                    });
                }
            }
        }

        if wallets_refreshed == 0 && wallets_failed == 0 {
            return Ok(HistoryRefreshOutcome::nothing());
        }
        let change = self
            .apply_transaction_command(crate::service::types::TransactionCommand::Merge {
                incoming,
                chain_name: chain.chain_display_name().to_string(),
                preserve_created_at_sentinel_unix: Some(SENTINEL_CREATED_AT_UNIX),
            })
            .await?;

        // A failed database write must not consume fetched history.
        for (wallet_id, next) in cursor_updates {
            self.advance_history_cursor(chain_id.clone(), wallet_id, next);
        }
        Ok(HistoryRefreshOutcome {
            wallets_refreshed,
            wallets_failed,
            added: change.added.len() as u32,
            updated: change.updated.len() as u32,
            exhausted,
            diagnostics,
        })
    }
}

/// One wallet's Bitcoin history page, and which source answered.
pub(crate) struct BitcoinHistoryPage {
    pub snapshots: Vec<crate::history::CoreBitcoinHistorySnapshot>,
    pub next_cursor: Option<String>,
    pub source_used: String,
    pub identifier: String,
}

/// What a diagnostics row calls this wallet: its address, else its xpub, else
/// its name.
fn bitcoin_identifier(wallet: &crate::store::state::WalletSummary) -> Option<String> {
    wallet
        .address_on(Chain::Bitcoin)
        .map(str::to_string)
        .or_else(|| wallet.xpub.clone())
        .or_else(|| Some(wallet.name.clone()))
}

impl WalletService {
    /// A single address and an HD account share the same buffered pager.
    async fn bitcoin_history_page(
        &self,
        wallet: &crate::store::state::WalletSummary,
        overrides: &crate::store::wallet_domain::SensitiveOverrides,
        network: Chain,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<BitcoinHistoryPage, SpectraBridgeError> {
        use crate::derivation::xpub_walker::{derive_children_on_network, HdNetwork};
        let account = self
            .bitcoin_account_xpub(wallet, overrides, network)
            .map(|(key, script)| (key, Some(script)))
            .or_else(|| {
                wallet
                    .xpub
                    .clone()
                    .filter(|x| !x.trim().is_empty())
                    .map(|x| (x, None))
            });
        let (addresses, identifier, source_used) = if let Some((xpub, script)) = account {
            let hd_network = if network == Chain::Bitcoin {
                HdNetwork::Mainnet
            } else {
                HdNetwork::Testnet
            };
            let mut children = derive_children_on_network(&xpub, 0, 0, 20, hd_network, script)?;
            children.extend(derive_children_on_network(
                &xpub, 1, 0, 10, hd_network, script,
            )?);
            (
                children.into_iter().map(|c| c.address).collect::<Vec<_>>(),
                xpub,
                "rust.hd",
            )
        } else {
            let address = wallet
                .address_on(network)
                .filter(|a| !a.trim().is_empty())
                .ok_or("this wallet has no Bitcoin address on the selected network".to_string())?
                .to_string();
            (vec![address.clone()], address, "rust")
        };
        let client = std::sync::Arc::new(crate::fetch::chains::bitcoin::BitcoinClient::new(
            crate::http::HttpClient::shared(),
            self.endpoints_for(network.str_id()).await,
        ));
        let page = crate::fetch::bitcoin_history::page(
            network.str_id(),
            &addresses,
            cursor,
            limit as usize,
            |address, after| {
                let client = client.clone();
                async move { client.fetch_history(&address, after.as_deref()).await }
            },
        )
        .await?;
        Ok(BitcoinHistoryPage {
            snapshots: page.items,
            next_cursor: page.next_cursor,
            source_used: source_used.into(),
            identifier,
        })
    }

    /// The account xpub for this wallet's Bitcoin path, when its seed is
    /// readable. A sealed wallet has none without its password.
    ///
    /// The phrase never leaves a `Zeroizing`: `wallet_seed_phrase` hands back a
    /// plain `String` copy of it, which drops without being wiped, and this
    /// runs on every Bitcoin refresh rather than only at send time.
    /// `load_signing_material` is the same read the send identity does, and
    /// keeps the wipe.
    fn bitcoin_account_xpub(
        &self,
        wallet: &crate::store::state::WalletSummary,
        overrides: &crate::store::wallet_domain::SensitiveOverrides,
        network: Chain,
    ) -> Option<(String, crate::derivation::xpub_walker::HdScriptType)> {
        use crate::store::wallet_secrets::{load_signing_material, SigningMaterial};
        let secrets = self.secrets().ok()?;
        // A private-key wallet has no range to walk; only a phrase derives one.
        let seed = match load_signing_material(&*secrets, &wallet.id, None).ok()? {
            SigningMaterial::Mnemonic(seed) => seed,
            SigningMaterial::PrivateKey(_) => return None,
        };
        if seed.trim().is_empty() {
            return None;
        }
        // The account is the first four segments — `m/84'/0'/0'` — of the path
        // the wallet derives with. This was string surgery on the front end
        // side, over a path it read out of its own copy of the wallet.
        let path = wallet
            .addresses
            .iter()
            .find(|a| a.chain_name == network.chain_display_name())
            .and_then(|a| a.derivation_path.clone())
            .or_else(|| wallet.derivation_path.clone())
            .unwrap_or_else(|| {
                crate::app_core::default_path_from_catalog(Chain::Bitcoin.chain_display_name())
                    .unwrap_or_default()
            });
        let account_path = path.split('/').take(4).collect::<Vec<_>>().join("/");
        // The wallet's own passphrase, not the empty string. Derived without
        // it, the xpub is a different wallet's: the range walked belongs to
        // nobody here, comes back empty and the refresh quietly falls through
        // to the single stored address — so a passphrase wallet never had HD
        // history at all. The send identity has always derived with it.
        use crate::derivation::xpub_walker::HdScriptType;
        let script = match path.split('/').nth(1)? {
            "44'" => HdScriptType::P2pkh,
            "49'" => HdScriptType::P2shP2wpkh,
            "84'" => HdScriptType::P2wpkh,
            "86'" => HdScriptType::P2tr,
            _ => return None,
        };
        crate::derivation::xpub_walker::derive_account_xpub(
            &seed,
            overrides.passphrase().unwrap_or_default(),
            &account_path,
        )
        .ok()
        .map(|key| (key, script))
    }
}

/// One HD snapshot as a stored record.
fn bitcoin_record(
    wallet: &crate::store::state::WalletSummary,
    chain: Chain,
    source_used: &str,
    snapshot: crate::history::CoreBitcoinHistorySnapshot,
) -> crate::fetch::transactions::CoreTransactionRecord {
    crate::fetch::transactions::CoreTransactionRecord {
        id: crate::store::new_transaction_id(),
        wallet_id: Some(wallet.id.clone()),
        kind: snapshot.kind,
        status: snapshot.status,
        wallet_name: wallet.name.clone(),
        asset_name: chain.chain_display_name().to_string(),
        symbol: chain.coin_symbol().to_string(),
        chain_name: chain.chain_display_name().to_string(),
        amount: snapshot.amount_btc,
        address: snapshot.counterparty_address,
        transaction_hash: Some(snapshot.txid).filter(|txid| !txid.is_empty()),
        ethereum_nonce: None,
        receipt_block_number: snapshot.block_height,
        receipt_gas_used: None,
        receipt_effective_gas_price_gwei: None,
        receipt_network_fee_eth: None,
        fee_priority_raw: None,
        fee_rate_description: None,
        confirmation_count: None,
        dogecoin_confirmed_network_fee_doge: None,
        dogecoin_estimated_fee_rate_doge_per_kb: None,
        used_change_output: None,
        source_derivation_path: None,
        change_derivation_path: None,
        source_address: None,
        change_address: None,
        signed_transaction_payload: None,
        signed_transaction_payload_format: None,
        failure_reason: None,
        transaction_history_source: Some(source_used.to_string()),
        created_at_unix: if snapshot.created_at_unix > 0.0 {
            snapshot.created_at_unix
        } else {
            SENTINEL_CREATED_AT_UNIX
        },
    }
}

#[cfg(test)]
mod tests;
