use super::*;

fn vectors() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/starknet-vectors.json"
    ))
    .unwrap()
}

fn felt(text: &str) -> Felt {
    parse_felt(text).unwrap()
}

#[test]
fn every_account_is_the_one_ready_derives_from_its_phrase() {
    let vectors = vectors();
    let chain = Chain::Starknet;
    for account in vectors["accounts"].as_array().unwrap() {
        let path = account["path"].as_str().unwrap();
        let index = account["account"].as_u64().unwrap() as u32;
        let profile = chain.derivation_profiles()[0];
        assert_eq!(
            chain.derivation_profile_path(profile, index).as_deref(),
            Some(path),
            "the catalog's path"
        );
        let passphrase = account["passphrase"].as_str().filter(|p| !p.is_empty());
        let derived = derive_from_seed_phrase(
            chain,
            account["mnemonic"].as_str().unwrap(),
            path,
            passphrase,
            Wants::ALL,
        )
        .unwrap();
        let address = account["address"].as_str().unwrap();
        assert_eq!(
            derived.address.as_deref(),
            Some(address),
            "{path} {passphrase:?}"
        );
        assert_eq!(
            format!("0x{}", derived.public_key_hex.unwrap()),
            account["public_key"].as_str().unwrap()
        );
        assert_eq!(
            format!("0x{}", derived.private_key_hex.unwrap().as_str()),
            account["private_key"].as_str().unwrap()
        );
        assert_eq!(
            constructor_calldata(&felt(account["public_key"].as_str().unwrap()))
                .iter()
                .map(crate::api::starknet_json_rpc::rpc_felt)
                .collect::<Vec<_>>(),
            account["constructor_calldata"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        );
        let checksum = account["checksum_address"].as_str().unwrap();
        assert_eq!(checksum_address(&felt(address)), checksum);
        // Stored and sent as its padded lowercase, however it was written.
        for spelling in [
            address.to_string(),
            checksum.to_string(),
            address.to_ascii_uppercase(),
        ] {
            assert!(
                crate::send::flow::is_valid_send_address(chain, spelling.clone()),
                "{spelling}"
            );
            assert_eq!(
                crate::send::flow::normalize_address(chain, &spelling),
                address
            );
        }
    }
    // The phrase whose Ethereum key starts with a zero byte, which the
    // second tree's seed leaves out.
    assert!(
        vectors["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|account| account["ethereum_key"]
                .as_str()
                .unwrap()
                .starts_with("0x00"))
    );
}

/// argent-x's own tests (packages/extension/test/keyDerivation.test.ts):
/// the Stark keys a wallet secret holds at indices 5 and 7, and one ground
/// key.
#[test]
fn the_wallet_secret_grinds_into_argent_xs_own_keys() {
    let secret =
        hex::decode("e6904d63affe7a13cd30345b000c9b1ffc087832332d7303cf237ffda8a177d0").unwrap();
    for (index, stark_key) in [
        (
            5,
            "0x05c7c65bfda7a85af0681c85c9c440f0aa6825feef6f9c96e55fb2ce08c8d4bc",
        ),
        (
            7,
            "0x0605d5a0ece3b316f0d72221228acb7f01dcb34db74e0c02790db156741f5a86",
        ),
    ] {
        let path =
            crate::derivation::bip32::parse_path(&format!("m/44'/9004'/0'/0/{index}")).unwrap();
        let child = crate::derivation::bip32::derive(&secret, &path).unwrap();
        let key = grind_key(&child.private_key.secret_bytes());
        assert_eq!(felt_hex(&public_key(&key)), stark_key);
    }
    let seed: [u8; 32] =
        hex::decode("86F3E7293141F20A8BAFF320E8EE4ACCB9D4A4BF2B4D295E8CEE784DB46E0519")
            .unwrap()
            .try_into()
            .unwrap();
    assert_eq!(
        grind_key(&seed),
        felt("0x5c8c8683596c732541a59e03007b2d30dbbbb873556fe65b5fb63c16688f941")
    );
}

#[test]
fn a_raw_key_holds_the_account_its_phrase_holds() {
    for account in vectors()["accounts"].as_array().unwrap() {
        let key = account["private_key"].as_str().unwrap();
        for written in [
            key.to_string(),
            format!("0x{}", key[2..].trim_start_matches('0')),
        ] {
            let parsed =
                crate::derivation::key_formats::parse_private_key(Chain::Starknet, &written)
                    .unwrap();
            let derived = crate::derivation::dispatch::derive_from_private_key(
                Chain::Starknet,
                parsed.to_string(),
                true,
                true,
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                derived.address.as_deref(),
                account["address"].as_str(),
                "{written}"
            );
            // Exported as Ready writes it, which reads back as itself.
            let exported = crate::derivation::key_formats::encode_private_key(
                Chain::Starknet,
                crate::derivation::setup::WalletSecretFormat::StarknetPrivateKey,
                &parsed,
            )
            .unwrap();
            assert_eq!(exported.as_str(), key);
        }
    }
    // Zero and the curve order and above are no key; nor is a 65th digit.
    for refused in [
        "0x0",
        "0x0800000000000010ffffffffffffffffb781126dcae7b2321e66a241adc64d2f",
        "0x0800000000000011000000000000000000000000000000000000000000000000",
        &format!("0x1{}", "0".repeat(64)),
        "0xg1",
        "",
    ] {
        assert!(
            crate::derivation::key_formats::parse_private_key(Chain::Starknet, refused).is_err(),
            "{refused}"
        );
    }
}

#[test]
fn an_address_is_refused_unless_it_is_a_starknet_account() {
    let chain = Chain::Starknet;
    let vectors = vectors();
    let account = &vectors["accounts"][0];
    let address = account["address"].as_str().unwrap();
    let checksum = account["checksum_address"].as_str().unwrap();
    // Short spellings are the same account; a 64-digit exchange spelling too.
    assert_eq!(
        decode_address(chain, "0x1234").unwrap(),
        Felt::from(0x1234u64)
    );
    assert_eq!(decode_address(chain, &address[3..]).unwrap(), felt(address));
    // A checksummed address with one letter's case flipped is a typo.
    let flipped: String = checksum
        .char_indices()
        .map(|(i, c)| {
            if i == checksum.find(|c: char| c.is_ascii_uppercase()).unwrap() {
                c.to_ascii_lowercase()
            } else {
                c
            }
        })
        .collect();
    assert!(decode_address(chain, &flipped).is_err());
    for refused in [
        "0x0",
        "",
        "0x",
        "0xzz",
        // The address bound, 2^251 - 256, and the field's prime.
        "0x07ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff00",
        "0x0800000000000011000000000000000000000000000000000000000000000001",
        &format!("0x{}", "1".repeat(65)),
        // An EIP-55 address: its mixed case is not the Starknet checksum
        // (`0x…5AAEb6053f3e94C9b9A09f33669435e7Ef1bEAeD`).
        "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
    ] {
        assert!(decode_address(chain, refused).is_err(), "{refused}");
    }
    assert!(
        decode_address(
            chain,
            "0x07fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffeff"
        )
        .is_ok()
    );
    // In one case, either one, there is no checksum to read; in starknet.js's
    // checksum case it is read and holds.
    assert!(decode_address(chain, "0X5AAEB6053F3E94C9B9A09F33669435E7EF1BEAED").is_ok());
    assert!(decode_address(chain, "0x5AAEb6053f3e94C9b9A09f33669435e7Ef1bEAeD").is_ok());
    // Another family's question is refused, not answered.
    assert!(decode_address(Chain::Ethereum, address).is_err());
}

#[test]
fn selectors_are_the_starknet_keccak_of_their_names() {
    for (name, expected) in vectors()["selectors"].as_object().unwrap() {
        assert_eq!(rpc(&selector(name)), expected.as_str().unwrap(), "{name}");
    }
}

fn rpc(felt: &Felt) -> String {
    crate::api::starknet_json_rpc::rpc_felt(felt)
}
