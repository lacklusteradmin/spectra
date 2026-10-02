//! Parse fixed-fee UTXO addresses without erasing the output script type.

use crate::derivation::error::DerivationError;
use crate::registry::Chain;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ParsedUtxoAddress {
    P2pkh([u8; 20]),
    P2sh([u8; 20]),
    Witness { version: u8, program: Vec<u8> },
}

impl ParsedUtxoAddress {
    pub(crate) fn script_pubkey(&self) -> Vec<u8> {
        match self {
            Self::P2pkh(hash) => {
                let mut script = vec![0x76, 0xa9, 0x14];
                script.extend(hash);
                script.extend([0x88, 0xac]);
                script
            }
            Self::P2sh(hash) => {
                let mut script = vec![0xa9, 0x14];
                script.extend(hash);
                script.push(0x87);
                script
            }
            Self::Witness { version, program } => {
                let opcode = if *version == 0 { 0 } else { 0x50 + version };
                let mut script = vec![opcode, program.len() as u8];
                script.extend(program);
                script
            }
        }
    }

    /// These wallets derive P2PKH spend keys; other scripts are recipients only.
    pub(crate) fn require_p2pkh(self) -> Result<[u8; 20], DerivationError> {
        match self {
            Self::P2pkh(hash) => Ok(hash),
            _ => Err(DerivationError::Invalid(
                "sending address must be P2PKH".into(),
            )),
        }
    }
}

pub(crate) fn parse_utxo_address(
    chain: Chain,
    address: &str,
) -> Result<ParsedUtxoAddress, DerivationError> {
    let invalid = || DerivationError::Invalid("invalid address for selected UTXO network".into());
    let address = address.trim();
    let (p2pkh, p2sh) = chain.fixed_utxo_address_versions().map_err(|_| invalid())?;
    if let Ok(decoded) = bs58::decode(address).with_check(None).into_vec()
        && decoded.len() == 21
    {
        let hash = decoded[1..].try_into().map_err(|_| invalid())?;
        return if decoded[0] == p2pkh {
            Ok(ParsedUtxoAddress::P2pkh(hash))
        } else if p2sh.contains(&decoded[0]) {
            Ok(ParsedUtxoAddress::P2sh(hash))
        } else {
            Err(invalid())
        };
    }
    if let Some(expected) = chain.fixed_utxo_segwit_hrp()
        && let Ok((hrp, version, program)) = bech32::segwit::decode(address)
        && hrp.as_str().eq_ignore_ascii_case(expected)
        && chain.fixed_utxo_supports_witness(version.to_u8(), program.len())
    {
        return Ok(ParsedUtxoAddress::Witness {
            version: version.to_u8(),
            program,
        });
    }
    if let Some(prefix) = chain.cashaddr_prefix() {
        return decode_cashaddr(address, prefix).ok_or_else(invalid);
    }
    Err(invalid())
}

/// CashAddr's version is an eight-bit byte in the decoded payload, not the
/// first five-bit character. Restrict supported outputs to HASH160 P2PKH/P2SH.
fn decode_cashaddr(address: &str, expected_prefix: &str) -> Option<ParsedUtxoAddress> {
    const CHARSET: &[u8] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";
    const GENERATORS: [u64; 5] = [
        0x98f2bc8e61,
        0x79b76d99e2,
        0xf33e5fb3c4,
        0xae2eabe2a8,
        0x1e4f43e470,
    ];
    let lower = address.to_ascii_lowercase();
    if address.bytes().any(|b| b.is_ascii_lowercase())
        && address.bytes().any(|b| b.is_ascii_uppercase())
    {
        return None;
    }
    let payload = match lower.split_once(':') {
        Some((prefix, payload)) if prefix == expected_prefix => payload,
        Some(_) => return None,
        None => lower.as_str(),
    };
    let values: Vec<u8> = payload
        .bytes()
        .map(|b| CHARSET.iter().position(|c| *c == b).map(|n| n as u8))
        .collect::<Option<_>>()?;
    if values.len() != 42 {
        return None;
    }
    let mut checksum = 1u64;
    for value in expected_prefix
        .bytes()
        .map(|b| b & 31)
        .chain([0])
        .chain(values.iter().copied())
    {
        let top = checksum >> 35;
        checksum = ((checksum & 0x07_ffff_ffff) << 5) ^ u64::from(value);
        for (i, generator) in GENERATORS.iter().enumerate() {
            if top & (1 << i) != 0 {
                checksum ^= generator;
            }
        }
    }
    if checksum != 1 {
        return None;
    }
    let mut decoded = Vec::with_capacity(21);
    let (mut bits, mut accumulator) = (0u32, 0u32);
    for value in &values[..values.len() - 8] {
        accumulator = ((accumulator << 5) | u32::from(*value)) & 0xffff;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            decoded.push((accumulator >> bits) as u8);
        }
    }
    if bits >= 5 || (accumulator & ((1 << bits) - 1)) != 0 {
        return None;
    }
    let hash = decoded.get(1..)?.try_into().ok()?;
    match decoded.first()? {
        0 => Some(ParsedUtxoAddress::P2pkh(hash)),
        8 => Some(ParsedUtxoAddress::P2sh(hash)),
        _ => None,
    }
}
