//! Bitcoin SV: address validation, BIP-39 + BIP-32 derivation, legacy P2PKH
//! base58check encoding.

// ── Address validation ───────────────────────────────────────────────────

/// BSV network encoded in the address version byte. Validation and output
/// construction must reject addresses belonging to the other network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BsvNetwork {
    Mainnet,
    Testnet,
}

impl BsvNetwork {
    /// The P2PKH and P2SH version bytes, in that order. One table, so the
    /// validator, the decoder and the deriver cannot disagree about which
    /// byte belongs where.
    pub(crate) const fn versions(self) -> [u8; 2] {
        match self {
            BsvNetwork::Mainnet => [0x00, 0x05],
            BsvNetwork::Testnet => [0x6f, 0xc4],
        }
    }

    /// The version byte a P2PKH address on this network carries.
    pub(crate) const fn p2pkh_version(self) -> u8 {
        self.versions()[0]
    }

    fn of_version(version: u8) -> Option<Self> {
        [BsvNetwork::Mainnet, BsvNetwork::Testnet]
            .into_iter()
            .find(|network| network.versions()[0] == version || network.versions()[1] == version)
    }
}

/// Base58check-decode a BSV address into its 20-byte hash and the network its
/// version byte names.
///
/// The network comes back with the hash because the caller always needs it:
/// two addresses in one transaction have to agree about which chain they are
/// on, and the hash alone cannot say.
pub(crate) fn decode_bsv_address(address: &str) -> Result<([u8; 20], BsvNetwork), String> {
    let decoded = bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("invalid bsv address: {e}"))?;
    if decoded.len() != 21 {
        return Err("bsv address wrong length".to_string());
    }
    let network = BsvNetwork::of_version(decoded[0])
        .ok_or_else(|| format!("unexpected bsv version byte: 0x{:02x}", decoded[0]))?;
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&decoded[1..21]);
    Ok((hash, network))
}

/// Whether `address` is valid BSV base58check **on the network asked about**.
pub fn validate_bsv_address(address: &str, testnet: bool) -> bool {
    let wanted = if testnet {
        BsvNetwork::Testnet
    } else {
        BsvNetwork::Mainnet
    };
    decode_bsv_address(address).is_ok_and(|(_, network)| network == wanted)
}

use crate::SpectraBridgeError;
use crate::derivation::bitcoin::derive_legacy_p2pkh;
use crate::derivation::types::{BitcoinScriptType, DerivationResult};

pub(crate) const BSV_MAINNET_VERSION: u8 = BsvNetwork::Mainnet.p2pkh_version();
pub(crate) const BSV_TESTNET_VERSION: u8 = BsvNetwork::Testnet.p2pkh_version();

/// UniFFI export: derive Bitcoin SV mainnet keys (P2PKH only).
pub fn derive_bitcoin_sv(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    derive_legacy_p2pkh(
        BSV_MAINNET_VERSION,
        seed_phrase,
        derivation_path,
        passphrase,
        script_type,
        want_address,
        want_public_key,
        want_private_key,
    )
}

/// UniFFI export: derive Bitcoin SV testnet keys (P2PKH only).
pub fn derive_bitcoin_sv_testnet(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    script_type: BitcoinScriptType,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    derive_legacy_p2pkh(
        BSV_TESTNET_VERSION,
        seed_phrase,
        derivation_path,
        passphrase,
        script_type,
        want_address,
        want_public_key,
        want_private_key,
    )
}

/// A version byte names a network, and BSV has two of them.
#[cfg(test)]
mod a_bsv_address_belongs_to_one_network {
    use super::*;
    use crate::derivation::bitcoin::base58check_encode;

    /// Build an address carrying `version` over a fixed hash.
    fn address(version: u8) -> String {
        base58check_encode(&[&[version][..], &[0x11u8; 20][..]].concat())
    }

    /// The four version bytes, each valid on exactly one network.
    ///
    /// `validate_bsv_address` took no network and accepted all four, and both
    /// `"bitcoinSV"` and `"bitcoinSVTestnet"` dispatched to it — so a mainnet
    /// send accepted an `m…`/`n…`/`2…` destination and a testnet send accepted
    /// a `1…`/`3…` one.
    #[test]
    fn each_version_byte_validates_on_its_own_network_only() {
        for (version, is_testnet, shape) in [
            (0x00u8, false, "mainnet P2PKH"),
            (0x05, false, "mainnet P2SH"),
            (0x6f, true, "testnet P2PKH"),
            (0xc4, true, "testnet P2SH"),
        ] {
            let address = address(version);
            assert!(
                validate_bsv_address(&address, is_testnet),
                "{shape} must validate on its own network"
            );
            assert!(
                !validate_bsv_address(&address, !is_testnet),
                "{shape} must not validate on the other network"
            );
        }
    }

    /// A byte belonging to neither is neither, and a corrupt checksum is not
    /// an address on any network.
    #[test]
    fn an_unknown_version_or_bad_checksum_is_refused_on_both() {
        for candidate in [address(0x01), address(0x80), "not-an-address".to_string()] {
            assert!(!validate_bsv_address(&candidate, false), "{candidate}");
            assert!(!validate_bsv_address(&candidate, true), "{candidate}");
        }
    }

    /// What the deriver produces is what the validator accepts, on both
    /// networks. The version constants and the validator read one table now,
    /// so they cannot answer differently.
    #[test]
    fn the_deriver_and_the_validator_agree() {
        assert_eq!(BSV_MAINNET_VERSION, BsvNetwork::Mainnet.p2pkh_version());
        assert_eq!(BSV_TESTNET_VERSION, BsvNetwork::Testnet.p2pkh_version());
        assert!(validate_bsv_address(&address(BSV_MAINNET_VERSION), false));
        assert!(validate_bsv_address(&address(BSV_TESTNET_VERSION), true));
    }

    /// The decoder hands back the network with the hash, which is what lets
    /// the signer refuse a transaction whose outputs straddle two chains.
    #[test]
    fn the_decoder_reports_which_network_it_read() {
        assert_eq!(
            decode_bsv_address(&address(0x00)).unwrap().1,
            BsvNetwork::Mainnet
        );
        assert_eq!(
            decode_bsv_address(&address(0xc4)).unwrap().1,
            BsvNetwork::Testnet
        );
        assert!(decode_bsv_address(&address(0x42)).is_err());
    }
}
