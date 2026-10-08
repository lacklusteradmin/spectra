//! Multisig accounts against bitcoinjs-lib's (`multisig-psbt.json`) and the
//! descriptor checksum against BIP-380's own vector.

use super::*;

pub(crate) fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/multisig-psbt.json")).unwrap()
}

/// A network's 2-of-3 as a descriptor with `/<0;1>/*` keys, no checksum.
pub(crate) fn descriptor(network: &serde_json::Value) -> String {
    let keys: Vec<String> = network["cosigners"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            format!(
                "[{}/{}]{}/<0;1>/*",
                c["fingerprint"].as_str().unwrap(),
                c["origin"].as_str().unwrap(),
                c["xpub"].as_str().unwrap()
            )
        })
        .collect();
    format!(
        "wsh(sortedmulti({},{}))",
        network["threshold"],
        keys.join(",")
    )
}

fn chain(network: &serde_json::Value) -> Chain {
    Chain::from_str_id(network["chain"].as_str().unwrap()).unwrap()
}

#[test]
fn the_checksum_is_bip380s() {
    assert_eq!(descriptor_checksum("raw(deadbeef)").unwrap(), "89f8spxm");
    assert_eq!(descriptor_checksum("raw(deadbeef)\u{e9}"), None);
}

/// Every receive and change address is bitcoinjs-lib's, the canonical
/// descriptor reads back as the same policy, and each phrase is its
/// cosigner.
#[test]
fn a_descriptor_derives_bitcoinjs_addresses_and_finds_its_cosigners() {
    let fixture = fixture();
    for network in fixture["networks"].as_array().unwrap() {
        let chain = chain(network);
        let policy = MultisigPolicy::parse(chain, &descriptor(network)).unwrap();
        for expected in network["addresses"].as_array().unwrap() {
            let place = (
                expected["branch"].as_u64().unwrap() as u32,
                expected["index"].as_u64().unwrap() as u32,
            );
            assert_eq!(
                policy.address(chain, place).unwrap(),
                expected["address"].as_str().unwrap(),
                "{chain} {place:?}"
            );
        }
        let canonical = policy.descriptor();
        assert_eq!(MultisigPolicy::parse(chain, &canonical).unwrap(), policy);
        // `/0/*` keys and `'` markers are the same account.
        let legacy = descriptor(network)
            .replace("/<0;1>/*", "/0/*")
            .replace("h/", "'/")
            .replace("h]", "']");
        assert_eq!(MultisigPolicy::parse(chain, &legacy).unwrap(), policy);
        for (index, phrase) in fixture["phrases"].as_array().unwrap().iter().enumerate() {
            let (cosigner, _) = policy
                .cosigner_of_phrase(chain, phrase.as_str().unwrap(), "")
                .unwrap();
            assert_eq!(cosigner, index, "{chain}");
        }
        let error = policy
            .cosigner_of_phrase(chain, fixture["phrases"][0].as_str().unwrap(), "TREZOR")
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("none of the wallet's cosigner keys"),
            "{error}"
        );
    }
}

/// A descriptor of another network, without origins, with a key whose
/// depth is not its origin's, a threshold past its keys, a key twice, a
/// wrong checksum or another script is refused.
#[test]
fn only_a_networks_wsh_sortedmulti_with_origins_is_read() {
    let fixture = fixture();
    let mainnet = descriptor(&fixture["networks"][0]);
    let testnet = descriptor(&fixture["networks"][1]);
    let checksum = descriptor_checksum(&mainnet).unwrap();
    let first_key = mainnet.split(',').nth(1).unwrap().to_string();
    for (chain, text) in [
        (Chain::Bitcoin, testnet.clone()),
        (Chain::BitcoinTestnet, mainnet.clone()),
        (Chain::Litecoin, mainnet.clone()),
        (
            Chain::Bitcoin,
            mainnet.replace("[73c5da0a/48h/0h/0h/2h]", ""),
        ),
        (
            Chain::Bitcoin,
            mainnet.replace("73c5da0a/48h/0h/0h/2h", "73c5da0a/48h/0h/0h"),
        ),
        (
            Chain::Bitcoin,
            mainnet.replace("sortedmulti(2,", "sortedmulti(4,"),
        ),
        (
            Chain::Bitcoin,
            mainnet.replace("sortedmulti(2,", "sortedmulti(0,"),
        ),
        (
            Chain::Bitcoin,
            mainnet.replace("))", &format!(",{first_key}))")),
        ),
        (Chain::Bitcoin, format!("{mainnet}#{}", "q".repeat(8))),
        (
            Chain::Bitcoin,
            mainnet.replace("wsh(sortedmulti", "sh(sortedmulti"),
        ),
        (Chain::Bitcoin, mainnet.replace("sortedmulti", "multi")),
        (Chain::Bitcoin, mainnet.replace("/<0;1>/*", "/1/*")),
    ] {
        assert!(
            MultisigPolicy::parse(chain, &text).is_err(),
            "{chain}: {text}"
        );
    }
    assert!(MultisigPolicy::parse(Chain::Bitcoin, &format!(" {mainnet}#{checksum} ")).is_ok());
}

/// The fixture's 2-of-3 on `chain`, derived here at BIP-48's
/// `m/48'/coin'/0'/2'`: for tests of networks the fixture does not cover.
pub(crate) fn descriptor_of_phrases(chain: Chain) -> String {
    let fixture = fixture();
    let network = chain.bitcoin_network().unwrap();
    let coin = if network == bitcoin::Network::Bitcoin {
        0
    } else {
        1
    };
    let secp = Secp256k1::new();
    let keys: Vec<String> = fixture["phrases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|phrase| {
            let seed = crate::derivation::primitives::derive_bip39_seed(
                phrase.as_str().unwrap(),
                "",
                0,
                None,
                None,
            )
            .unwrap();
            let master = Xpriv::new_master(NetworkKind::from(network), seed.as_ref()).unwrap();
            let origin = DerivationPath::from_str(&format!("m/48'/{coin}'/0'/2'")).unwrap();
            let key = Xpub::from_priv(&secp, &master.derive_priv(&secp, &origin).unwrap());
            format!(
                "[{}/48h/{coin}h/0h/2h]{key}/<0;1>/*",
                master.fingerprint(&secp)
            )
        })
        .collect();
    format!("wsh(sortedmulti(2,{}))", keys.join(","))
}
