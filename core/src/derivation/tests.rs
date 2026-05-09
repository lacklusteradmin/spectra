use ed25519_dalek::SigningKey;
use crate::derivation::chains::bitcoin::{derive_bip39_seed, ExtendedPrivateKey};
use crate::derivation::chains::cardano::derive_cardano_icarus_xprv_root;
use crate::derivation::chains::monero::monero_base58_encode;
use crate::derivation::chains::polkadot::derive_substrate_sr25519_material;
use crate::derivation::chains::ton::{crc16_xmodem, derive_ton_seed, v4r2_code_hash_and_depth};
use crate::derivation::chains::aptos::derive_aptos;
use crate::derivation::chains::bitcoin::derive_bitcoin;
use crate::derivation::chains::bitcoin_cash::derive_bitcoin_cash;
use crate::derivation::chains::bitcoin_sv::derive_bitcoin_sv;
use crate::derivation::chains::dogecoin::derive_dogecoin;
use crate::derivation::chains::litecoin::derive_litecoin;
use crate::derivation::chains::cardano::derive_cardano;
use crate::derivation::chains::evm::{
    derive_arbitrum, derive_avalanche, derive_ethereum, derive_ethereum_classic,
    derive_hyperliquid, derive_optimism,
};
use crate::derivation::chains::icp::derive_icp;
use crate::derivation::chains::monero::derive_monero;
use crate::derivation::chains::near::derive_near;
use crate::derivation::chains::polkadot::derive_polkadot;
use crate::derivation::chains::solana::derive_solana;
use crate::derivation::chains::stellar::derive_stellar;
use crate::derivation::chains::sui::derive_sui;
use crate::derivation::chains::ton::derive_ton;
use crate::derivation::chains::tron::derive_tron;
use crate::derivation::types::BitcoinScriptType;
use crate::derivation::chains::xrp::derive_xrp;

const MNEMONIC: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const ALL_ALL: &str = "all all all all all all all all all all all all";
const SEP_0005: &str = "illness spike retreat truth genius clock brain pass fit cave bargain toe";

#[test]
fn derives_all_supported_chains() {
    macro_rules! ok {
        ($label:literal, $e:expr) => {
            match $e {
                Ok(r) => {
                    assert!(
                        r.address.as_deref().is_some_and(|v| !v.trim().is_empty()),
                        "missing address for {}", $label
                    );
                    assert!(
                        r.public_key_hex.as_deref().is_some_and(|v| !v.trim().is_empty()),
                        "missing public key for {}", $label
                    );
                    assert!(
                        r.private_key_hex.as_deref().is_some_and(|v| !v.trim().is_empty()),
                        "missing private key for {}", $label
                    );
                }
                Err(e) => panic!("failed to derive {}: {e:?}", $label),
            }
        };
    }
    let m = MNEMONIC.to_string();
    ok!("bitcoin",           derive_bitcoin(m.clone(), "m/84'/0'/0'/0/0".into(), None, BitcoinScriptType::P2wpkh, true, true, true));
    ok!("bitcoin_cash",      derive_bitcoin_cash(m.clone(), "m/44'/145'/0'/0/0".into(), None, BitcoinScriptType::P2pkh, true, true, true));
    ok!("bitcoin_sv",        derive_bitcoin_sv(m.clone(), "m/44'/236'/0'/0/0".into(), None, BitcoinScriptType::P2pkh, true, true, true));
    ok!("litecoin",          derive_litecoin(m.clone(), "m/44'/2'/0'/0/0".into(), None, BitcoinScriptType::P2pkh, true, true, true));
    ok!("dogecoin",          derive_dogecoin(m.clone(), "m/44'/3'/0'/0/0".into(), None, BitcoinScriptType::P2pkh, true, true, true));
    ok!("ethereum",          derive_ethereum(m.clone(), "m/44'/60'/0'/0/0".into(), None, true, true, true));
    ok!("ethereum_classic",  derive_ethereum_classic(m.clone(), "m/44'/61'/0'/0/0".into(), None, true, true, true));
    ok!("arbitrum",          derive_arbitrum(m.clone(), "m/44'/60'/0'/0/0".into(), None, true, true, true));
    ok!("optimism",          derive_optimism(m.clone(), "m/44'/60'/0'/0/0".into(), None, true, true, true));
    ok!("avalanche",         derive_avalanche(m.clone(), "m/44'/60'/0'/0/0".into(), None, true, true, true));
    ok!("hyperliquid",       derive_hyperliquid(m.clone(), "m/44'/60'/0'/0/0".into(), None, true, true, true));
    ok!("tron",              derive_tron(m.clone(), "m/44'/195'/0'/0/0".into(), None, true, true, true));
    ok!("xrp",               derive_xrp(m.clone(), "m/44'/144'/0'/0/0".into(), None, true, true, true));
    ok!("solana",            derive_solana(m.clone(), "m/44'/501'/0'/0'".into(), None, None, true, true, true));
    ok!("stellar",           derive_stellar(m.clone(), "m/44'/148'/0'".into(), None, None, true, true, true));
    ok!("cardano",           derive_cardano(m.clone(), Some("m/1852'/1815'/0'/0/0".into()), None, true, true, true));
    ok!("sui",               derive_sui(m.clone(), "m/44'/784'/0'/0'/0'".into(), None, true, true, true));
    ok!("aptos",             derive_aptos(m.clone(), "m/44'/637'/0'/0'/0'".into(), None, true, true, true));
    ok!("ton",               derive_ton(m.clone(), None, true, true, true));
    ok!("internet_computer", derive_icp(m.clone(), "m/44'/223'/0'/0'/0'".into(), None, true, true, true));
    ok!("near",              derive_near(m.clone(), None, true, true, true));
    ok!("polkadot",          derive_polkadot(m.clone(), None, None, true, true, true));
    ok!("monero",            derive_monero(m.clone(), true, true, true));
}

