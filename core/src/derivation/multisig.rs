//! A Bitcoin multisig account: `wsh(sortedmulti(k, …))`, the policy its
//! output descriptor names (BIP-380, BIP-383, BIP-389's `<0;1>` for the
//! receive and change branches). Each cosigner is an account public key with
//! the fingerprint and path it was derived along, which a PSBT names so each
//! signer finds its key. Every address of the account pays the P2WSH of the
//! k-of-n script over the cosigners' keys at one receive or change index,
//! sorted (BIP-67).

use bitcoin::bip32::{ChildNumber, DerivationPath, Fingerprint, Xpriv, Xpub};
use bitcoin::secp256k1::{PublicKey, Secp256k1};
use bitcoin::{Address, KnownHrp, NetworkKind, ScriptBuf};
use std::str::FromStr;

use crate::derivation::error::DerivationError;
use crate::registry::Chain;

/// `OP_CHECKMULTISIG`'s limit, and Bitcoin Core's for `sortedmulti` in `wsh`.
pub(crate) const MAX_COSIGNERS: usize = 20;

/// One cosigner: an account public key and where it was derived from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cosigner {
    pub fingerprint: Fingerprint,
    pub origin: DerivationPath,
    pub key: Xpub,
}

/// A k-of-n P2WSH account over sorted keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MultisigPolicy {
    pub threshold: usize,
    pub cosigners: Vec<Cosigner>,
}

/// Where an address sits in the account: the receive (0) or change (1)
/// branch, and its index.
pub(crate) type Place = (u32, u32);

impl MultisigPolicy {
    /// `text` as a multisig account on `chain`: a `wsh(sortedmulti(…))`
    /// descriptor whose keys carry their origin and the network's version
    /// (`xpub` on mainnet, `tpub` on a test network), each followed by
    /// `/<0;1>/*` or `/0/*`. A checksum, when present, must be the
    /// descriptor's.
    pub(crate) fn parse(chain: Chain, text: &str) -> Result<Self, DerivationError> {
        let network = chain.bitcoin_network().ok_or_else(|| {
            DerivationError::refused("%@ has no multisig wallets.", [chain.chain_display_name()])
        })?;
        let text: String = text.split_whitespace().collect();
        let (body, checksum) = match text.split_once('#') {
            Some((body, checksum)) => (body, Some(checksum)),
            None => (text.as_str(), None),
        };
        if let Some(checksum) = checksum
            && descriptor_checksum(body).as_deref() != Some(checksum)
        {
            return Err(DerivationError::invalid(
                "The descriptor's checksum does not match it.",
            ));
        }
        let not_one = || {
            DerivationError::invalid(
                "A multisig wallet is a wsh(sortedmulti(…)) descriptor whose keys name their origin.",
            )
        };
        let inner = body
            .strip_prefix("wsh(sortedmulti(")
            .and_then(|rest| rest.strip_suffix("))"))
            .ok_or_else(not_one)?;
        let mut parts = inner.split(',');
        let threshold: usize = parts
            .next()
            .and_then(|k| k.parse().ok())
            .ok_or_else(not_one)?;
        let cosigners = parts
            .map(|key| parse_key(key, network).ok_or_else(not_one))
            .collect::<Result<Vec<_>, _>>()?;
        if threshold == 0 || threshold > cosigners.len() || cosigners.len() > MAX_COSIGNERS {
            return Err(DerivationError::invalid(
                "A multisig wallet needs between 1 and 20 keys, and a threshold no larger than its keys.",
            ));
        }
        for (index, cosigner) in cosigners.iter().enumerate() {
            if cosigners[..index]
                .iter()
                .any(|other| other.key.public_key == cosigner.key.public_key)
            {
                return Err(DerivationError::invalid(
                    "A multisig wallet's keys must be distinct.",
                ));
            }
        }
        Ok(Self {
            threshold,
            cosigners,
        })
    }

