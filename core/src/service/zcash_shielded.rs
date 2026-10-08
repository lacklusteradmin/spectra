//! A Zcash wallet's shielded funds: scanning, balances, its unified address,
//! and building the transactions that spend or shield them.
//!
//! A Zcash wallet restored from its phrase is a ZIP-32 account: the account
//! its standard path names, whose transparent receiver at index 0 is the
//! wallet's transparent address. Its shielded state is a librustzcash wallet
//! database of its own (`wallet_db::zcash`), filled from a lightwalletd
//! server in bounded batches: each call advances the scan, never loses what
//! a previous one stored, and can be cancelled between calls. The first
//! batch needs the seed — the account is created from it, from the wallet's
//! restore height — and none after it does; spending needs it again.

use super::*;
use crate::api::lightwalletd::{LightwalletdClient, LightwalletdSession, ShieldedProtocol};
use crate::send::stages::{
    PreparedPayload, SendArtifact, SendArtifactReview, SendStage, StoredSend, WalletOperation,
};
use crate::send::zcash_shielded::PreparedZcashShielded;
use crate::store::state::{WalletSigning, WalletState};
use crate::wallet_db::zcash::{ZcashDb, open_zcash_db, zcash_db_path};
use zcash_client_backend::data_api::chain::{
    BlockSource, ChainState, CommitmentTreeRoot, error::Error as ChainError, scan_cached_blocks,
};
use zcash_client_backend::data_api::scanning::ScanPriority;
use zcash_client_backend::data_api::wallet::ConfirmationsPolicy;
use zcash_client_backend::data_api::{
    Account as _, AccountBirthday, TransactionDataRequest, TransactionStatus,
    WalletCommitmentTrees, WalletRead, WalletWrite,
};
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zcash_keys::keys::{ReceiverRequirement, UnifiedAddressRequest};
use zcash_protocol::consensus::{BlockHeight, BranchId, Network};
use zeroize::Zeroizing;

/// Blocks scanned per batch: a few minutes of a busy stretch of chain on a
/// phone, and seconds of a quiet one.
const BATCH_BLOCKS: u32 = 1_000;
/// Transactions fetched whole per batch, for their memos and spends.
const BATCH_ENHANCEMENTS: usize = 20;

/// Where a wallet's shielded scan stands, and what it holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ZcashShieldedStatus {
    pub wallet_id: String,
    /// The shielded account exists: a first sync, with the seed, made it.
    pub ready: bool,
    /// The height the scan starts at: the wallet's restore height.
    pub restore_height: u64,
    /// Every block through this height is scanned.
    pub scanned_height: u64,
    /// The chain tip the last sync read.
    pub chain_tip_height: u64,
    /// How much of the scan is done, in thousandths.
    pub progress_permille: u32,
    /// Transactions the scan found that are still to be read whole, for
    /// their memos and spends.
    pub unread_transactions: u32,
    /// Nothing is left to scan below the tip, and every transaction found
    /// has been read whole.
    pub complete: bool,
    /// Shielded ZEC that can be spent now, as an exact decimal.
    pub spendable: String,
    /// Shielded ZEC received, or returned as change, that is not spendable
    /// yet.
    pub pending: String,
    /// Transparent ZEC the account can move into its shielded pool now.
    pub shieldable: String,
    /// The wallet's unified address, with Orchard and Sapling receivers.
    pub address: Option<String>,
}

/// The blocks one batch scans, as the scanner reads them.
struct Blocks(Vec<CompactBlock>);

impl BlockSource for Blocks {
    type Error = std::convert::Infallible;

    fn with_blocks<F, WalletErrT>(
        &self,
        from_height: Option<BlockHeight>,
        limit: Option<usize>,
        mut with_block: F,
    ) -> Result<(), ChainError<WalletErrT, Self::Error>>
    where
        F: FnMut(CompactBlock) -> Result<(), ChainError<WalletErrT, Self::Error>>,
    {
        let from = from_height.map_or(0, u64::from);
        for block in self
            .0
            .iter()
            .filter(|block| block.height >= from)
            .take(limit.unwrap_or(usize::MAX))
        {
            with_block(block.clone())?;
        }
        Ok(())
    }
}

/// A failure inside the wallet's database or a server's answer, worded by
/// whoever raised it.
const NO_SEED: &str = "Shielded funds need a Zcash wallet restored from its seed phrase.";

fn internal(error: impl std::fmt::Display) -> SpectraBridgeError {
    SpectraBridgeError::failure(error)
}

