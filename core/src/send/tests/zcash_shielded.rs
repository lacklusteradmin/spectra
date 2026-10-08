use super::*;
use crate::wallet_db::zcash::open_zcash_db;
use zcash_keys::keys::{ReceiverRequirement, UnifiedAddressRequest};

fn vectors() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/zcash-addresses.json")).unwrap()
}

/// An Orchard and Sapling address, with no transparent receiver: the one
/// Spectra shows.
fn shielded_only() -> UnifiedAddressRequest {
    UnifiedAddressRequest::unsafe_custom(
        ReceiverRequirement::Require,
        ReceiverRequirement::Require,
        ReceiverRequirement::Omit,
    )
}

/// A wallet database with no account yet, in a directory of its own.
fn empty_db() -> (std::path::PathBuf, ZcashDb) {
    let directory = std::env::temp_dir().join(crate::store::new_event_id());
    let db = open_zcash_db(&directory.join("wallet.sqlite"), Network::MainNetwork).unwrap();
    (directory, db)
}

/// The keys a seed derives under ZIP-32 give the receivers the reference
/// implementation gives (zcash-test-vectors), account by account and
/// diversifier by diversifier.
#[test]
fn shielded_receivers_match_the_reference_vectors() {
    let vectors = vectors();
    let vectors = vectors["unified"]["vectors"].as_array().unwrap();
    assert_eq!(vectors.len(), 15);
    for vector in vectors {
        let seed = hex::decode(vector["root_seed"].as_str().unwrap()).unwrap();
        let account = u32::try_from(vector["account"].as_u64().unwrap()).unwrap();
        let index = u128::from(vector["diversifier_index"].as_u64().unwrap());
        let usk = UnifiedSpendingKey::from_seed(
            &Network::MainNetwork,
            &seed,
            zip32::AccountId::try_from(account).unwrap(),
        )
        .unwrap();
        let address = usk
            .to_unified_full_viewing_key()
            .address(
                zip32::DiversifierIndex::try_from(index).unwrap(),
                shielded_only(),
            )
            .unwrap();
        assert_eq!(
            hex::encode(address.sapling().unwrap().to_bytes()),
            vector["sapling_raw_addr"].as_str().unwrap(),
            "account {account} index {index}"
        );
        assert_eq!(
            hex::encode(address.orchard().unwrap().to_raw_address_bytes()),
            vector["orchard_raw_addr"].as_str().unwrap(),
            "account {account} index {index}"
        );
        assert!(address.transparent().is_none());
        // Where the reference address has no other receiver, it is this
        // one, byte for byte.
        if vector["p2pkh_bytes"].is_null() && vector["unknown_typecode"].is_null() {
            assert_eq!(
                address.encode(&Network::MainNetwork),
                vector["unified_addr"].as_str().unwrap()
            );
        }
    }
}

/// A TEX address carries the key hash of its transparent address
/// (ZIP-320's reference pair), on its own network only.
#[test]
fn a_tex_address_is_its_transparent_key_hash() {
    let vectors = vectors();
    let pair = &vectors["tex"]["vectors"][0];
    let network = Network::MainNetwork;
    let Some(Address::Tex(hash)) = Address::decode(&network, pair["tex"].as_str().unwrap()) else {
        panic!("not a TEX address");
    };
    let Some(Address::Transparent(transparent)) =
        Address::decode(&network, pair["transparent"].as_str().unwrap())
    else {
        panic!("not a transparent address");
    };
    assert_eq!(
        transparent,
        zcash_transparent::address::TransparentAddress::PublicKeyHash(hash)
    );
    assert!(Address::decode(&Network::TestNetwork, pair["tex"].as_str().unwrap()).is_none());
}

/// What is refused before any note is chosen: an address that is not one
/// on this network, a TEX address (the transparent balance pays it), a memo
/// a transparent recipient cannot read or one past 512 bytes, nothing to
/// send, and no account to send from.
#[test]
fn payments_are_refused_before_notes_are_chosen() {
    let (directory, mut db) = empty_db();
    let network = Network::MainNetwork;
    let vectors = vectors();
    let unified = vectors["unified"]["vectors"][3]["unified_addr"]
        .as_str()
        .unwrap()
        .to_string();
    let tex = vectors["tex"]["vectors"][0]["tex"].as_str().unwrap();
    let transparent = vectors["tex"]["vectors"][0]["transparent"]
        .as_str()
        .unwrap();
    let testnet_unified =
        UnifiedSpendingKey::from_seed(&Network::TestNetwork, &[7; 32], zip32::AccountId::ZERO)
            .unwrap()
            .to_unified_full_viewing_key()
            .default_address(shielded_only())
            .unwrap()
            .0
            .encode(&Network::TestNetwork);
    let mut refused = |recipient: &str, amount: u64, memo: Option<&str>| {
        propose_payment(&mut db, &network, recipient, amount, memo)
            .unwrap_err()
            .to_string()
    };
    assert_eq!(
        refused("not an address", 1, None),
        "Not a Zcash address on this network"
    );
    assert_eq!(
        refused(&testnet_unified, 1, None),
        "Not a Zcash address on this network"
    );
    assert_eq!(
        refused(tex, 1, None),
        "A TEX address is paid from the wallet's transparent balance."
    );
    assert_eq!(
        refused(transparent, 1, Some("hello")),
        "A memo reaches only a shielded recipient."
    );
    assert_eq!(
        refused(&unified, 1, Some(&"x".repeat(513))),
        "A memo is at most 512 bytes."
    );
    assert_eq!(refused(&unified, 0, None), "The amount must be positive.");
    // A blank memo is no memo.
    assert_eq!(
        refused(transparent, 1, Some("  ")),
        "Sync the shielded wallet before sending from it."
    );
    assert_eq!(
        refused(&unified, 1, Some(&"x".repeat(512))),
        "Sync the shielded wallet before sending from it."
    );
    assert_eq!(
        propose_shielding_all(&mut db, &network)
            .unwrap_err()
            .to_string(),
        "Sync the shielded wallet before sending from it."
    );
    std::fs::remove_dir_all(directory).unwrap();
}

/// A parameter file is used only when it is exactly the published one: its
/// pinned length and BLAKE2b-512 hash.
#[test]
fn only_the_published_sapling_parameters_are_used() {
    let (name, _, size) = SAPLING_PARAMETER_FILES[1];
    assert_eq!(name, "sapling-output.params");
    assert!(!sapling_parameter_is_genuine(name, &vec![0; size]));
    assert!(!sapling_parameter_is_genuine(name, &vec![0; size - 1]));
    assert!(!sapling_parameter_is_genuine("sapling-other.params", &[]));
    // Each pinned hash is one BLAKE2b-512 digest.
    for (_, hash, _) in SAPLING_PARAMETER_FILES {
        assert_eq!(hex::decode(hash).unwrap().len(), 64);
    }
}

/// A reviewed proposal that no longer decodes is not signed.
#[test]
fn an_undecodable_proposal_is_not_signed() {
    let (directory, mut db) = empty_db();
    let prepared = PreparedZcashShielded {
        proposal_hex: "ff".into(),
        payments: Vec::new(),
        fee_zat: 10_000,
        transparent_in_zat: 0,
        shielded_in_zat: 0,
        change_zat: 0,
        spends_sapling: false,
        uses_sapling: false,
    };
    let error = sign(&mut db, &Network::MainNetwork, &[0; 32], 0, &prepared, None).unwrap_err();
    assert_eq!(error.to_string(), "The reviewed proposal no longer decodes");
    std::fs::remove_dir_all(directory).unwrap();
}
