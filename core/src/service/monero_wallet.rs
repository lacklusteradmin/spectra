//! Core-owned local Monero sync cache and send preparation.
use super::*;
use crate::send::monero_local::{self, LocalWallet, PreparedMoneroTransaction};
use crate::store::secret_store::SecretClass;
use ::monero_wallet::{
    address::{MoneroAddress, Network},
    ed25519::Scalar,
    ViewPair,
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
fn status(wallet: &LocalWallet) -> Result<MoneroSyncStatus, String> {
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
        let state = self.app_state().await;
        let wallet = state
            .wallets
            .iter()
            .find(|w| w.id == wallet_id)
            .ok_or("Wallet removed")?;
        let chain = chain_for_id(&wallet.chain_id)?;
        if chain.mainnet_counterpart() != Chain::Monero {
            return Ok(None);
        }
        let db = self.bound_database().await?;
        if crate::wallet_db::monero_load(&db, &wallet_id, chain.str_id())?.is_none() {
            return Ok(Some(MoneroSyncStatus {
                wallet_id,
                scanned_height: 0,
                target_height: 0,
                unlocked_piconeros: 0,
                complete: false,
            }));
        }
        let (_, cached, _) = self.load_monero(&wallet_id).await?;
        Ok(Some(status(&cached)?))
    }

    /// Bounded, durable scan batch. Both shells can await batches until complete;
    /// cancellation between batches loses no progress. No key is sent to a server.
    pub async fn sync_monero_wallet(
        &self,
        wallet_id: String,
        password: Option<String>,
        restore_height: Option<u64>,
    ) -> Result<MoneroSyncStatus, SpectraBridgeError> {
        let password = password.map(Zeroizing::new);
        let state = self.app_state().await;
        let wallet = state
            .wallets
            .iter()
            .find(|w| w.id == wallet_id)
            .ok_or("Wallet removed")?;
        let chain = chain_for_id(&wallet.chain_id)?;
        chain.monero_network_name()?;
        let signer = self
            .resolve_send_identity(chain, &wallet_id, password.as_ref().map(|p| p.as_str()))
            .await?;
        let _guard = self.lock_sender(chain, &signer.from_address).await?;
        let secret = Zeroizing::new(
            hex::decode(signer.private_key_hex.as_str()).map_err(|e| e.to_string())?,
        );
        if secret.len() != 64 {
            return Err("Invalid Monero key material".into());
        }
        let store = self.secrets()?;
        store
            .save_secret(
                SecretClass::Generic,
                format!("{wallet_id}.monero-view"),
                hex::encode(&secret[32..]),
            )
            .map_err(|e| e.to_string())?;
        let db = self.bound_database().await?;
        let (revision, mut cached, key) =
            if crate::wallet_db::monero_load(&db, &wallet_id, chain.str_id())?.is_some() {
                if restore_height.is_some() {
                    return Err(
                        "Restore height can only be set before the first Monero sync".into(),
                    );
                }
                let (r, w, k) = self.load_monero(&wallet_id).await?;
                (Some(r), w, k)
            } else {
                (
                    None,
                    LocalWallet {
                        wallet_id: wallet_id.clone(),
                        chain_id: chain.str_id().into(),
                        sender: signer.from_address.clone(),
                        restore_height: restore_height.unwrap_or(0),
                        next_height: restore_height.unwrap_or(0),
                        timestamps: Vec::new(),
                        last_hash: None,
                        target_height: 0,
                        outputs: Vec::new(),
                        transfers: Vec::new(),
                    },
                    cache_key(&wallet_id, &secret[32..]),
                )
            };
        let endpoint = self.monero_endpoint(chain).await?;
        let rpc = monero_local::daemon(&endpoint, chain).await?;
        monero_local::scan(&mut cached, &rpc, &signer.private_key_hex, 500).await?;
        self.save_monero(revision, &cached, &key).await?;
        Ok(status(&cached)?)
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
            .find(|w| w.chain_id == chain.str_id() && w.address_on(chain) == Some(address))
            .ok_or("Monero history requires an owned local wallet")?;
        let (_, wallet, _) = self.load_monero(&owner.id).await?;
        if wallet.next_height < wallet.target_height {
            return Err("Finish Monero sync before reading history".into());
        }
        Ok(serde_json::to_string(&wallet.transfers)?)
    }
    pub(super) async fn monero_endpoint(&self, chain: Chain) -> Result<String, SpectraBridgeError> {
        self.endpoints_for(chain.str_id())
            .await
            .first()
            .cloned()
            .ok_or("Configure a Monero daemon endpoint before syncing".into())
    }
    async fn load_monero(
        &self,
        wallet_id: &str,
    ) -> Result<(u64, LocalWallet, Zeroizing<Vec<u8>>), SpectraBridgeError> {
        let view = Zeroizing::new(
            self.secrets()?
                .load_secret(SecretClass::Generic, format!("{wallet_id}.monero-view"))
                .map_err(|e| e.to_string())?,
        );
        let view = Zeroizing::new(hex::decode(view.as_str()).map_err(|e| e.to_string())?);
        if view.len() != 32 {
            return Err("Invalid Monero local view key".into());
        }
        let key = cache_key(wallet_id, &view);
        let state = self.app_state().await;
        let owner = state
            .wallets
            .iter()
            .find(|w| w.id == wallet_id)
            .ok_or("Wallet removed")?;
        let chain = chain_for_id(&owner.chain_id)?;
        let (revision, payload) = crate::wallet_db::monero_load(
            self.bound_database().await?.as_ref(),
            wallet_id,
            chain.str_id(),
        )?
        .ok_or("Sync the local Monero wallet before building a transaction")?;
        let plaintext = Zeroizing::new(crate::store::seed_envelope::decrypt(
            payload.as_bytes(),
            &key,
        )?);
        let wallet: LocalWallet = serde_json::from_str(&plaintext)?;

        if wallet.wallet_id != wallet_id
            || wallet.chain_id != owner.chain_id
            || owner.address_on(chain) != Some(wallet.sender.as_str())
        {
            return Err("Monero scan cache identity mismatch".into());
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
            .map_err(|e| e.to_string())?;
        let _writer = self.state_writer.lock().await;
        let state = self.wallet_state.read().await;
        if !state
            .wallets
            .iter()
            .any(|w| w.id == wallet.wallet_id && w.chain_id == wallet.chain_id)
        {
            return Err("Wallet removed during Monero sync".into());
        }
        crate::wallet_db::monero_save(
            self.bound_database().await?.as_ref(),
            &wallet.wallet_id,
            &wallet.chain_id,
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
        let chain = chain_for_id(&initial.chain_id)?;
        let _guard = self.lock_sender(chain, &initial.sender).await?;
        let (_, mut wallet, key) = self.load_monero(&request.wallet_id).await?;
        let db = self.bound_database().await?;
        for saved in crate::wallet_db::send_list(&db)? {
            if saved.view.wallet_id == request.wallet_id
                && saved.view.stage == crate::send::stages::SendStage::Signed
            {
                if let crate::send::stages::PreparedPayload::Monero(p) = saved.prepared {
                    for output in &mut wallet.outputs {
                        if p.input_key_images.contains(&output.key_image) {
                            output.spent = true;
                        }
                    }
                }
            }
        }
        let view = Zeroizing::new(
            self.secrets()?
                .load_secret(
                    SecretClass::Generic,
                    format!("{}.monero-view", request.wallet_id),
                )
                .map_err(|e| e.to_string())?,
        );
        let view = Zeroizing::new(hex::decode(view.as_str()).map_err(|e| e.to_string())?);
        let scalar = Scalar::read(&mut view.as_slice()).map_err(|e| e.to_string())?;
        let network = if chain == Chain::Monero {
            Network::Mainnet
        } else {
            Network::Stagenet
        };
        let address =
            MoneroAddress::from_str(network, &wallet.sender).map_err(|e| e.to_string())?;
        let pair =
            ViewPair::new(address.spend(), Zeroizing::new(scalar)).map_err(|e| e.to_string())?;
        let rpc = monero_local::daemon(&self.monero_endpoint(chain).await?, chain).await?;
        Ok(monero_local::prepare(
            &wallet,
            &rpc,
            pair,
            &request.to_address,
            amount,
            &key,
            request.monero_priority.unwrap_or(2),
        )
        .await?)
    }
    /// Called under the sender lock by sign_send. Scan new blocks before spending.
    pub(super) async fn sign_monero(
        &self,
        prepared: &PreparedMoneroTransaction,
        wallet_id: &str,
        private: &str,
    ) -> Result<(String, String), SpectraBridgeError> {
        let (revision, mut wallet, key) = self.load_monero(wallet_id).await?;
        let chain = chain_for_id(&wallet.chain_id)?;
        let rpc = monero_local::daemon(&self.monero_endpoint(chain).await?, chain).await?;
        monero_local::scan(&mut wallet, &rpc, private, 500).await?;
        self.save_monero(Some(revision), &wallet, &key).await?;
        if wallet.next_height < wallet.target_height {
            return Err("Monero sync is behind; finish syncing before signing".into());
        }
        Ok(prepared.sign(private, &key, &wallet)?)
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
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into(),
            true, true, true,
        ).unwrap().private_key_hex.unwrap();
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
}
