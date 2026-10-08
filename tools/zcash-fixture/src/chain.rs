//! The synthetic chain: compact blocks, the note commitment trees after each,
//! whole transactions, transparent outputs, and the nullifiers spent.

use std::collections::{BTreeMap, HashMap, HashSet};

use incrementalmerkletree::frontier::CommitmentTree;
use orchard::tree::MerkleHashOrchard;
use rand::RngCore;
use zcash_client_backend::proto::compact_formats::{
    ChainMetadata, CompactBlock, CompactOrchardAction, CompactSaplingOutput, CompactSaplingSpend,
    CompactTx, CompactTxIn, TxOut as CompactTxOut,
};
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::{BlockHeight, BranchId, Network};

pub type SaplingTree = CommitmentTree<sapling_crypto::Node, 32>;
pub type OrchardTree = CommitmentTree<MerkleHashOrchard, 32>;

/// The trees as of the end of one block.
#[derive(Clone)]
pub struct Trees {
    pub sapling: SaplingTree,
    pub orchard: OrchardTree,
    pub ironwood: OrchardTree,
}

/// A transparent output the chain holds unspent.
#[derive(Clone)]
pub struct Utxo {
    pub txid: [u8; 32],
    pub index: u32,
    pub value: u64,
    pub script: Vec<u8>,
    pub height: u32,
    pub address: String,
}

pub struct Chain {
    pub network: Network,
    pub start: u32,
    pub blocks: Vec<CompactBlock>,
    /// The trees after each block, by height; the trees before `start` are
    /// empty.
    pub trees: BTreeMap<u32, Trees>,
    pub transactions: HashMap<[u8; 32], (Vec<u8>, u32)>,
    pub utxos: Vec<Utxo>,
    pub nullifiers: HashSet<Vec<u8>>,
    /// Every Orchard-family and Sapling anchor the chain has had.
    pub orchard_anchors: HashSet<[u8; 32]>,
    pub ironwood_anchors: HashSet<[u8; 32]>,
    pub sapling_anchors: HashSet<[u8; 32]>,
    /// Transparent addresses the chain records outputs to, as UTXOs.
    pub watched: Vec<String>,
}

fn empty_trees() -> Trees {
    Trees {
        sapling: CommitmentTree::empty(),
        orchard: CommitmentTree::empty(),
        ironwood: CommitmentTree::empty(),
    }
}

impl Chain {
    pub fn new(network: Network, start: u32, watched: Vec<String>) -> Self {
        let empty = empty_trees();
        let mut chain = Self {
            network,
            start,
            blocks: Vec::new(),
            trees: BTreeMap::new(),
            transactions: HashMap::new(),
            utxos: Vec::new(),
            nullifiers: HashSet::new(),
            orchard_anchors: HashSet::new(),
            ironwood_anchors: HashSet::new(),
            sapling_anchors: HashSet::new(),
            watched,
        };
        chain.record_anchors(&empty);
        chain
    }

    pub fn tip(&self) -> u32 {
        self.start + self.blocks.len() as u32 - 1
    }

    pub fn next_height(&self) -> u32 {
        self.start + self.blocks.len() as u32
    }

    pub fn branch(&self) -> BranchId {
        BranchId::for_height(&self.network, BlockHeight::from_u32(self.next_height()))
    }

    pub fn trees_at(&self, height: u32) -> Trees {
        self.trees
            .range(..=height)
            .next_back()
            .map(|(_, trees)| trees.clone())
            .unwrap_or_else(empty_trees)
    }

    fn record_anchors(&mut self, trees: &Trees) {
        self.sapling_anchors.insert(trees.sapling.root().to_bytes());
        self.orchard_anchors.insert(trees.orchard.root().to_bytes());
        self.ironwood_anchors
            .insert(trees.ironwood.root().to_bytes());
    }

