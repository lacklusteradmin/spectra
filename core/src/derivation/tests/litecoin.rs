use super::*;

#[test]
fn private_key_one_uses_the_exact_litecoin_network() {
    // Generator public key and Hash160 from the Bitcoin address test vector;
    // version bytes 48/111 are Litecoin Core's mainnet/testnet chainparams.
    let key = format!("{:064x}", 1);
    for (chain, expected) in [
        (Chain::Litecoin, "LVuDpNCSSj6pQ7t9Pv6d6sUkLKoqDEVUnJ"),
        (Chain::LitecoinTestnet, "mrCDrCybB6J1vRfbwM5hemdJz73FwDBC8r"),
    ] {
        let result =
            crate::derivation::dispatch::derive_from_private_key(chain, key.clone(), true, true)
                .unwrap()
                .unwrap();
        assert_eq!(result.address.as_deref(), Some(expected));
        assert_eq!(
            result.public_key_hex.as_deref(),
            Some("0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798")
        );
        assert!(crate::send::flow::is_valid_send_address(
            chain,
            expected.to_string()
        ));
        let other = if chain.is_testnet() {
            Chain::Litecoin
        } else {
            Chain::LitecoinTestnet
        };
        assert!(!crate::send::flow::is_valid_send_address(
            other,
            expected.to_string()
        ));
    }
}

#[test]
fn litecoin_does_not_derive_an_input_script_it_cannot_sign() {
    const PHRASE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    for chain in [Chain::Litecoin, Chain::LitecoinTestnet] {
        assert!(
            derive_litecoin_on_network(
                chain,
                PHRASE.into(),
                "m/86'/2'/0'/0/0".into(),
                None,
                BitcoinScriptType::P2tr,
                false,
                true,
                true
            )
            .is_err()
        );
    }
}
