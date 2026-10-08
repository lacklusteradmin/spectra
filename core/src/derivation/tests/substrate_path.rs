//! Substrate junction paths against polkadot.js (`substrate-paths.json`,
//! from scripts/generate-substrate-path-vectors.cjs).

use super::*;

fn fixtures() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/substrate-paths.json")).unwrap()
}

fn text<'a>(value: &'a serde_json::Value, key: &str) -> &'a str {
    value[key].as_str().unwrap()
}

/// Every path derives polkadot.js's key: its public key, and when every
/// junction is hard the whole secret key, which the derived seed expands
/// to; after a soft junction the scalar half, and no seed.
#[test]
fn paths_derive_polkadot_js_keys() {
    let fixtures = fixtures();
    for vector in fixtures["vectors"].as_array().unwrap() {
        let path = text(vector, "path");
        let seed = crate::derivation::primitives::derive_substrate_mini_secret(
            text(&fixtures, "phrase"),
            text(vector, "passphrase"),
            None,
            None,
            0,
        )
        .unwrap();
        let junctions = parse(path).unwrap();
        let (secret, derived_seed) =
            derive(&seed, schnorrkel::ExpansionMode::Ed25519, &junctions).unwrap();
        assert_eq!(
            hex::encode(secret.to_public().to_bytes()),
            text(vector, "publicKey"),
            "{path:?}"
        );
        let expected = hex::decode(text(vector, "secretKey")).unwrap();
        let ed25519 = secret.to_ed25519_bytes();
        if vector["hardOnly"].as_bool().unwrap() {
            assert_eq!(ed25519.to_vec(), expected, "{path:?}");
            let derived_seed = derived_seed.expect("a hard path has a seed");
            let expanded = schnorrkel::MiniSecretKey::from_bytes(&*derived_seed)
                .unwrap()
                .expand(schnorrkel::ExpansionMode::Ed25519);
            assert_eq!(expanded.to_ed25519_bytes(), ed25519, "{path:?}");
        } else {
            assert_eq!(ed25519[..32].to_vec(), expected, "{path:?}");
            assert!(derived_seed.is_none(), "{path:?}");
        }
    }
}

/// The signing key the derivation hands out signs as the account: a seed
/// on a hard path, the expanded key after a soft junction.
#[test]
fn derived_signing_keys_sign_as_their_account() {
    let fixtures = fixtures();
    for vector in fixtures["vectors"].as_array().unwrap() {
        let path = text(vector, "path");
        let (key, public) = crate::derivation::primitives::derive_substrate_sr25519_material(
            text(&fixtures, "phrase"),
            text(vector, "passphrase"),
            None,
            None,
            0,
            Some(path),
            false,
        )
        .unwrap();
        assert_eq!(hex::encode(public), text(vector, "publicKey"), "{path:?}");
        let hard_only = vector["hardOnly"].as_bool().unwrap();
        assert_eq!(key.len(), if hard_only { 32 } else { 64 }, "{path:?}");
        let pair = signing_keypair(&key, &public).unwrap();
        let signature = pair.sign_simple(b"substrate", b"message");
        schnorrkel::PublicKey::from_bytes(&public)
            .unwrap()
            .verify_simple(b"substrate", b"message", &signature)
            .unwrap();
        let mut other = public;
        other[0] ^= 1;
        assert!(signing_keypair(&key, &other).is_err(), "{path:?}");
    }
}

#[test]
fn a_path_is_hard_and_soft_junctions_only() {
    for refused in [
        "polkadot",
        "/",
        "//",
        "//polkadot/",
        "//a//",
        "///password",
        "//polkadot///password",
        // polkadot.js reads these as bytes and as a 256-bit number; sp-core
        // reads both as text.
        "//0x1234",
        "//0x",
        "//18446744073709551616",
    ] {
        assert!(parse(refused).is_err(), "{refused:?}");
    }
    assert_eq!(parse("").unwrap(), Vec::new());
    assert_eq!(parse("  ").unwrap(), Vec::new());
    assert_eq!(
        normalized(" //polkadot/0 ").unwrap().as_deref(),
        Some("//polkadot/0")
    );
    assert_eq!(normalized("").unwrap(), None);
    // An odd count of hex digits is text to both.
    assert!(parse("//0xabc").is_ok());
    let mut seven = [0u8; 32];
    seven[0] = 7;
    assert_eq!(parse("//007").unwrap(), vec![Junction::Hard(seven)]);
    assert_eq!(parse("/7").unwrap(), vec![Junction::Soft(seven)]);
}
