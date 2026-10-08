//! A Litecoin wallet's MWEB funds: scanning for them on this device, the
//! stealth address that receives them, moving transparent LTC in and paying
//! out of them.
//!
//! MWEB keys belong to the phrase (`send::litecoin_mweb::keys`), whatever
//! path the wallet's transparent address is at, so one wallet per phrase and
//! network holds them. Its first sync derives them, keeps the scan secret in
//! the SecretStore and starts a cache of what scans find, encrypted under a
//! key derived from it (`wallet_db::scan_cache`); later syncs need no
//! password. Each sync anchors at the block the wallet's indexer names as
//! its tip, reads the headers after it from a Litecoin node and checks them,
//! then reads the node's tip's MWEB header, leafset and unspent outputs,
//! each proved (`api::litecoin_p2p`). Every output is tried with the scan
//! key and the wallet's are kept. The whole unspent set is a few dozen pages,
//! so a wallet's first scan reads all of it and later ones what is new; a
//! chain that no longer holds the last scan's tip is scanned again whole. A
//! batch reads a bounded number of pages; callers repeat until `complete`.
//! Spending needs the seed again.
//!
//! A light client reads unspent outputs only. The wallet's history holds
//! what scans find received at the addresses it gives out, dated by the
//! scan that found it, and what this device sent once its inputs are spent.

use super::*;
use crate::EndpointApi;
use crate::api::litecoin_p2p::{
    self, BlockHeader, LeafOutput, Leafset, LitecoinP2pClient, MAX_UTXOS_PER_REQUEST, Session,
    display, parse_display,
};
use crate::api::utxo::UtxoClient;
use crate::send::litecoin_mweb::keys::{self, ViewKeys};
use crate::send::litecoin_mweb::output::{self, OwnedOutput};
use crate::send::litecoin_mweb::prepared::{self, PreparedMwebSpend};
use crate::send::stages::{
    PreparedPayload, SendArtifact, SendArtifactReview, SendStage, StoredSend, SubmissionOutcome,
    WalletOperation,
};
use crate::store::secret_store::{SecretClass, SecretStoreError};
use crate::store::state::{WalletSigning, WalletState};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use zeroize::Zeroizing;

/// Pages of up to 4,096 unspent outputs one batch reads.
const PAGES_PER_BATCH: usize = 8;
/// The most nodes one batch tries.
const NODES_PER_BATCH: usize = 4;

const NO_SEED: &str = "MWEB funds need a Litecoin wallet restored from its seed phrase.";

/// Where a wallet's MWEB scan stands, and what it holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct LitecoinMwebStatus {
    pub wallet_id: String,
    /// The MWEB keys are derived and a scan has begun.
    pub ready: bool,
    /// The block the last sync's answers were proved in.
    pub scanned_height: u64,
    /// How much of the scan is done, in thousandths.
    pub progress_permille: u32,
    /// Every output of the last proved block is scanned.
    pub complete: bool,
    /// MWEB LTC no signed transaction spends, as an exact decimal.
    pub spendable: String,
    /// MWEB LTC on its way to the wallet: the change and peg-ins of its
    /// broadcast transactions, until a scan finds them.
    pub pending: String,
    /// The stealth address the wallet receives at.
    pub address: Option<String>,
}

/// An output a scan found: where, the block of the scan that found it, and
/// whether it is still unspent.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Found {
    coin: OwnedOutput,
    leaf: u64,
    found_height: u64,
    found_time: u64,
    spent: bool,
}

/// The block a sync proved its answers in.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Tip {
    height: u64,
    hash: String,
    time: u64,
    /// The outputs the chain had created through it.
    leaves: u64,
}

/// What the scans found, encrypted under the scan key.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Cache {
    wallet_id: String,
    chain: Chain,
    #[serde(with = "hex::serde")]
    spend_public: [u8; 33],
    /// The first leaf not yet scanned.
    next_leaf: u64,
    tip: Option<Tip>,
    outputs: Vec<Found>,
}

fn internal(error: impl std::fmt::Display) -> SpectraBridgeError {
    SpectraBridgeError::failure(error)
}

fn cache_key(wallet_id: &str, scan: &[u8]) -> Zeroizing<Vec<u8>> {
    let mut hash = Sha256::new();
    hash.update(b"Spectra MWEB scan cache v1");
    hash.update(wallet_id.as_bytes());
    hash.update(scan);
    Zeroizing::new(hash.finalize().to_vec())
}