    /// The policy as a descriptor with its checksum, in the one spelling
    /// this module writes: `h` for hardened steps and `<0;1>` for the
    /// branches.
    pub(crate) fn descriptor(&self) -> String {
        let keys: Vec<String> = self
            .cosigners
            .iter()
            .map(|cosigner| {
                let mut origin = cosigner.fingerprint.to_string();
                for step in &cosigner.origin {
                    origin.push('/');
                    origin.push_str(&match step {
                        ChildNumber::Hardened { index } => format!("{index}h"),
                        ChildNumber::Normal { index } => index.to_string(),
                    });
                }
                format!("[{origin}]{}/<0;1>/*", cosigner.key)
            })
            .collect();
        let body = format!("wsh(sortedmulti({},{}))", self.threshold, keys.join(","));
        let checksum = descriptor_checksum(&body).unwrap_or_default();
        format!("{body}#{checksum}")
    }

    /// Each cosigner's key at `place`, in cosigner order, with the
    /// fingerprint and full path a PSBT names it by.
    pub(crate) fn keys(
        &self,
        (branch, index): Place,
    ) -> Result<Vec<(PublicKey, Fingerprint, DerivationPath)>, DerivationError> {
        let steps = [
            ChildNumber::from_normal_idx(branch).map_err(DerivationError::invalid)?,
            ChildNumber::from_normal_idx(index).map_err(DerivationError::invalid)?,
        ];
        if branch > 1 {
            return Err(DerivationError::invalid(
                "A multisig address is on the receive or the change branch.",
            ));
        }
        let secp = Secp256k1::verification_only();
        self.cosigners
            .iter()
            .map(|cosigner| {
                let child = cosigner
                    .key
                    .derive_pub(&secp, &steps)
                    .map_err(DerivationError::invalid)?;
                Ok((
                    child.public_key,
                    cosigner.fingerprint,
                    cosigner.origin.extend(steps),
                ))
            })
            .collect()
    }

    /// The k-of-n script over the keys at `place`, sorted.
    pub(crate) fn witness_script(&self, place: Place) -> Result<ScriptBuf, DerivationError> {
        let mut keys: Vec<[u8; 33]> = self
            .keys(place)?
            .into_iter()
            .map(|(key, _, _)| key.serialize())
            .collect();
        keys.sort_unstable();
        let mut builder = bitcoin::script::Builder::new().push_int(self.threshold as i64);
        for key in &keys {
            builder = builder.push_slice(key);
        }
        Ok(builder
            .push_int(keys.len() as i64)
            .push_opcode(bitcoin::opcodes::all::OP_CHECKMULTISIG)
            .into_script())
    }

    /// The address at `place` on `chain`.
    pub(crate) fn address(&self, chain: Chain, place: Place) -> Result<String, DerivationError> {
        let network = chain
            .bitcoin_network()
            .ok_or_else(|| DerivationError::invalid("Multisig addresses are Bitcoin addresses"))?;
        Ok(Address::p2wsh(&self.witness_script(place)?, KnownHrp::from(network)).to_string())
    }

    /// Where `path`, from the cosigner `fingerprint` names, sits in the
    /// account: one of the cosigners' origins followed by a branch and a
    /// non-hardened index.
    pub(crate) fn place_of(
        &self,
        fingerprint: Fingerprint,
        path: &DerivationPath,
    ) -> Option<Place> {
        self.cosigners.iter().find_map(|cosigner| {
            let steps: &[ChildNumber] = path.as_ref();
            let origin: &[ChildNumber] = cosigner.origin.as_ref();
            if cosigner.fingerprint != fingerprint
                || steps.len() != origin.len() + 2
                || &steps[..origin.len()] != origin
            {
                return None;
            }
            match steps[origin.len()..] {
                [
                    ChildNumber::Normal { index: branch },
                    ChildNumber::Normal { index },
                ] if branch <= 1 => Some((branch, index)),
                _ => None,
            }
        })
    }

