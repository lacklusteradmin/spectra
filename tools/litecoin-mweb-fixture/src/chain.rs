//! The fixture's chain: its blocks, each with a HogEx committing to its MWEB
//! header; the output MMR and the leafset of every block; transparent outputs
//! by address; and what a light client and an indexer read of them.
//!
//! A transaction handed to the indexer is checked as a node checks one —
//! its MWEB part by `mweb::check_transaction`, its canonical inputs' outputs
//! and signatures, and the peg-in rules of Litecoin Core's `IsStandardTx` —
//! then mined into a block of its own and journaled.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;

use litecoin::blockdata::mimblewimble as mw;
use litecoin::hashes::Hash as _;
use litecoin::{
    Amount, BlockHash, CompressedPublicKey, EcdsaSighashType, NetworkKind, OutPoint, ScriptBuf,
    Sequence, Transaction, TxIn, TxOut, Txid, Witness, absolute, block, transaction,
};
use secp256k1::{Message, PublicKey, SECP256K1};
use serde_json::json;

use crate::mweb::{self, Keys};

const SPACING: u32 = 150;

fn node_of(leaf: u64) -> u64 {
    2 * leaf - u64::from(leaf.count_ones())
}

fn all_ones(x: u64) -> bool {
    x != 0 && x & (x + 1) == 0
}

/// The height of the node at `pos`, in post-order.
fn height(pos: u64) -> u32 {
    let mut n = pos + 1;
    while !all_ones(n) {
        n -= (1u64 << (63 - n.leading_zeros())) - 1;
    }
    63 - n.leading_zeros()
}

fn peaks(size: u64) -> Vec<u64> {
    let (mut out, mut left, mut before) = (Vec::new(), size, 0);
    while left > 0 {
        let mut tree = (1u64 << (64 - left.leading_zeros())) - 1;
        if tree > left {
            tree >>= 1;
        }
        out.push(before + tree - 1);
        before += tree;
        left -= tree;
    }
    out
}

fn leaf_hash(pos: u64, id: &[u8; 32]) -> [u8; 32] {
    let mut preimage = pos.to_le_bytes().to_vec();
    preimage.push(32);
    preimage.extend(id);
    mweb::blake(&preimage)
}

fn parent_hash(pos: u64, left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut preimage = pos.to_le_bytes().to_vec();
    preimage.extend(left);
    preimage.extend(right);
    mweb::blake(&preimage)
}

/// A Merkle Mountain Range over 32-byte ids, its nodes in post-order.
#[derive(Default)]
struct Mmr {
    nodes: Vec<[u8; 32]>,
}

impl Mmr {
    fn append(&mut self, id: &[u8; 32]) {
        let pos = self.nodes.len() as u64;
        self.nodes.push(leaf_hash(pos, id));
        let mut h = 0;
        while height(self.nodes.len() as u64) > h {
            let parent = self.nodes.len() as u64;
            let left = (parent - (1u64 << (h + 1))) as usize;
            let right = (parent - 1) as usize;
            let hash = parent_hash(parent, &self.nodes[left], &self.nodes[right]);
            self.nodes.push(hash);
            h += 1;
        }
    }

    /// The peaks of the range's first `leaves` leaves, bagged from the right.
    fn root(&self, leaves: u64) -> [u8; 32] {
        let size = node_of(leaves);
        self.bag(size, &peaks(size))
    }

    fn bag(&self, size: u64, peaks: &[u64]) -> [u8; 32] {
        let Some((&last, rest)) = peaks.split_last() else {
            return [0; 32];
        };
        rest.iter()
            .rev()
            .fold(self.nodes[last as usize], |bagged, &peak| {
                parent_hash(size, &self.nodes[peak as usize], &bagged)
            })
    }
}

fn unspent_in(leafset: &[u8], leaf: u64) -> bool {
    leafset
        .get((leaf / 8) as usize)
        .is_some_and(|byte| byte & (0x80 >> (leaf % 8)) != 0)
}

