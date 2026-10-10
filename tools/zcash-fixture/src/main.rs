//! A loopback lightwalletd over a synthetic Zcash chain, for the CLI
//! acceptance suite.
//!
//! The chain starts at `--start` with empty note commitment trees and funds
//! the wallet named by `--wallet-phrase-env` with real transactions: an
//! Ironwood (Orchard before NU6.3) note with a memo, a Sapling note, and a
//! transparent output to its standard address. Each is built and signed by
//! librustzcash from a faucet's transparent coins; the Sapling output's
//! proof is a placeholder, as no Sapling parameters are at hand and nothing
//! that scans checks it. Transactions handed to `SendTransaction` are checked
//! as a node would (`verify`), then mined and journaled.
//!
//! Prints one JSON line — the port and the wallet's and an outside
//! recipient's addresses — then serves until killed.

mod chain;
mod grpc;
mod verify;

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use zcash_keys::keys::{ReceiverRequirement, UnifiedAddressRequest, UnifiedSpendingKey};
use zcash_primitives::transaction::builder::{BuildConfig, Builder, BundlePadding};
use zcash_primitives::transaction::fees::zip317::FeeRule;
use zcash_protocol::consensus::{BlockHeight, Network, NetworkUpgrade, Parameters};
use zcash_protocol::memo::MemoBytes;
use zcash_protocol::value::Zatoshis;
use zcash_transparent::address::TransparentAddress;
use zcash_transparent::builder::TransparentSigningSet;
use zcash_transparent::bundle::{OutPoint, TxOut};
use zcash_transparent::keys::{IncomingViewingKey, NonHardenedChildIndex};

/// What the fixture funds the wallet with, in zatoshis, one transaction each.
const FUND_SHIELDED: u64 = 150_000_000;
const FUND_SAPLING: u64 = 25_000_000;
const FUND_TRANSPARENT: u64 = 50_000_000;

struct Args {
    start: u32,
    blocks: u32,
    wallet: String,
    outsider: String,
    journal: Option<std::path::PathBuf>,
}

fn args() -> Args {
    let mut values: HashMap<String, String> = HashMap::new();
    let mut raw = std::env::args().skip(1);
    while let Some(key) = raw.next() {
        let value = raw.next().unwrap_or_else(|| panic!("{key} needs a value"));
        values.insert(key.trim_start_matches("--").to_string(), value);
    }
    let phrase = |key: &str| {
        let var = values
            .get(key)
            .unwrap_or_else(|| panic!("--{key} is required"));
        std::env::var(var).unwrap_or_else(|_| panic!("{var} is not set"))
    };
    let number = |key: &str, default: u64| {
        values
            .get(key)
            .map_or(default, |value| value.parse().expect("a number"))
    };
    Args {
        start: number("start", 3_430_000) as u32,
        blocks: number("blocks", 30) as u32,
        wallet: phrase("wallet-phrase-env"),
        outsider: phrase("outsider-phrase-env"),
        journal: values.get("journal").map(Into::into),
    }
}

/// An account's spending key, from a BIP-39 phrase without a passphrase, as
/// ZIP-32 derives account 0.
fn account(network: &Network, phrase: &str) -> UnifiedSpendingKey {
    let seed = bip39::Mnemonic::parse(phrase)
        .expect("a BIP-39 phrase")
        .to_seed("");
    UnifiedSpendingKey::from_seed(network, &seed, zip32::AccountId::ZERO).expect("keys")
}

fn shielded_address(
    network: &Network,
    usk: &UnifiedSpendingKey,
) -> zcash_keys::address::UnifiedAddress {
    let _ = network;
    usk.to_unified_full_viewing_key()
        .default_address(UnifiedAddressRequest::unsafe_custom(
            ReceiverRequirement::Require,
            ReceiverRequirement::Require,
            ReceiverRequirement::Omit,
        ))
        .expect("an address")
        .0
}

/// The account's transparent address at index 0: `m/44'/coin'/0'/0/0`.
fn transparent_address(usk: &UnifiedSpendingKey) -> TransparentAddress {
    usk.to_unified_full_viewing_key()
        .transparent()
        .expect("a transparent key")
        .derive_external_ivk()
        .expect("an external key")
        .derive_address(NonHardenedChildIndex::ZERO)
        .expect("an address")
}

/// A Sapling prover with a placeholder proof: no parameters are at hand,
/// and nothing that scans checks one.
struct PlaceholderProver;