#[test]
fn custom_hmac_key_changes_secp_derivation() {
    // Different HMAC keys produce different BIP-32 master keys from the same seed.
    let seed = derive_bip39_seed(MNEMONIC, "", 0, None, None).unwrap();
    let btc = ExtendedPrivateKey::master_from_seed(b"Bitcoin seed", seed.as_ref())
        .unwrap().private_key.secret_bytes();
    let nostr = ExtendedPrivateKey::master_from_seed(b"Nostr seed", seed.as_ref())
        .unwrap().private_key.secret_bytes();
    assert_ne!(btc, nostr);
}

#[test]
fn custom_hmac_key_changes_slip10_derivation() {
    let path = "m/44'/501'/0'/0'".to_string();
    let baseline = derive_solana(MNEMONIC.into(), path.clone(), None, None, false, false, true)
        .expect("baseline slip10").private_key_hex;
    let tweaked = derive_solana(MNEMONIC.into(), path, None, Some("custom ed25519 seed".into()), false, false, true)
        .expect("tweaked slip10").private_key_hex;
    assert_ne!(baseline, tweaked);
}

#[test]
fn custom_salt_prefix_changes_seed() {
    let seed_default = derive_bip39_seed(MNEMONIC, "", 0, None, None).unwrap();
    let seed_electrum = derive_bip39_seed(MNEMONIC, "", 0, None, Some("electrum")).unwrap();
    assert_ne!(&seed_default[..], &seed_electrum[..]);
}

#[test]
fn custom_iteration_count_changes_seed() {
    let seed_default = derive_bip39_seed(MNEMONIC, "", 0, None, None).unwrap();
    let seed_custom = derive_bip39_seed(MNEMONIC, "", 4096, None, None).unwrap();
    assert_ne!(&seed_default[..], &seed_custom[..]);
}

#[test]
fn default_hmac_key_matches_standard_seed() {
    // Explicitly passing "Bitcoin seed" must match the hardcoded default path.
    let seed = derive_bip39_seed(MNEMONIC, "", 0, None, None).unwrap();
    let k1 = ExtendedPrivateKey::master_from_seed(b"Bitcoin seed", seed.as_ref())
        .unwrap().private_key.secret_bytes();
    let k2 = ExtendedPrivateKey::master_from_seed(b"Bitcoin seed", seed.as_ref())
        .unwrap().private_key.secret_bytes();
    assert_eq!(k1, k2);

    // For SLIP-10: hmac_key=None and hmac_key=Some("ed25519 seed") are equivalent.
    let path = "m/44'/501'/0'/0'".to_string();
    let sol_none = derive_solana(MNEMONIC.into(), path.clone(), None, None, false, false, true)
        .expect("solana none").private_key_hex;
    let sol_explicit = derive_solana(MNEMONIC.into(), path, None, Some("ed25519 seed".into()), false, false, true)
        .expect("solana explicit").private_key_hex;
    assert_eq!(sol_none, sol_explicit);
}