/// The ZIP-32 account a Zcash wallet's shielded funds belong to: the account
/// its standard path names, so its transparent receiver is the wallet's
/// address. Anything else has no shielded account here.
pub(super) fn shielded_account(wallet: &WalletState) -> Result<u32, SpectraBridgeError> {
    let chain = wallet.chain_id;
    if chain.mainnet_counterpart() != Chain::Zcash {
        return Err(SpectraBridgeError::invalid(
            "Only a Zcash wallet holds shielded funds.",
        ));
    }
    if !matches!(wallet.signing, WalletSigning::SeedPhrase { .. }) {
        return Err(SpectraBridgeError::invalid(
            "Shielded funds need a Zcash wallet restored from its seed phrase.",
        ));
    }
    if wallet
        .derivation_overrides
        .hmac_key
        .as_deref()
        .is_some_and(|key| !key.is_empty())
    {
        return Err(SpectraBridgeError::invalid(
            "Shielded funds use the standard key derivation; this wallet's custom HMAC key has none.",
        ));
    }
    let coin = if chain == Chain::Zcash { 133 } else { 1 };
    let path = wallet
        .address_record_on(chain)
        .and_then(|address| address.derivation_path.as_deref())
        .or(wallet.derivation_path.as_deref())
        .unwrap_or("");
    let parts: Vec<&str> = path.split('/').collect();
    let account = match parts.as_slice() {
        ["m", "44'", coin_part, account, "0", "0"] if *coin_part == format!("{coin}'") => account
            .strip_suffix('\'')
            .and_then(|index| index.parse::<u32>().ok())
            .filter(|index| *index < (1 << 31)),
        _ => None,
    };
    account.ok_or_else(|| {
        SpectraBridgeError::invalid(
            "Shielded funds follow the standard path m/44'/133'/account'/0/0; this wallet's path has none.",
        )
    })
}

/// The unified address the account receives shielded funds at: Orchard and
/// Sapling receivers, and no transparent one, which would tie it to the
/// wallet's public address.
fn unified_address(db: &ZcashDb, network: &Network) -> Result<Option<String>, SpectraBridgeError> {
    let Some(account) = db.get_account_ids().map_err(internal)?.into_iter().next() else {
        return Ok(None);
    };
    let Some(account) = db.get_account(account).map_err(internal)? else {
        return Ok(None);
    };
    let Some(ufvk) = account.ufvk() else {
        return Ok(None);
    };
    let request = UnifiedAddressRequest::unsafe_custom(
        ReceiverRequirement::Require,
        ReceiverRequirement::Require,
        ReceiverRequirement::Omit,
    );
    let (address, _) = ufvk.default_address(request).map_err(internal)?;
    Ok(Some(address.encode(network)))
}

fn decimal(zatoshis: zcash_protocol::value::Zatoshis) -> String {
    crate::decimal::from_units(u128::from(u64::from(zatoshis)), 8)
}

/// The account's shielded activity as history rows: what it received into
/// its shielded pools from others, what it sent out of them, and what it
/// shielded from its own transparent address. Value that only moved through
/// the transparent address is the transparent history's.
fn shielded_history(
    path: &std::path::Path,
    chain: Chain,
) -> Result<Vec<crate::fetch::history_decode::NormalizedHistoryItem>, SpectraBridgeError> {
    let conn = rusqlite::Connection::open(path).map_err(internal)?;
    let mut statement = conn
        .prepare(
            "SELECT t.txid, t.mined_height, t.block_time, t.is_shielding, t.expired_unmined,
                (SELECT IFNULL(SUM(o.value), 0) FROM v_tx_outputs o
                  WHERE o.txid = t.txid AND o.to_account_uuid = t.account_uuid
                    AND o.output_pool != 0 AND IFNULL(o.is_change, 0) = 0),
                (SELECT IFNULL(SUM(o.value), 0) FROM v_tx_outputs o
                  WHERE o.txid = t.txid AND o.to_account_uuid = t.account_uuid
                    AND o.output_pool != 0),
                (SELECT IFNULL(SUM(o.value), 0) FROM v_tx_outputs o
                  WHERE o.txid = t.txid AND o.from_account_uuid = t.account_uuid
                    AND (o.to_account_uuid IS NULL OR o.to_account_uuid != t.account_uuid
                         OR o.output_pool = 0)
                    AND IFNULL(o.is_change, 0) = 0),
                (SELECT o.to_address FROM v_tx_outputs o
                  WHERE o.txid = t.txid AND o.from_account_uuid = t.account_uuid
                    AND (o.to_account_uuid IS NULL OR o.to_account_uuid != t.account_uuid
                         OR o.output_pool = 0)
                    AND IFNULL(o.is_change, 0) = 0
                  ORDER BY o.output_index LIMIT 1)
             FROM v_transactions t",
        )
        .map_err(internal)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<bool>>(3)?.unwrap_or(false),
                row.get::<_, Option<bool>>(4)?.unwrap_or(false),
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, Option<String>>(8)?,
            ))
        })
        .map_err(internal)?;
    let mut items = Vec::new();
    for row in rows {
        let (txid, mined, time, shielding, expired, received, received_all, sent, recipient) =
            row.map_err(internal)?;
        let txid: [u8; 32] = txid
            .try_into()
            .map_err(|_| internal("zcash database: invalid txid"))?;
        let txid = zcash_primitives::transaction::TxId::from_bytes(txid).to_string();
        let status = if mined.is_some() {
            "confirmed"
        } else if expired {
            "failed"
        } else {
            "pending"
        };
        let item = |kind: &str, zatoshis: i64, counterparty: String| {
            crate::fetch::history_decode::NormalizedHistoryItem {
                deployment_id: crate::tokens::deployment_id_for(chain, None),
                kind: kind.into(),
                status: status.into(),
                asset_display_name: chain.coin_name().into(),
                symbol: chain.coin_symbol().into(),
                chain_id: chain,
                amount: crate::decimal::from_units(u128::try_from(zatoshis).unwrap_or(0), 8),
                counterparty,
                tx_hash: txid.clone(),
                block_height: mined,
                timestamp: time.map_or_else(crate::store::now_unix, |time| time as f64),
            }
        };
        if shielding {
            if received_all > 0 {
                items.push(item("shield", received_all, String::new()));
            }
            continue;
        }
        if received > 0 {
            items.push(item("receive", received, String::new()));
        }
        if sent > 0 {
            items.push(item("send", sent, recipient.unwrap_or_default()));
        }
    }
    Ok(items)
}

