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

/// CashAddr's version byte for a 160-bit key hash, and for a 160-bit
/// script hash.
const CASHADDR_P2PKH: u8 = 0x00;
const CASHADDR_P2SH: u8 = 0x08;

/// `address` in `chain`'s CashAddr encoding, with its prefix: a P2PKH or
/// P2SH hash. Kaspa's address codec is CashAddr's, with its own versions.
pub(crate) fn encode_cashaddr(
    chain: Chain,
    address: &ParsedUtxoAddress,
) -> Result<String, DerivationError> {
    let prefix = chain
        .cashaddr_prefix()
        .ok_or_else(|| DerivationError::invalid("network has no CashAddr prefix"))?;
    let (version, hash) = match address {
        ParsedUtxoAddress::P2pkh(hash) => (CASHADDR_P2PKH, hash),
        ParsedUtxoAddress::P2sh(hash) => (CASHADDR_P2SH, hash),
        ParsedUtxoAddress::Witness { .. } => {
            return Err(DerivationError::invalid("CashAddr has no witness programs"));
        }
    };
    crate::derivation::kaspa::encode_kaspa_address(version, hash, prefix)
}

/// `script` as an address on `chain`: P2PKH and P2SH in the network's
/// Base58 versions, or in CashAddr where the network has no Base58 ones, a
/// witness program in its SegWit encoding. `None` for a script no address
/// on the network names.
pub(crate) fn script_address(chain: Chain, script: &[u8]) -> Option<String> {
    use crate::derivation::bitcoin::base58check_encode;
    let Ok((p2pkh, p2sh)) = chain.fixed_utxo_address_versions() else {
        let parsed = match script {
            [0x76, 0xa9, 0x14, hash @ .., 0x88, 0xac] => {
                ParsedUtxoAddress::P2pkh(hash.try_into().ok()?)
            }
            [0xa9, 0x14, hash @ .., 0x87] => ParsedUtxoAddress::P2sh(hash.try_into().ok()?),
            _ => return None,
        };
        return encode_cashaddr(chain, &parsed).ok();
    };
    match script {
        [0x76, 0xa9, 0x14, hash @ .., 0x88, 0xac] if hash.len() == 20 => {
            Some(base58check_encode(&[&[p2pkh], hash].concat()))
        }
        [0xa9, 0x14, hash @ .., 0x87] if hash.len() == 20 => {
            Some(base58check_encode(&[&[*p2sh.first()?], hash].concat()))
        }
        [opcode, length, program @ ..] if usize::from(*length) == program.len() => {
            let version = match opcode {
                0 => 0,
                0x51..=0x60 => opcode - 0x50,
                _ => return None,
            };
            if !chain.fixed_utxo_supports_witness(version, program.len()) {
                return None;
            }
            let hrp = bech32::Hrp::parse(chain.fixed_utxo_segwit_hrp()?).ok()?;
            let version = bech32::Fe32::try_from(version).ok()?;
            bech32::segwit::encode(hrp, version, program).ok()
        }
        _ => None,
    }
}

pub(crate) fn parse_utxo_address(
    chain: Chain,
    address: &str,
) -> Result<ParsedUtxoAddress, DerivationError> {
    let invalid = || DerivationError::Invalid("invalid address for selected UTXO network".into());
    let address = address.trim();
    // eCash's wallets write CashAddr alone, whose payloads are Bitcoin's
    // Base58 ones: it has no Base58 versions of its own.
    let versions = chain.fixed_utxo_address_versions().ok();
    if versions.is_none() && chain.cashaddr_prefix().is_none() {
        return Err(invalid());
    }
    if let Some((p2pkh, p2sh)) = versions
        && let Ok(decoded) = bs58::decode(address).with_check(None).into_vec()
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
