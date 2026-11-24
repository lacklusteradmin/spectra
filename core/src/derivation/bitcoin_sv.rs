//! Bitcoin SV: its networks' P2PKH and P2SH version bytes. Its keys and
//! addresses derive with the Bitcoin family (`bitcoin::derive_from_seed_phrase`).

/// A version byte names a network, and BSV has two of them.
#[cfg(test)]
mod a_bsv_address_belongs_to_one_network {
    use crate::derivation::bitcoin::base58check_encode;
    use crate::derivation::utxo_address::parse_utxo_address;
    use crate::registry::Chain;

    /// Build an address carrying `version` over a fixed hash.
    fn address(version: u8) -> String {
        base58check_encode(&[&[version][..], &[0x11u8; 20][..]].concat())
    }

    #[test]
    fn each_version_byte_validates_on_its_own_network_only() {
        for (version, chain, other_chain, shape) in [
            (
                0x00u8,
                Chain::BitcoinSV,
                Chain::BitcoinSVTestnet,
                "mainnet P2PKH",
            ),
            (
                0x05,
                Chain::BitcoinSV,
                Chain::BitcoinSVTestnet,
                "mainnet P2SH",
            ),
            (
                0x6f,
                Chain::BitcoinSVTestnet,
                Chain::BitcoinSV,
                "testnet P2PKH",
            ),
            (
                0xc4,
                Chain::BitcoinSVTestnet,
                Chain::BitcoinSV,
                "testnet P2SH",
            ),
        ] {
            let address = address(version);
            assert!(
                parse_utxo_address(chain, &address).is_ok(),
                "{shape} must validate on its own network"
            );
            assert!(
                parse_utxo_address(other_chain, &address).is_err(),
                "{shape} must not validate on the other network"
            );
        }
    }

    #[test]
    fn an_unknown_version_or_bad_checksum_is_refused_on_both() {
        let mut corrupt = bs58::decode(address(0x00)).into_vec().unwrap();
        *corrupt.last_mut().unwrap() ^= 1;
        for candidate in [
            address(0x01),
            address(0x80),
            bs58::encode(corrupt).into_string(),
            "not-an-address".to_string(),
        ] {
            for chain in [Chain::BitcoinSV, Chain::BitcoinSVTestnet] {
                assert!(
                    parse_utxo_address(chain, &candidate).is_err(),
                    "{candidate}"
                );
            }
        }
    }
}