/// An output of the MMR, as the fixture keeps it.
struct Leaf {
    id: [u8; 32],
    compact: Vec<u8>,
    commitment: [u8; 33],
    receiver: PublicKey,
    unspent: bool,
    /// Placeholder traffic, which no wallet holds.
    noise: bool,
}

pub struct Block {
    pub height: u32,
    pub hash: BlockHash,
    pub time: u32,
    header: block::Header,
    hogex: Transaction,
    txids: Vec<Txid>,
    mweb_header: Vec<u8>,
    /// The leaves the output MMR held through this block, and which were
    /// unspent.
    leaves: u64,
    leafset: Vec<u8>,
}

/// A transaction as the indexer lists it under the addresses it touches.
struct Indexed {
    txid: Txid,
    height: u32,
    time: u32,
    inputs: Vec<(Option<String>, u64)>,
    outputs: Vec<(Option<String>, u64)>,
    fee: u64,
}

pub struct Chain {
    pub network: NetworkKind,
    pub blocks: Vec<Block>,
    leaves: Vec<Leaf>,
    outputs: Mmr,
    kernels: Mmr,
    kernel_count: u64,
    hogaddr: Option<(OutPoint, u64)>,
    utxos: BTreeMap<OutPoint, TxOut>,
    indexed: Vec<Indexed>,
    journal: Option<std::fs::File>,
    /// The key sets the journal names outputs by.
    wallets: Vec<(&'static str, Keys)>,
}

fn coinbase(height: u32) -> Transaction {
    Transaction {
        version: transaction::Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: litecoin::script::Builder::new()
                .push_int(i64::from(height))
                .into_script(),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::ZERO,
            script_pubkey: ScriptBuf::from_bytes(vec![0x51]),
        }],
        mw_tx: None,
        is_hog_ex: false,
    }
}

/// Litecoin Core's dust threshold at its 30,000-litoshi relay rate.
fn dust(script: &litecoin::Script) -> u64 {
    let spend = if script.is_witness_program() { 67 } else { 148 };
    (8 + 1 + script.len() as u64 + spend) * 30
}

impl Chain {
    pub fn new(
        network: NetworkKind,
        start: u32,
        journal: Option<std::path::PathBuf>,
        wallets: Vec<(&'static str, Keys)>,
    ) -> Self {
        let mut chain = Self {
            network,
            blocks: Vec::new(),
            leaves: Vec::new(),
            outputs: Mmr::default(),
            kernels: Mmr::default(),
            kernel_count: 0,
            hogaddr: None,
            utxos: BTreeMap::new(),
            indexed: Vec::new(),
            journal: journal.map(|path| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .expect("a journal")
            }),
            wallets,
        };
        chain.mine(start, Vec::new(), &[], 0);
        chain
    }

