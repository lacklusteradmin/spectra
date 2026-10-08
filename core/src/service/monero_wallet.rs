//! Core-owned local Monero sync cache and send preparation.
use super::*;
use crate::send::monero_local::{self, LocalWallet, PreparedMoneroTransaction};
use crate::store::secret_store::SecretClass;
use ::monero_wallet::{
    ViewPair,
    address::{MoneroAddress, Network},
    ed25519::Scalar,
};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct MoneroSyncStatus {
    pub wallet_id: String,
    pub scanned_height: u64,
    pub target_height: u64,
    pub unlocked_piconeros: u64,
    pub complete: bool,
}
fn status(wallet: &LocalWallet) -> Result<MoneroSyncStatus, SpectraBridgeError> {
    Ok(MoneroSyncStatus {
        wallet_id: wallet.wallet_id.clone(),
        scanned_height: wallet.next_height,
        target_height: wallet.target_height,
        unlocked_piconeros: wallet.balance()?,
        complete: wallet.target_height > 0 && wallet.next_height >= wallet.target_height,
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
                }));
            }
            let (_, cached, _) = this.load_monero(&wallet_id).await?;
            Ok(Some(status(&cached)?))
        })
        .await
    }

    /// Bounded, durable scan batch. Both shells can await batches until complete;
    /// cancellation between batches loses no progress. No key is sent to a server.
    /// The first batch starts at the restore height the wallet was imported
    /// with, which nothing changes afterwards.
    pub async fn sync_monero_wallet(
        &self,
        wallet_id: String,
        password: Option<String>,
    ) -> Result<MoneroSyncStatus, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let password = password.map(Zeroizing::new);
            let state = this.app_state().await;
            let wallet = state
                .wallets
                .iter()
                .find(|w| w.id == wallet_id)
                .ok_or_else(|| SpectraBridgeError::failure("Wallet removed"))?;
            let chain = wallet.chain_id;
            let restore_height = wallet.restore_height.unwrap_or(0);
            chain.monero_network_name()?;
            let signer = this
                .resolve_send_identity(chain, &wallet_id, password.as_ref().map(|p| p.as_str()))
                .await?;
            let _guard = this.lock_sender(chain, &signer.from_address).await?;
            let secret = Zeroizing::new(hex::decode(signer.private_key_hex.as_str())?);
            if secret.len() != 64 {
                return Err(SpectraBridgeError::failure("Invalid Monero key material"));
            }
            let store = this.secrets()?;
            store.save_secret(
                SecretClass::Generic,
                format!("{wallet_id}.scan-key"),
                hex::encode(&secret[32..]),
            )?;
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
                            sender: signer.from_address.clone(),
                            restore_height,
                            next_height: restore_height,
                            timestamps: Vec::new(),
                            last_hash: None,
                            target_height: 0,
                            outputs: Vec::new(),
                            transfers: Vec::new(),
                        },
                        cache_key(&wallet_id, &secret[32..]),
                    )
                };
            let endpoint = this
                .monero_endpoint(chain, &[EndpointCapability::Verification])
                .await?;
            let rpc = crate::api::monero_daemon_rpc::daemon(&endpoint, chain).await?;
            monero_local::scan(&mut cached, &rpc, &signer.private_key_hex, 500).await?;
            this.save_monero(revision, &cached, &key).await?;
            status(&cached)
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
            if reserved.contains(&output.key_image) {
                output.spent = true;
            }
        }
        let view = Zeroizing::new(self.secrets()?.load_secret(
            SecretClass::Generic,
            format!("{}.scan-key", request.wallet_id),
        )?);
        let view = Zeroizing::new(hex::decode(view.as_str())?);
        let scalar = Scalar::read(&mut view.as_slice()).map_err(SpectraBridgeError::failure)?;
        let network = if chain == Chain::Monero {
            Network::Mainnet
        } else {
            Network::Stagenet
        };
        let address = MoneroAddress::from_str(network, &wallet.sender)
            .map_err(SpectraBridgeError::failure)?;
        let pair = ViewPair::new(address.spend(), Zeroizing::new(scalar))
            .map_err(SpectraBridgeError::failure)?;
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
        monero_local::scan(&mut wallet, &rpc, private, 500).await?;
        self.save_monero(Some(revision), &wallet, &key).await?;
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