#[test]
fn unknown_wordlist_is_rejected() {
    let err = derive_bip39_seed(MNEMONIC, "", 0, Some("klingon"), None)
        .expect_err("klingon wordlist should not resolve");
    assert!(err.to_lowercase().contains("wordlist"), "got: {err}");
}

#[test]
fn near_direct_seed_vector() {
    // NEAR uses the MyNearWallet / near-seed-phrase convention:
    // priv = BIP-39 PBKDF2 seed[0..32]. For "abandon abandon … about"
    // with EMPTY passphrase, the 64-byte seed begins with the publicly
    // documented constant 5eb00bbd…
    let result = derive_near(MNEMONIC.into(), None, true, true, true)
        .expect("near direct-seed derive");
    let priv_hex = result.private_key_hex.expect("near priv");
    let pub_hex = result.public_key_hex.expect("near pub");
    let address = result.address.expect("near address");

    assert_eq!(priv_hex, "5eb00bbddcf069084889a8ab9155568165f5c453ccb85e70811aaed6f6da5fc1");

    let priv_bytes = hex::decode(&priv_hex).expect("decode priv");
    let mut priv_arr = [0u8; 32];
    priv_arr.copy_from_slice(&priv_bytes);
    let expected_pub = hex::encode(SigningKey::from_bytes(&priv_arr).verifying_key().to_bytes());
    assert_eq!(pub_hex, expected_pub);
    // NEAR implicit account id = hex(public_key).
    assert_eq!(address, expected_pub);
}

#[test]
fn ton_mnemonic_structure() {
    // TON mnemonic scheme: entropy = HMAC-SHA512(mnemonic, passphrase);
    // seed = PBKDF2(entropy, "TON default seed", 100_000, 64); priv = seed[0..32].
    // derive_ton always returns V4R2 bounceable mainnet format.
    let result = derive_ton(MNEMONIC.into(), None, true, true, true)
        .expect("ton derive");
    let priv_hex = result.private_key_hex.expect("ton priv");
    let pub_hex = result.public_key_hex.expect("ton pub");
    let address = result.address.expect("ton address");

    assert_eq!(priv_hex.len(), 64);
    let priv_bytes = hex::decode(&priv_hex).expect("decode priv");
    let mut priv_arr = [0u8; 32];
    priv_arr.copy_from_slice(&priv_bytes);
    let expected_pub = hex::encode(SigningKey::from_bytes(&priv_arr).verifying_key().to_bytes());
    assert_eq!(pub_hex, expected_pub);
    // V4R2 bounceable mainnet: "EQ" prefix, exactly 48 base64url chars.
    assert!(address.starts_with("EQ"), "expected V4R2 bounceable address, got: {address}");
    assert_eq!(address.len(), 48, "V4R2 must be 48 chars: {address}");
}

#[test]
fn ton_mnemonic_diverges_from_slip10() {
    // TON uses PBKDF2-based seed derivation, not SLIP-10.
    let ton_priv = derive_ton(MNEMONIC.into(), None, false, false, true)
        .expect("ton priv").private_key_hex;
    // Solana at the TON coin-type path uses SLIP-10 — result must differ.
    let slip10_priv = derive_solana(MNEMONIC.into(), "m/44'/607'/0'".into(), None, None, false, false, true)
        .expect("slip10 priv").private_key_hex;
    assert_ne!(ton_priv, slip10_priv);
}

#[test]
fn ton_mnemonic_honors_iteration_count() {
    // Default (100_000 when 0 is passed) vs explicit 50_000 PBKDF2 iterations.
    let seed_default = derive_ton_seed(MNEMONIC, "", None, 0).unwrap();
    let seed_custom = derive_ton_seed(MNEMONIC, "", None, 50_000).unwrap();
    assert_ne!(&seed_default[..32], &seed_custom[..32]);
}

#[test]
fn cardano_icarus_enterprise_address_structure() {
    // CIP-1852 / CIP-3 Icarus + CIP-19 Shelley enterprise address:
    // header byte is 0x61 (type 6 = enterprise + payment-key hash, network 1 = mainnet).
    // Bech32 HRP = "addr"; payload is 29 bytes (1 header + 28 Blake2b-224).
    let result = derive_cardano(
        MNEMONIC.into(), Some("m/1852'/1815'/0'/0/0".into()), None, true, true, true,
    ).expect("cardano derive");
    let priv_hex = result.private_key_hex.expect("cardano priv");
    let pub_hex = result.public_key_hex.expect("cardano pub");
    let address = result.address.expect("cardano address");

    assert_eq!(priv_hex.len(), 64);
    assert_eq!(pub_hex.len(), 64);
    assert!(address.starts_with("addr1"), "address: {address}");
    let (hrp, data) = bech32::decode(&address).expect("bech32 decode must succeed");
    assert_eq!(hrp.as_str(), "addr");
    assert_eq!(data.len(), 29);
    assert_eq!(data[0], 0x61);
}

