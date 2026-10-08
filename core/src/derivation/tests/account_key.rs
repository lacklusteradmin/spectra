//! Account public keys against the bip32 library's encodings
//! (`account-keys.json`) and independent addresses (`derivation-profiles.json`).

use super::*;

fn fixture(name: &str) -> serde_json::Value {
    serde_json::from_str(
        &std::fs::read_to_string(format!(
            "{}/tests/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap()
}

/// Every encoding a network reads is the library's, and its first receive
/// address the profile's address at account 0; every encoding the library
/// writes for a network is one the network reads.
#[test]
fn every_networks_account_keys_watch_its_profile_addresses() {
    let keys = fixture("account-keys.json");
    let profiles = fixture("derivation-profiles.json");
    let address = |chain: &str, profile: &str| {
        profiles["vectors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["chain"] == chain && v["profile"] == profile && v["account"] == 0)
            .unwrap_or_else(|| panic!("{chain} {profile}"))["address"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let mut read = std::collections::HashSet::new();
    for vector in keys["vectors"].as_array().unwrap() {
        let chain = Chain::from_str_id(vector["chain"].as_str().unwrap()).unwrap();
        let xpub = vector["xpub"].as_str().unwrap();
        let account = parse(chain, &format!("  {xpub}  ")).unwrap();
        assert_eq!(account.version.prefix, vector["prefix"].as_str().unwrap());
        assert_eq!(
            first_receive_address(chain, xpub).unwrap(),
            address(
                vector["chain"].as_str().unwrap(),
                vector["profile"].as_str().unwrap()
            ),
            "{chain} {}",
            account.version.prefix
        );
        read.insert((chain, account.version.prefix));
    }
    for chain in Chain::all() {
        for version in chain.account_key_versions() {
            assert!(
                read.contains(&(chain, version.prefix)),
                "{chain} {}",
                version.prefix
            );
        }
    }
}

/// A key of the family's other network, another network's encoding, a key
/// off an account's depth or index, and a corrupted key are refused; the
/// same account in two encodings is one account.
#[test]
fn a_key_is_read_only_as_its_networks_account() {
    let keys = fixture("account-keys.json");
    let key = |chain: &str, prefix: &str| {
        keys["vectors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["chain"] == chain && v["prefix"] == prefix)
            .unwrap()["xpub"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let refusal = |chain, text: &str| parse(chain, text).err().unwrap().to_string();
    assert!(refusal(Chain::Bitcoin, &key("bitcoin-testnet", "vpub")).contains("different network"));
    assert!(refusal(Chain::BitcoinTestnet, &key("bitcoin", "zpub")).contains("different network"));
    assert!(refusal(Chain::KaspaTestnet, &key("kaspa", "kpub")).contains("different network"));
    // Another network's encoding, and a SegWit key on a network that signs
    // only P2PKH.
    for (chain, text) in [
        (Chain::Dogecoin, key("litecoin", "Ltub")),
        (Chain::Decred, key("bitcoin", "xpub")),
        (Chain::BitcoinGold, key("bitcoin", "zpub")),
        (Chain::Ethereum, key("bitcoin", "xpub")),
    ] {
        assert!(
            refusal(chain, &text).contains("Not an account public key"),
            "{chain}"
        );
    }
    let account = parse(Chain::Bitcoin, &key("bitcoin", "xpub")).unwrap();
    let secp = secp256k1::Secp256k1::new();
    let child = account.key.derive_child(&secp, 0).unwrap();
    assert!(
        refusal(
            Chain::Bitcoin,
            &child.to_xpub_string(account.version.version)
        )
        .contains("depth")
    );
    let mut unhardened = account.key.clone();
    unhardened.child_number = 0;
    assert!(
        refusal(
            Chain::Bitcoin,
            &unhardened.to_xpub_string(account.version.version)
        )
        .contains("depth")
    );
    // A bad checksum, a payload that is no public key, a short payload and
    // no key at all.
    use crate::derivation::bitcoin::{base58check_decode, base58check_encode};
    let mut corrupted = key("bitcoin", "xpub").into_bytes();
    let last = corrupted.last_mut().unwrap();
    *last = if *last == b'1' { b'2' } else { b'1' };
    let payload = base58check_decode(&key("bitcoin", "xpub")).unwrap();
    let mut no_point = payload.clone();
    no_point[45..].fill(0);
    for text in [
        String::from_utf8(corrupted).unwrap(),
        base58check_encode(&no_point),
        base58check_encode(&payload[..77]),
        "zpub-whatever".to_string(),
    ] {
        assert!(
            refusal(Chain::Bitcoin, &text).contains("Not an account public key"),
            "{text}"
        );
    }

    assert!(same_account(&key("dash", "xpub"), &key("dash", "drkp")));
    assert!(!same_account(
        &key("bitcoin", "xpub"),
        &key("bitcoin", "zpub")
    ));
}
