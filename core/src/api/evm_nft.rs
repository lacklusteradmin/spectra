//! ERC-721 and ERC-1155: the two standards, and the token ids and quantities
//! they move.
//!
//! A token id names one token of a collection and an ERC-1155 quantity
//! counts copies of it. Neither is an amount of anything: each is a whole
//! number below 2^256, carried as its canonical decimal and encoded as a
//! 32-byte ABI word, and nothing scales either by decimals.

use crate::api::error::ApiError;
use num_bigint::BigUint;

const SEL_SUPPORTS_INTERFACE: [u8; 4] = [0x01, 0xff, 0xc9, 0xa7]; // supportsInterface(bytes4)
const SEL_OWNER_OF: [u8; 4] = [0x63, 0x52, 0x21, 0x1e]; // ownerOf(uint256)
const SEL_BALANCE_OF_ID: [u8; 4] = [0x00, 0xfd, 0xd5, 0x8e]; // balanceOf(address,uint256)
/// ERC-165's own "no interface": a contract that claims it claims anything.
pub(crate) const INVALID_INTERFACE: [u8; 4] = [0xff; 4];

/// A non-fungible token standard, as a contract reports it through ERC-165.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, uniffi::Enum,
)]
pub enum NftStandard {
    /// One owner per token id.
    #[serde(rename = "ERC-721")]
    Erc721,
    /// A quantity of each token id per owner.
    #[serde(rename = "ERC-1155")]
    Erc1155,
}

impl NftStandard {
    /// The spelling indexers and people use.
    pub fn label(self) -> &'static str {
        match self {
            Self::Erc721 => "ERC-721",
            Self::Erc1155 => "ERC-1155",
        }
    }

    pub fn parse(label: &str) -> Option<Self> {
        match label {
            "ERC-721" => Some(Self::Erc721),
            "ERC-1155" => Some(Self::Erc1155),
            _ => None,
        }
    }

    /// The ERC-165 interface id a collection of this standard reports.
    pub(crate) fn interface_id(self) -> [u8; 4] {
        match self {
            Self::Erc721 => [0x80, 0xac, 0x58, 0xcd],
            Self::Erc1155 => [0xd9, 0xb6, 0x7a, 0x26],
        }
    }
}

/// A token id or quantity written in decimal: digits only, below 2^256.
pub(crate) fn parse_uint256(decimal: &str) -> Result<BigUint, ApiError> {
    let invalid = || ApiError::invalid("A token id or quantity is a whole number below 2^256");
    if decimal.is_empty() || decimal.len() > 78 || !decimal.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    let value = BigUint::parse_bytes(decimal.as_bytes(), 10).ok_or_else(invalid)?;
    if value.bits() > 256 {
        return Err(invalid());
    }
    Ok(value)
}

/// The one spelling of a token id or quantity: no sign, no leading zeros.
pub(crate) fn canonical_uint256(decimal: &str) -> Option<String> {
    parse_uint256(decimal.trim())
        .ok()
        .map(|value| value.to_string())
}

/// A whole number below 2^256 as its ABI word.
pub(crate) fn uint256_word(value: &BigUint) -> [u8; 32] {
    let bytes = value.to_bytes_be();
    assert!(bytes.len() <= 32, "a uint256 fits 32 bytes");
    let mut word = [0u8; 32];
    word[32 - bytes.len()..].copy_from_slice(&bytes);
    word
}

/// An address as its left-padded ABI word.
pub(crate) fn address_word(address: &str) -> Result<[u8; 32], ApiError> {
    let bytes = crate::api::evm_json_rpc::decode_hex(address)?;
    if bytes.len() != 20 {
        return Err(ApiError::invalid("invalid EVM address length"));
    }
    let mut word = [0u8; 32];
    word[12..].copy_from_slice(&bytes);
    Ok(word)
}

/// `supportsInterface(interface)`.
pub(crate) fn supports_interface_call(interface: [u8; 4]) -> Vec<u8> {
    let mut data = SEL_SUPPORTS_INTERFACE.to_vec();
    data.extend(interface);
    data.extend([0u8; 28]);
    data
}

/// `ownerOf(tokenId)`.
pub(crate) fn owner_of_call(token_id: &str) -> Result<Vec<u8>, ApiError> {
    let mut data = SEL_OWNER_OF.to_vec();
    data.extend(uint256_word(&parse_uint256(token_id)?));
    Ok(data)
}

/// `balanceOf(owner, id)`.
pub(crate) fn balance_of_call(owner: &str, token_id: &str) -> Result<Vec<u8>, ApiError> {
    let mut data = SEL_BALANCE_OF_ID.to_vec();
    data.extend(address_word(owner)?);
    data.extend(uint256_word(&parse_uint256(token_id)?));
    Ok(data)
}

/// Whether an ABI answer is the boolean true: a single word equal to one.
pub(crate) fn is_abi_true(word: &[u8]) -> bool {
    word.len() == 32 && word[..31] == [0; 31] && word[31] == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_ids_are_whole_numbers_below_two_to_the_256() {
        let max = (BigUint::from(1u8) << 256u32) - 1u8;
        assert_eq!(canonical_uint256(&max.to_string()), Some(max.to_string()));
        assert_eq!(canonical_uint256(" 007 "), Some("7".into()));
        assert_eq!(canonical_uint256("0"), Some("0".into()));
        let too_big = BigUint::from(1u8) << 256u32;
        for bad in [
            too_big.to_string(),
            String::new(),
            "1.5".into(),
            "-1".into(),
            "1e3".into(),
            "0x10".into(),
            "1 000".into(),
            "١".into(),
        ] {
            assert_eq!(canonical_uint256(&bad), None, "{bad}");
        }
        // Seventy-eight digits of zeros pad a small number, not a large one.
        assert_eq!(
            canonical_uint256(&format!("{}5", "0".repeat(77))),
            Some("5".into())
        );
        assert_eq!(uint256_word(&max), [0xff; 32]);
    }

    #[test]
    fn only_one_word_equal_to_one_is_true() {
        let mut word = [0u8; 32];
        word[31] = 1;
        assert!(is_abi_true(&word));
        word[0] = 1;
        assert!(!is_abi_true(&word));
        assert!(!is_abi_true(&[1]));
        assert!(!is_abi_true(&[0; 32]));
        let mut two = [0u8; 32];
        two[31] = 2;
        assert!(!is_abi_true(&two));
    }
}