#[test]
fn cardano_icarus_diverges_from_slip10() {
    // Icarus BIP-32-Ed25519 with Khovratovich-Law clamping produces a
    // different scalar than SLIP-10 ed25519 for the same phrase.
    let icarus_priv = derive_cardano(
        MNEMONIC.into(), Some("m/1852'/1815'/0'/0/0".into()), None, false, false, true,
    ).expect("icarus").private_key_hex;
    // SLIP-10 ed25519 via Solana at a comparable path (all-hardened as required).
    let slip10_priv = derive_solana(
        MNEMONIC.into(), "m/1852'/1815'/0'/0'/0'".into(), None, None, false, false, true,
    ).expect("slip10").private_key_hex;
    assert_ne!(icarus_priv, slip10_priv);
}

#[test]
fn cardano_icarus_passphrase_changes_root() {
    let baseline = derive_cardano(
        MNEMONIC.into(), Some("m/1852'/1815'/0'/0/0".into()), None, false, false, true,
    ).expect("baseline cardano").private_key_hex;
    let tweaked = derive_cardano(
        MNEMONIC.into(), Some("m/1852'/1815'/0'/0/0".into()), Some("TREZOR".into()), false, false, true,
    ).expect("tweaked cardano").private_key_hex;
    assert_ne!(baseline, tweaked);
}

#[test]
fn cardano_icarus_root_is_clamped() {
    // Khovratovich-Law clamping on the root xprv: kL[31] top 3 bits are
    // cleared then bit 6 is set — so byte 31 & 0b1110_0000 == 0b0100_0000.
    let root = derive_cardano_icarus_xprv_root(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        "",
        Some("english"),
        0,
    ).expect("icarus root");
    assert_eq!(root[0] & 0b0000_0111, 0, "kL[0] low 3 bits must be clear");
    assert_eq!(root[31] & 0b1110_0000, 0b0100_0000, "kL[31] top 3 bits must be 010");
}

#[test]
fn polkadot_substrate_address_structure() {
    // SS58 Polkadot mainnet (network prefix 0): 1-byte prefix + 32-byte
    // pubkey + 2-byte Blake2b-512("SS58PRE"||…) checksum, base58-encoded.
    let result = derive_polkadot(MNEMONIC.into(), None, None, true, true, true)
        .expect("polkadot derive");
    let priv_hex = result.private_key_hex.expect("polkadot mini-secret");
    let pub_hex = result.public_key_hex.expect("polkadot pub");
    let address = result.address.expect("polkadot address");

    assert_eq!(priv_hex.len(), 64, "mini-secret = 32 bytes");
    assert_eq!(pub_hex.len(), 64, "sr25519 pub = 32 bytes");
    assert_ne!(priv_hex, pub_hex, "mini-secret must differ from pub");
    assert!(address.starts_with('1'), "Polkadot mainnet starts with '1', got {address}");
    assert!(
        (47..=48).contains(&address.len()),
        "polkadot address length out of range: {} ({address})",
        address.len()
    );
}

#[test]
fn polkadot_substrate_passphrase_changes_mini_secret() {
    let baseline = derive_polkadot(MNEMONIC.into(), None, None, false, false, true)
        .expect("baseline polkadot").private_key_hex;
    let tweaked = derive_polkadot(MNEMONIC.into(), Some("TREZOR".into()), None, false, false, true)
        .expect("tweaked polkadot").private_key_hex;
    assert_ne!(baseline, tweaked);
}

#[test]
fn polkadot_substrate_diverges_from_bip39_seed_prefix() {
    // substrate-bip39 uses the BIP-39 *entropy* (16 bytes for 12 words)
    // as the PBKDF2 password — NOT the mnemonic string. So the resulting
    // mini-secret must differ from the first 32 bytes of the standard BIP-39 seed.
    let result = derive_polkadot(MNEMONIC.into(), None, None, false, false, true)
        .expect("polkadot");
    let mini_secret = result.private_key_hex.expect("mini-secret");
    let bip39_seed_prefix = "5eb00bbddcf069084889a8ab9155568165f5c453ccb85e70811aaed6f6da5fc1";
    assert_ne!(mini_secret, bip39_seed_prefix);
}