    /// The genesis block of the network the chain stands on.
    pub fn genesis(&self) -> &'static str {
        match self.network {
            NetworkKind::Main => "12a765e31ffd4059bada1e25190f6e98c99d9714d334efa41a195a7e7e04bfe2",
            NetworkKind::Test => "4966625a4b2851d9fdee139e56211a0d88575f59ed816ff5e6a63deb4e3e29a0",
        }
    }

    pub fn tip(&self) -> &Block {
        self.blocks.last().expect("a genesis block")
    }

    pub fn block_at(&self, height: u64) -> Option<&Block> {
        let first = u64::from(self.blocks[0].height);
        self.blocks
            .get(usize::try_from(height.checked_sub(first)?).ok()?)
    }

    pub fn block(&self, hash: &BlockHash) -> Option<&Block> {
        self.blocks.iter().find(|block| block.hash == *hash)
    }

    fn address_of(&self, script: &litecoin::Script) -> Option<String> {
        let network = match self.network {
            NetworkKind::Main => litecoin::Network::Bitcoin,
            NetworkKind::Test => litecoin::Network::Testnet4,
        };
        litecoin::Address::from_script(script, network)
            .ok()
            .map(|address| address.to_string())
    }

    /// Append an output, as a block that creates it does.
    pub fn add_output(&mut self, output: &mw::Output) {
        let id = mweb::output_id(output);
        self.outputs.append(&id);
        self.leaves.push(Leaf {
            id,
            compact: mweb::compact_output(output),
            commitment: output.commitment,
            receiver: output.receiver_public_key,
            unspent: true,
            noise: false,
        });
    }

    /// Append `count` outputs to keys nobody holds, with placeholder proofs
    /// and signatures: a light client reads them as it reads any output and
    /// proves them into the root, and nothing spends them.
    pub fn add_noise(&mut self, count: usize) {
        // A random x on the curve: cheaper than a multiplication.
        let key = || loop {
            let mut bytes = [0x02u8; 33];
            bytes[1..].copy_from_slice(&rand::random::<[u8; 32]>());
            if let Ok(key) = PublicKey::from_slice(&bytes) {
                break key;
            }
        };
        for _ in 0..count {
            let mut compact = vec![0x08];
            compact.extend(rand::random::<[u8; 32]>());
            let commitment: [u8; 33] = compact[..33].try_into().unwrap();
            let (sender, receiver) = (key(), key());
            compact.extend(sender.serialize());
            compact.extend(receiver.serialize());
            let mut message = vec![1];
            message.extend(key().serialize());
            message.push(rand::random());
            message.extend(rand::random::<[u8; 24]>());
            compact.extend(&message);
            let proof_hash: [u8; 32] = rand::random();
            compact.extend(proof_hash);
            let signature: [u8; 64] = std::array::from_fn(|_| rand::random());
            let mut preimage = commitment.to_vec();
            preimage.extend(sender.serialize());
            preimage.extend(receiver.serialize());
            preimage.extend(mweb::blake(&message));
            preimage.extend(proof_hash);
            preimage.extend(signature);
            compact.extend(signature);
            let id = mweb::blake(&preimage);
            self.outputs.append(&id);
            self.leaves.push(Leaf {
                id,
                compact,
                commitment,
                receiver,
                unspent: true,
                noise: true,
            });
        }
    }

    /// Mark every `every`th placeholder output so far spent, and those in
    /// a run of leaves, as earlier blocks spent them.
    pub fn spend_noise(&mut self, every: usize, run: std::ops::Range<usize>) {
        for (index, leaf) in self.leaves.iter_mut().enumerate() {
            if leaf.noise && (index % every == every - 1 || run.contains(&index)) {
                leaf.unspent = false;
            }
        }
    }

    /// Pay `value` to `script` from nowhere: a transparent output the
    /// indexer lists, as if an exchange had sent it.
    pub fn fund(&mut self, script: ScriptBuf, value: u64) {
        let tx = Transaction {
            version: transaction::Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::new(Txid::from_byte_array(rand::random()), 0),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::MAX,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(value),
                script_pubkey: script,
            }],
            mw_tx: None,
            is_hog_ex: false,
        };
        let height = self.tip().height + 1;
        self.mine(height, vec![tx], &[], 0);
    }

    /// Mine a block of `canonical` transactions, the MWEB outputs added
    /// since the last block, and `pegouts`; `fees` are what its kernels pay.
    pub fn mine(&mut self, height: u32, canonical: Vec<Transaction>, pegouts: &[TxOut], fees: u64) {
        let time = self
            .blocks
            .last()
            .map_or(1_791_000_000, |tip| tip.time + SPACING);
        let leaves = self.leaves.len() as u64;
        let mut leafset = vec![0u8; leaves.div_ceil(8) as usize];
        for (index, leaf) in self.leaves.iter().enumerate() {
            if leaf.unspent {
                leafset[index / 8] |= 0x80 >> (index % 8);
            }
        }
        let mut header = Vec::new();
        mweb::varint(u64::from(height), &mut header);
        header.extend(self.outputs.root(leaves));
        header.extend(self.kernels.root(self.kernel_count));
        header.extend(mweb::blake(&leafset));
        header.extend([0u8; 32]);
        header.extend([0u8; 32]);
        mweb::varint(leaves, &mut header);
        mweb::varint(self.kernel_count, &mut header);

        // The HogEx spends the last one's HogAddr and each peg-in, and pays
        // the new HogAddr and each peg-out.
        let pegins: Vec<(OutPoint, u64)> = canonical
            .iter()
            .flat_map(|tx| {
                let txid = tx.compute_txid();
                tx.output.iter().enumerate().filter_map(move |(vout, out)| {
                    let script = out.script_pubkey.as_bytes();
                    (script.len() == 34 && script[..2] == [0x59, 0x20])
                        .then(|| (OutPoint::new(txid, vout as u32), out.value.to_sat()))
                })
            })
            .collect();
        // The MWEB supply before the fixture's first block: what the
        // outputs it starts with, and every other, hold.
        let previous = self.hogaddr.map_or(10_000_000_000_000, |(_, value)| value);
        let total = previous + pegins.iter().map(|(_, value)| value).sum::<u64>()
            - pegouts.iter().map(|out| out.value.to_sat()).sum::<u64>()
            - fees;
        let mut output = vec![TxOut {
            value: Amount::from_sat(total),
            script_pubkey: ScriptBuf::from_bytes(mweb::hogaddr_script(&mweb::blake(&header))),
        }];
        output.extend(pegouts.iter().cloned());
        let hogex = Transaction {
            version: transaction::Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: self
                .hogaddr
                .iter()
                .map(|(outpoint, _)| *outpoint)
                .chain(pegins.iter().map(|(outpoint, _)| *outpoint))
                .map(|previous_output| TxIn {
                    previous_output,
                    script_sig: ScriptBuf::new(),
                    sequence: Sequence::MAX,
                    witness: Witness::new(),
                })
                .collect(),
            output,
            mw_tx: None,
            is_hog_ex: true,
        };
        let hogex_txid = hogex.compute_txid();
        self.hogaddr = Some((OutPoint::new(hogex_txid, 0), total));

        let mut txids = vec![coinbase(height).compute_txid()];
        txids.extend(canonical.iter().map(Transaction::compute_txid));
        txids.push(hogex_txid);
        let merkle_root =
            litecoin::merkle_tree::calculate_root(txids.iter().map(|txid| txid.to_raw_hash()))
                .map(litecoin::TxMerkleNode::from_raw_hash)
                .expect("a transaction");
        let block_header = block::Header {
            version: block::Version::from_consensus(0x2000_0000),
            prev_blockhash: self
                .blocks
                .last()
                .map_or(BlockHash::all_zeros(), |tip| tip.hash),
            merkle_root,
            time,
            bits: litecoin::CompactTarget::from_consensus(0x1e0f_ffff),
            nonce: 0,
        };

        // The indexer's view: canonical transactions, then the HogEx's
        // peg-outs.
        for tx in &canonical {
            let txid = tx.compute_txid();
            let inputs: Vec<(Option<String>, u64)> = tx
                .input
                .iter()
                .map(|input| {
                    self.utxos
                        .remove(&input.previous_output)
                        .map_or((None, 0), |spent| {
                            (self.address_of(&spent.script_pubkey), spent.value.to_sat())
                        })
                })
                .collect();
            let outputs: Vec<(Option<String>, u64)> = tx
                .output
                .iter()
                .map(|out| (self.address_of(&out.script_pubkey), out.value.to_sat()))
                .collect();
            for (vout, out) in tx.output.iter().enumerate() {
                if !pegins
                    .iter()
                    .any(|(outpoint, _)| *outpoint == OutPoint::new(txid, vout as u32))
                {
                    self.utxos
                        .insert(OutPoint::new(txid, vout as u32), out.clone());
                }
            }
            let spent: u64 = inputs.iter().map(|(_, value)| value).sum();
            let paid: u64 = outputs.iter().map(|(_, value)| value).sum();
            self.indexed.push(Indexed {
                txid,
                height,
                time,
                inputs,
                outputs,
                fee: spent.saturating_sub(paid),
            });
        }
        if !pegouts.is_empty() {
            for (vout, out) in hogex.output.iter().enumerate().skip(1) {
                self.utxos
                    .insert(OutPoint::new(hogex_txid, vout as u32), out.clone());
            }
            self.indexed.push(Indexed {
                txid: hogex_txid,
                height,
                time,
                inputs: Vec::new(),
                outputs: pegouts
                    .iter()
                    .map(|out| (self.address_of(&out.script_pubkey), out.value.to_sat()))
                    .collect(),
                fee: 0,
            });
        }
        self.blocks.push(Block {
            height,
            hash: block_header.block_hash(),
            time,
            header: block_header,
            hogex,
            txids,
            mweb_header: header,
            leaves,
            leafset,
        });
    }

    /// An `mwebheader` answer: the block's Merkle branch to its HogEx, the
    /// HogEx, and the MWEB header it commits to.
    pub fn mweb_header_answer(&self, block: &Block) -> Vec<u8> {
        let hogex_txid = *block.txids.last().unwrap();
        let merkle = litecoin::MerkleBlock::from_header_txids_with_predicate(
            &block.header,
            &block.txids,
            |txid| *txid == hogex_txid,
        );
        let mut out = litecoin::consensus::encode::serialize(&merkle);
        out.extend(litecoin::consensus::encode::serialize(&block.hogex));
        out.extend(&block.mweb_header);
        out
    }

    pub fn leafset_answer(&self, block: &Block) -> Vec<u8> {
        let mut out = block.hash.to_byte_array().to_vec();
        mweb::compact_size(block.leafset.len() as u64, &mut out);
        out.extend(&block.leafset);
        out
    }

    /// An `mwebutxos` answer: up to `count` unspent outputs of `block` from
    /// leaf `start`, and the hashes that prove them, as Litecoin Core's
    /// `SegmentFactory` assembles them. `None` when `start` is not an
    /// unspent leaf, which a node answers by disconnecting.
    pub fn utxos_answer(&self, block: &Block, start: u64, count: u16) -> Option<Vec<u8>> {
        if !unspent_in(&block.leafset, start) || start >= block.leaves {
            return None;
        }
        let mut chosen = Vec::new();
        let mut leaf = start;
        while chosen.len() < usize::from(count) && leaf < block.leaves {
            if unspent_in(&block.leafset, leaf) {
                chosen.push(leaf);
            }
            leaf += 1;
        }
        let last = *chosen.last()?;
        let mut out = block.hash.to_byte_array().to_vec();
        mweb::compact_size(start, &mut out);
        out.push(2);
        mweb::compact_size(chosen.len() as u64, &mut out);
        for &leaf in &chosen {
            mweb::compact_size(leaf, &mut out);
            out.extend(&self.leaves[leaf as usize].compact);
        }
        let hashes = self.segment_hashes(block, start, last);
        mweb::compact_size(hashes.len() as u64, &mut out);
        for hash in hashes {
            out.extend(hash);
        }
        Some(out)
    }

    /// The hashes beside the leaves from `first` to `last`: the peaks before
    /// the first leaf, the left edge of its mountain, the roots of the
    /// wholly spent subtrees between the two leaves and the right edge of
    /// the last one's mountain, by position; then the peaks after that
    /// mountain, bagged.
    fn segment_hashes(&self, block: &Block, first: u64, last: u64) -> Vec<[u8; 32]> {
        let size = node_of(block.leaves);
        let peaks = peaks(size);
        let (first_node, last_node) = (node_of(first), node_of(last));
        let mut wanted = BTreeSet::new();
        wanted.extend(peaks.iter().copied().filter(|&peak| peak < first_node));
        // Up from the first leaf: each left sibling.
        let (mut pos, mut h) = (first_node, 0u32);
        while !peaks.contains(&pos) {
            if height(pos + 1) == h + 1 {
                wanted.insert(pos + 1 - (1u64 << (h + 1)));
                pos += 1;
            } else {
                pos += 1u64 << (h + 1);
            }
            h += 1;
        }
        // Up from the last leaf: each right sibling.
        let (mut pos, mut h) = (last_node, 0u32);
        while !peaks.contains(&pos) {
            if height(pos + 1) == h + 1 {
                pos += 1;
            } else {
                wanted.insert(pos + (1u64 << (h + 1)) - 1);
                pos += 1u64 << (h + 1);
            }
            h += 1;
        }
        // Between them, the largest subtrees with no unspent leaf.
        let mut lo = 0u64;
        for &peak in &peaks {
            let h = height(peak);
            self.pruned(block, peak, h, lo, first, last, &mut wanted);
            lo += 1u64 << h;
        }
        let mut hashes: Vec<[u8; 32]> = wanted
            .into_iter()
            .map(|pos| self.outputs.nodes[pos as usize])
            .collect();
        let mountain = *peaks.iter().find(|&&peak| peak >= last_node).unwrap();
        let after: Vec<u64> = peaks
            .iter()
            .copied()
            .filter(|&peak| peak > mountain)
            .collect();
        if !after.is_empty() {
            hashes.push(self.outputs.bag(size, &after));
        }
        hashes
    }

    /// Add to `wanted` the roots of the wholly spent subtrees under `pos`
    /// (height `h`, leaves from `lo`) that lie strictly between `first` and
    /// `last`.
    #[allow(clippy::too_many_arguments)]
    fn pruned(
        &self,
        block: &Block,
        pos: u64,
        h: u32,
        lo: u64,
        first: u64,
        last: u64,
        wanted: &mut BTreeSet<u64>,
    ) {
        let hi = lo + (1u64 << h);
        if hi <= first + 1 || lo >= last {
            return;
        }
        if lo > first && hi <= last && (lo..hi).all(|leaf| !unspent_in(&block.leafset, leaf)) {
            wanted.insert(pos);
            return;
        }
        if h == 0 {
            return;
        }
        let half = 1u64 << (h - 1);
        self.pruned(block, pos - (1u64 << h), h - 1, lo, first, last, wanted);
        self.pruned(block, pos - 1, h - 1, lo + half, first, last, wanted);
    }

    /// The headers after the block `after`, through the tip.
    pub fn headers_after(&self, after: &BlockHash) -> Vec<block::Header> {
        let start = self
            .blocks
            .iter()
            .position(|block| block.hash == *after)
            .map_or(0, |index| index + 1);
        self.blocks[start..]
            .iter()
            .take(2000)
            .map(|block| block.header)
            .collect()
    }

    fn unspent_output(&self, id: &[u8; 32]) -> Option<mweb::Unspent> {
        self.leaves
            .iter()
            .find(|leaf| leaf.unspent && leaf.id == *id)
            .map(|leaf| mweb::Unspent {
                commitment: leaf.commitment,
                receiver: leaf.receiver,
            })
    }

    /// Check a transaction as a node would, mine it into a block of its own
    /// and journal it; its id as the node names it.
    pub fn accept(&mut self, raw: &[u8]) -> Result<String, String> {
        let tx: Transaction = litecoin::consensus::encode::deserialize(raw)
            .map_err(|error| format!("TX decode failed: {error}"))?;
        // The canonical inputs: each an unspent output, each signed.
        let mut spent = 0u64;
        for (index, input) in tx.input.iter().enumerate() {
            let prevout = self
                .utxos
                .get(&input.previous_output)
                .ok_or("bad-txns-inputs-missingorspent")?;
            if !signed(&tx, index, prevout) {
                return Err("mandatory-script-verify-flag-failed".into());
            }
            spent += prevout.value.to_sat();
        }
        let paid: u64 = tx.output.iter().map(|out| out.value.to_sat()).sum();
        let canonical_fee = spent.checked_sub(paid).ok_or("bad-txns-in-belowout")?;
        if tx
            .output
            .iter()
            .any(|out| out.value.to_sat() < dust(&out.script_pubkey))
        {
            return Err("dust".into());
        }
        let canonical = !tx.input.is_empty() || !tx.output.is_empty();
        let mut kernel_fee = 0;
        let mut pegouts = Vec::new();
        let mut journal = json!({ "canonical_fee": canonical_fee });
        if let Some(mweb_tx) = &tx.mw_tx {
            let checked = mweb::check_transaction(mweb_tx, |id| self.unspent_output(id))?;
            // Each peg-in kernel is paid by the canonical output that names
            // it, and a transaction with a canonical part has no other
            // kernel (`MWEB::Policy::IsStandardTx`).
            let named: BTreeMap<Vec<u8>, u64> = tx
                .output
                .iter()
                .filter(|out| out.script_pubkey.as_bytes().starts_with(&[0x59, 0x20]))
                .map(|out| (out.script_pubkey.to_bytes(), out.value.to_sat()))
                .collect();
            let pegins: BTreeMap<Vec<u8>, u64> = checked
                .pegins
                .iter()
                .map(|(id, value)| (mweb::pegin_script(id), *value))
                .collect();
            if named != pegins {
                return Err("bad-pegin-outputs".into());
            }
            if canonical && pegins.len() != mweb_tx.body.kernels.len() {
                return Err("kernel-mismatch".into());
            }
            for (script, value) in &checked.pegouts {
                if *value < dust(script) {
                    return Err("pegout-dust".into());
                }
                pegouts.push(TxOut {
                    value: Amount::from_sat(*value),
                    script_pubkey: script.clone(),
                });
            }
            kernel_fee = checked.fee;
            let mut paid_to = Vec::new();
            for output in &mweb_tx.body.outputs {
                let owner = self.wallets.iter().find_map(|(name, keys)| {
                    keys.rewind(output)
                        .map(|(index, value)| (name, index, value))
                });
                paid_to.push(match owner {
                    Some((name, index, value)) => {
                        json!({"owner": name, "index": index, "value": value})
                    }
                    None => json!({"owner": null}),
                });
                self.add_output(output);
            }
            for input in &mweb_tx.body.inputs {
                if let Some(leaf) = self
                    .leaves
                    .iter_mut()
                    .find(|leaf| leaf.id == input.output_id)
                {
                    leaf.unspent = false;
                }
            }
            for kernel in &mweb_tx.body.kernels {
                self.kernels.append(&mweb::kernel_id(kernel));
                self.kernel_count += 1;
            }
            journal["mweb_outputs"] = json!(paid_to);
            journal["mweb_inputs"] = json!(mweb_tx.body.inputs.len());
            journal["kernel_fee"] = json!(kernel_fee);
            journal["pegin"] = json!(checked.pegins.iter().map(|(_, value)| value).sum::<u64>());
            journal["pegouts"] = json!(
                checked
                    .pegouts
                    .iter()
                    .map(|(script, value)| json!({"address": self.address_of(script), "value": value}))
                    .collect::<Vec<_>>()
            );
        } else if !canonical {
            return Err("an empty transaction".into());
        }
        let txid = if canonical {
            tx.compute_txid().to_string()
        } else {
            // An MWEB-only transaction is named by its first kernel.
            let mut id = mweb::kernel_id(&tx.mw_tx.as_ref().unwrap().body.kernels[0]);
            id.reverse();
            hex::encode(id)
        };
        journal["txid"] = json!(txid);
        journal["canonical_outputs"] = json!(
            tx.output
                .iter()
                .map(|out| json!({"address": self.address_of(&out.script_pubkey), "value": out.value.to_sat()}))
                .collect::<Vec<_>>()
        );
        let height = self.tip().height + 1;
        let canonical_txs = if canonical { vec![tx] } else { Vec::new() };
        self.mine(height, canonical_txs, &pegouts, kernel_fee);
        journal["height"] = json!(height);
        if let Some(file) = &mut self.journal {
            writeln!(file, "{journal}").expect("a journal line");
        }
        Ok(txid)
    }

    pub fn address_json(&self, address: &str) -> serde_json::Value {
        let (mut funded, mut spent, mut count) = (0u64, 0u64, 0u64);
        for tx in &self.indexed {
            let out: u64 = tx
                .outputs
                .iter()
                .filter(|(a, _)| a.as_deref() == Some(address))
                .map(|(_, v)| v)
                .sum();
            let r#in: u64 = tx
                .inputs
                .iter()
                .filter(|(a, _)| a.as_deref() == Some(address))
                .map(|(_, v)| v)
                .sum();
            if out > 0 || r#in > 0 {
                count += 1;
            }
            funded += out;
            spent += r#in;
        }
        let zero = json!({"funded_txo_sum": 0, "spent_txo_sum": 0, "tx_count": 0});
        json!({
            "address": address,
            "chain_stats": {"funded_txo_sum": funded, "spent_txo_sum": spent, "tx_count": count},
            "mempool_stats": zero,
        })
    }

    pub fn utxos_json(&self, address: &str) -> serde_json::Value {
        let height = |txid: &Txid| {
            self.indexed
                .iter()
                .find(|tx| tx.txid == *txid)
                .map(|tx| tx.height)
        };
        json!(
            self.utxos
                .iter()
                .filter(|(_, out)| self.address_of(&out.script_pubkey).as_deref() == Some(address))
                .map(|(outpoint, out)| json!({
                    "txid": outpoint.txid.to_string(),
                    "vout": outpoint.vout,
                    "value": out.value.to_sat(),
                    "status": {"confirmed": true, "block_height": height(&outpoint.txid)},
                }))
                .collect::<Vec<_>>()
        )
    }

    pub fn history_json(&self, address: &str) -> serde_json::Value {
        let row = |(address, value): &(Option<String>, u64)| json!({"scriptpubkey_address": address, "value": value});
        json!(
            self.indexed
                .iter()
                .rev()
                .filter(|tx| {
                    tx.inputs.iter().chain(&tx.outputs).any(|(a, _)| a.as_deref() == Some(address))
                })
                .map(|tx| json!({
                    "txid": tx.txid.to_string(),
                    "status": {"confirmed": true, "block_height": tx.height, "block_time": tx.time},
                    "vin": tx.inputs.iter().map(|input| json!({"prevout": row(input)})).collect::<Vec<_>>(),
                    "vout": tx.outputs.iter().map(row).collect::<Vec<_>>(),
                    "fee": tx.fee,
                }))
                .collect::<Vec<_>>()
        )
    }

    pub fn status_json(&self, txid: &str) -> Option<serde_json::Value> {
        let tx = self.indexed.iter().find(|tx| tx.txid.to_string() == txid)?;
        Some(json!({"confirmed": true, "block_height": tx.height, "block_time": tx.time}))
    }
}