fn decimal(litoshis: u64) -> String {
    crate::decimal::from_units(u128::from(litoshis), 8)
}

/// Whether `wallet` can hold MWEB funds: a Litecoin wallet restored from a
/// phrase, which the MWEB keys derive from.
pub(super) fn holds_mweb_keys(wallet: &WalletState) -> Result<(), SpectraBridgeError> {
    if wallet.chain_id.mainnet_counterpart() != Chain::Litecoin {
        return Err(SpectraBridgeError::invalid(
            "Only a Litecoin wallet holds MWEB funds.",
        ));
    }
    if !matches!(wallet.signing, WalletSigning::SeedPhrase { .. }) {
        return Err(SpectraBridgeError::invalid(NO_SEED));
    }
    Ok(())
}

/// The headers a sync checked: from the indexer's tip to the node's.
struct CheckedChain {
    anchor_height: u64,
    headers: Vec<BlockHeader>,
}

impl CheckedChain {
    fn tip_height(&self) -> u64 {
        self.anchor_height + self.headers.len() as u64 - 1
    }

    fn tip(&self) -> &BlockHeader {
        self.headers.last().expect("the anchor at least")
    }
}

/// What one batch read from a node.
struct Read {
    checked: CheckedChain,
    leafset: Leafset,
    outputs: Vec<LeafOutput>,
    /// The chain no longer holds the last scan's tip: the outputs were read
    /// from the first leaf.
    reorganized: bool,
    /// The first leaf left unread.
    next_leaf: u64,
}

/// What the wallet's signed MWEB transactions do to its balance until a
/// scan sees them mined: the outputs they spend, and the value they pay it.
struct InFlight {
    locked: HashSet<[u8; 32]>,
    incoming: u64,
}

impl Cache {
    fn new(wallet: &WalletState, view: &ViewKeys) -> Self {
        Self {
            wallet_id: wallet.id.clone(),
            chain: wallet.chain_id,
            spend_public: view.spend.serialize(),
            next_leaf: 0,
            tip: None,
            outputs: Vec::new(),
        }
    }

    fn status(
        &self,
        in_flight: &InFlight,
        view: &ViewKeys,
    ) -> Result<LitecoinMwebStatus, SpectraBridgeError> {
        let spendable = self
            .outputs
            .iter()
            .filter(|found| !found.spent && !in_flight.locked.contains(&found.coin.output_id))
            .try_fold(0u64, |total, found| total.checked_add(found.coin.value))
            .ok_or_else(|| internal("MWEB balance overflow"))?;
        let leaves = self.tip.as_ref().map_or(0, |tip| tip.leaves);
        Ok(LitecoinMwebStatus {
            wallet_id: self.wallet_id.clone(),
            ready: true,
            scanned_height: self.tip.as_ref().map_or(0, |tip| tip.height),
            progress_permille: self
                .next_leaf
                .min(leaves)
                .saturating_mul(1000)
                .checked_div(leaves)
                .map_or(0, |permille| u32::try_from(permille).unwrap_or(1000)),
            complete: self.tip.is_some() && self.next_leaf >= leaves,
            spendable: decimal(spendable),
            pending: decimal(in_flight.incoming),
            address: Some(view.address(keys::RECEIVE_INDEX)?.encode(self.chain)?),
        })
    }
}

impl WalletService {
    async fn mweb_wallet(&self, wallet_id: &str) -> Result<WalletState, SpectraBridgeError> {
        let wallet = self.stored_wallet(wallet_id).await?;
        holds_mweb_keys(&wallet)?;
        Ok(wallet)
    }