#[test]
fn polkadot_path_with_junction_is_rejected() {
    let err = derive_substrate_sr25519_material(MNEMONIC, "", None, None, 0, Some("//Alice"), false)
        .expect_err("junction path must be rejected");
    assert!(
        err.to_lowercase().contains("junction"),
        "error should mention junctions, got: {err}"
    );
}

#[test]
fn polkadot_ss58_round_trip() {
    // Re-decode the SS58 address and verify the embedded pubkey + checksum.
    use blake2::digest::consts::U64;
    use blake2::digest::Digest;
    use blake2::Blake2b;
    type Blake2b512 = Blake2b<U64>;

    let result = derive_polkadot(MNEMONIC.into(), None, None, true, true, false)
        .expect("polkadot");
    let address = result.address.expect("polkadot address");
    let pub_hex = result.public_key_hex.expect("polkadot pub");

    let decoded = bs58::decode(&address).into_vec().expect("base58 decode");
    // 1-byte prefix + 32-byte pubkey + 2-byte checksum = 35 bytes.
    assert_eq!(decoded.len(), 35);
    assert_eq!(decoded[0], 0x00, "Polkadot mainnet prefix is 0");
    assert_eq!(hex::encode(&decoded[1..33]), pub_hex);

    let mut hasher = Blake2b512::new();
    hasher.update(b"SS58PRE");
    hasher.update(&decoded[..33]);
    let checksum = hasher.finalize();
    assert_eq!(&decoded[33..35], &checksum[..2]);
}

#[test]
fn monero_address_structure() {
    // Monero mainnet standard address: 0x12 || spend(32) || view(32) ||
    // keccak256(prev)[..4] = 69 bytes → 95 chars chunked Base58.
    // Network byte 0x12 forces the first character to be '4'.
    let result = derive_monero(MNEMONIC.into(), true, true, true)
        .expect("monero derive");
    let priv_hex = result.private_key_hex.expect("monero priv");
    let pub_hex = result.public_key_hex.expect("monero pub");
    let address = result.address.expect("monero address");

    assert_eq!(priv_hex.len(), 128, "spend+view privkeys = 64 bytes hex");
    assert_eq!(pub_hex.len(), 128, "spend+view pubs = 64 bytes hex");
    assert_eq!(address.len(), 95, "Monero mainnet address is 95 chars");
    assert!(address.starts_with('4'), "Monero mainnet starts with '4', got {address}");
}

#[test]
fn monero_keys_match_reduced_bip39_seed_prefix() {
    // The BIP-39 seed prefix for "abandon … about" with empty passphrase
    // is 5eb0…fc1; that 32-byte LE integer exceeds the curve25519 group
    // order ℓ, so sc_reduce32 reduces it to the value below.
    use curve25519_dalek::scalar::Scalar as DalekScalar;
    let bip39_prefix =
        hex::decode("5eb00bbddcf069084889a8ab9155568165f5c453ccb85e70811aaed6f6da5fc1")
            .expect("decode bip39 prefix");
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&bip39_prefix);
    let expected = hex::encode(DalekScalar::from_bytes_mod_order(bytes).to_bytes());

    let result = derive_monero(MNEMONIC.into(), false, false, true).expect("monero");
    let priv_hex = result.private_key_hex.expect("priv");
    assert_eq!(&priv_hex[..64], expected, "private_key_hex[0..64] must be sc_reduce32(seed prefix)");
}

#[test]
fn monero_view_key_is_keccak_of_spend() {
    // Reconstruct view = sc_reduce32(Keccak256(spend)) and check that
    // public_view inside the address payload matches.
    use curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
    use curve25519_dalek::scalar::Scalar as DalekScalar;

    let result = derive_monero(MNEMONIC.into(), false, true, true).expect("monero");
    let pub_hex = result.public_key_hex.expect("pub");
    let priv_bytes = hex::decode(result.private_key_hex.expect("priv")).expect("decode priv");
    assert_eq!(priv_bytes.len(), 64, "private_key_hex must encode spend||view = 64 bytes");
    let mut spend = [0u8; 32];
    spend.copy_from_slice(&priv_bytes[..32]);

    use sha3::{Digest, Keccak256};
    let spend_hash: [u8; 32] = Keccak256::digest(&spend).into();
    let view_scalar = DalekScalar::from_bytes_mod_order(spend_hash);
    let private_view_expected = view_scalar.to_bytes();
    let public_view = (view_scalar * ED25519_BASEPOINT_POINT).compress().to_bytes();

    assert_eq!(&pub_hex[64..128], hex::encode(public_view), "public_view key mismatch");
    assert_eq!(&priv_bytes[32..], private_view_expected, "private_view must be sc_reduce32(Keccak256(spend))");
}