/// The status a database answers, offline.
fn status_of(
    db: &ZcashDb,
    network: &Network,
    wallet_id: &str,
    restore_height: u64,
) -> Result<ZcashShieldedStatus, SpectraBridgeError> {
    let mut status = ZcashShieldedStatus {
        wallet_id: wallet_id.to_string(),
        ready: false,
        restore_height,
        scanned_height: restore_height.saturating_sub(1),
        chain_tip_height: 0,
        progress_permille: 0,
        unread_transactions: 0,
        complete: false,
        spendable: "0".into(),
        pending: "0".into(),
        shieldable: "0".into(),
        address: unified_address(db, network)?,
    };
    let Some(account) = db.get_account_ids().map_err(internal)?.into_iter().next() else {
        return Ok(status);
    };
    status.ready = true;
    let Some(summary) = db
        .get_wallet_summary(ConfirmationsPolicy::default())
        .map_err(internal)?
    else {
        return Ok(status);
    };
    status.chain_tip_height = u64::from(summary.chain_tip_height());
    status.scanned_height = u64::from(summary.fully_scanned_height());
    let scan = summary.progress().scan();
    status.progress_permille = if *scan.denominator() == 0 {
        1000
    } else {
        u32::try_from(scan.numerator().saturating_mul(1000) / scan.denominator())
            .unwrap_or(1000)
            .min(1000)
    };
    // Scanned through the tip, and every transaction found read whole for
    // its memos and spends. A pending transaction's status is asked after
    // on every sync, and leaves nothing incomplete.
    status.unread_transactions = u32::try_from(
        db.transaction_data_requests()
            .map_err(internal)?
            .iter()
            .filter(|request| matches!(request, TransactionDataRequest::Enhancement(_)))
            .count(),
    )
    .unwrap_or(u32::MAX);
    status.complete = status.scanned_height >= status.chain_tip_height
        && db.suggest_scan_ranges().map_err(internal)?.is_empty()
        && status.unread_transactions == 0;
    if let Some(balance) = summary.account_balances().get(&account) {
        let pools = [
            balance.sapling_balance(),
            balance.orchard_balance(),
            balance.ironwood_balance(),
        ];
        let sum = |value: fn(
            &zcash_client_backend::data_api::Balance,
        ) -> zcash_protocol::value::Zatoshis| {
            pools.iter().map(|pool| u64::from(value(pool))).sum::<u64>()
        };
        let spendable = sum(|pool| pool.spendable_value());
        let pending = sum(|pool| pool.change_pending_confirmation())
            + sum(|pool| pool.value_pending_spendability())
            + sum(|pool| pool.locked_value());
        status.spendable = crate::decimal::from_units(u128::from(spendable), 8);
        status.pending = crate::decimal::from_units(u128::from(pending), 8);
        status.shieldable = decimal(balance.unshielded_balance().spendable_value());
    }
    Ok(status)
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Where the wallet's shielded scan stands and what it holds, from its
    /// database alone.
    pub async fn zcash_shielded_status(
        &self,
        wallet_id: String,
    ) -> Result<ZcashShieldedStatus, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let (wallet, network, path) = this.zcash_shielded_wallet(&wallet_id).await?;
            let restore_height = wallet.restore_height.unwrap_or_default();
            tokio::task::spawn_blocking(move || {
                let db = open_zcash_db(&path, network)?;
                status_of(&db, &network, &wallet_id, restore_height)
            })
            .await?
        })
        .await
    }

    /// Advance the wallet's shielded scan by one bounded batch, from a
    /// lightwalletd server. The first batch creates the shielded account
    /// from the seed, so it takes the wallet's password when it has one;
    /// later batches need none. Callers repeat until `complete`.
    pub async fn sync_zcash_shielded(
        &self,
        wallet_id: String,
        password: Option<String>,
    ) -> Result<ZcashShieldedStatus, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let password = password.map(Zeroizing::new);
            this.zcash_sync_batch(&wallet_id, password.as_ref().map(|p| p.as_str()))
                .await
        })
        .await
    }

    /// Build the transaction that pays `amount` ZEC to `recipient` from the
    /// wallet's shielded funds, with a memo for a shielded recipient. Stored
    /// like any send, to be signed and broadcast through the same stages.
    pub async fn build_zcash_shielded_send(
        &self,
        wallet_id: String,
        recipient: String,
        amount: String,
        memo: Option<String>,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let (wallet, network, path) = this.zcash_shielded_wallet(&wallet_id).await?;
            let zatoshis = crate::send::amount_input::parse_raw_amount(&amount, 8)?;
            let zatoshis = u64::try_from(zatoshis)
                .map_err(|_| SpectraBridgeError::invalid("Invalid Zcash amount"))?;
            let to = recipient.trim().to_string();
            let memo_text = memo.clone();
            let prepared = this
                .with_locked_zcash_db(&wallet, network, path, move |db| {
                    Ok(crate::send::zcash_shielded::propose_payment(
                        db,
                        &network,
                        &to,
                        zatoshis,
                        memo_text.as_deref(),
                    )?)
                })
                .await?;
            let amount = crate::decimal::from_units(u128::from(zatoshis), 8);
            let operation = WalletOperation::ShieldedPayment {
                memo: prepared
                    .payments
                    .first()
                    .and_then(|payment| payment.memo.clone()),
                network_fee: crate::decimal::from_units(u128::from(prepared.fee_zat), 8),
            };
            this.store_zcash_shielded(&wallet, recipient.trim(), &amount, prepared, operation)
                .await
        })
        .await
    }

    /// Build the transaction that moves every spendable transparent output
    /// of the wallet into its Orchard pool, less the fee.
    pub async fn build_zcash_shielding(
        &self,
        wallet_id: String,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let (wallet, network, path) = this.zcash_shielded_wallet(&wallet_id).await?;
            let prepared = this
                .with_locked_zcash_db(&wallet, network, path.clone(), move |db| {
                    Ok(crate::send::zcash_shielded::propose_shielding_all(
                        db, &network,
                    )?)
                })
                .await?;
            let address = tokio::task::spawn_blocking(move || {
                let db = open_zcash_db(&path, network)?;
                unified_address(&db, &network)
            })
            .await??
            .ok_or_else(|| internal("The shielded account has no address"))?;
            let amount = crate::decimal::from_units(u128::from(prepared.change_zat), 8);
            let operation = WalletOperation::ShieldTransparent {
                amount: amount.clone(),
                network_fee: crate::decimal::from_units(u128::from(prepared.fee_zat), 8),
            };
            this.store_zcash_shielded(&wallet, &address, &amount, prepared, operation)
                .await
        })
        .await
    }
}

