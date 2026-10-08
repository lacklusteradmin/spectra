//! Subaddresses and view keys against wallet2's own
//! (`monero-subaddresses.json`, from monero-ts).

use super::*;

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/monero-subaddresses.json"
    ))
    .unwrap()
}

fn chain(network: &serde_json::Value) -> Chain {
    Chain::from_str_id(network["chain"].as_str().unwrap()).unwrap()
}

/// The phrase's scan keys are wallet2's, and every subaddress — another
/// account's, past the lookahead, past two bytes — is wallet2's.
#[test]
fn subaddresses_are_wallet2s() {
    let fixture = fixture();
    let phrase = fixture["phrase"].as_str().unwrap().to_string();
    for network in fixture["networks"].as_array().unwrap() {
        let chain = chain(network);
        let derived = if chain == Chain::Monero {
            derive_monero(phrase.clone(), false, false, true)
        } else {
            derive_monero_stagenet(phrase.clone(), false, false, true)
        }
        .unwrap();
        let keys = ViewKeys::from_private(chain, &derived.private_key_hex.unwrap()).unwrap();
        assert_eq!(
            keys.address.to_string(),
            network["primary"].as_str().unwrap()
        );
        assert_eq!(
            hex::encode(*keys.view),
            network["private_view_key"].as_str().unwrap()
        );
        for expected in network["subaddresses"].as_array().unwrap() {
            let (account, index) = (
                expected["account"].as_u64().unwrap() as u32,
                expected["address"].as_u64().unwrap() as u32,
            );
            assert_eq!(
                subaddress(&keys, account, index).unwrap(),
                expected["encoded"].as_str().unwrap(),
                "{chain} {account}/{index}"
            );
        }
    }
}

/// A primary address and its own view key are a view-only wallet; anything
/// else is refused before it is stored.
#[test]
fn a_view_key_is_read_only_with_its_own_primary_address() {
    let fixture = fixture();
    let mainnet = &fixture["networks"][0];
    let stagenet = &fixture["networks"][1];
    let (primary, view) = (
        mainnet["primary"].as_str().unwrap(),
        mainnet["private_view_key"].as_str().unwrap(),
    );
    let keys = view_keys(Chain::Monero, &format!(" {primary} "), &format!(" {view} ")).unwrap();
    assert_eq!(keys.address.to_string(), primary);
    let subaddress = mainnet["subaddresses"][1]["encoded"].as_str().unwrap();
    let mut other_view = hex::decode(view).unwrap();
    other_view[0] ^= 1;
    for (chain, address, key, refusal) in [
        (
            Chain::Monero,
            subaddress,
            view.to_string(),
            "primary address",
        ),
        (
            Chain::Monero,
            primary,
            hex::encode(other_view),
            "does not belong",
        ),
        (Chain::Monero, primary, "ff".repeat(32), "64 hex digits"),
        (Chain::Monero, primary, "zz".repeat(32), "64 hex digits"),
        (
            Chain::Monero,
            primary,
            view[2..].to_string(),
            "64 hex digits",
        ),
        (
            Chain::Monero,
            stagenet["primary"].as_str().unwrap(),
            stagenet["private_view_key"].as_str().unwrap().to_string(),
            "Not a Monero address",
        ),
        (Chain::MoneroStagenet, primary, view.to_string(), "Not a"),
        (
            Chain::Bitcoin,
            primary,
            view.to_string(),
            "not a Monero network",
        ),
    ] {
        let error = view_keys(chain, address, &key).err().unwrap().to_string();
        assert!(error.contains(refusal), "{chain} {address}: {error}");
    }
}
