//! Device-local Monero scanning and CLSAG/Bulletproof+ signing.
//! Only public daemon requests cross the transport; keys and scan results stay local.
use crate::{fetch::http::HttpClient, registry::Chain};
use monero_daemon_rpc::{HttpTransport, MoneroDaemon};
use monero_wallet::{
    address::{MoneroAddress, Network},
    ed25519::{Point, Scalar},
    interface::prelude::*,
    ringct::RctType,
    send::{Change, SignableTransaction},
    transaction::{Input, Timelock},
    OutputWithDecoys, Scanner, ViewPair, WalletOutput,
};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

#[derive(Clone)]
pub(crate) struct DaemonTransport {
    endpoint: String,
}
impl HttpTransport for DaemonTransport {
    async fn post(
        &self,
        route: &str,
        body: Vec<u8>,
        limit: Option<usize>,
    ) -> Result<Vec<u8>, InterfaceError> {
        let error = |e: String| InterfaceError::InterfaceError(e);
        let url = format!("{}/{}", self.endpoint.trim_end_matches('/'), route);
        let mut response = HttpClient::shared()
            .reqwest_client()
            .post(url)
            .body(body)
            .send()
            .await
            .map_err(|e| error(e.to_string()))?
            .error_for_status()
            .map_err(|e| error(e.to_string()))?;
        let limit = limit.unwrap_or(100 * 1024 * 1024).min(100 * 1024 * 1024);
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| error(e.to_string()))? {
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err(error("Monero response exceeds size limit".into()));
            }
            bytes.extend(chunk);
        }
        Ok(bytes)
    }
}
pub(crate) type Daemon = MoneroDaemon<DaemonTransport>;
pub(crate) async fn daemon(endpoint: &str, chain: Chain) -> Result<Daemon, String> {
    let transport = DaemonTransport {
        endpoint: endpoint.into(),
    };
    let info: serde_json::Value = serde_json::from_slice(
        &transport
            .post("get_info", b"{}".to_vec(), Some(1024 * 1024))
            .await
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if info["nettype"].as_str() != Some(chain.monero_network_name()?)
        || info["synchronized"].as_bool() != Some(true)
    {
        return Err("Monero daemon is on the wrong network or is not synchronized".into());
    }
    let fork: serde_json::Value = serde_json::from_slice(
        &transport
            .post(
                "json_rpc",
                br#"{"jsonrpc":"2.0","id":"0","method":"hard_fork_info"}"#.to_vec(),
                Some(1024 * 1024),
            )
            .await
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if fork["result"]["version"].as_u64() != Some(16) {
        return Err("Unsupported Monero hard fork; update before sending".into());
    }
    MoneroDaemon::new(transport)
        .await
        .map_err(|e| e.to_string())
}

#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub(crate) struct LocalOutput {
    pub encoded: String,
    pub key_image: String,
    pub received_height: u64,
    pub spent: bool,
}
#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub(crate) struct LocalTransfer {
    pub txid: String,
    pub timestamp: u64,
    pub amount_piconeros: u64,
    pub fee_piconeros: u64,
    pub is_incoming: bool,
    pub block_height: u64,
}
#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub(crate) struct LocalWallet {
    pub wallet_id: String,
    pub chain_id: String,
    pub sender: String,
    pub restore_height: u64,
    pub next_height: u64,
    pub timestamps: Vec<u64>,
    pub last_hash: Option<[u8; 32]>,
    pub target_height: u64,
    pub outputs: Vec<LocalOutput>,
    pub transfers: Vec<LocalTransfer>,
}
impl LocalWallet {
    pub fn unlocked(&self) -> Result<Vec<WalletOutput>, String> {
        let mut timestamps = self.timestamps.clone();
        timestamps.sort_unstable();
        let chain_time = timestamps.get(timestamps.len() / 2).copied().unwrap_or(0);
        self.outputs
            .iter()
            .filter(|o| !o.spent && o.received_height.saturating_add(10) <= self.next_height)
            .map(|o| {
                WalletOutput::read(
                    &mut hex::decode(&o.encoded)
                        .map_err(|e| e.to_string())?
                        .as_slice(),
                )
                .map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|outputs| {
                outputs
                    .into_iter()
                    .filter(|o| match o.additional_timelock() {
                        Timelock::None => true,
                        Timelock::Block(height) => (height as u64) < self.next_height,
                        Timelock::Time(time) => time <= chain_time,
                    })
                    .collect()
            })
    }

    pub fn balance(&self) -> Result<u64, String> {
        self.unlocked()?.iter().try_fold(0u64, |sum, o| {
            sum.checked_add(o.commitment().amount)
                .ok_or("Monero balance overflow".into())
        })
    }
}

pub(crate) fn keys(private: &str) -> Result<(Zeroizing<Scalar>, ViewPair), String> {
    let raw = Zeroizing::new(hex::decode(private).map_err(|e| e.to_string())?);
    if raw.len() != 64 {
        return Err("Monero signing identity must contain spend and view keys".into());
    }
    let spend = Scalar::read(&mut &raw[..32]).map_err(|e| e.to_string())?;
    let view = Scalar::read(&mut &raw[32..]).map_err(|e| e.to_string())?;
    let spend_point = curve25519_dalek::constants::ED25519_BASEPOINT_TABLE * &spend.into();
    Ok((
        Zeroizing::new(spend),
        ViewPair::new(Point::from(spend_point), Zeroizing::new(view)).map_err(|e| e.to_string())?,
    ))
}

/// Scan a bounded batch and account for spent outputs locally, never querying key images.
pub(crate) async fn scan(
    wallet: &mut LocalWallet,
    rpc: &Daemon,
    private: &str,
    batch: u32,
) -> Result<(), String> {
    let (spend, pair) = keys(private)?;
    let target = rpc.latest_block_number().await.map_err(|e| e.to_string())? as u64;
    wallet.target_height = target.checked_add(1).ok_or("Monero height overflow")?;
    if wallet.last_hash.is_some() && wallet.next_height > wallet.target_height {
        wallet.outputs.clear();
        wallet.transfers.clear();
        wallet.timestamps.clear();
        wallet.next_height = wallet.restore_height;
        wallet.last_hash = None;
    }
    if wallet.last_hash.is_some() {
        let previous = rpc
            .scannable_block_by_number((wallet.next_height - 1) as usize)
            .await
            .map_err(|e| e.to_string())?;
        if Some(previous.block.hash()) != wallet.last_hash {
            wallet.outputs.clear();
            wallet.transfers.clear();
            wallet.timestamps.clear();
            wallet.next_height = wallet.restore_height;
            wallet.last_hash = None;
        }
    }
    if wallet.next_height > wallet.target_height {
        return Err("Monero restore height is ahead of the chain".into());
    }
    let end = wallet
        .next_height
        .saturating_add(u64::from(batch.clamp(1, 500)))
        .min(wallet.target_height);
    if end <= wallet.next_height {
        return Ok(());
    }
    let blocks = rpc
        .contiguous_scannable_blocks(wallet.next_height as usize..=(end - 1) as usize)
        .await
        .map_err(|e| e.to_string())?;
    let mut scanner = Scanner::new(pair);
    for block in blocks {
        let height = block.block.number() as u64;
        if height != wallet.next_height
            || wallet
                .last_hash
                .is_some_and(|hash| hash != block.block.header.previous)
        {
            return Err("Monero scan chain is discontinuous".into());
        }
        let hash = block.block.hash();
        wallet.timestamps.push(block.block.header.timestamp);
        if wallet.timestamps.len() > 60 {
            wallet.timestamps.remove(0);
        }
        let timestamp = block.block.header.timestamp;
        let mut transfers = std::collections::BTreeMap::<String, (u64, u64, u64)>::new();
        for (txid, tx) in block.block.transactions.iter().zip(&block.transactions) {
            let mut debit = 0_u64;
            for input in &tx.prefix().inputs {
                if let Input::ToKey { key_image, .. } = input {
                    let image = hex::encode(key_image.to_bytes());
                    if let Some(output) = wallet
                        .outputs
                        .iter()
                        .find(|o| o.key_image == image && !o.spent)
                    {
                        let output = WalletOutput::read(
                            &mut hex::decode(&output.encoded)
                                .map_err(|e| e.to_string())?
                                .as_slice(),
                        )
                        .map_err(|e| e.to_string())?;
                        debit = debit
                            .checked_add(output.commitment().amount)
                            .ok_or("Monero debit overflow")?;
                    }
                }
            }
            if debit > 0 {
                let fee = match tx {
                    monero_wallet::transaction::Transaction::V2 {
                        proofs: Some(proofs),
                        ..
                    } => proofs.base.fee,
                    _ => 0,
                };
                transfers.insert(hex::encode(txid), (0, debit, fee));
            }
        }
        let spent: std::collections::HashSet<String> = block
            .transactions
            .iter()
            .flat_map(|t| t.prefix().inputs.iter())
            .filter_map(|input| match input {
                Input::ToKey { key_image, .. } => Some(hex::encode(key_image.to_bytes())),
                _ => None,
            })
            .collect();
        for output in scanner
            .scan(block)
            .map_err(|e| e.to_string())?
            .ignore_additional_timelock()
        {
            let offset: curve25519_dalek::scalar::Scalar = output.key_offset().into();
            let spend_scalar: curve25519_dalek::scalar::Scalar = (*spend).into();
            let scalar = Zeroizing::new(spend_scalar + offset);
            let point: curve25519_dalek::EdwardsPoint =
                Point::biased_hash(output.key().compress().to_bytes()).into();
            let image = hex::encode((point * *scalar).compress().to_bytes());
            if wallet.outputs.iter().any(|o| o.key_image == image) {
                return Err("Duplicate Monero output".into());
            }
            let entry = transfers
                .entry(hex::encode(output.transaction()))
                .or_default();
            entry.0 = entry
                .0
                .checked_add(output.commitment().amount)
                .ok_or("Monero credit overflow")?;
            wallet.outputs.push(LocalOutput {
                encoded: hex::encode(output.serialize()),
                key_image: image,
                received_height: height,
                spent: false,
            });
        }
        for output in &mut wallet.outputs {
            if spent.contains(&output.key_image) {
                output.spent = true;
            }
        }
        for (txid, (credit, debit, fee)) in transfers {
            wallet.transfers.push(LocalTransfer {
                txid,
                timestamp,
                block_height: height,
                is_incoming: credit >= debit,
                amount_piconeros: if credit >= debit {
                    credit - debit
                } else {
                    debit
                        .checked_sub(credit)
                        .and_then(|n| n.checked_sub(fee))
                        .ok_or("Invalid Monero net amount")?
                },
                fee_piconeros: if debit > 0 { fee } else { 0 },
            });
        }
        wallet.next_height = height + 1;
        wallet.last_hash = Some(hash);
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PreparedMoneroTransaction {
    pub sender: String,
    pub recipient: String,
    pub amount: u64,
    pub fee: u64,
    pub input_key_images: Vec<String>,
    pub encrypted_plan: String,
}

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
struct PrivatePlan {
    sender: String,
    recipient: String,
    amount: u64,
    fee: u64,
    inputs: Vec<String>,
    encoded: String,
}

pub(crate) async fn prepare(
    wallet: &LocalWallet,
    rpc: &Daemon,
    pair: ViewPair,
    recipient: &str,
    amount: u64,
    encryption_key: &[u8],
    priority: u32,
) -> Result<PreparedMoneroTransaction, String> {
    let chain = Chain::from_str_id(&wallet.chain_id).ok_or("Unknown Monero chain")?;
    let network = match chain.monero_network_name()? {
        "mainnet" => Network::Mainnet,
        "stagenet" => Network::Stagenet,
        _ => return Err("Unsupported Monero network".into()),
    };
    let recipient_address =
        MoneroAddress::from_str(network, recipient).map_err(|e| e.to_string())?;
    if wallet.next_height < wallet.target_height {
        return Err("Monero local wallet must finish syncing before building".into());
    }
    let fee_priority = match priority {
        1 => FeePriority::Unimportant,
        2 => FeePriority::Normal,
        3 => FeePriority::Elevated,
        4 => FeePriority::Priority,
        _ => return Err("Invalid Monero priority".into()),
    };
    let fee_rate = rpc
        .fee_rate(fee_priority, 1_000_000_000)
        .await
        .map_err(|e| e.to_string())?;
    let mut candidates = wallet.unlocked()?;
    candidates.sort_by_key(|o| o.commitment().amount);
    let mut inputs = Vec::new();
    let mut images = Vec::new();
    let mut rng = rand::rngs::OsRng;
    let outgoing = Zeroizing::new(rand::random::<[u8; 32]>());
    for output in candidates {
        let image = wallet
            .outputs
            .iter()
            .find(|o| o.encoded == hex::encode(output.serialize()))
            .ok_or("Missing Monero output")?
            .key_image
            .clone();
        inputs.push(
            OutputWithDecoys::new(&mut rng, rpc, 16, (wallet.next_height - 1) as usize, output)
                .await
                .map_err(|e| e.to_string())?,
        );
        images.push(image);
        match SignableTransaction::new(
            RctType::ClsagBulletproofPlus,
            outgoing.clone(),
            inputs.clone(),
            vec![(recipient_address, amount)],
            Change::new(pair.clone(), None),
            vec![],
            fee_rate,
        ) {
            Ok(plan) => {
                let fee = plan.necessary_fee();
                let private = PrivatePlan {
                    sender: wallet.sender.clone(),
                    recipient: recipient.into(),
                    amount,
                    fee,
                    inputs: images.clone(),
                    encoded: hex::encode(plan.serialize()),
                };
                let json = Zeroizing::new(serde_json::to_vec(&private).map_err(|e| e.to_string())?);
                let encrypted = crate::store::seed_envelope::encrypt(&json, encryption_key)?;
                return Ok(PreparedMoneroTransaction {
                    sender: wallet.sender.clone(),
                    recipient: recipient.into(),
                    amount,
                    fee,
                    input_key_images: images,
                    encrypted_plan: String::from_utf8(encrypted).map_err(|e| e.to_string())?,
                });
            }
            Err(monero_wallet::send::SendError::NotEnoughFunds { .. }) => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("Insufficient unlocked Monero funds".into())
}

impl PreparedMoneroTransaction {
    pub(crate) fn sign(
        &self,
        private: &str,
        encryption_key: &[u8],
        wallet: &LocalWallet,
    ) -> Result<(String, String), String> {
        let json = Zeroizing::new(crate::store::seed_envelope::decrypt(
            self.encrypted_plan.as_bytes(),
            encryption_key,
        )?);
        let plan: PrivatePlan = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        if plan.sender != self.sender
            || plan.recipient != self.recipient
            || plan.amount != self.amount
            || plan.fee != self.fee
            || plan.inputs != self.input_key_images
        {
            return Err("Monero reviewed content changed".into());
        }
        if wallet.sender != self.sender {
            return Err("Monero sender mismatch".into());
        }
        let unlocked = wallet.unlocked()?;
        for image in &self.input_key_images {
            if !wallet.outputs.iter().any(|o| {
                &o.key_image == image
                    && !o.spent
                    && unlocked
                        .iter()
                        .any(|u| hex::encode(u.serialize()) == o.encoded)
            }) {
                return Err("Monero input spent or unavailable; sync and rebuild".into());
            }
        }
        let raw = Zeroizing::new(hex::decode(&plan.encoded).map_err(|e| e.to_string())?);
        let mut reader = raw.as_slice();
        let plan = SignableTransaction::read(&mut reader).map_err(|e| e.to_string())?;
        if !reader.is_empty() {
            return Err("Trailing data in Monero plan".into());
        }
        let (spend, _) = keys(private)?;
        let transaction = plan
            .sign(&mut rand::rngs::OsRng, &spend)
            .map_err(|e| e.to_string())?;
        Ok((
            hex::encode(transaction.serialize()),
            hex::encode(transaction.hash()),
        ))
    }
}