impl WalletService {
    /// The shielded ZEC a wallet's last scan found, spendable or pending, as
    /// an exact decimal; `None` for a wallet with no shielded account.
    pub(super) async fn zcash_shielded_total(
        &self,
        wallet_id: &str,
    ) -> Result<Option<String>, SpectraBridgeError> {
        let wallet = self.stored_wallet(wallet_id).await?;
        if shielded_account(&wallet).is_err() {
            return Ok(None);
        }
        let network = wallet.chain_id.zcash_network()?;
        let database = self.bound_database().await?;
        let path = zcash_db_path(&database, wallet_id)?;
        if !path.exists() {
            return Ok(None);
        }
        let restore_height = wallet.restore_height.unwrap_or_default();
        let wallet_id = wallet_id.to_string();
        let status = tokio::task::spawn_blocking(move || {
            let db = open_zcash_db(&path, network)?;
            status_of(&db, &network, &wallet_id, restore_height)
        })
        .await??;
        if !status.ready {
            return Ok(None);
        }
        Ok(crate::decimal::add(&status.spendable, &status.pending))
    }

    /// The stored Zcash wallet with a shielded account, its network and its
    /// database's path.
    pub(super) async fn zcash_shielded_wallet(
        &self,
        wallet_id: &str,
    ) -> Result<(WalletState, Network, std::path::PathBuf), SpectraBridgeError> {
        let wallet = self.stored_wallet(wallet_id).await?;
        shielded_account(&wallet)?;
        let network = wallet.chain_id.zcash_network()?;
        let database = self.bound_database().await?;
        let path = zcash_db_path(&database, wallet_id)?;
        Ok((wallet, network, path))
    }