#[test]
fn monero_passphrase_changes_keys() {
    // The public API has no passphrase for Monero; passphrase affects the
    // upstream BIP-39 seed, which determines the spend key.
    let seed_empty = derive_bip39_seed(MNEMONIC, "", 0, None, None).unwrap();
    let seed_trezor = derive_bip39_seed(MNEMONIC, "TREZOR", 0, None, None).unwrap();
    assert_ne!(&seed_empty[..32], &seed_trezor[..32]);
}

#[test]
fn monero_electrum_seed_decodes_known_vector() {
    // Test vector sourced from the libmonero crate's own doc-test.
    // 24 data words encode the spend secret; word 25 ("rounded") is the CRC checksum.
    use crate::derivation::chains::monero::decode_monero_electrum_seed;
    let phrase = "tissue raking haunted huts afraid volcano howls liar egotistic \
                  befit rounded older bluntly imbalance pivot exotic tuxedo amaze \
                  mostly lukewarm macro vocal hounded biplane rounded";
    let seed = decode_monero_electrum_seed(phrase).expect("decode failed");
    assert_eq!(
        hex::encode(*seed),
        "f7b3beabc9bd6ced864096c0891a8fdf94dc714178a09828775dba01b4df9ab8"
    );
}

#[test]
fn monero_electrum_seed_bad_checksum_rejected() {
    use crate::derivation::chains::monero::decode_monero_electrum_seed;
    // Replace the checksum word with a wrong word.
    let phrase = "tissue raking haunted huts afraid volcano howls liar egotistic \
                  befit rounded older bluntly imbalance pivot exotic tuxedo amaze \
                  mostly lukewarm macro vocal hounded biplane abbey";
    let err = decode_monero_electrum_seed(phrase).expect_err("should fail checksum");
    assert!(err.contains("checksum"), "expected checksum error, got: {err}");
}

#[test]
fn monero_electrum_and_bip39_produce_different_addresses() {
    // The same 12-word BIP-39 mnemonic treated as a Monero Electrum seed must fail
    // because the BIP-39 word list ≠ Monero word list.
    use crate::derivation::chains::monero::decode_monero_electrum_seed;
    let bip39_12 = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    assert!(decode_monero_electrum_seed(bip39_12).is_err(), "12-word BIP-39 must not parse as Electrum");
}

#[test]
fn monero_base58_encodes_known_length_pattern() {
    // Sanity check the chunked Base58: a 69-byte input must produce 95
    // characters (8 full blocks * 11 chars + 1 5-byte trailing block * 7).
    let payload = [0u8; 69];
    let encoded = monero_base58_encode(&payload);
    assert_eq!(encoded.len(), 95);
    // All-zero input encodes as all '1's (the alphabet's index-0 char).
    assert!(encoded.chars().all(|c| c == '1'));
}

#[test]
fn ton_v4r2_code_hash_matches_known_constant() {
    // Ensures the embedded BOC + our parser produce the canonical v4R2
    // root hash. If either changes, every v4R2 address we emit is wrong.
    let (hash, _depth) = v4r2_code_hash_and_depth().expect("v4r2 code hash");
    assert_eq!(
        hex::encode(hash),
        "feb5ff6820e2ff0d9483e7e0d62c817d846789fb4ae580c878866d959dabd5c0"
    );
}

#[test]
fn ton_v4r2_address_structure_and_determinism() {
    // User-friendly base64url addresses are exactly 48 chars (36 bytes
    // → 48 chars under base64url-no-pad). v4R2 bounceable mainnet
    // addresses all start with "EQ" because tag=0x11, workchain=0x00
    // decodes to base64 `EQ`.
    let a = derive_ton(MNEMONIC.into(), None, true, false, false)
        .expect("ton v4r2").address.expect("address");
    let b = derive_ton(MNEMONIC.into(), None, true, false, false)
        .expect("ton v4r2 again").address.expect("address");
    assert_eq!(a, b, "v4r2 address must be deterministic");
    assert_eq!(a.len(), 48, "v4r2 address must be 48 chars: {a}");
    assert!(a.starts_with("EQ"), "v4r2 bounceable-mainnet prefix: {a}");
    // Must not contain '+' or '/' (base64url uses '-' and '_' instead).
    assert!(!a.contains('+'));
    assert!(!a.contains('/'));
}