impl sapling_crypto::prover::SpendProver for PlaceholderProver {
    type Proof = ();
    fn prepare_circuit(
        proof_generation_key: sapling_crypto::ProofGenerationKey,
        diversifier: sapling_crypto::Diversifier,
        rseed: sapling_crypto::Rseed,
        value: sapling_crypto::value::NoteValue,
        alpha: jubjub::Fr,
        rcv: sapling_crypto::value::ValueCommitTrapdoor,
        anchor: bls12_381::Scalar,
        merkle_path: sapling_crypto::MerklePath,
    ) -> Option<sapling_crypto::circuit::Spend> {
        <sapling_crypto::circuit::SpendParameters as sapling_crypto::prover::SpendProver>::prepare_circuit(
            proof_generation_key, diversifier, rseed, value, alpha, rcv, anchor, merkle_path,
        )
    }
    fn create_proof<R: rand::RngCore>(&self, _: sapling_crypto::circuit::Spend, _: &mut R) {}
    fn encode_proof(_: ()) -> sapling_crypto::bundle::GrothProofBytes {
        [0u8; 192]
    }
}

impl sapling_crypto::prover::OutputProver for PlaceholderProver {
    type Proof = ();
    fn prepare_circuit(
        esk: &sapling_crypto::keys::EphemeralSecretKey,
        payment_address: sapling_crypto::PaymentAddress,
        rcm: jubjub::Fr,
        value: sapling_crypto::value::NoteValue,
        rcv: sapling_crypto::value::ValueCommitTrapdoor,
    ) -> sapling_crypto::circuit::Output {
        <sapling_crypto::circuit::OutputParameters as sapling_crypto::prover::OutputProver>::prepare_circuit(
            esk, payment_address, rcm, value, rcv,
        )
    }
    fn create_proof<R: rand::RngCore>(&self, _: sapling_crypto::circuit::Output, _: &mut R) {}
    fn encode_proof(_: ()) -> sapling_crypto::bundle::GrothProofBytes {
        [0u8; 192]
    }
}

/// The faucet: a transparent key whose coins nothing else knows.
struct Faucet {
    key: secp256k1::SecretKey,
    address: TransparentAddress,
    next: u8,
}

fn config() -> BuildConfig {
    BuildConfig::Standard {
        sapling_anchor: Some(sapling_crypto::Anchor::empty_tree()),
        orchard_anchor: Some(orchard::Anchor::empty_tree()),
        ironwood_anchor: Some(orchard::Anchor::empty_tree()),
        orchard_padding: BundlePadding::DEFAULT,
        ironwood_padding: BundlePadding::DEFAULT,
    }
}

impl Faucet {
    fn new() -> Self {
        let mut bytes = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut bytes);
        let key = secp256k1::SecretKey::from_slice(&bytes).expect("a key");
        let pubkey = secp256k1::PublicKey::from_secret_key(secp256k1::SECP256K1, &key);
        Self {
            key,
            address: TransparentAddress::from_pubkey(&pubkey),
            next: 0,
        }
    }

    /// A transaction at `height` spending one faucet coin of 10 ZEC: `pay`
    /// adds outputs worth `paid`, the rest returns to the faucet less the
    /// ZIP-317 fee.
    fn pay(
        &mut self,
        network: Network,
        height: u32,
        paid: u64,
        pay: impl Fn(&mut Builder<Network, ()>),
    ) -> zcash_primitives::transaction::Transaction {
        self.next += 1;
        let coin = Zatoshis::const_from_u64(1_000_000_000);
        let outpoint = OutPoint::new([self.next; 32], 0);
        let pubkey = secp256k1::PublicKey::from_secret_key(secp256k1::SECP256K1, &self.key);
        let builder = |change: Zatoshis| {
            let mut builder = Builder::new(network, BlockHeight::from_u32(height), config());
            builder
                .add_transparent_p2pkh_input(
                    pubkey,
                    outpoint.clone(),
                    TxOut::new(coin, self.address.script().into()),
                )
                .expect("faucet input");
            pay(&mut builder);
            builder
                .add_transparent_output(&self.address, change)
                .expect("faucet change");
            builder
        };
        // The fee depends on the shape, not on the change: price a draft.
        let fee = builder(Zatoshis::const_from_u64(1))
            .get_fee(&FeeRule::standard())
            .expect("a fee");
        let change = Zatoshis::from_u64(1_000_000_000 - paid - u64::from(fee)).expect("change");
        let mut signing = TransparentSigningSet::new();
        signing.add_key(self.key);
        builder(change)
            .build(
                &signing,
                &[],
                &[],
                rand::rngs::OsRng,
                &PlaceholderProver,
                &PlaceholderProver,
                &FeeRule::standard(),
            )
            .expect("a funding transaction")
            .transaction()
            .clone()
    }
}