    /// The scan secret this wallet's first sync kept, if it has synced.
    fn mweb_scan_secret(
        &self,
        wallet_id: &str,
    ) -> Result<Option<Zeroizing<String>>, SpectraBridgeError> {
        match self
            .secrets()?
            .load_secret(SecretClass::Generic, format!("{wallet_id}.scan-key"))
        {
            Ok(stored) => Ok(Some(Zeroizing::new(stored))),
            Err(SecretStoreError::NotFound) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// The wallet's view keys and scan cache, as its last sync left them.
    async fn load_mweb(
        &self,
        wallet: &WalletState,
    ) -> Result<Option<(u64, Cache, ViewKeys)>, SpectraBridgeError> {
        let db = self.bound_database().await?;
        let Some((revision, payload)) =
            crate::wallet_db::scan_cache_load(&db, &wallet.id, wallet.chain_id)?
        else {
            return Ok(None);
        };
        let scan = self
            .mweb_scan_secret(&wallet.id)?
            .ok_or_else(|| internal("The MWEB scan key is missing"))?;
        let scan = Zeroizing::new(hex::decode(scan.as_str())?);
        let plaintext = Zeroizing::new(crate::store::seed_envelope::decrypt(
            payload.as_bytes(),
            &cache_key(&wallet.id, &scan),
        )?);
        let cache: Cache = serde_json::from_str(&plaintext)?;
        if cache.wallet_id != wallet.id || cache.chain != wallet.chain_id {
            return Err(internal("MWEB scan cache identity mismatch"));
        }
        let view = ViewKeys {
            scan: crate::send::litecoin_mweb::primitives::secret(
                scan.as_slice()
                    .try_into()
                    .map_err(|_| internal("Invalid MWEB scan key"))?,
            )?,
            spend: secp256k1::PublicKey::from_slice(&cache.spend_public)
                .map_err(|_| internal("Invalid MWEB spend key"))?,
        };
        Ok(Some((revision, cache, view)))
    }

    async fn save_mweb(
        &self,
        revision: Option<u64>,
        cache: &Cache,
        view: &ViewKeys,
    ) -> Result<(), SpectraBridgeError> {
        let plain = Zeroizing::new(serde_json::to_vec(cache)?);
        let encrypted = String::from_utf8(crate::store::seed_envelope::encrypt(
            &plain,
            &cache_key(&cache.wallet_id, &view.scan.secret_bytes()),
        )?)
        .map_err(internal)?;
        let _writer = self.state_writer.lock().await;
        let state = self.wallet_state.read().await;
        if !state
            .wallets
            .iter()
            .any(|w| w.id == cache.wallet_id && w.chain_id == cache.chain)
        {
            return Err(internal("Wallet removed during MWEB sync"));
        }
        crate::wallet_db::scan_cache_save(
            self.bound_database().await?.as_ref(),
            &cache.wallet_id,
            cache.chain,
            revision,
            &encrypted,
        )?;
        Ok(())
    }

    /// Refuse a second wallet of one phrase on one network: both would hold
    /// the same MWEB funds.
    async fn mweb_keys_unclaimed(
        &self,
        wallet: &WalletState,
        scan_hex: &str,
    ) -> Result<(), SpectraBridgeError> {
        let others: Vec<(String, String)> = self
            .wallet_state
            .read()
            .await
            .wallets
            .iter()
            .filter(|other| other.chain_id == wallet.chain_id && other.id != wallet.id)
            .map(|other| (other.id.clone(), other.name.clone()))
            .collect();
        for (id, name) in others {
            if self
                .mweb_scan_secret(&id)?
                .is_some_and(|other| other.as_str() == scan_hex)
            {
                return Err(SpectraBridgeError::refused(
                    "Another wallet, %@, already holds this phrase's MWEB funds.",
                    [name],
                ));
            }
        }
        Ok(())
    }

    /// The wallet's signed MWEB transactions, with what they spend and what
    /// they pay it. A payment's inputs stay locked until a scan finds one
    /// spent; what a broadcast one pays the wallet is pending until a scan
    /// finds it.
    async fn mweb_in_flight(
        &self,
        wallet: &WalletState,
        cache: &Cache,
        view: &ViewKeys,
    ) -> Result<(InFlight, Vec<StoredSend>), SpectraBridgeError> {
        let db = self.bound_database().await?;
        let (chain, wallet_id) = (wallet.chain_id, wallet.id.clone());
        let signed = tokio::task::spawn_blocking(move || {
            crate::wallet_db::signed_sends_for_wallet(&db, chain, &wallet_id)
        })
        .await??;
        let found: HashMap<[u8; 32], bool> = cache
            .outputs
            .iter()
            .map(|found| (found.coin.output_id, found.spent))
            .collect();
        let addresses = view.spend_keys()?;
        let mut in_flight = InFlight {
            locked: HashSet::new(),
            incoming: 0,
        };
        let mut sends = Vec::new();
        for stored in signed {
            let inputs: &[prepared::CoinRef] = match &stored.prepared {
                PreparedPayload::LitecoinMweb(prepared) => &prepared.inputs,
                PreparedPayload::LitecoinPegIn(_) => &[],
                _ => continue,
            };
            // Mined once any input is gone: Litecoin takes a transaction
            // whole.
            let mined = inputs
                .iter()
                .any(|coin| found.get(&coin.output_id) != Some(&false));
            if !mined {
                in_flight
                    .locked
                    .extend(inputs.iter().map(|coin| coin.output_id));
                // What it pays the wallet is on its way once a node may have
                // taken it: some attempt not refused.
                let submitted = stored
                    .view
                    .attempts
                    .iter()
                    .any(|attempt| attempt.outcome != SubmissionOutcome::Rejected);
                if let Some(raw) = stored.view.signed_payload.as_deref().filter(|_| submitted) {
                    let raw = hex::decode(raw)?;
                    for paid in crate::send::litecoin_mweb::transaction::mweb_outputs(&raw)? {
                        if found.contains_key(&paid.id()) {
                            continue;
                        }
                        if let Some(coin) = output::rewind(&paid, view, &addresses) {
                            in_flight.incoming = in_flight
                                .incoming
                                .checked_add(coin.value)
                                .ok_or_else(|| internal("MWEB balance overflow"))?;
                        }
                    }
                }
            }
            sends.push(stored);
        }
        Ok((in_flight, sends))
    }

    /// The MWEB LTC a wallet's last scan found, less what its signed
    /// payments spend and with what they return to it; `None` for a wallet
    /// with no MWEB keys or no scan yet.
    pub(super) async fn litecoin_mweb_total(
        &self,
        wallet_id: &str,
    ) -> Result<Option<String>, SpectraBridgeError> {
        let wallet = self.stored_wallet(wallet_id).await?;
        if holds_mweb_keys(&wallet).is_err() {
            return Ok(None);
        }
        let Some((_, cache, view)) = self.load_mweb(&wallet).await? else {
            return Ok(None);
        };
        let (in_flight, _) = self.mweb_in_flight(&wallet, &cache, &view).await?;
        let status = cache.status(&in_flight, &view)?;
        Ok(crate::decimal::add(&status.spendable, &status.pending))
    }

    /// The headers from the indexer's tip to the node's, checked.
    async fn mweb_chain(
        &self,
        chain: Chain,
        session: &mut Session,
        indexer: &UtxoClient,
    ) -> Result<CheckedChain, SpectraBridgeError> {
        let anchor_height = indexer.fetch_tip_height().await?;
        if anchor_height < 2 {
            return Err(internal("The Litecoin indexer has no chain"));
        }
        let anchor = parse_display(&indexer.fetch_block_hash(anchor_height).await?)?;
        let before = parse_display(&indexer.fetch_block_hash(anchor_height - 1).await?)?;
        let headers = session.headers_after(before).await?;
        if headers.first().map(|header| header.hash) != Some(anchor) {
            return Err(SpectraBridgeError::failure(
                "The Litecoin node is behind the indexer or on another chain; try another node",
            ));
        }
        let checked = CheckedChain {
            anchor_height,
            headers,
        };
        // The first time of each retarget's window, read the same way: the
        // indexer names the block, the node gives its header.
        let mut windows = HashMap::new();
        for height in (anchor_height + 1)..=checked.tip_height() {
            if !height.is_multiple_of(litecoin_p2p::RETARGET_INTERVAL) {
                continue;
            }
            let first = height - 1 - litecoin_p2p::RETARGET_INTERVAL;
            let named = parse_display(&indexer.fetch_block_hash(first).await?)?;
            let before = parse_display(&indexer.fetch_block_hash(first - 1).await?)?;
            let header = session
                .headers_after(before)
                .await?
                .into_iter()
                .next()
                .filter(|header| header.hash == named)
                .ok_or_else(|| internal("The Litecoin node does not have the indexer's chain"))?;
            windows.insert(first, header.time());
        }
        litecoin_p2p::verify_headers(
            &litecoin_p2p::params(chain)?,
            &checked.headers[0],
            anchor_height,
            &checked.headers[1..],
            &|height| windows.get(&height).copied(),
        )?;
        Ok(checked)
    }

    /// Whether the chain a sync checked still holds the last scan's tip.
    async fn mweb_tip_stands(
        &self,
        tip: &Tip,
        checked: &CheckedChain,
        indexer: &UtxoClient,
    ) -> Result<bool, SpectraBridgeError> {
        if tip.height > checked.tip_height() {
            return Ok(false);
        }
        let hash = match tip.height.checked_sub(checked.anchor_height) {
            Some(offset) => display(&checked.headers[offset as usize].hash),
            None => indexer.fetch_block_hash(tip.height).await?,
        };
        Ok(hash == tip.hash)
    }

    /// One batch's reads from `session`: the chain from the indexer's tip to
    /// the node's, checked; its tip's MWEB header and leafset, proved; and up
    /// to `PAGES_PER_BATCH` pages of unspent outputs, each proved, from the
    /// first leaf `cache` has not scanned — from the first of all when the
    /// chain no longer holds the last scan's tip, or holds fewer outputs,
    /// and may have moved any of them.
    async fn mweb_read(
        &self,
        chain: Chain,
        session: &mut Session,
        indexer: &UtxoClient,
        cache: &Cache,
    ) -> Result<Read, SpectraBridgeError> {
        let checked = self.mweb_chain(chain, session, indexer).await?;
        let proved = session.mweb_header(&checked.tip().hash).await?;
        let leafset = session.leafset(&proved).await?;
        let reorganized = match &cache.tip {
            Some(last) => {
                leafset.size < last.leaves || !self.mweb_tip_stands(last, &checked, indexer).await?
            }
            None => false,
        };
        let mut next_leaf = if reorganized { 0 } else { cache.next_leaf };
        let mut outputs = Vec::new();
        for _ in 0..PAGES_PER_BATCH {
            let Some(first) = leafset.unspent_from(next_leaf) else {
                break;
            };
            let page = session
                .unspent_outputs(&proved, &leafset, first, MAX_UTXOS_PER_REQUEST)
                .await?;
            next_leaf = page
                .last()
                .map(|leaf_output| leaf_output.leaf + 1)
                .ok_or_else(|| internal("An empty page of MWEB outputs"))?;
            outputs.extend(page);
        }
        if leafset.unspent_from(next_leaf).is_none() {
            next_leaf = leafset.size;
        }
        Ok(Read {
            checked,
            leafset,
            outputs,
            reorganized,
            next_leaf,
        })
    }

    async fn mweb_sync_batch(
        &self,
        wallet_id: &str,
        password: Option<&str>,
    ) -> Result<LitecoinMwebStatus, SpectraBridgeError> {
        let wallet = self.mweb_wallet(wallet_id).await?;
        let chain = wallet.chain_id;
        let nodes = self
            .api_endpoints(
                chain,
                EndpointApi::LitecoinP2p,
                &[EndpointCapability::History],
            )
            .await?;
        if nodes.is_empty() {
            return Err(SpectraBridgeError::refused(
                "%@ needs a Litecoin node to scan MWEB funds. Add one for it.",
                [chain.chain_display_name()],
            ));
        }
        let _guard = self
            .lock_sender(chain, &format!("litecoin-mweb:{wallet_id}"))
            .await?;
        let (revision, mut cache, view) = match self.load_mweb(&wallet).await? {
            Some((revision, cache, view)) => (Some(revision), cache, view),
            None => {
                let seed = self
                    .resolve_bip39_seed(wallet_id, password, NO_SEED)
                    .await?;
                let (view, _) = ViewKeys::from_seed(&seed)?;
                let scan = Zeroizing::new(hex::encode(view.scan.secret_bytes()));
                self.mweb_keys_unclaimed(&wallet, &scan).await?;
                self.secrets()?.save_secret(
                    SecretClass::Generic,
                    format!("{wallet_id}.scan-key"),
                    scan.to_string(),
                )?;
                (None, Cache::new(&wallet, &view), view)
            }
        };

        let indexer = self
            .utxo_client(chain, &[EndpointCapability::Verification])
            .await;
        // A node that drops the light client mid-batch, or answers what does
        // not prove, leaves the batch to the next one.
        let client = LitecoinP2pClient::new(Arc::new(nodes));
        let mut tried = HashSet::new();
        let mut failure = None;
        let read = loop {
            let mut session = match client.session(chain, &tried).await {
                Ok(session) => session,
                Err(error) => return Err(failure.unwrap_or_else(|| error.into())),
            };
            match self.mweb_read(chain, &mut session, &indexer, &cache).await {
                Ok(read) => break read,
                Err(error) => {
                    tried.insert(session.node.clone());
                    if tried.len() >= NODES_PER_BATCH {
                        return Err(error);
                    }
                    failure = Some(error);
                }
            }
        };
        if read.reorganized {
            cache.outputs.clear();
        }
        let tip = read.checked.tip();
        let found_height = read.checked.tip_height();
        let found_time = u64::from(tip.time());
        let addresses = view.spend_keys()?;
        let known: HashSet<[u8; 32]> = cache
            .outputs
            .iter()
            .map(|found| found.coin.output_id)
            .collect();
        for leaf_output in &read.outputs {
            if let Some(coin) = output::rewind(&leaf_output.output, &view, &addresses)
                && !known.contains(&coin.output_id)
            {
                cache.outputs.push(Found {
                    coin,
                    leaf: leaf_output.leaf,
                    found_height,
                    found_time,
                    spent: false,
                });
            }
        }
        let leafset = &read.leafset;
        // What the leafset no longer holds is spent.
        for found in &mut cache.outputs {
            found.spent = !leafset.contains(found.leaf);
        }
        cache.next_leaf = read.next_leaf;
        cache.tip = Some(Tip {
            height: found_height,
            hash: display(&tip.hash),
            time: found_time,
            leaves: leafset.size,
        });
        self.save_mweb(revision, &cache, &view).await?;
        let (in_flight, sends) = self.mweb_in_flight(&wallet, &cache, &view).await?;
        self.record_mweb_history(&wallet, &cache, &sends).await?;
        cache.status(&in_flight, &view)
    }

    /// The history of what scans found: each output received at an address
    /// the wallet gives out, and each MWEB payment this device made once an
    /// input of it is spent. Change and peg-ins arrive at addresses the
    /// wallet keeps for itself, and are the transactions that made them.
    async fn record_mweb_history(
        &self,
        wallet: &WalletState,
        cache: &Cache,
        sends: &[StoredSend],
    ) -> Result<(), SpectraBridgeError> {
        let chain = wallet.chain_id;
        let tip = cache
            .tip
            .as_ref()
            .ok_or_else(|| internal("An MWEB scan with no tip"))?;
        let item = |kind: &str,
                    litoshis: u64,
                    counterparty: String,
                    hash: String,
                    height: u64,
                    time: u64| {
            crate::fetch::history_decode::NormalizedHistoryItem {
                deployment_id: crate::tokens::deployment_id_for(chain, None),
                kind: kind.into(),
                status: "confirmed".into(),
                asset_display_name: chain.coin_name().into(),
                symbol: chain.coin_symbol().into(),
                chain_id: chain,
                amount: decimal(litoshis),
                counterparty,
                tx_hash: hash,
                block_height: i64::try_from(height).ok(),
                timestamp: time as f64,
            }
        };
        let mut items: Vec<_> = cache
            .outputs
            .iter()
            .filter(|found| found.coin.address_index >= keys::RECEIVE_INDEX)
            .map(|found| {
                item(
                    "receive",
                    found.coin.value,
                    String::new(),
                    hex::encode(found.coin.output_id),
                    found.found_height,
                    found.found_time,
                )
            })
            .collect();
        let spent: HashSet<[u8; 32]> = cache
            .outputs
            .iter()
            .filter(|found| found.spent)
            .map(|found| found.coin.output_id)
            .collect();
        for stored in sends {
            let PreparedPayload::LitecoinMweb(prepared) = &stored.prepared else {
                continue;
            };
            if let Some(hash) = &stored.view.transaction_hash
                && !stored.view.attempts.is_empty()
                && prepared
                    .inputs
                    .iter()
                    .any(|coin| spent.contains(&coin.output_id))
            {
                items.push(item(
                    "send",
                    prepared.amount,
                    prepared.recipient.clone(),
                    hash.clone(),
                    tip.height,
                    tip.time,
                ));
            }
        }
        let target = super::history_refresh::Target {
            wallet_id: wallet.id.clone(),
            wallet_name: wallet.name.clone(),
            address: wallet.address_on(chain).unwrap_or_default().to_string(),
            network: chain,
        };
        self.merge_fetched_history(
            items
                .into_iter()
                .map(|item| super::history_refresh::record_for(&target, &HashMap::new(), item))
                .collect(),
        )
        .await?;
        Ok(())
    }

    /// Store a built MWEB transaction for review.
    async fn store_mweb_send(
        &self,
        wallet: &WalletState,
        request: crate::send::SendExecutionRequest,
        prepared: PreparedPayload,
        operation: WalletOperation,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let chain = wallet.chain_id;
        let sender = wallet
            .address_on(chain)
            .ok_or_else(|| internal("Wallet has no address on this network"))?
            .to_string();
        // A payment is reviewed as any send is; a peg-in moves the wallet's
        // own funds and warns of nothing.
        let review = match &operation {
            WalletOperation::ShieldedPayment { .. } => self.staged_send_review(&request).await?,
            _ => SendArtifactReview::default(),
        };
        let mut stored = StoredSend {
            view: SendArtifact {
                id: crate::store::new_transaction_id(),
                revision: 0,
                stage: SendStage::Prepared,
                wallet_id: wallet.id.clone(),
                chain_id: chain,
                sender,
                recipient: request.to_address.clone(),
                amount: request.amount_str.clone(),
                asset: chain.coin_symbol().into(),
                symbol: chain.coin_symbol().into(),
                staking: None,
                operation: Some(operation),
                created_at: crate::store::now_unix().floor(),
                review_digest: String::new(),
                review,
                prepared_details: serde_json::to_string_pretty(&prepared)?,
                signing_payload_hex: String::new(),
                signed_payload: None,
                transaction_hash: None,
                attempts: Vec::new(),
                selected_endpoints: Vec::new(),
            },
            request,
            prepared,
            submission: None,
            signed_digest: None,
            substrate_verified_through: None,
            icp_staking_receipts: vec![],
        };
        stored.view.review_digest = stored.digest()?;
        self.save_send_artifact(&stored, Vec::new()).await?;
        Ok(stored.view)
    }

    /// Sign a reviewed payment out of MWEB funds: each input still one of the
    /// wallet's unspent outputs, as its last scan found them.
    pub(super) async fn sign_litecoin_mweb(
        &self,
        stored: &StoredSend,
        prepared: &PreparedMwebSpend,
        password: Option<&str>,
    ) -> Result<(crate::send::payload::PreparedSubmission, Vec<String>), SpectraBridgeError> {
        let wallet = self.mweb_wallet(&stored.view.wallet_id).await?;
        let chain = wallet.chain_id;
        let (_, cache, view) = self.load_mweb(&wallet).await?.ok_or_else(|| {
            SpectraBridgeError::invalid("Sync the MWEB funds before sending from them.")
        })?;
        let seed = self
            .resolve_bip39_seed(&wallet.id, password, NO_SEED)
            .await?;
        let (derived, spend_secret) = ViewKeys::from_seed(&seed)?;
        if derived.spend != view.spend || derived.scan.secret_bytes() != view.scan.secret_bytes() {
            return Err(internal("The MWEB keys do not match the wallet's scan"));
        }
        let coins: HashMap<[u8; 32], OwnedOutput> = cache
            .outputs
            .iter()
            .filter(|found| !found.spent)
            .map(|found| (found.coin.output_id, found.coin.clone()))
            .collect();
        let signed = prepared::sign_spend(chain, prepared, &coins, &view, &spend_secret)?;
        Ok((
            crate::send::payload::PreparedSubmission {
                payload: hex::encode(&signed.raw),
                result_field: "txid".into(),
                transaction_hash: Some(signed.txid),
                nonce: None,
            },
            prepared
                .inputs
                .iter()
                .map(|coin| format!("{}:mweb:{}", chain.str_id(), hex::encode(coin.output_id)))
                .collect(),
        ))
    }
}

/// The request a wallet-built MWEB transaction is stored with: its whole
/// network fee, as any Litecoin send's.
fn mweb_request(
    wallet: &WalletState,
    recipient: &str,
    amount: u64,
    fee: Option<u64>,
) -> crate::send::SendExecutionRequest {
    crate::send::SendExecutionRequest {
        chain_id: wallet.chain_id,
        wallet_id: wallet.id.clone(),
        password: None,
        to_address: recipient.to_string(),
        amount_str: decimal(amount),
        contract_address: None,
        token_standard: None,
        token_decimals: None,
        fee_rate_svb: None,
        fee_sat: fee,
        gas_budget: None,
        fee_amount: fee.map(decimal),
        evm_overrides: None,
        sign_only: false,
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Where the wallet's MWEB scan stands and what it holds, from its cache
    /// alone.
    pub async fn litecoin_mweb_status(
        &self,
        wallet_id: String,
    ) -> Result<LitecoinMwebStatus, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let wallet = this.mweb_wallet(&wallet_id).await?;
            let Some((_, cache, view)) = this.load_mweb(&wallet).await? else {
                return Ok(LitecoinMwebStatus {
                    wallet_id,
                    ready: false,
                    scanned_height: 0,
                    progress_permille: 0,
                    complete: false,
                    spendable: "0".into(),
                    pending: "0".into(),
                    address: None,
                });
            };
            let (in_flight, _) = this.mweb_in_flight(&wallet, &cache, &view).await?;
            cache.status(&in_flight, &view)
        })
        .await
    }

