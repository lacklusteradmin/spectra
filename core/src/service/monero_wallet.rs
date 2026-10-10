//! Core-owned local Monero sync cache and send preparation.
use super::*;
use crate::send::monero_local::{self, LocalWallet, PreparedMoneroTransaction};
use crate::store::secret_store::SecretClass;
use ::monero_wallet::address::MoneroAddress;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct MoneroSyncStatus {
    pub wallet_id: String,
    pub scanned_height: u64,
    pub target_height: u64,
    pub unlocked_piconeros: u64,
    pub complete: bool,
    /// Whether the wallet sees what it spends. A view-only wallet has no
    /// spend key, so no key image: its balance is what it received, and an
    /// output spent elsewhere still counts in it.
    pub spends_known: bool,
    /// Each account an output arrived in, with its highest address index
    /// one arrived at: what the scan watches wallet2's lookahead past.
    pub used_subaddresses: Vec<MoneroSubaddress>,
}

/// A Monero subaddress index: account, then address within it; `(0, 0)` is
/// the primary address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct MoneroSubaddress {
    pub account: u32,
    pub address: u32,
}

fn status(
    wallet: &LocalWallet,
    spends_known: bool,
) -> Result<MoneroSyncStatus, SpectraBridgeError> {
    Ok(MoneroSyncStatus {
        wallet_id: wallet.wallet_id.clone(),
        scanned_height: wallet.next_height,
        target_height: wallet.target_height,
        unlocked_piconeros: wallet.balance()?,
        complete: wallet.target_height > 0 && wallet.next_height >= wallet.target_height,
        spends_known,
        used_subaddresses: wallet
            .used_subaddresses
            .iter()
            .map(|(&account, &address)| MoneroSubaddress { account, address })
            .collect(),
    })
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    pub async fn monero_sync_status(
        &self,
        wallet_id: String,
    ) -> Result<Option<MoneroSyncStatus>, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let state = this.app_state().await;
            let wallet = state
                .wallets
                .iter()
                .find(|w| w.id == wallet_id)
                .ok_or_else(|| SpectraBridgeError::failure("Wallet removed"))?;
            let chain = wallet.chain_id;
            if chain.mainnet_counterpart() != Chain::Monero {
                return Ok(None);
            }
            let spends_known = !wallet.is_watch_only();
            let db = this.bound_database().await?;
            if crate::wallet_db::scan_cache_load(&db, &wallet_id, chain)?.is_none() {
                // Not scanned yet: the scan will start at the restore height
                // the wallet was imported with.
                return Ok(Some(MoneroSyncStatus {
                    wallet_id,
                    scanned_height: wallet.restore_height.unwrap_or(0),
                    target_height: 0,
                    unlocked_piconeros: 0,
                    complete: false,
                    spends_known,
                    used_subaddresses: Vec::new(),
                }));
            }
            let (_, cached, _) = this.load_monero(&wallet_id).await?;
            Ok(Some(status(&cached, spends_known)?))
        })
        .await
    }

    /// Bounded, durable scan batch. Both shells can await batches until complete;
    /// cancellation between batches loses no progress. No key is sent to a server.
    /// The first batch starts at the restore height the wallet was imported
    /// with, which nothing changes afterwards. A view-only wallet scans with
    /// its view key alone and takes no password; a signing wallet unseals its
    /// spend key, which the key images of what it spends need.
    pub async fn sync_monero_wallet(
        &self,
        wallet_id: String,
        password: Option<String>,
    ) -> Result<MoneroSyncStatus, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let password = password.map(Zeroizing::new);
            let wallet = this.stored_wallet(&wallet_id).await?;
            let chain = wallet.chain_id;
            let restore_height = wallet.restore_height.unwrap_or(0);
            chain.monero_network_name()?;
            let address = wallet
                .address_on(chain)
                .ok_or_else(|| SpectraBridgeError::failure("Monero wallet has no address"))?
                .to_string();
            let spends_known = !wallet.is_watch_only();
            let keys = if spends_known {
                let signer = this
                    .resolve_send_identity(chain, &wallet_id, password.as_ref().map(|p| p.as_str()))
                    .await?;
                let keys = monero_local::ScanKeys::signing(chain, &signer.private_key_hex)?;
                // A wallet copied from another network's phrase first meets
                // its view key here.
                this.secrets()?.save_secret(
                    SecretClass::Generic,
                    format!("{wallet_id}.scan-key"),
                    hex::encode(*keys.view.view),
                )?;
                keys
            } else {
                monero_local::ScanKeys {
                    view: this
                        .monero_view_keys(&wallet_id, chain, &address)?
                        .ok_or_else(|| SpectraBridgeError::failure("Monero view key missing"))?,
                    spend: None,
                }
            };
            if keys.view.address.to_string() != address {
                return Err(SpectraBridgeError::failure(
                    "Monero keys do not belong to the wallet's address",
                ));
            }
            let _guard = this.lock_sender(chain, &address).await?;
            let db = this.bound_database().await?;
            let (revision, mut cached, key) =
                if crate::wallet_db::scan_cache_load(&db, &wallet_id, chain)?.is_some() {
                    let (r, w, k) = this.load_monero(&wallet_id).await?;
                    (Some(r), w, k)
                } else {
                    (
                        None,
                        LocalWallet {
                            wallet_id: wallet_id.clone(),
                            chain_id: chain,
                            sender: address.clone(),
                            restore_height,
                            next_height: restore_height,
                            timestamps: Vec::new(),
                            last_hash: None,
                            target_height: 0,
                            outputs: Vec::new(),
                            transfers: Vec::new(),
                            used_subaddresses: Default::default(),
                        },
                        cache_key(&wallet_id, keys.view.view.as_slice()),
                    )
                };
            let endpoint = this
                .monero_endpoint(chain, &[EndpointCapability::Verification])
                .await?;
            let rpc = crate::api::monero_daemon_rpc::daemon(&endpoint, chain).await?;
            let handed_out = this.monero_handed_out(&wallet_id, chain).await?;
            monero_local::scan(&mut cached, &rpc, &keys, handed_out, 500).await?;
            this.save_monero(revision, &cached, &key).await?;
            this.rotate_monero_receive(&wallet_id, chain, &cached)
                .await?;
            status(&cached, spends_known)
        })
        .await
    }
}
fn cache_key(wallet_id: &str, view: &[u8]) -> Zeroizing<Vec<u8>> {
    let mut hash = Sha256::new();
    hash.update(b"Spectra Monero local cache v1");
    hash.update(wallet_id.as_bytes());
    hash.update(view);
    Zeroizing::new(hash.finalize().to_vec())
}
impl WalletService {
    /// The wallet's scan keys from its stored view key and primary address;
    /// `None` until the view key is stored.
    pub(super) fn monero_view_keys(
        &self,
        wallet_id: &str,
        chain: Chain,
        address: &str,
    ) -> Result<Option<crate::derivation::monero::ViewKeys>, SpectraBridgeError> {
        let view = match self
            .secrets()?
            .load_secret(SecretClass::Generic, format!("{wallet_id}.scan-key"))
        {
            Ok(view) => Zeroizing::new(view),
            Err(crate::store::secret_store::SecretStoreError::NotFound) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        Ok(Some(crate::derivation::monero::view_keys(
            chain, address, &view,
        )?))
    }

    /// The highest account-0 address index the wallet has handed out.
    async fn monero_handed_out(
        &self,
        wallet_id: &str,
        chain: Chain,
    ) -> Result<u32, SpectraBridgeError> {
        let reserved = self
            .keypool_state(wallet_id.to_string(), chain)
            .await?
            .reserved_receive_index
            .unwrap_or(0);
        Ok(u32::try_from(reserved).unwrap_or(0))
    }

    /// Move the receive address past one an output has arrived at, so the
    /// next payer gets a fresh subaddress.
    async fn rotate_monero_receive(
        &self,
        wallet_id: &str,
        chain: Chain,
        wallet: &LocalWallet,
    ) -> Result<(), SpectraBridgeError> {
        if let Some(&used) = wallet.used_subaddresses.get(&0) {
            self.advance_receive_past(wallet_id.to_string(), chain, i64::from(used))
                .await?;
        }
        Ok(())
    }

    /// Account 0's address at the wallet's reserved receive index: its
    /// primary address until an output arrives there, then the next
    /// subaddress. `None` until the view key is stored.
    pub(super) async fn monero_receive_address(
        &self,
        wallet_id: &str,
        chain: Chain,
        primary: &str,
        reserve: bool,
    ) -> Result<Option<String>, SpectraBridgeError> {
        let Some(keys) = self.monero_view_keys(wallet_id, chain, primary)? else {
            return Ok(None);
        };
        let index = if reserve {
            self.reserve_receive_index(wallet_id.to_string(), chain, 0)
                .await?
        } else {
            self.keypool_state(wallet_id.to_string(), chain)
                .await?
                .reserved_receive_index
                .unwrap_or(0)
        };
        let index = u32::try_from(index)
            .map_err(|_| SpectraBridgeError::failure("receive index is out of range"))?;
        let address = crate::derivation::monero::subaddress(&keys, 0, index)?;
        if reserve {
            self.register_owned_address(
                wallet_id.to_string(),
                chain,
                address.clone(),
                None,
                Some("external".to_string()),
                Some(i64::from(index)),
            )
            .await?;
        }
        Ok(Some(address))
    }

    pub(super) async fn monero_history(
        &self,
        chain: Chain,
        address: &str,
    ) -> Result<String, SpectraBridgeError> {
        let state = self.app_state().await;
        let owner = state
            .wallets
            .iter()
            .find(|w| w.chain_id == chain && w.address_on(chain) == Some(address))
            .ok_or_else(|| {
                SpectraBridgeError::failure("Monero history requires an owned local wallet")
            })?;
        let (_, wallet, _) = self.load_monero(&owner.id).await?;
        if wallet.next_height < wallet.target_height {
            return Err(SpectraBridgeError::failure(
                "Finish Monero sync before reading history",
            ));
        }
        Ok(serde_json::to_string(&wallet.transfers)?)
    }
    pub(super) async fn monero_endpoint(
        &self,
        chain: Chain,
        required: &[EndpointCapability],
    ) -> Result<String, SpectraBridgeError> {
        self.endpoints_for(chain, required)
            .await
            .first()
            .cloned()
            .ok_or_else(|| {
                crate::SpectraBridgeError::failure(
                    "Configure a Monero daemon endpoint before syncing",
                )
            })
    }
    async fn load_monero(
        &self,
        wallet_id: &str,
    ) -> Result<(u64, LocalWallet, Zeroizing<Vec<u8>>), SpectraBridgeError> {
        let view = Zeroizing::new(
            self.secrets()?
                .load_secret(SecretClass::Generic, format!("{wallet_id}.scan-key"))?,
        );
        let view = Zeroizing::new(hex::decode(view.as_str())?);
        if view.len() != 32 {
            return Err(SpectraBridgeError::failure("Invalid Monero local view key"));
        }
        let key = cache_key(wallet_id, &view);
        let state = self.app_state().await;
        let owner = state
            .wallets
            .iter()
            .find(|w| w.id == wallet_id)
            .ok_or_else(|| SpectraBridgeError::failure("Wallet removed"))?;
        let chain = owner.chain_id;
        let (revision, payload) = crate::wallet_db::scan_cache_load(
            self.bound_database().await?.as_ref(),
            wallet_id,
            chain,
        )?
        .ok_or_else(|| {
            SpectraBridgeError::failure(
                "Sync the local Monero wallet before building a transaction",
            )
        })?;
        let plaintext = Zeroizing::new(crate::store::seed_envelope::decrypt(
            payload.as_bytes(),
            &key,
        )?);
        let wallet: LocalWallet = serde_json::from_str(&plaintext)?;

        if wallet.wallet_id != wallet_id
            || wallet.chain_id != owner.chain_id
            || owner.address_on(chain) != Some(wallet.sender.as_str())
        {
            return Err(SpectraBridgeError::failure(
                "Monero scan cache identity mismatch",
            ));
        }
        Ok((revision, wallet, key))
    }
    async fn save_monero(
        &self,
        revision: Option<u64>,
        wallet: &LocalWallet,
        key: &[u8],
    ) -> Result<(), SpectraBridgeError> {
        let plain = Zeroizing::new(serde_json::to_vec(wallet)?);
        let encrypted = String::from_utf8(crate::store::seed_envelope::encrypt(&plain, key)?)
            .map_err(SpectraBridgeError::failure)?;
        let _writer = self.state_writer.lock().await;
        let state = self.wallet_state.read().await;
        if !state
            .wallets
            .iter()
            .any(|w| w.id == wallet.wallet_id && w.chain_id == wallet.chain_id)
        {
            return Err(SpectraBridgeError::failure(
                "Wallet removed during Monero sync",
            ));
        }
        crate::wallet_db::scan_cache_save(
            self.bound_database().await?.as_ref(),
            &wallet.wallet_id,
            wallet.chain_id,
            revision,
            &encrypted,
        )?;
        Ok(())
    }
    pub(super) async fn prepare_monero(
        &self,
        request: &crate::send::SendExecutionRequest,
        amount: u64,
    ) -> Result<PreparedMoneroTransaction, SpectraBridgeError> {
        let (_, initial, _) = self.load_monero(&request.wallet_id).await?;
        let chain = initial.chain_id;
        let _guard = self.lock_sender(chain, &initial.sender).await?;
        let (_, mut wallet, key) = self.load_monero(&request.wallet_id).await?;
        let db = self.bound_database().await?;
        let wallet_id = request.wallet_id.clone();
        let saved_sends = tokio::task::spawn_blocking(move || {
            crate::wallet_db::signed_sends_for_wallet(&db, chain, &wallet_id)
        })
        .await??;
        let reserved: std::collections::HashSet<_> = saved_sends
            .into_iter()
            .filter_map(|saved| {
                if let crate::send::stages::PreparedPayload::Monero(p) = saved.prepared {
                    Some(p.input_key_images)
                } else {
                    None
                }
            })
            .flatten()
            .collect();
        for output in &mut wallet.outputs {
            if output
                .key_image
                .as_ref()
                .is_some_and(|image| reserved.contains(image))
            {
                output.spent = true;
            }
        }
        let pair = self
            .monero_view_keys(&request.wallet_id, chain, &wallet.sender)?
            .ok_or_else(|| SpectraBridgeError::failure("Monero view key missing"))?
            .pair()?;
        let rpc = crate::api::monero_daemon_rpc::daemon(
            &self
                .monero_endpoint(
                    chain,
                    &[EndpointCapability::Verification, EndpointCapability::Fee],
                )
                .await?,
            chain,
        )
        .await?;
        Ok(monero_local::prepare(&wallet, &rpc, pair, &request.to_address, amount, &key).await?)
    }
    /// Called under the sender lock by sign_send. Scan new blocks before spending.
    pub(super) async fn sign_monero(
        &self,
        prepared: &PreparedMoneroTransaction,
        wallet_id: &str,
        private: &str,
    ) -> Result<(String, String), SpectraBridgeError> {
        let (revision, mut wallet, key) = self.load_monero(wallet_id).await?;
        let chain = wallet.chain_id;
        let rpc = crate::api::monero_daemon_rpc::daemon(
            &self
                .monero_endpoint(chain, &[EndpointCapability::Verification])
                .await?,
            chain,
        )
        .await?;
        let keys = monero_local::ScanKeys::signing(chain, private)?;
        let handed_out = self.monero_handed_out(wallet_id, chain).await?;
        monero_local::scan(&mut wallet, &rpc, &keys, handed_out, 500).await?;
        self.save_monero(Some(revision), &wallet, &key).await?;
        self.rotate_monero_receive(wallet_id, chain, &wallet)
            .await?;
        if wallet.next_height < wallet.target_height {
            return Err(SpectraBridgeError::failure(
                "Monero sync is behind; finish syncing before signing",
            ));
        }
        Ok(prepared.sign(private, &key, &wallet)?)
    }
}

/// What proves a Monero payment to its recipient: the transaction, its
/// key and the address it paid, which monero-wallet-cli checks with
/// `check_tx_key <txid> <tx key> <address>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct MoneroPaymentProof {
    pub txid: String,
    /// The transaction key `r`, hex. It shows whoever holds it what the
    /// transaction paid the address, and nothing else.
    pub tx_key: String,
    pub address: String,
    /// What the proof shows the address received, as an exact decimal of
    /// XMR.
    pub amount: String,
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// The proof of a payment the wallet sent, derived again from the
    /// transaction's stored plan and checked against its signed bytes before
    /// it is shown. `None` for a transaction this device did not sign.
    pub async fn monero_payment_proof(
        &self,
        wallet_id: String,
        txid: String,
    ) -> Result<Option<MoneroPaymentProof>, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let wallet = this.stored_wallet(&wallet_id).await?;
            let chain = wallet.chain_id;
            if chain.mainnet_counterpart() != Chain::Monero {
                return Ok(None);
            }
            let db = this.bound_database().await?;
            let id = wallet_id.clone();
            let sends = tokio::task::spawn_blocking(move || {
                crate::wallet_db::signed_sends_for_wallet(&db, chain, &id)
            })
            .await??;
            let Some((prepared, raw)) = sends.into_iter().find_map(|stored| {
                let crate::send::stages::PreparedPayload::Monero(prepared) = stored.prepared else {
                    return None;
                };
                let submission = stored.submission?;
                (submission.transaction_hash.as_deref() == Some(txid.as_str()))
                    .then_some((prepared, submission.payload))
            }) else {
                return Ok(None);
            };
            let (_, _, key) = this.load_monero(&wallet_id).await?;
            let tx_key = prepared.transaction_key(&key)?;
            let raw = hex::decode(raw)?;
            let transaction = ::monero_wallet::transaction::Transaction::read(&mut raw.as_slice())
                .map_err(SpectraBridgeError::failure)?;
            if hex::encode(transaction.hash()) != txid {
                return Err(SpectraBridgeError::failure(
                    "The stored Monero transaction is not the one asked for",
                ));
            }
            let address = MoneroAddress::from_str_with_unchecked_network(&prepared.recipient)
                .map_err(SpectraBridgeError::failure)?;
            let received = monero_local::received_with_tx_key(&transaction, &tx_key, &address)?;
            // A key that does not prove the payment is not shown as its proof.
            if !received.contains(&prepared.amount) {
                return Err(SpectraBridgeError::failure(
                    "The Monero transaction key does not prove this payment",
                ));
            }
            let total: u128 = received.iter().map(|amount| u128::from(*amount)).sum();
            Ok(Some(MoneroPaymentProof {
                txid,
                tx_key: hex::encode(*tx_key),
                address: prepared.recipient,
                amount: crate::decimal::from_units(total, u32::from(chain.native_decimals())),
            }))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_daemon_fixture_keeps_keys_local_and_binds_reviewed_content() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/monero-local.json")).unwrap();
        let private = crate::derivation::monero::derive_monero(
            crate::derivation::phrase::test_phrase(crate::registry::Chain::Monero).into(),
            true,
            true,
            true,
        )
        .unwrap()
        .private_key_hex
        .unwrap();
        let key = cache_key(
            fixture["wallet_id"].as_str().unwrap(),
            &hex::decode(&private).unwrap()[32..],
        );
        let wallet: LocalWallet = serde_json::from_str(
            &crate::store::seed_envelope::decrypt(
                fixture["cache"].as_str().unwrap().as_bytes(),
                &key,
            )
            .unwrap(),
        )
        .unwrap();
        let prepared: PreparedMoneroTransaction =
            serde_json::from_value(fixture["prepared"].clone()).unwrap();
        assert!(wallet.balance().unwrap() > prepared.amount + prepared.fee);
        let raw = hex::decode(fixture["accepted_raw"].as_str().unwrap()).unwrap();
        let accepted =
            ::monero_wallet::transaction::Transaction::read(&mut raw.as_slice()).unwrap();
        assert_eq!(hex::encode(accepted.hash()), fixture["txid"]);
        let (raw, hash) = prepared.sign(&private, &key, &wallet).unwrap();
        let raw = hex::decode(raw).unwrap();
        let signed = ::monero_wallet::transaction::Transaction::read(&mut raw.as_slice()).unwrap();
        assert_eq!(hash, hex::encode(signed.hash()));
        assert_eq!(signed.prefix(), accepted.prefix());
        let mut changed = prepared.clone();
        changed.amount += 1;
        assert!(changed.sign(&private, &key, &wallet).is_err());
        changed = prepared.clone();
        changed.recipient.push('1');
        assert!(changed.sign(&private, &key, &wallet).is_err());
        assert!(prepared.sign(&private, &[0; 32], &wallet).is_err());
        let mut spent = wallet.clone();
        for o in &mut spent.outputs {
            o.spent = true;
        }
        assert!(prepared.sign(&private, &key, &spent).is_err());
        let mut locked = wallet.clone();
        locked.next_height = locked.restore_height + 1;
        assert_eq!(locked.balance().unwrap(), 0);
        assert!(prepared.sign(&private, &key, &locked).is_err());
    }

    /// The key derived from the stored plan is the one the daemon-accepted
    /// transaction was built with, and proves exactly the payment.
    #[test]
    fn the_tx_key_proves_the_payment_the_daemon_accepted() {
        use curve25519_dalek::{constants::ED25519_BASEPOINT_TABLE, scalar::Scalar as Dalek};
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/monero-local.json")).unwrap();
        let private = crate::derivation::monero::derive_monero(
            crate::derivation::phrase::test_phrase(crate::registry::Chain::Monero).into(),
            true,
            true,
            true,
        )
        .unwrap()
        .private_key_hex
        .unwrap();
        let key = cache_key(
            fixture["wallet_id"].as_str().unwrap(),
            &hex::decode(&private).unwrap()[32..],
        );
        let prepared: PreparedMoneroTransaction =
            serde_json::from_value(fixture["prepared"].clone()).unwrap();
        let tx_key = prepared.transaction_key(&key).unwrap();
        let raw = hex::decode(fixture["accepted_raw"].as_str().unwrap()).unwrap();
        let accepted =
            ::monero_wallet::transaction::Transaction::read(&mut raw.as_slice()).unwrap();
        let recipient =
            MoneroAddress::from_str_with_unchecked_network(&prepared.recipient).unwrap();
        // Its public half is the transaction's own key; no additional keys.
        let (keys, additional) =
            ::monero_wallet::extra::Extra::read(&mut accepted.prefix().extra.as_slice())
                .unwrap()
                .keys()
                .unwrap();
        assert!(additional.is_none());
        let r = Dalek::from_canonical_bytes(*tx_key).unwrap();
        let public = if recipient.is_subaddress() {
            r * recipient.spend().into()
        } else {
            &r * ED25519_BASEPOINT_TABLE
        };
        assert_eq!(keys[0].compress().to_bytes(), public.compress().to_bytes());
        // The fixture pays its own address: the payment and the change both
        // go to it, and the payment is one of them.
        assert_eq!(prepared.sender, prepared.recipient);
        let received =
            crate::send::monero_local::received_with_tx_key(&accepted, &tx_key, &recipient)
                .unwrap();
        assert_eq!(received.len(), 2);
        assert!(received.contains(&prepared.amount));
        // Another address received nothing, and another key proves nothing.
        let other_address = MoneroAddress::from_str_with_unchecked_network(
            "44AFFq5kSiGBoZ4NMDwYtN18obc8AemS33DBLWs3H7otXft3XjrpDtQGv7SqSsaBYBb98uNbr2VBBEt7f2wfn3RVGQBEP3A",
        )
        .unwrap();
        assert!(
            crate::send::monero_local::received_with_tx_key(&accepted, &tx_key, &other_address)
                .unwrap()
                .is_empty()
        );
        let mut other = *tx_key;
        other[0] ^= 1;
        assert!(
            crate::send::monero_local::received_with_tx_key(&accepted, &other, &recipient)
                .unwrap()
                .is_empty()
        );
    }
}

#[cfg(test)]
#[path = "tests/monero_wallet.rs"]
mod monero_wallet_tests;