/// Whether canonical input `index` is signed by the key its output names:
/// P2PKH over the legacy hash, P2WPKH over BIP-143's.
fn signed(tx: &Transaction, index: usize, prevout: &TxOut) -> bool {
    let mut cache = litecoin::sighash::SighashCache::new(tx);
    let script = &prevout.script_pubkey;
    let (signature, key, digest) = if script.is_p2wpkh() {
        let witness = &tx.input[index].witness;
        let (Some(signature), Some(key)) = (witness.nth(0), witness.nth(1)) else {
            return false;
        };
        let Ok(key) = CompressedPublicKey::from_slice(key) else {
            return false;
        };
        if ScriptBuf::new_p2wpkh(&key.wpubkey_hash()) != *script {
            return false;
        }
        let Ok(digest) =
            cache.p2wpkh_signature_hash(index, script, prevout.value, EcdsaSighashType::All)
        else {
            return false;
        };
        (signature.to_vec(), key.0, digest.to_byte_array())
    } else if script.is_p2pkh() {
        let pushes: Vec<Vec<u8>> = tx.input[index]
            .script_sig
            .instructions()
            .filter_map(|instruction| match instruction {
                Ok(litecoin::script::Instruction::PushBytes(bytes)) => {
                    Some(bytes.as_bytes().to_vec())
                }
                _ => None,
            })
            .collect();
        let [signature, key] = pushes.as_slice() else {
            return false;
        };
        let Ok(key) = litecoin::PublicKey::from_slice(key) else {
            return false;
        };
        if ScriptBuf::new_p2pkh(&key.pubkey_hash()) != *script {
            return false;
        }
        let Ok(digest) = cache.legacy_signature_hash(index, script, EcdsaSighashType::All as u32)
        else {
            return false;
        };
        (signature.clone(), key.inner, digest.to_byte_array())
    } else {
        return false;
    };
    let Ok(signature) = litecoin::ecdsa::Signature::from_slice(&signature) else {
        return false;
    };
    signature.sighash_type == EcdsaSighashType::All
        && SECP256K1
            .verify_ecdsa(&Message::from_digest(digest), &signature.signature, &key)
            .is_ok()
}