    /// The cosigner a BIP-39 phrase (with its passphrase) holds, and its
    /// account private key: the one whose fingerprint is the phrase's and
    /// whose public key the phrase derives along its origin.
    pub(crate) fn cosigner_of_phrase(
        &self,
        chain: Chain,
        phrase: &str,
        passphrase: &str,
    ) -> Result<(usize, Xpriv), DerivationError> {
        let network = chain
            .bitcoin_network()
            .ok_or_else(|| DerivationError::invalid("Multisig wallets are Bitcoin wallets"))?;
        let seed =
            crate::derivation::primitives::derive_bip39_seed(phrase, passphrase, 0, None, None)?;
        let secp = Secp256k1::new();
        let master = Xpriv::new_master(NetworkKind::from(network), seed.as_ref())
            .map_err(DerivationError::invalid)?;
        let fingerprint = master.fingerprint(&secp);
        let index = self
            .cosigners
            .iter()
            .position(|cosigner| cosigner.fingerprint == fingerprint)
            .ok_or_else(|| {
                DerivationError::invalid("This phrase holds none of the wallet's cosigner keys.")
            })?;
        let account = master
            .derive_priv(&secp, &self.cosigners[index].origin)
            .map_err(DerivationError::invalid)?;
        let public = Xpub::from_priv(&secp, &account);
        if public.public_key != self.cosigners[index].key.public_key
            || public.chain_code != self.cosigners[index].key.chain_code
        {
            return Err(DerivationError::invalid(
                "This phrase's key at the cosigner's path is not the descriptor's: check its passphrase.",
            ));
        }
        Ok((index, account))
    }
}

/// `[fingerprint/path]key/<0;1>/*` or `[fingerprint/path]key/0/*`, the key
/// on `network`'s version, its depth and child number its origin's.
fn parse_key(text: &str, network: bitcoin::Network) -> Option<Cosigner> {
    let (origin, rest) = text.strip_prefix('[')?.split_once(']')?;
    let key = rest
        .strip_suffix("/<0;1>/*")
        .or_else(|| rest.strip_suffix("/0/*"))?;
    let mut steps = origin.split('/');
    let fingerprint = Fingerprint::from_str(steps.next()?).ok()?;
    let origin = steps
        .map(|step| {
            let (number, hardened) = match step.strip_suffix(['h', 'H', '\'']) {
                Some(number) => (number, true),
                None => (step, false),
            };
            let index: u32 = number.parse().ok()?;
            if hardened {
                ChildNumber::from_hardened_idx(index).ok()
            } else {
                ChildNumber::from_normal_idx(index).ok()
            }
        })
        .collect::<Option<Vec<_>>>()?;
    let key = Xpub::from_str(key).ok()?;
    let network_kind = NetworkKind::from(network);
    if key.network != network_kind
        || usize::from(key.depth) != origin.len()
        || origin.last().is_some_and(|last| *last != key.child_number)
    {
        return None;
    }
    Some(Cosigner {
        fingerprint,
        origin: DerivationPath::from(origin),
        key,
    })
}

/// BIP-380's descriptor checksum of `descriptor`, or `None` when it holds a
/// character outside the descriptor alphabet.
pub(crate) fn descriptor_checksum(descriptor: &str) -> Option<String> {
    const INPUT: &str = "0123456789()[],'/*abcdefgh@:$%{}IJKLMNOPQRSTUVWXYZ&+-.;<=>?!^_|~ijklmnopqrstuvwxyzABCDEFGH`#\"\\ ";
    const CHECKSUM: &[u8] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";
    const GENERATOR: [u64; 5] = [
        0xf5dee51989,
        0xa9fdca3312,
        0x1bab10e32d,
        0x3706b1677a,
        0x644d626ffd,
    ];
    fn polymod(chk: u64, value: u64) -> u64 {
        let top = chk >> 35;
        let mut chk = ((chk & 0x7ffffffff) << 5) ^ value;
        for (bit, generator) in GENERATOR.iter().enumerate() {
            if (top >> bit) & 1 == 1 {
                chk ^= generator;
            }
        }
        chk
    }
    let mut chk = 1u64;
    let mut groups = Vec::with_capacity(3);
    for character in descriptor.chars() {
        let value = INPUT.find(character)? as u64;
        chk = polymod(chk, value & 31);
        groups.push(value >> 5);
        if groups.len() == 3 {
            chk = polymod(chk, groups[0] * 9 + groups[1] * 3 + groups[2]);
            groups.clear();
        }
    }
    match groups[..] {
        [a] => chk = polymod(chk, a),
        [a, b] => chk = polymod(chk, a * 3 + b),
        _ => {}
    }
    for _ in 0..8 {
        chk = polymod(chk, 0);
    }
    chk ^= 1;
    Some(
        (0..8)
            .map(|i| CHECKSUM[((chk >> (5 * (7 - i))) & 31) as usize] as char)
            .collect(),
    )
}

#[cfg(test)]
#[path = "tests/multisig.rs"]
pub(crate) mod tests;
