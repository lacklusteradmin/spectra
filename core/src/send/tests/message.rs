//! Message signatures against each network's own SDK
//! (`message-signatures.json`, from scripts/generate-message-signature-vectors.cjs)
//! and BIP-322's own test vectors.

use super::*;

fn fixtures() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/message-signatures.json"
    ))
    .unwrap()
}

fn text(value: &serde_json::Value, key: &str) -> String {
    value[key].as_str().unwrap().to_string()
}

fn chain_of(vector: &serde_json::Value, default: Chain) -> Chain {
    vector
        .get("chain")
        .and_then(|chain| chain.as_str())
        .map(|chain| Chain::from_str_id(chain).unwrap())
        .unwrap_or(default)
}

/// Where the scheme is deterministic, the signature is the SDK's, byte for
/// byte; where it is randomized (Taproot's and sr25519's), each side's
/// signature verifies under the other's.
#[test]
fn signatures_are_the_ones_each_networks_sdk_makes() {
    let fixtures = fixtures();
    let groups = [
        ("legacy", Chain::Bitcoin, MessageScheme::SignedMessage, true),
        ("evm", Chain::Ethereum, MessageScheme::PersonalSign, true),
        ("tron", Chain::Tron, MessageScheme::TronSignedMessage, true),
        ("sui", Chain::Sui, MessageScheme::SuiPersonalMessage, true),
        (
            "substrate",
            Chain::Polkadot,
            MessageScheme::SubstrateBytes,
            false,
        ),
    ];
    for (group, default, scheme, deterministic) in groups {
        for vector in fixtures[group].as_array().unwrap() {
            let chain = chain_of(vector, default);
            let address = text(vector, "address");
            let message = text(vector, "message");
            let signature = text(vector, "signature");
            assert_eq!(
                scheme_for(chain, &address),
                Some(scheme),
                "{chain} {address}"
            );
            assert!(
                verify_message(chain, address.clone(), message.clone(), signature.clone()),
                "{chain} verifies the SDK's signature over {message:?}"
            );
            let (signed_as, ours) =
                sign_message(chain, &address, &text(vector, "key"), &message).unwrap();
            assert_eq!(signed_as, scheme);
            if deterministic {
                assert_eq!(
                    ours.to_lowercase(),
                    signature.to_lowercase(),
                    "{chain} {message:?}"
                );
            }
            assert!(verify_message(chain, address, message, ours), "{chain}");
        }
    }
}

#[test]
fn solana_signs_the_message_bytes() {
    for vector in fixtures()["solana"].as_array().unwrap() {
        let public = hex::decode(text(vector, "publicKey")).unwrap();
        let address = bs58::encode(&public).into_string();
        let message = text(vector, "message");
        let (_, ours) =
            sign_message(Chain::Solana, &address, &text(vector, "key"), &message).unwrap();
        assert_eq!(
            hex::encode(bs58::decode(&ours).into_vec().unwrap()),
            text(vector, "signature")
        );
        assert!(verify_message(Chain::Solana, address, message, ours));
    }
}

/// BIP-322 simple signatures: native and nested SegWit exactly as bip322-js
/// writes them, Taproot verified both ways.
#[test]
fn bip322_matches_bip322_js() {
    for vector in fixtures()["bip322"].as_array().unwrap() {
        let address = text(vector, "address");
        let message = text(vector, "message");
        let signature = text(vector, "signature");
        assert_eq!(
            scheme_for(Chain::Bitcoin, &address),
            Some(MessageScheme::Bip322)
        );
        assert!(
            verify_message(
                Chain::Bitcoin,
                address.clone(),
                message.clone(),
                signature.clone()
            ),
            "{address} {message:?}"
        );
        let (_, ours) =
            sign_message(Chain::Bitcoin, &address, &text(vector, "key"), &message).unwrap();
        if text(vector, "script") != "taproot" {
            assert_eq!(ours, signature, "{address} {message:?}");
        }
        assert!(verify_message(Chain::Bitcoin, address, message, ours));
    }
}

/// The test vectors in BIP-322 itself.
#[test]
fn bip322s_own_vectors_verify() {
    let address = "bc1q9vza2e8x573nczrlzms0wvx3gsqjx7vavgkx0l";
    for (message, signature) in [
        (
            "",
            "AkcwRAIgM2gBAQqvZX15ZiysmKmQpDrG83avLIT492QBzLnQIxYCIBaTpOaD20qRlEylyxFSeEA2ba9YOixpX8z46TSDtS40ASECx/EgAxlkQpQ9hYjgGu6EBCPMVPwVIVJqO4XCsMvViHI=",
        ),
        (
            "Hello World",
            "AkcwRAIgZRfIY3p7/DoVTty6YZbWS71bc5Vct9p9Fia83eRmw2QCICK/ENGfwLtptFluMGs2KsqoNSk89pO7F29zJLUx9a/sASECx/EgAxlkQpQ9hYjgGu6EBCPMVPwVIVJqO4XCsMvViHI=",
        ),
    ] {
        assert!(
            verify_message(
                Chain::Bitcoin,
                address.into(),
                message.into(),
                signature.into()
            ),
            "{message:?}"
        );
        assert!(!verify_message(
            Chain::Bitcoin,
            address.into(),
            format!("{message}!"),
            signature.into()
        ));
    }
}