    /// Run `f` on the wallet's database, one caller at a time.
    async fn with_locked_zcash_db<T: Send + 'static>(
        &self,
        wallet: &WalletState,
        network: Network,
        path: std::path::PathBuf,
        f: impl FnOnce(&mut ZcashDb) -> Result<T, SpectraBridgeError> + Send + 'static,
    ) -> Result<T, SpectraBridgeError> {
        let _guard = self
            .lock_sender(wallet.chain_id, &format!("zcash-shielded:{}", wallet.id))
            .await?;
        tokio::task::spawn_blocking(move || {
            let mut db = open_zcash_db(&path, network)?;
            f(&mut db)
        })
        .await?
    }

    async fn zcash_sync_batch(
        &self,
        wallet_id: &str,
        password: Option<&str>,
    ) -> Result<ZcashShieldedStatus, SpectraBridgeError> {
        let (wallet, network, path) = self.zcash_shielded_wallet(wallet_id).await?;
        let chain = wallet.chain_id;
        let restore_height = wallet
            .restore_height
            .unwrap_or_else(|| crate::restore_heights::default_restore_height(chain));
        let endpoints = self
            .api_endpoints(
                chain,
                crate::EndpointApi::Lightwalletd,
                &[EndpointCapability::History],
            )
            .await?;
        if endpoints.is_empty() {
            return Err(SpectraBridgeError::refused(
                "%@ needs a lightwalletd server to scan shielded funds. Add one for it.",
                [chain.chain_display_name()],
            ));
        }
        let _guard = self
            .lock_sender(chain, &format!("zcash-shielded:{wallet_id}"))
            .await?;
        let mut session = LightwalletdClient::new(Arc::new(endpoints))
            .session(chain)
            .await?;
        let tip = session.tip;
        if u64::from(tip) < restore_height {
            return Err(SpectraBridgeError::invalid(
                "That restore height is past the chain's current height.",
            ));
        }
        let db_path = path.clone();
        let exists = tokio::task::spawn_blocking(move || {
            let db = open_zcash_db(&db_path, network)?;
            Ok::<_, SpectraBridgeError>(!db.get_account_ids().map_err(internal)?.is_empty())
        })
        .await??;
        if !exists {
            self.create_zcash_account(
                &wallet,
                password,
                network,
                &path,
                &mut session,
                restore_height,
            )
            .await?;
        }

        // The tip, and the completed subtrees of each note commitment tree:
        // Sapling's, Orchard's and Ironwood's.
        let roots_from = {
            let path = path.clone();
            tokio::task::spawn_blocking(move || {
                let mut db = open_zcash_db(&path, network)?;
                db.update_chain_tip(BlockHeight::from(tip))
                    .map_err(internal)?;
                let summary = db
                    .get_wallet_summary(ConfirmationsPolicy::default())
                    .map_err(internal)?;
                Ok::<_, SpectraBridgeError>(summary.map_or((0, 0, 0), |summary| {
                    (
                        summary.next_sapling_subtree_index(),
                        summary.next_orchard_subtree_index(),
                        summary.next_ironwood_subtree_index(),
                    )
                }))
            })
            .await??
        };
        let sapling_roots = session
            .subtree_roots(ShieldedProtocol::Sapling, roots_from.0 as u32)
            .await?;
        let orchard_roots = session
            .subtree_roots(ShieldedProtocol::Orchard, roots_from.1 as u32)
            .await?;
        let ironwood_roots = session
            .subtree_roots(ShieldedProtocol::Ironwood, roots_from.2 as u32)
            .await?;
        {
            let path = path.clone();
            tokio::task::spawn_blocking(move || {
                let mut db = open_zcash_db(&path, network)?;
                let read = |roots: &[crate::api::lightwalletd::SubtreeRoot]| {
                    roots
                        .iter()
                        .map(|root| {
                            Ok(CommitmentTreeRoot::from_parts(
                                BlockHeight::from_u32(
                                    u32::try_from(root.completing_block_height)
                                        .map_err(|_| internal("subtree root height"))?,
                                ),
                                root.root_hash.clone(),
                            ))
                        })
                        .collect::<Result<Vec<_>, SpectraBridgeError>>()
                };
                let sapling = read(&sapling_roots)?
                    .into_iter()
                    .map(|root| {
                        let hash = <sapling_crypto::Node as zcash_primitives::merkle_tree::HashSer>::read(
                            &root.root_hash()[..],
                        )
                        .map_err(internal)?;
                        Ok(CommitmentTreeRoot::from_parts(root.subtree_end_height(), hash))
                    })
                    .collect::<Result<Vec<_>, SpectraBridgeError>>()?;
                // Orchard and Ironwood trees hash alike.
                let orchard_family = |roots: &[crate::api::lightwalletd::SubtreeRoot]| {
                    read(roots)?
                        .into_iter()
                        .map(|root| {
                            let hash = <orchard::tree::MerkleHashOrchard as zcash_primitives::merkle_tree::HashSer>::read(
                                &root.root_hash()[..],
                            )
                            .map_err(internal)?;
                            Ok(CommitmentTreeRoot::from_parts(root.subtree_end_height(), hash))
                        })
                        .collect::<Result<Vec<_>, SpectraBridgeError>>()
                };
                let orchard = orchard_family(&orchard_roots)?;
                let ironwood = orchard_family(&ironwood_roots)?;
                db.put_sapling_subtree_roots(roots_from.0, &sapling)
                    .map_err(internal)?;
                db.put_orchard_subtree_roots(roots_from.1, &orchard)
                    .map_err(internal)?;
                db.put_ironwood_subtree_roots(roots_from.2, &ironwood)
                    .map_err(internal)?;
                Ok::<_, SpectraBridgeError>(())
            })
            .await??;
        }

        // Transparent outputs the account can shield.
        let (addresses, utxos_from) = {
            let path = path.clone();
            tokio::task::spawn_blocking(move || {
                let db = open_zcash_db(&path, network)?;
                let account = crate::send::zcash_shielded::account_id(&db)?;
                let addresses = db
                    .get_transparent_receivers(account, true, false)
                    .map_err(internal)?
                    .into_keys()
                    .map(|address| zcash_keys::encoding::AddressCodec::encode(&address, &network))
                    .collect::<Vec<String>>();
                let from = u32::from(db.utxo_query_height(account).map_err(internal)?);
                Ok::<_, SpectraBridgeError>((addresses, from))
            })
            .await??
        };
        let utxos = session.utxos(addresses, utxos_from).await?;
        {
            let path = path.clone();
            tokio::task::spawn_blocking(move || {
                let mut db = open_zcash_db(&path, network)?;
                let account = crate::send::zcash_shielded::account_id(&db)?;
                for reply in utxos {
                    let outpoint = zcash_transparent::bundle::OutPoint::new(
                        reply.txid[..]
                            .try_into()
                            .map_err(|_| internal("lightwalletd: invalid outpoint"))?,
                        u32::try_from(reply.index)
                            .map_err(|_| internal("lightwalletd: invalid outpoint"))?,
                    );
                    let txout = zcash_transparent::bundle::TxOut::new(
                        zcash_protocol::value::Zatoshis::from_nonnegative_i64(reply.value_zat)
                            .map_err(|_| internal("lightwalletd: invalid output value"))?,
                        zcash_transparent::address::Script(zcash_script::script::Code(
                            reply.script,
                        )),
                    );
                    let output = zcash_client_backend::wallet::WalletTransparentOutput::from_parts(
                        outpoint,
                        txout,
                        Some(BlockHeight::from_u32(u32::try_from(reply.height).map_err(
                            |_| internal("lightwalletd: invalid output height"),
                        )?)),
                        Some(account),
                        None,
                        None,
                    )
                    .ok_or_else(|| internal("lightwalletd: unreadable transparent output"))?;
                    db.put_received_transparent_utxo(&output)
                        .map_err(internal)?;
                }
                Ok::<_, SpectraBridgeError>(())
            })
            .await??;
        }

        // One range of blocks, the most urgent first.
        let range = {
            let path = path.clone();
            tokio::task::spawn_blocking(move || {
                let db = open_zcash_db(&path, network)?;
                Ok::<_, SpectraBridgeError>(
                    db.suggest_scan_ranges()
                        .map_err(internal)?
                        .into_iter()
                        .find(|range| range.priority() > ScanPriority::Scanned)
                        .map(|range| {
                            let start = u32::from(range.block_range().start);
                            let end = u32::from(range.block_range().end)
                                .min(start.saturating_add(BATCH_BLOCKS));
                            (start, end)
                        }),
                )
            })
            .await??
        };
        let range = range.filter(|(start, end)| end > start);
        let scanned = range.is_some();
        if let Some((start, end)) = range {
            let blocks = session.blocks(start, end - 1).await?;
            let prior = session.tree_state(start - 1).await?;
            let path = path.clone();
            tokio::task::spawn_blocking(move || {
                let mut db = open_zcash_db(&path, network)?;
                let state: ChainState = prior.to_chain_state().map_err(internal)?;
                let count = blocks.len();
                match scan_cached_blocks(
                    &network,
                    &Blocks(blocks),
                    &mut db,
                    BlockHeight::from_u32(start),
                    &state,
                    count,
                ) {
                    Ok(_) => Ok(()),
                    // The server's chain moved under the stored one: rewind,
                    // and the next batch rescans from there.
                    Err(ChainError::Scan(error)) if error.is_continuity_error() => {
                        db.truncate_to_height(error.at_height().saturating_sub(10))
                            .map_err(internal)?;
                        Ok(())
                    }
                    Err(error) => Err(internal(format!("Zcash scan: {error}"))),
                }
            })
            .await??;
        }

        // Whole transactions, for what compact blocks leave out.
        let requests = {
            let path = path.clone();
            tokio::task::spawn_blocking(move || {
                let db = open_zcash_db(&path, network)?;
                Ok::<_, SpectraBridgeError>(
                    db.transaction_data_requests()
                        .map_err(internal)?
                        .into_iter()
                        .filter_map(|request| match request {
                            TransactionDataRequest::GetStatus(txid) => Some((txid, false)),
                            TransactionDataRequest::Enhancement(txid) => Some((txid, true)),
                            _ => None,
                        })
                        .take(BATCH_ENHANCEMENTS)
                        .collect::<Vec<_>>(),
                )
            })
            .await??
        };
        let read = requests.iter().any(|(_, enhance)| *enhance);
        for (txid, enhance) in requests {
            let fetched = session.transaction(*txid.as_ref()).await?;
            let path = path.clone();
            tokio::task::spawn_blocking(move || {
                let mut db = open_zcash_db(&path, network)?;
                match fetched {
                    None => db
                        .set_transaction_status(txid, TransactionStatus::TxidNotRecognized)
                        .map_err(internal),
                    Some((_, None)) if !enhance => db
                        .set_transaction_status(txid, TransactionStatus::NotInMainChain)
                        .map_err(internal),
                    Some((_, Some(height))) if !enhance => db
                        .set_transaction_status(
                            txid,
                            TransactionStatus::Mined(BlockHeight::from_u32(height)),
                        )
                        .map_err(internal),
                    Some((raw, height)) => {
                        let branch = BranchId::for_height(
                            &network,
                            BlockHeight::from_u32(height.unwrap_or(tip + 1)),
                        );
                        let tx = zcash_primitives::transaction::Transaction::read(&raw[..], branch)
                            .map_err(internal)?;
                        zcash_client_backend::data_api::wallet::decrypt_and_store_transaction(
                            &network,
                            &mut db,
                            &tx,
                            height.map(BlockHeight::from_u32),
                        )
                        .map_err(internal)
                    }
                }
            })
            .await??;
        }

        // What the scan found, in the wallet's history.
        let history = {
            let path = path.clone();
            tokio::task::spawn_blocking(move || shielded_history(&path, chain)).await??
        };
        let target = super::history_refresh::Target {
            wallet_id: wallet.id.clone(),
            wallet_name: wallet.name.clone(),
            address: wallet.address_on(chain).unwrap_or_default().to_string(),
            network: chain,
        };
        self.merge_fetched_history(
            history
                .into_iter()
                .map(|item| super::history_refresh::record_for(&target, &HashMap::new(), item))
                .collect(),
        )
        .await?;

        let wallet_id = wallet_id.to_string();
        let status = tokio::task::spawn_blocking(move || {
            let db = open_zcash_db(&path, network)?;
            status_of(&db, &network, &wallet_id, restore_height)
        })
        .await??;
        // A batch either moves the scan on or finishes it: callers repeat
        // until `complete`, and one that moved nothing would be repeated
        // forever.
        if !status.complete && !scanned && !read {
            return Err(SpectraBridgeError::failure(
                "The shielded scan made no progress; try again later.",
            ));
        }
        Ok(status)
    }

    /// Create the wallet's shielded account from its seed, as of its restore
    /// height.
    async fn create_zcash_account(
        &self,
        wallet: &WalletState,
        password: Option<&str>,
        network: Network,
        path: &std::path::Path,
        session: &mut LightwalletdSession,
        restore_height: u64,
    ) -> Result<(), SpectraBridgeError> {
        let account = shielded_account(wallet)?;
        let seed = self
            .resolve_bip39_seed(&wallet.id, password, NO_SEED)
            .await?;
        let height = u32::try_from(restore_height)
            .map_err(|_| SpectraBridgeError::invalid("Invalid restore height"))?;
        let tree = session.tree_state(height.saturating_sub(1)).await?;
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let birthday = AccountBirthday::from_treestate(tree, None)
                .map_err(|_| internal("lightwalletd: unreadable tree state"))?;
            let mut db = open_zcash_db(&path, network)?;
            db.import_account_hd(
                "Spectra",
                &secrecy::SecretVec::new(seed.to_vec()),
                zip32::AccountId::try_from(account)
                    .map_err(|_| SpectraBridgeError::invalid("Invalid Zcash account"))?,
                &birthday,
                None,
            )
            .map_err(internal)?;
            Ok::<_, SpectraBridgeError>(())
        })
        .await?
    }

    /// Store a built shielded transaction as a send artifact.
    async fn store_zcash_shielded(
        &self,
        wallet: &WalletState,
        recipient: &str,
        amount: &str,
        prepared: PreparedZcashShielded,
        operation: WalletOperation,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let chain = wallet.chain_id;
        let sender = wallet
            .address_on(chain)
            .ok_or_else(|| internal("Wallet has no address on this network"))?
            .to_string();
        let request = crate::send::SendExecutionRequest {
            chain_id: chain,
            wallet_id: wallet.id.clone(),
            password: None,
            to_address: recipient.to_string(),
            amount_str: amount.to_string(),
            contract_address: None,
            token_standard: None,
            token_decimals: None,
            fee_rate_svb: None,
            fee_sat: None,
            gas_budget: None,
            fee_amount: Some(crate::decimal::from_units(u128::from(prepared.fee_zat), 8)),
            evm_overrides: None,
            sign_only: false,
            memo: None,
        };
        // A payment is reviewed as any send is; shielding moves the
        // wallet's own funds and warns of nothing.
        let review = match &operation {
            WalletOperation::ShieldedPayment { .. } => self.staged_send_review(&request).await?,
            _ => SendArtifactReview::default(),
        };
        let prepared = PreparedPayload::ZcashShielded(prepared);
        let mut stored = StoredSend {
            view: SendArtifact {
                id: crate::store::new_transaction_id(),
                revision: 0,
                stage: SendStage::Prepared,
                wallet_id: wallet.id.clone(),
                chain_id: chain,
                sender,
                recipient: recipient.to_string(),
                amount: amount.to_string(),
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
                memo: None,
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
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// The servers a signed shielded transaction can be handed to: the
    /// network's lightwalletd servers.
    pub async fn zcash_shielded_broadcast_endpoints(
        &self,
        chain: Chain,
    ) -> Result<Vec<String>, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            this.api_endpoints(
                chain,
                crate::EndpointApi::Lightwalletd,
                &[EndpointCapability::Broadcast],
            )
            .await
        })
        .await
    }
}

