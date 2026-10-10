//! A loopback Litecoin node and indexer over a synthetic chain, for the CLI
//! acceptance suite.
//!
//! The chain's first blocks hold tens of thousands of placeholder MWEB
//! outputs, some spent, among which real outputs pay the wallet named by
//! `--wallet-phrase-env` — at its receive address and at a later one — and
//! transparent outputs pay its standard legacy and native SegWit addresses.
//! A light client reads the chain over the peer-to-peer protocol (`p2p`);
//! the wallet's indexer over Esplora's API (`http`), whose broadcast checks
//! each transaction as a node would and mines it (`chain`), journaling what
//! it paid whom as the wallet's and an outsider's keys read it.
//!
//! Prints one JSON line — the ports, the addresses and the tip — then serves
//! until killed.

mod chain;
mod http;
mod mweb;
mod p2p;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use litecoin::NetworkKind;
use secp256k1::{SECP256K1, SecretKey};

struct Args {
    network: NetworkKind,
    start: u32,
    noise: usize,
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
    Args {
        network: match values.get("network").map(String::as_str) {
            Some("test") => NetworkKind::Test,
            _ => NetworkKind::Main,
        },
        start: values
            .get("start")
            .map_or(3_000_000, |v| v.parse().expect("a height")),
        noise: values
            .get("noise")
            .map_or(40_000, |v| v.parse().expect("a count")),
        wallet: phrase("wallet-phrase-env"),
        outsider: phrase("outsider-phrase-env"),
        journal: values.get("journal").map(Into::into),
    }
}

/// A transparent key of a phrase at `path`, as BIP-32 derives it.
fn transparent_key(phrase: &str, path: &str) -> litecoin::CompressedPublicKey {
    let seed = bip39::Mnemonic::parse(phrase)
        .expect("a phrase")
        .to_seed("");
    let master =
        litecoin::bip32::Xpriv::new_master(NetworkKind::Main, &seed).expect("a master key");
    let key = master
        .derive_priv(
            SECP256K1,
            &path.parse::<litecoin::bip32::DerivationPath>().unwrap(),
        )
        .unwrap()
        .private_key;
    litecoin::CompressedPublicKey(secp256k1::PublicKey::from_secret_key(SECP256K1, &key))
}

fn main() {
    let args = args();
    let network = args.network;
    let coin = if network == NetworkKind::Main { 2 } else { 1 };
    let hrp = if network == NetworkKind::Main {
        litecoin::KnownHrp::Mainnet
    } else {
        litecoin::KnownHrp::Testnets
    };
    let wallet = mweb::Keys::from_phrase(&args.wallet);
    let outsider = mweb::Keys::from_phrase(&args.outsider);
    let mweb_address = |keys: &mweb::Keys, index: u32| keys.encoded(index, network);
    let legacy = |phrase: &str| {
        litecoin::Address::p2pkh(
            transparent_key(phrase, &format!("m/44'/{coin}'/0'/0/0")),
            network,
        )
    };
    let segwit = |phrase: &str| {
        litecoin::Address::p2wpkh(
            &transparent_key(phrase, &format!("m/84'/{coin}'/0'/0/0")),
            hrp,
        )
    };
    let info = serde_json::json!({
        "wallet": {
            "mweb": mweb_address(&wallet, 2),
            "pegin": mweb_address(&wallet, 1),
            "legacy": legacy(&args.wallet).to_string(),
        },
        "outsider": {
            "mweb": mweb_address(&outsider, 2),
            "legacy": legacy(&args.outsider).to_string(),
        },
    });
    let (scan, spend) = wallet.address(2);
    let (later_scan, later_spend) = wallet.address(7);
    let faucet = || SecretKey::from_slice(&rand::random::<[u8; 32]>()).expect("a key");

    let mut chain = chain::Chain::new(
        network,
        args.start,
        args.journal,
        vec![("wallet", wallet), ("outsider", outsider)],
    );
    // A block of traffic with the wallet's MWEB outputs among it, a run of
    // it spent and every tenth placeholder spent since.
    let third = args.noise / 3;
    chain.add_noise(third);
    chain.add_output(&mweb::create_output(&scan, &spend, 150_000_000, &faucet()));
    chain.add_noise(third);
    chain.add_output(&mweb::create_output(&scan, &spend, 25_000_000, &faucet()));
    chain.add_noise(args.noise - 2 * third);
    chain.add_output(&mweb::create_output(
        &later_scan,
        &later_spend,
        10_000_000,
        &faucet(),
    ));
    chain.spend_noise(10, 100..400);
    chain.mine(args.start + 1, Vec::new(), &[], 0);
    chain.fund(legacy(&args.wallet).script_pubkey(), 50_000_000);
    chain.fund(segwit(&args.wallet).script_pubkey(), 50_000_000);
    while chain.tip().height < args.start + 12 {
        let height = chain.tip().height + 1;
        chain.mine(height, Vec::new(), &[], 0);
    }
    let tip = chain.tip().height;

    let runtime = tokio::runtime::Runtime::new().expect("a runtime");
    runtime.block_on(async move {
        let chain = Arc::new(Mutex::new(chain));
        let p2p = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut info = info;
        info["p2p"] = p2p.local_addr().unwrap().port().into();
        info["http"] = http.local_addr().unwrap().port().into();
        info["tip"] = tip.into();
        println!("{info}");
        let magic = if network == NetworkKind::Main {
            [0xfb, 0xc0, 0xb6, 0xdb]
        } else {
            [0xfd, 0xd2, 0xc8, 0xf1]
        };
        tokio::join!(
            p2p::serve(p2p, chain.clone(), magic),
            http::serve(http, chain),
        );
    });
}