    /// Mine a block holding `transactions`, in order.
    pub fn mine(&mut self, transactions: Vec<Transaction>) {
        let height = self.next_height();
        let mut trees = self.trees_at(height.saturating_sub(1));
        let mut vtx = Vec::new();
        for (index, tx) in transactions.into_iter().enumerate() {
            let txid = *tx.txid().as_ref();
            let mut compact = CompactTx {
                index: index as u64,
                txid: txid.to_vec(),
                ..Default::default()
            };
            if let Some(bundle) = tx.transparent_bundle() {
                for input in &bundle.vin {
                    let prevout = input.prevout();
                    compact.vin.push(CompactTxIn {
                        prevout_txid: prevout.hash().to_vec(),
                        prevout_index: prevout.n(),
                    });
                    self.utxos.retain(|utxo| {
                        !(utxo.txid == *prevout.hash() && utxo.index == prevout.n())
                    });
                }
                for (n, output) in bundle.vout.iter().enumerate() {
                    compact.vout.push(CompactTxOut {
                        value: u64::from(output.value()),
                        script_pub_key: output.script_pubkey().0.0.clone(),
                    });
                    if let Some(address) = output.recipient_address() {
                        let address =
                            zcash_keys::encoding::AddressCodec::encode(&address, &self.network);
                        if self.watched.contains(&address) {
                            self.utxos.push(Utxo {
                                txid,
                                index: n as u32,
                                value: u64::from(output.value()),
                                script: output.script_pubkey().0.0.clone(),
                                height,
                                address,
                            });
                        }
                    }
                }
            }
            if let Some(bundle) = tx.sapling_bundle() {
                for spend in bundle.shielded_spends() {
                    self.nullifiers.insert(spend.nullifier().0.to_vec());
                    compact.spends.push(CompactSaplingSpend {
                        nf: spend.nullifier().0.to_vec(),
                    });
                }
                for output in bundle.shielded_outputs() {
                    trees
                        .sapling
                        .append(sapling_crypto::Node::from_cmu(output.cmu()))
                        .expect("tree not full");
                    compact.outputs.push(CompactSaplingOutput {
                        cmu: output.cmu().to_bytes().to_vec(),
                        ephemeral_key: output.ephemeral_key().0.to_vec(),
                        ciphertext: output.enc_ciphertext()[..52].to_vec(),
                    });
                }
            }
            for (bundle, ironwood) in [(tx.orchard_bundle(), false), (tx.ironwood_bundle(), true)] {
                let Some(bundle) = bundle else { continue };
                for action in bundle.actions() {
                    self.nullifiers
                        .insert(action.nullifier().to_bytes().to_vec());
                    let leaf = MerkleHashOrchard::from_cmx(action.cmx());
                    let tree = if ironwood {
                        &mut trees.ironwood
                    } else {
                        &mut trees.orchard
                    };
                    tree.append(leaf).expect("tree not full");
                    let compact_action = CompactOrchardAction {
                        nullifier: action.nullifier().to_bytes().to_vec(),
                        cmx: action.cmx().to_bytes().to_vec(),
                        ephemeral_key: action.encrypted_note().epk_bytes.to_vec(),
                        ciphertext: action.encrypted_note().enc_ciphertext[..52].to_vec(),
                    };
                    if ironwood {
                        compact.ironwood_actions.push(compact_action);
                    } else {
                        compact.actions.push(compact_action);
                    }
                }
            }
            let mut raw = Vec::new();
            tx.write(&mut raw).expect("a transaction writes");
            self.transactions.insert(txid, (raw, height));
            vtx.push(compact);
        }
        let mut hash = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut hash);
        let prev_hash = self
            .blocks
            .last()
            .map(|block| block.hash.clone())
            .unwrap_or_else(|| vec![0u8; 32]);
        self.blocks.push(CompactBlock {
            height: u64::from(height),
            hash: hash.to_vec(),
            prev_hash,
            time: 1_790_000_000 + 75 * (height - self.start),
            header: vec![],
            vtx,
            chain_metadata: Some(ChainMetadata {
                sapling_commitment_tree_size: trees.sapling.size() as u32,
                orchard_commitment_tree_size: trees.orchard.size() as u32,
                ironwood_commitment_tree_size: trees.ironwood.size() as u32,
            }),
        });
        self.record_anchors(&trees);
        self.trees.insert(height, trees);
    }

    pub fn block(&self, height: u32) -> Option<&CompactBlock> {
        height
            .checked_sub(self.start)
            .and_then(|offset| self.blocks.get(offset as usize))
    }
}