impl WalletService {
    /// Prove and sign a shielded artifact with the account's spending key,
    /// after checking the network is under a consensus branch this build
    /// knows: a transaction built for another would be refused.
    pub(super) async fn sign_zcash_shielded(
        &self,
        stored: &StoredSend,
        prepared: &PreparedZcashShielded,
        password: Option<&str>,
    ) -> Result<(crate::send::payload::PreparedSubmission, Vec<String>), SpectraBridgeError> {
        let (wallet, network, path) = self.zcash_shielded_wallet(&stored.view.wallet_id).await?;
        let chain = wallet.chain_id;
        let account = shielded_account(&wallet)?;
        let endpoints = self
            .api_endpoints(
                chain,
                crate::EndpointApi::Lightwalletd,
                &[EndpointCapability::History],
            )
            .await?;
        let session = LightwalletdClient::new(Arc::new(endpoints))
            .session(chain)
            .await?;
        let known = u32::from(BranchId::for_height(
            &network,
            BlockHeight::from_u32(session.tip + 1),
        ));
        if session.branch != known {
            return Err(SpectraBridgeError::failure(
                "Zcash consensus upgrade is unsupported or inconsistent; update before sending",
            ));
        }
        let sapling = if prepared.uses_sapling {
            Some(self.sapling_parameters().await?)
        } else {
            None
        };
        let seed = self
            .resolve_bip39_seed(&wallet.id, password, NO_SEED)
            .await?;
        let prepared = prepared.clone();
        let transactions = self
            .with_locked_zcash_db(&wallet, network, path, move |db| {
                Ok(crate::send::zcash_shielded::sign(
                    db,
                    &network,
                    &seed,
                    account,
                    &prepared,
                    sapling.as_ref(),
                )?)
            })
            .await?;
        let [(txid, raw)] = transactions.as_slice() else {
            return Err(internal("A shielded send is one transaction"));
        };
        Ok((
            crate::send::payload::PreparedSubmission {
                payload: hex::encode(raw),
                result_field: "txid".into(),
                transaction_hash: Some(
                    zcash_primitives::transaction::TxId::from_bytes(*txid).to_string(),
                ),
                nonce: None,
            },
            Vec::new(),
        ))
    }