#[test]
fn ton_v4r2_diverges_from_raw_account_id() {
    // The V4R2 smart-contract address differs from the raw "0:<pubkey_hex>" form.
    let result = derive_ton(MNEMONIC.into(), None, true, true, false)
        .expect("ton derive");
    let v4_addr = result.address.expect("v4r2 address");
    let pub_hex = result.public_key_hex.expect("pub key");
    let raw_addr = format!("0:{pub_hex}");
    assert_ne!(raw_addr, v4_addr);
}

#[test]
fn ton_v4r2_changes_with_mnemonic() {
    let addr_a = derive_ton(MNEMONIC.into(), None, true, false, false)
        .expect("a").address.expect("a addr");
    let addr_b = derive_ton(
        "legal winner thank year wave sausage worth useful legal winner thank yellow".into(),
        None, true, false, false,
    ).expect("b").address.expect("b addr");
    assert_ne!(addr_a, addr_b);
}

#[test]
fn crc16_xmodem_known_vector() {
    // Standard CRC-16/XMODEM test vector: "123456789" → 0x31C3.
    assert_eq!(crc16_xmodem(b"123456789"), 0x31C3);
}

// ──────────────────────────────────────────────────────────────────
//  canonical external golden vectors.
//
// Each entry pins our derivation output against an address emitted
// by a reference wallet / official spec for the same mnemonic + path.
// ──────────────────────────────────────────────────────────────────

