//! Device-local Monero scanning and CLSAG/Bulletproof+ signing.
//! Only public daemon requests cross the transport; keys and scan results stay local.

use crate::api::monero_daemon_rpc::Daemon;
use crate::derivation::monero::ViewKeys;
use crate::send::error::SendError;
use monero_wallet::{
    OutputWithDecoys, Scanner, ViewPair, WalletOutput,
    address::{MoneroAddress, SubaddressIndex},
    ed25519::{Point, Scalar},
    interface::prelude::*,
    ringct::RctType,
    send::{Change, SignableTransaction},
    transaction::{Input, Timelock},
};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

/// The outputs `tx_key` proves pay `address` in `tx`, each amount in
/// piconeros, as monero-wallet-cli's `check_tx_key` finds them (it reports
/// their sum): the derivation `8·r·A` with the address's view key `A`, each
/// output whose key is `Hs(derivation ‖ i)·G + B` paying the address, its
/// amount decrypted with that shared secret.
pub(crate) fn received_with_tx_key(
    tx: &monero_wallet::transaction::Transaction,
    tx_key: &[u8; 32],
    address: &MoneroAddress,
) -> Result<Vec<u64>, SendError> {
    use curve25519_dalek::{constants::ED25519_BASEPOINT_TABLE, scalar::Scalar as Dalek};
    use monero_wallet::{ringct::EncryptedAmount, transaction::Transaction};
    let r = Option::<Dalek>::from(Dalek::from_canonical_bytes(*tx_key))
        .ok_or_else(|| SendError::invalid("Not a Monero transaction key"))?;
    let derivation = (r * address.view().into()).mul_by_cofactor().compress();
    let Transaction::V2 {
        prefix,
        proofs: Some(proofs),
    } = tx
    else {
        return Err(SendError::invalid("Not a RingCT Monero transaction"));
    };
    let mut received = Vec::new();
    for (index, output) in prefix.outputs.iter().enumerate() {
        let mut hashed = derivation.to_bytes().to_vec();
        // The output index as a Monero varint.
        let mut value = index;
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                hashed.push(byte);
                break;
            }
            hashed.push(byte | 0x80);
        }
        let shared = Dalek::from_bytes_mod_order(monero_wallet::primitives::keccak256(&hashed));
        let expected = (&shared * ED25519_BASEPOINT_TABLE + address.spend().into()).compress();
        if output.key.to_bytes() != expected.to_bytes() {
            continue;
        }
        let Some(EncryptedAmount::Compact { amount }) = proofs.base.encrypted_amounts.get(index)
        else {
            return Err(SendError::invalid("Monero output without a compact amount"));
        };
        let mut masked = b"amount".to_vec();
        masked.extend(shared.to_bytes());
        let mask = monero_wallet::primitives::keccak256(&masked);
        let mut plain = [0u8; 8];
        for (byte, (value, key)) in plain.iter_mut().zip(amount.iter().zip(mask)) {
            *byte = value ^ key;
        }
        received.push(u64::from_le_bytes(plain));
    }
    Ok(received)
}

/// wallet2's default subaddress lookahead (`--subaddress-lookahead 50:200`):
/// how many accounts past the highest one used, and addresses past each
/// account's highest used, a scan watches.
pub(crate) const LOOKAHEAD_ACCOUNTS: u32 = 50;
pub(crate) const LOOKAHEAD_ADDRESSES: u32 = 200;

#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub(crate) struct LocalOutput {
    pub encoded: String,
    /// `None` for a wallet scanned without its spend key, which cannot
    /// know it.
    pub key_image: Option<String>,
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
    #[zeroize(skip)]
    pub chain_id: crate::registry::Chain,
    pub sender: String,
    pub restore_height: u64,
    pub next_height: u64,
    pub timestamps: Vec<u64>,
    pub last_hash: Option<[u8; 32]>,
    pub target_height: u64,
    pub outputs: Vec<LocalOutput>,
    pub transfers: Vec<LocalTransfer>,
    /// Each account's highest address index an output arrived at, its
    /// primary address being 0: what the scan's lookahead is counted from,
    /// so the subaddresses it watches survive a restart.
    #[zeroize(skip)]
    #[serde(default)]
    pub used_subaddresses: std::collections::BTreeMap<u32, u32>,
}
/// The subaddresses a scan watches: each account's addresses below the
/// bound it maps to.
type Window = std::collections::BTreeMap<u32, u32>;