    /// The Sapling parameters, from the data directory, or downloaded once
    /// and kept there; each checked against its pinned hash every time.
    async fn sapling_parameters(
        &self,
    ) -> Result<crate::send::zcash_shielded::SaplingParameters, SpectraBridgeError> {
        use crate::send::zcash_shielded::{
            SAPLING_PARAMETER_FILES, SAPLING_PARAMETERS_URL, SaplingParameters,
            sapling_parameter_is_genuine,
        };
        let database = self.bound_database().await?;
        let directory = std::path::Path::new(database.path())
            .parent()
            .ok_or_else(|| internal("Shielded funds need a database on disk"))?
            .join("zcash-params");
        let mut files = Vec::new();
        for (name, _, size) in SAPLING_PARAMETER_FILES {
            let path = directory.join(name);
            let read = path.clone();
            let stored = tokio::task::spawn_blocking(move || std::fs::read(read).ok()).await?;
            let bytes = match stored.filter(|bytes| sapling_parameter_is_genuine(name, bytes)) {
                Some(bytes) => bytes,
                None => {
                    let bytes = crate::api::http::HttpClient::shared()
                        .get_bytes(&format!("{SAPLING_PARAMETERS_URL}/{name}"), size)
                        .await
                        .map_err(|error| {
                            SpectraBridgeError::failed(
                                "Spending Sapling funds needs the Sapling parameters (about 50 MB), which could not be downloaded: %@",
                                [error],
                            )
                        })?;
                    if !sapling_parameter_is_genuine(name, &bytes) {
                        return Err(SpectraBridgeError::failed(
                            "The downloaded %@ is not the published Sapling parameter file.",
                            [name],
                        ));
                    }
                    let (directory, written) = (directory.clone(), bytes.clone());
                    tokio::task::spawn_blocking(move || {
                        std::fs::create_dir_all(&directory)?;
                        std::fs::write(&path, &written)
                    })
                    .await?
                    .map_err(internal)?;
                    bytes
                }
            };
            files.push(bytes);
        }
        let output = files.pop().expect("two parameter files");
        let spend = files.pop().expect("two parameter files");
        Ok(SaplingParameters { spend, output })
    }
}

fn read_transaction(
    network: &Network,
    raw: &[u8],
) -> Result<zcash_primitives::transaction::Transaction, SpectraBridgeError> {
    // The branch only selects how the bytes parse; every V5 and V6
    // transaction states its own.
    zcash_primitives::transaction::Transaction::read(
        raw,
        BranchId::for_height(network, BlockHeight::from_u32(u32::MAX)),
    )
    .map_err(internal)
}

/// The height a signed shielded transaction expires at, read from its bytes.
pub(super) fn zcash_expiry_height(
    network: &Network,
    raw_hex: &str,
) -> Result<u32, SpectraBridgeError> {
    Ok(u32::from(
        read_transaction(network, &hex::decode(raw_hex)?)?.expiry_height(),
    ))
}

/// A signed transaction's id, as explorers write it.
pub(super) fn zcash_txid(network: &Network, raw: &[u8]) -> Result<String, SpectraBridgeError> {
    Ok(read_transaction(network, raw)?.txid().to_string())
}

#[cfg(test)]
#[path = "tests/zcash_shielded.rs"]
mod tests;
