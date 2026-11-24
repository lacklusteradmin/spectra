use super::*;

fn vectors() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/cosmos-vectors.json")).unwrap()
}

#[test]
fn every_network_derives_the_address_cosmjs_derives() {
    let vectors = vectors();
    let mnemonic = vectors["mnemonic"].as_str().unwrap();
    let addresses = vectors["addresses"].as_array().unwrap();
    let networks: Vec<Chain> = Chain::all().filter(|chain| chain.is_cosmos()).collect();
    for chain in &networks {
        assert!(
            addresses.iter().any(|row| row["chain"] == chain.str_id()),
            "no reference for {chain}"
        );
    }
    for row in addresses {
        let chain = Chain::parse(row["chain"].as_str().unwrap()).unwrap();
        let path = row["path"].as_str().unwrap();
        let account = row["account"].as_u64().unwrap() as u32;
        let profile = chain.derivation_profiles()[0];
        assert_eq!(
            chain.derivation_profile_path(profile, account).as_deref(),
            Some(path),
            "{chain}: the catalog's path"
        );
        let derived = derive_from_seed_phrase(chain, mnemonic, path, None, Wants::ALL).unwrap();
        assert_eq!(
            derived.address.as_deref(),
            row["address"].as_str(),
            "{chain}"
        );
        assert_eq!(
            derived.public_key_hex.as_deref(),
            row["public_key"].as_str(),
            "{chain}"
        );
        let address = row["address"].as_str().unwrap();
        assert!(crate::send::flow::is_valid_send_address(
            chain,
            address.into()
        ));
        assert!(
            crate::send::flow::is_valid_send_address(chain, address.to_ascii_uppercase()),
            "{chain}: bech32 in one case"
        );
        // Another network's prefix is another network's account.
        for other in networks
            .iter()
            .filter(|other| other.cosmos().unwrap().hrp != chain.cosmos().unwrap().hrp)
        {
            assert!(
                !crate::send::flow::is_valid_send_address(*other, address.into()),
                "{chain} on {other}"
            );
        }
    }
}

#[test]
fn an_address_is_refused_unless_it_is_this_networks_bech32() {
    let chain = Chain::CosmosHub;
    let good = "cosmos19rl4cm2hmr8afy4kldpxz3fka4jguq0auqdal4";
    assert_eq!(decode_address(chain, good).unwrap().len(), 20);
    // A typo breaks the checksum; mixed case is not bech32.
    assert!(decode_address(chain, "cosmos19rl4cm2hmr8afy4kldpxz3fka4jguq0auqdal5").is_err());
    assert!(decode_address(chain, "Cosmos19rl4cm2hmr8afy4kldpxz3fka4jguq0auqdal4").is_err());
    // The same bytes under bech32m, and a 32-byte module account.
    let bytes = decode_address(chain, good).unwrap();
    let hrp = Hrp::parse("cosmos").unwrap();
    let bech32m = bech32::encode::<bech32::Bech32m>(hrp, &bytes).unwrap();
    assert!(decode_address(chain, &bech32m).is_err());
    let module = bech32::encode::<Bech32>(hrp, &[7; 32]).unwrap();
    assert_eq!(decode_address(chain, &module).unwrap().len(), 32);
    let short = bech32::encode::<Bech32>(hrp, &[7; 19]).unwrap();
    assert!(decode_address(chain, &short).is_err());
}

#[test]
fn a_raw_key_holds_the_account_its_phrase_holds() {
    let vectors = vectors();
    for transfer in vectors["transfers"].as_array().unwrap() {
        let chain = Chain::parse(transfer["chain"].as_str().unwrap()).unwrap();
        let derived = crate::derivation::dispatch::derive_from_private_key(
            chain,
            transfer["private_key"].as_str().unwrap().into(),
            true,
            true,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            derived.address.as_deref(),
            transfer["sender"].as_str(),
            "{chain}"
        );
    }
}