#[tokio::main]
async fn main() {
    let args = args();
    let network = Network::MainNetwork;
    let wallet = account(&network, &args.wallet);
    let outsider = account(&network, &args.outsider);
    let wallet_ua = shielded_address(&network, &wallet);
    let wallet_t = transparent_address(&wallet);
    let outsider_ua = shielded_address(&network, &outsider);
    let outsider_t = transparent_address(&outsider);
    let encode_t = |address: &TransparentAddress| {
        zcash_keys::encoding::AddressCodec::encode(address, &network)
    };
    let ironwood = network.is_nu_active(NetworkUpgrade::Nu6_3, BlockHeight::from_u32(args.start));

    let mut chain = chain::Chain::new(network, args.start, vec![encode_t(&wallet_t)]);
    let mut faucet = Faucet::new();
    chain.mine(vec![]);
    let memo = MemoBytes::from_bytes(b"fixture funding").expect("a memo");
    let orchard_to = *wallet_ua.orchard().expect("an Orchard receiver");
    let sapling_to = *wallet_ua.sapling().expect("a Sapling receiver");
    let shielded = Zatoshis::from_u64(FUND_SHIELDED).expect("a value");
    let funding = faucet.pay(
        network,
        chain.next_height(),
        FUND_SHIELDED,
        move |builder| {
            if ironwood {
                builder
                    .add_ironwood_output::<Infallible>(None, orchard_to, shielded, memo.clone())
                    .expect("an Ironwood output");
            } else {
                builder
                    .add_orchard_output::<Infallible>(None, orchard_to, shielded, memo.clone())
                    .expect("an Orchard output");
            }
        },
    );
    chain.mine(vec![funding]);
    let sapling = Zatoshis::from_u64(FUND_SAPLING).expect("a value");
    let funding = faucet.pay(network, chain.next_height(), FUND_SAPLING, move |builder| {
        builder
            .add_sapling_output::<Infallible>(None, sapling_to, sapling, MemoBytes::empty())
            .expect("a Sapling output");
    });
    chain.mine(vec![funding]);
    let transparent = Zatoshis::from_u64(FUND_TRANSPARENT).expect("a value");
    let funding = faucet.pay(
        network,
        chain.next_height(),
        FUND_TRANSPARENT,
        move |builder| {
            builder
                .add_transparent_output(&wallet_t, transparent)
                .expect("a transparent output");
        },
    );
    chain.mine(vec![funding]);
    while chain.blocks.len() < args.blocks as usize {
        chain.mine(vec![]);
    }

    let mut accounts = HashMap::new();
    accounts.insert("wallet", wallet.to_unified_full_viewing_key());
    accounts.insert("outsider", outsider.to_unified_full_viewing_key());
    let tip = chain.tip();
    let state = grpc::State {
        chain,
        accounts,
        journal: args.journal,
        confirmations: 10,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback port");
    let port = listener.local_addr().expect("an address").port();
    println!(
        "{}",
        serde_json::json!({
            "port": port,
            "tip": tip,
            "wallet_address": wallet_ua.encode(&network),
            "wallet_transparent": encode_t(&wallet_t),
            "outsider_address": outsider_ua.encode(&network),
            // The receiver a payment to the outsider reaches, as a wallet
            // that recovers the payment from the chain can know it.
            "outsider_orchard": zcash_keys::address::UnifiedAddress::from_receivers(
                outsider_ua.orchard().copied(),
                None,
                None,
            )
            .expect("an Orchard receiver")
            .encode(&network),
            "outsider_transparent": encode_t(&outsider_t),
            "pool": if ironwood { "ironwood" } else { "orchard" },
        })
    );
    use std::io::Write;
    std::io::stdout().flush().expect("stdout");
    tonic::transport::Server::builder()
        .add_service(grpc::Streamer(Arc::new(Mutex::new(state))))
        .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
        .await
        .expect("the server");
}