    /// Advance the wallet's MWEB scan by one bounded batch, from a Litecoin
    /// node. The first batch derives the MWEB keys from the seed, so it takes
    /// the wallet's password when it has one; later batches need none.
    /// Callers repeat until `complete`.
    pub async fn sync_litecoin_mweb(
        &self,
        wallet_id: String,
        password: Option<String>,
    ) -> Result<LitecoinMwebStatus, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let password = password.map(Zeroizing::new);
            this.mweb_sync_batch(&wallet_id, password.as_ref().map(|p| p.as_str()))
                .await
        })
        .await
    }

    /// Build a payment of `amount` LTC from the wallet's MWEB funds: to an
    /// MWEB address inside MWEB, or to any other Litecoin address by a
    /// peg-out. Stored like any send, signed and broadcast through the same
    /// stages.
    pub async fn build_litecoin_mweb_send(
        &self,
        wallet_id: String,
        recipient: String,
        amount: String,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let wallet = this.mweb_wallet(&wallet_id).await?;
            let chain = wallet.chain_id;
            let recipient = crate::send::flow::normalize_address(chain, &recipient);
            if !crate::send::flow::is_valid_send_address(chain, recipient.clone()) {
                return Err(SpectraBridgeError::invalid(
                    "Not a Litecoin address on this network",
                ));
            }
            let litoshis = u64::try_from(crate::send::amount_input::parse_raw_amount(&amount, 8)?)
                .map_err(|_| SpectraBridgeError::invalid("Invalid Litecoin amount"))?;
            let (_, cache, view) = this.load_mweb(&wallet).await?.ok_or_else(|| {
                SpectraBridgeError::invalid("Sync the MWEB funds before sending from them.")
            })?;
            let (in_flight, _) = this.mweb_in_flight(&wallet, &cache, &view).await?;
            let coins: Vec<OwnedOutput> = cache
                .outputs
                .iter()
                .filter(|found| !found.spent && !in_flight.locked.contains(&found.coin.output_id))
                .map(|found| found.coin.clone())
                .collect();
            let prepared = prepared::plan_spend(chain, &coins, &recipient, litoshis)?;
            let fee = prepared.fee;
            this.store_mweb_send(
                &wallet,
                mweb_request(&wallet, &recipient, litoshis, Some(fee)),
                PreparedPayload::LitecoinMweb(prepared),
                WalletOperation::ShieldedPayment {
                    memo: None,
                    network_fee: decimal(fee),
                },
            )
            .await
        })
        .await
    }

    /// Build the transaction that moves `amount` LTC of the wallet's
    /// transparent funds into its MWEB funds: a peg-in to the address the
    /// wallet keeps for them.
    pub async fn build_litecoin_mweb_pegin(
        &self,
        wallet_id: String,
        amount: String,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let wallet = this.mweb_wallet(&wallet_id).await?;
            let chain = wallet.chain_id;
            let (_, _, view) = this.load_mweb(&wallet).await?.ok_or_else(|| {
                SpectraBridgeError::invalid("Sync the MWEB funds before moving funds into them.")
            })?;
            let litoshis = u64::try_from(crate::send::amount_input::parse_raw_amount(&amount, 8)?)
                .map_err(|_| SpectraBridgeError::invalid("Invalid Litecoin amount"))?;
            let sender = wallet
                .address_on(chain)
                .ok_or_else(|| internal("Wallet has no address on this network"))?
                .to_string();
            let address = view.address(keys::PEGIN_INDEX)?.encode(chain)?;
            let PreparedPayload::LitecoinPegIn(pegin) = this
                .prepare_litecoin(
                    chain,
                    &mweb_request(&wallet, &address, litoshis, None),
                    &sender,
                    litoshis,
                )
                .await?
            else {
                return Err(internal("A peg-in pays an MWEB address"));
            };
            let fee = pegin
                .mweb_fee
                .checked_add(pegin.canonical_fee)
                .ok_or_else(|| internal("Fee overflow"))?;
            this.store_mweb_send(
                &wallet,
                mweb_request(&wallet, &address, litoshis, Some(fee)),
                PreparedPayload::LitecoinPegIn(pegin),
                WalletOperation::ShieldTransparent {
                    amount: decimal(litoshis),
                    network_fee: decimal(fee),
                },
            )
            .await
        })
        .await
    }
}