/// A signature says nothing about another message, another address or
/// another network.
#[test]
fn a_signature_does_not_cover_anything_else() {
    let fixtures = fixtures();
    for (group, chain) in [
        ("legacy", Chain::Bitcoin),
        ("evm", Chain::Ethereum),
        ("sui", Chain::Sui),
    ] {
        let vector = &fixtures[group][1];
        let chain = chain_of(vector, chain);
        let address = text(vector, "address");
        let signature = text(vector, "signature");
        assert!(!verify_message(
            chain,
            address.clone(),
            "Hello world".into(),
            signature.clone()
        ));
        assert!(!verify_message(
            chain,
            address,
            text(vector, "message"),
            "not a signature".into()
        ));
    }
    // Litecoin's magic is not Bitcoin's.
    let litecoin = fixtures["legacy"]
        .as_array()
        .unwrap()
        .iter()
        .find(|vector| vector["chain"] == "litecoin" && vector["message"] == "Hello World")
        .unwrap();
    let bitcoin_signature = sign_message(
        Chain::Bitcoin,
        &text(&fixtures["legacy"][1], "address"),
        &text(litecoin, "key"),
        "Hello World",
    )
    .unwrap()
    .1;
    assert!(!verify_message(
        Chain::Litecoin,
        text(litecoin, "address"),
        "Hello World".into(),
        bitcoin_signature
    ));
}

/// What is not a plain message is refused, and networks without a
/// message standard have no scheme.
#[test]
fn what_is_not_a_message_is_refused() {
    let key = "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
    let typed = r#"{"domain":{"name":"USD Coin","chainId":1},"types":{"Permit":[]},"primaryType":"Permit","message":{"spender":"0x0","value":"1"}}"#;
    let evm = "0x2c7536e3605d9c16a7a3d7b1898e529396a65c23";
    assert!(sign_message(Chain::Ethereum, evm, key, typed).is_err());
    assert!(sign_message(Chain::Ethereum, evm, key, "a plain message").is_ok());
    // A legacy transaction message: one signer, one key, a blockhash and
    // no instructions.
    let mut transaction = vec![1, 0, 0, 1];
    transaction.extend([7; 32]);
    transaction.extend([9; 32]);
    transaction.push(0);
    let transaction = String::from_utf8(transaction).unwrap();
    assert!(is_solana_transaction(transaction.as_bytes()));
    let solana = "BLeUXTx9thHGT7VJUtF9vHEmfMDgW1nnKZ9UVer2CoLX";
    assert!(sign_message(Chain::Solana, solana, key, &transaction).is_err());
    assert!(!is_solana_transaction(b"Hello World"));
    for (chain, address) in [
        (
            Chain::Aptos,
            "0x2c7536e3605d9c16a7a3d7b1898e529396a65c232c7536e3605d9c16a7a3d7b1",
        ),
        (
            Chain::Litecoin,
            "ltc1qjmxnz78nmc8nq77wuxh25n2es7rzm5c2rkk4wh",
        ),
    ] {
        assert_eq!(scheme_for(chain, address), None, "{chain}");
        assert!(sign_message(chain, address, key, "hello").is_err());
    }
}

/// SEP-53 exactly as the Stellar SDK for Python signs it.
#[test]
fn stellar_signs_as_sep_53() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/message-signatures-stellar.json"
    ))
    .unwrap();
    for vector in fixture["stellar"].as_array().unwrap() {
        let (address, message) = (text(vector, "address"), text(vector, "message"));
        assert_eq!(
            scheme_for(Chain::Stellar, &address),
            Some(MessageScheme::StellarSignedMessage)
        );
        let (_, ours) =
            sign_message(Chain::Stellar, &address, &text(vector, "key"), &message).unwrap();
        assert_eq!(ours, text(vector, "signature"), "{message:?}");
        assert!(verify_message(
            Chain::Stellar,
            address.clone(),
            message.clone(),
            ours
        ));
        assert!(!verify_message(
            Chain::Stellar,
            address,
            format!("{message}!"),
            text(vector, "signature")
        ));
    }
}

/// Cardano's CIP-8 data signature byte for byte as
/// @emurgo/cardano-message-signing builds it; Kaspa's and Monero's
/// randomized signatures each verify under the other side.
#[test]
fn cardano_kaspa_and_monero_sign_as_their_sdks() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/message-signatures-more.json"
    ))
    .unwrap();
    for (group, chain, scheme, deterministic) in [
        (
            "cardano",
            Chain::Cardano,
            MessageScheme::CardanoDataSignature,
            true,
        ),
        (
            "kaspa",
            Chain::Kaspa,
            MessageScheme::KaspaPersonalMessage,
            false,
        ),
        (
            "monero",
            Chain::Monero,
            MessageScheme::MoneroSignature,
            false,
        ),
    ] {
        for vector in fixture[group].as_array().unwrap() {
            let (address, message, signature) = (
                text(vector, "address"),
                text(vector, "message"),
                text(vector, "signature"),
            );
            assert_eq!(scheme_for(chain, &address), Some(scheme), "{chain}");
            assert!(
                verify_message(chain, address.clone(), message.clone(), signature.clone()),
                "{chain} verifies the SDK's signature over {message:?}"
            );
            let (_, ours) = sign_message(chain, &address, &text(vector, "key"), &message).unwrap();
            if deterministic {
                assert_eq!(ours, signature, "{chain} {message:?}");
            }
            assert!(
                verify_message(chain, address.clone(), message.clone(), ours),
                "{chain}"
            );
            assert!(
                !verify_message(chain, address, format!("{message}!"), signature),
                "{chain} binds the message"
            );
        }
    }
    // A key that is not the address's is refused, not signed with.
    let cardano = &fixture["cardano"][0];
    let monero = &fixture["monero"][0];
    let mut other = hex::decode(text(cardano, "key")).unwrap();
    other[1] ^= 1;
    assert!(
        sign_message(
            Chain::Cardano,
            &text(cardano, "address"),
            &hex::encode(other),
            "x"
        )
        .is_err()
    );
    assert!(
        sign_message(
            Chain::Monero,
            &text(monero, "address"),
            &"11".repeat(64),
            "x"
        )
        .is_err()
    );
}