impl LocalWallet {
    pub fn unlocked(&self) -> Result<Vec<WalletOutput>, SendError> {
        let mut timestamps = self.timestamps.clone();
        timestamps.sort_unstable();
        let chain_time = timestamps.get(timestamps.len() / 2).copied().unwrap_or(0);
        self.outputs
            .iter()
            .filter(|o| !o.spent && o.received_height.saturating_add(10) <= self.next_height)
            .map(|o| {
                WalletOutput::read(
                    &mut hex::decode(&o.encoded)
                        .map_err(SendError::invalid)?
                        .as_slice(),
                )
                .map_err(SendError::invalid)
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
            .map_err(SendError::invalid)
    }

    pub fn balance(&self) -> Result<u64, SendError> {
        self.unlocked()?.iter().try_fold(0u64, |sum, o| {
            sum.checked_add(o.commitment().amount)
                .ok_or_else(|| SendError::invalid("Monero balance overflow"))
        })
    }

    /// Start the scan over from the restore height.
    fn reset(&mut self) {
        self.outputs.clear();
        self.transfers.clear();
        self.timestamps.clear();
        self.used_subaddresses.clear();
        self.next_height = self.restore_height;
        self.last_hash = None;
    }

    /// wallet2's lookahead past what was used: `LOOKAHEAD_ACCOUNTS`
    /// accounts past the highest used, and in each `LOOKAHEAD_ADDRESSES`
    /// addresses past its highest used — in account 0 past `handed_out`
    /// too, the receive index the wallet last gave out.
    fn window(&self, handed_out: u32) -> Window {
        let past = |used: Option<u32>| used.map_or(0, |used| used.saturating_add(1));
        let accounts = past(self.used_subaddresses.keys().next_back().copied())
            .saturating_add(LOOKAHEAD_ACCOUNTS);
        (0..accounts)
            .map(|account| {
                let mut bound = past(self.used_subaddresses.get(&account).copied());
                if account == 0 {
                    bound = bound.max(handed_out);
                }
                (account, bound.saturating_add(LOOKAHEAD_ADDRESSES))
            })
            .collect()
    }
}

/// Register on `scanner` the subaddresses `to` holds and `from` does not.
fn register(scanner: &mut Scanner, from: &Window, to: &Window) {
    for (&account, &bound) in to {
        for address in from.get(&account).copied().unwrap_or(0)..bound {
            if let Some(index) = SubaddressIndex::new(account, address) {
                scanner.register_subaddress(index);
            }
        }
    }
}

pub(crate) fn keys(private: &str) -> Result<(Zeroizing<Scalar>, ViewPair), SendError> {
    let raw = Zeroizing::new(hex::decode(private)?);
    if raw.len() != 64 {
        return Err(SendError::Invalid(
            "Monero signing identity must contain spend and view keys".into(),
        ));
    }
    let spend = Scalar::read(&mut &raw[..32]).map_err(SendError::invalid)?;
    let view = Scalar::read(&mut &raw[32..]).map_err(SendError::invalid)?;
    let spend_point = curve25519_dalek::constants::ED25519_BASEPOINT_TABLE * &spend.into();
    Ok((
        Zeroizing::new(spend),
        ViewPair::new(Point::from(spend_point), Zeroizing::new(view))
            .map_err(SendError::invalid)?,
    ))
}

/// What a scan reads with: the wallet's view keys, and its spend key when
/// the wallet holds one. Without it no output's key image is known, so
/// neither is what the wallet spent.
pub(crate) struct ScanKeys {
    pub view: ViewKeys,
    pub spend: Option<Zeroizing<Scalar>>,
}

impl ScanKeys {
    /// A signing wallet's, from its spend and view keys in hex.
    pub(crate) fn signing(chain: crate::registry::Chain, private: &str) -> Result<Self, SendError> {
        let (spend, _) = keys(private)?;
        Ok(Self {
            view: ViewKeys::from_private(chain, private)?,
            spend: Some(spend),
        })
    }
}

/// Scan a bounded batch and account for spent outputs locally, never
/// querying key images. Every subaddress in wallet2's lookahead is watched;
/// a block whose outputs widen it is scanned again under the wider window.
pub(crate) async fn scan(
    wallet: &mut LocalWallet,
    rpc: &Daemon,
    keys: &ScanKeys,
    handed_out: u32,
    batch: u32,
) -> Result<(), SendError> {
    let pair = keys.view.pair()?;
    let target = rpc
        .latest_block_number()
        .await
        .map_err(SendError::invalid)? as u64;
    wallet.target_height = target
        .checked_add(1)
        .ok_or_else(|| SendError::Invalid("Monero height overflow".into()))?;
    // A wallet that holds its spend key now scans again what it scanned
    // without it: no output found then has a key image, so no spend of one
    // was seen.
    if keys.spend.is_some() && wallet.outputs.iter().any(|o| o.key_image.is_none()) {
        wallet.reset();
    }
    if wallet.last_hash.is_some() && wallet.next_height > wallet.target_height {
        wallet.reset();
    }
    if wallet.last_hash.is_some() {
        let previous = rpc
            .scannable_block_by_number((wallet.next_height - 1) as usize)
            .await
            .map_err(SendError::invalid)?;
        if Some(previous.block.hash()) != wallet.last_hash {
            wallet.reset();
        }
    }
    if wallet.next_height > wallet.target_height {
        return Err(SendError::Invalid(
            "Monero restore height is ahead of the chain".into(),
        ));
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
        .map_err(SendError::invalid)?;
    let mut window = wallet.window(handed_out);
    let mut scanner = Scanner::new(pair);
    register(&mut scanner, &Window::new(), &window);
    for block in blocks {
        let height = block.block.number() as u64;
        if height != wallet.next_height
            || wallet
                .last_hash
                .is_some_and(|hash| hash != block.block.header.previous)
        {
            return Err(SendError::Invalid(
                "Monero scan chain is discontinuous".into(),
            ));
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
                        .find(|o| o.key_image.as_deref() == Some(image.as_str()) && !o.spent)
                    {
                        let output =
                            WalletOutput::read(&mut hex::decode(&output.encoded)?.as_slice())
                                .map_err(SendError::invalid)?;
                        debit = debit
                            .checked_add(output.commitment().amount)
                            .ok_or_else(|| SendError::Invalid("Monero debit overflow".into()))?;
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
        // The block's outputs, scanned again while what they were sent to
        // widens the window.
        let mut found: Vec<WalletOutput> = Vec::new();
        loop {
            for output in scanner
                .scan(block.clone())
                .map_err(SendError::invalid)?
                .ignore_additional_timelock()
            {
                if found.iter().any(|seen| {
                    seen.transaction() == output.transaction()
                        && seen.index_in_transaction() == output.index_in_transaction()
                }) {
                    continue;
                }
                let (account, address) = output
                    .subaddress()
                    .map_or((0, 0), |index| (index.account(), index.address()));
                let used = wallet.used_subaddresses.entry(account).or_insert(address);
                *used = (*used).max(address);
                found.push(output);
            }
            let wider = wallet.window(handed_out);
            if wider == window {
                break;
            }
            register(&mut scanner, &window, &wider);
            window = wider;
        }
        for output in found {
            let image = keys.spend.as_ref().map(|spend| {
                let offset: curve25519_dalek::scalar::Scalar = output.key_offset().into();
                let spend_scalar: curve25519_dalek::scalar::Scalar = (**spend).into();
                let scalar = Zeroizing::new(spend_scalar + offset);
                let point: curve25519_dalek::EdwardsPoint =
                    Point::biased_hash(output.key().compress().to_bytes()).into();
                hex::encode((point * *scalar).compress().to_bytes())
            });
            let encoded = hex::encode(output.serialize());
            if wallet.outputs.iter().any(|o| match (&o.key_image, &image) {
                (Some(held), Some(image)) => held == image,
                _ => o.encoded == encoded,
            }) {
                return Err(SendError::Invalid("Duplicate Monero output".into()));
            }
            let entry = transfers
                .entry(hex::encode(output.transaction()))
                .or_default();
            entry.0 = entry
                .0
                .checked_add(output.commitment().amount)
                .ok_or_else(|| SendError::Invalid("Monero credit overflow".into()))?;
            wallet.outputs.push(LocalOutput {
                encoded,
                key_image: image,
                received_height: height,
                spent: false,
            });
        }
        for output in &mut wallet.outputs {
            if output
                .key_image
                .as_ref()
                .is_some_and(|image| spent.contains(image))
            {
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
                        .ok_or_else(|| SendError::Invalid("Invalid Monero net amount".into()))?
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
) -> Result<PreparedMoneroTransaction, SendError> {
    let network = crate::derivation::monero::address_network(wallet.chain_id)?;
    let recipient_address =
        MoneroAddress::from_str(network, recipient).map_err(SendError::invalid)?;
    if wallet.next_height < wallet.target_height {
        return Err(SendError::Invalid(
            "Monero local wallet must finish syncing before building".into(),
        ));
    }
    // Normal, the priority the preview's `priorityLabel` names; there is no
    // other to choose.
    let fee_rate = rpc
        .fee_rate(FeePriority::Normal, 1_000_000_000)
        .await
        .map_err(SendError::invalid)?;
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
            .ok_or_else(|| SendError::Invalid("Missing Monero output".into()))?
            .key_image
            .clone()
            .ok_or_else(|| SendError::invalid("A view-only Monero wallet cannot spend."))?;
        inputs.push(
            OutputWithDecoys::new(&mut rng, rpc, 16, (wallet.next_height - 1) as usize, output)
                .await
                .map_err(SendError::invalid)?,
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
                let json = Zeroizing::new(serde_json::to_vec(&private)?);
                let encrypted = crate::store::seed_envelope::encrypt(&json, encryption_key)?;
                return Ok(PreparedMoneroTransaction {
                    sender: wallet.sender.clone(),
                    recipient: recipient.into(),
                    amount,
                    fee,
                    input_key_images: images,
                    encrypted_plan: String::from_utf8(encrypted).map_err(SendError::invalid)?,
                });
            }
            Err(monero_wallet::send::SendError::NotEnoughFunds { .. }) => continue,
            Err(e) => return Err(SendError::Internal(e.to_string())),
        }
    }
    Err(SendError::InsufficientFunds(
        "Insufficient unlocked Monero funds".into(),
    ))
}

impl PreparedMoneroTransaction {
    /// The transaction key `r` the plan's transaction is built with, the
    /// one monero-wallet-cli's `check_tx_key` takes: the first key
    /// monero-wallet's `TransactionKeys` derives from the plan's outgoing
    /// view key and inputs. Spectra pays one address plus change, so the
    /// transaction carries no additional keys.
    pub(crate) fn transaction_key(
        &self,
        encryption_key: &[u8],
    ) -> Result<Zeroizing<[u8; 32]>, SendError> {
        use std::io::Read;
        let json = Zeroizing::new(crate::store::seed_envelope::decrypt(
            self.encrypted_plan.as_bytes(),
            encryption_key,
        )?);
        let plan: PrivatePlan = serde_json::from_str(&json)?;
        let raw = Zeroizing::new(hex::decode(&plan.encoded)?);
        // SignableTransaction::write: the RingCT type, the outgoing view
        // key, then the inputs.
        let mut reader = raw
            .get(1..)
            .ok_or_else(|| SendError::invalid("Empty Monero plan"))?;
        let mut outgoing = Zeroizing::new([0u8; 32]);
        reader
            .read_exact(outgoing.as_mut())
            .map_err(SendError::invalid)?;
        let inputs = monero_wallet::io::read_vec(OutputWithDecoys::read, None, &mut reader)
            .map_err(SendError::invalid)?;
        let mut keys = monero_wallet::send::TransactionKeys::new(
            &outgoing,
            inputs
                .iter()
                .map(|input| (input.key(), input.commitment().commit()))
                .collect(),
        );
        let key = keys
            .next()
            .ok_or_else(|| SendError::Internal("no Monero transaction key".into()))?;
        Ok(Zeroizing::new((*key).into().to_bytes()))
    }

    pub(crate) fn sign(
        &self,
        private: &str,
        encryption_key: &[u8],
        wallet: &LocalWallet,
    ) -> Result<(String, String), SendError> {
        let json = Zeroizing::new(crate::store::seed_envelope::decrypt(
            self.encrypted_plan.as_bytes(),
            encryption_key,
        )?);
        let plan: PrivatePlan = serde_json::from_str(&json)?;
        if plan.sender != self.sender
            || plan.recipient != self.recipient
            || plan.amount != self.amount
            || plan.fee != self.fee
            || plan.inputs != self.input_key_images
        {
            return Err(SendError::Invalid("Monero reviewed content changed".into()));
        }
        if wallet.sender != self.sender {
            return Err(SendError::Invalid("Monero sender mismatch".into()));
        }
        let unlocked = wallet.unlocked()?;
        for image in &self.input_key_images {
            if !wallet.outputs.iter().any(|o| {
                o.key_image.as_ref() == Some(image)
                    && !o.spent
                    && unlocked
                        .iter()
                        .any(|u| hex::encode(u.serialize()) == o.encoded)
            }) {
                return Err(SendError::Invalid(
                    "Monero input spent or unavailable; sync and rebuild".into(),
                ));
            }
        }
        let raw = Zeroizing::new(hex::decode(&plan.encoded)?);
        let mut reader = raw.as_slice();
        let plan = SignableTransaction::read(&mut reader).map_err(SendError::invalid)?;
        if !reader.is_empty() {
            return Err(SendError::Invalid("Trailing data in Monero plan".into()));
        }
        let (spend, _) = keys(private)?;
        let transaction = plan
            .sign(&mut rand::rngs::OsRng, &spend)
            .map_err(SendError::invalid)?;
        Ok((
            hex::encode(transaction.serialize()),
            hex::encode(transaction.hash()),
        ))
    }
}

#[cfg(test)]
mod window_tests {
    use super::*;

    fn wallet(used: &[(u32, u32)]) -> LocalWallet {
        LocalWallet {
            wallet_id: "w".into(),
            chain_id: crate::registry::Chain::Monero,
            sender: String::new(),
            restore_height: 0,
            next_height: 0,
            timestamps: Vec::new(),
            last_hash: None,
            target_height: 0,
            outputs: Vec::new(),
            transfers: Vec::new(),
            used_subaddresses: used.iter().copied().collect(),
        }
    }

    /// wallet2's 50:200 from nothing; past each account's highest used and
    /// the highest account used; and in account 0 past what was handed out.
    #[test]
    fn the_window_is_wallet2s_lookahead_past_what_was_used() {
        let fresh = wallet(&[]).window(0);
        assert_eq!(fresh.len(), 50);
        assert!(fresh.values().all(|bound| *bound == 200));
        let used = wallet(&[(0, 300), (49, 199)]).window(0);
        assert_eq!(used.len(), 100);
        assert_eq!(
            (used[&0], used[&1], used[&49], used[&99]),
            (501, 200, 400, 200)
        );
        assert_eq!(wallet(&[(0, 300)]).window(700)[&0], 900);
        assert_eq!(wallet(&[(0, 0)]).window(0)[&0], 201);
    }
}