#[test]
fn canonical_external_golden_vectors_all_pass() {
    let mut failures: Vec<String> = Vec::new();

    macro_rules! check {
        ($label:literal, $source:literal, $path:literal, $ci:literal, $e:expr, $expected:literal) => {{
            match $e {
                Err(e) => failures.push(format!(
                    "{} — derivation error: {e:?} (source: {}, path: {})",
                    $label, $source, $path
                )),
                Ok(r) => match r.address {
                    None => failures.push(format!("{} — no address returned", $label)),
                    Some(addr) => {
                        let ok = if $ci {
                            addr.eq_ignore_ascii_case($expected)
                        } else {
                            addr == $expected
                        };
                        if !ok {
                            failures.push(format!(
                                "{} mismatch\n    expected: {}\n    actual:   {}\n    source:   {}\n    path:     {}",
                                $label, $expected, addr, $source, $path
                            ));
                        }
                    }
                }
            }
        }};
    }

    check!(
        "Bitcoin P2WPKH (BIP-84)",
        "github.com/bitcoin/bips/blob/master/bip-0084.mediawiki",
        "m/84'/0'/0'/0/0", false,
        derive_bitcoin(MNEMONIC.into(), "m/84'/0'/0'/0/0".into(), None, BitcoinScriptType::P2wpkh, true, false, false),
        "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu"
    );
    check!(
        "Bitcoin P2TR (BIP-86)",
        "github.com/bitcoin/bips/blob/master/bip-0086.mediawiki",
        "m/86'/0'/0'/0/0", false,
        derive_bitcoin(MNEMONIC.into(), "m/86'/0'/0'/0/0".into(), None, BitcoinScriptType::P2tr, true, false, false),
        "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr"
    );
    check!(
        "Bitcoin P2PKH legacy",
        "trezor-firmware/tests/device_tests/bitcoin/test_getaddress.py",
        "m/44'/0'/0'/0/0", false,
        derive_bitcoin(ALL_ALL.into(), "m/44'/0'/0'/0/0".into(), None, BitcoinScriptType::P2pkh, true, false, false),
        "1JAd7XCBzGudGpJQSDSfpmJhiygtLQWaGL"
    );
    check!(
        "Bitcoin P2WPKH (all-all seed)",
        "trezor-firmware/tests/device_tests/bitcoin/test_getaddress_segwit_native.py",
        "m/84'/0'/0'/0/0", false,
        derive_bitcoin(ALL_ALL.into(), "m/84'/0'/0'/0/0".into(), None, BitcoinScriptType::P2wpkh, true, false, false),
        "bc1qannfxke2tfd4l7vhepehpvt05y83v3qsf6nfkk"
    );
    check!(
        "Litecoin P2PKH legacy",
        "trezor-firmware/tests/device_tests/bitcoin/test_getaddress.py",
        "m/44'/2'/0'/0/0", false,
        derive_litecoin(ALL_ALL.into(), "m/44'/2'/0'/0/0".into(), None, BitcoinScriptType::P2pkh, true, false, false),
        "LcubERmHD31PWup1fbozpKuiqjHZ4anxcL"
    );
    check!(
        "Ethereum",
        "trezor-firmware/common/tests/fixtures/ethereum/getaddress.json",
        "m/44'/60'/0'/0/0", true,
        derive_ethereum(ALL_ALL.into(), "m/44'/60'/0'/0/0".into(), None, true, false, false),
        "0x73d0385F4d8E00C5e6504C6030F47BF6212736A8"
    );
    check!(
        "Ethereum Classic",
        "trezor-firmware/common/tests/fixtures/ethereum/getaddress.json",
        "m/44'/61'/0'/0/0", true,
        derive_ethereum_classic(ALL_ALL.into(), "m/44'/61'/0'/0/0".into(), None, true, false, false),
        "0xF410e37E9C8BCf8CF319c84Ae9dCEbe057804a04"
    );
    check!(
        "Tron",
        "trezor-firmware/common/tests/fixtures/tron/get_address.json",
        "m/44'/195'/0'/0/0", false,
        derive_tron(ALL_ALL.into(), "m/44'/195'/0'/0/0".into(), None, true, false, false),
        "TY72iA3SBtrds3QLYsS7LwYfkzXwAXCRWT"
    );
    check!(
        "XRP Ledger",
        "trezor-firmware/tests/device_tests/ripple/test_get_address.py",
        "m/44'/144'/0'/0/0", false,
        derive_xrp(ALL_ALL.into(), "m/44'/144'/0'/0/0".into(), None, true, false, false),
        "rNaqKtKrMSwpwZSzRckPf7S96DkimjkF4H"
    );
    check!(
        "Solana",
        "trezor-firmware/common/tests/fixtures/solana/get_address.json",
        "m/44'/501'/0'/0'", false,
        derive_solana(ALL_ALL.into(), "m/44'/501'/0'/0'".into(), None, None, true, false, false),
        "14CCvQzQzHCVgZM3j9soPnXuJXh1RmCfwLVUcdfbZVBS"
    );
    check!(
        "Stellar (SEP-0005 Test 1 #0)",
        "trezor-firmware/common/tests/fixtures/stellar/get_address.json",
        "m/44'/148'/0'", false,
        derive_stellar(SEP_0005.into(), "m/44'/148'/0'".into(), None, None, true, false, false),
        "GDRXE2BQUC3AZNPVFSCEZ76NJ3WWL25FYFK6RGZGIEKWE4SOOHSUJUJ6"
    );
    check!(
        "Cardano Shelley enterprise",
        "trezor-firmware/common/tests/fixtures/cardano/get_enterprise_address.json",
        "m/1852'/1815'/0'/0/0", false,
        derive_cardano(ALL_ALL.into(), Some("m/1852'/1815'/0'/0/0".into()), None, true, false, false),
        "addr1vxq0nckg3ekgzuqg7w5p9mvgnd9ym28qh5grlph8xd2z92su77c6m"
    );

    assert!(
        failures.is_empty(),
        "{} canonical vector(s) failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn evm_replica_chains_share_ethereum_golden() {
    // Every EVM-compatible chain reuses Ethereum's BIP-44 derivation
    // (m/44'/60'/…) and Keccak address algorithm, so the same mnemonic
    // must produce the same address across all EVM replicas.
    let expected = "0x73d0385F4d8E00C5e6504C6030F47BF6212736A8";
    let path = "m/44'/60'/0'/0/0".to_string();
    let results = [
        ("arbitrum",    derive_arbitrum(ALL_ALL.into(),    path.clone(), None, true, false, false)),
        ("optimism",    derive_optimism(ALL_ALL.into(),    path.clone(), None, true, false, false)),
        ("avalanche",   derive_avalanche(ALL_ALL.into(),   path.clone(), None, true, false, false)),
        ("hyperliquid", derive_hyperliquid(ALL_ALL.into(), path.clone(), None, true, false, false)),
    ];
    for (label, result) in results {
        let addr = result
            .unwrap_or_else(|e| panic!("evm replica {label} failed: {e:?}"))
            .address
            .unwrap();
        assert!(
            addr.eq_ignore_ascii_case(expected),
            "EVM replica {label} drifted: got {addr}"
        );
    }
}
