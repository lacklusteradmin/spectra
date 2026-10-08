//! MWEB keys and stealth addresses.
//!
//! A wallet's MWEB keys are the BIP-32 keys at `m/1000'/0'` (scan) and
//! `m/1000'/1'` (spend) of its BIP-39 seed, where Cake Wallet derives them,
//! so a phrase restored here finds what it received there. Address `i` is
//! `(Aᵢ, Bᵢ)` with `Bᵢ = B + mᵢ·G`, `mᵢ = BLAKE3('A' ‖ i ‖ a)` (i as four
//! little-endian bytes), and `Aᵢ = a·Bᵢ`; it is written as bech32 (not
//! bech32m) of witness version 0 and the 66 bytes `Aᵢ ‖ Bᵢ`, under `ltcmweb`
//! on mainnet and `tmweb` on testnet, with no length limit.

use bech32::primitives::decode::CheckedHrpstring;
use bech32::{Bech32, ByteIterExt, Fe32, Fe32IterExt, Hrp};
use secp256k1::{PublicKey, SecretKey};
use zeroize::Zeroizing;

use super::primitives::{self, hashed, tag};
use crate::registry::Chain;
use crate::send::error::SendError;

/// Address 0 receives change, as Litecoin Core reserves it.
pub(crate) const CHANGE_INDEX: u32 = 0;
/// Address 1 receives peg-ins, as Litecoin Core reserves it.
pub(crate) const PEGIN_INDEX: u32 = 1;
/// The address a wallet shows to receive at: the first Litecoin Core does
/// not reserve.
pub(crate) const RECEIVE_INDEX: u32 = 2;
/// How many addresses a scan recognizes. An output to a scan key at any
/// other spend key is not the wallet's: anyone who knows one address can
/// scale both of its keys into another that scans under the same key.
pub(crate) const ADDRESS_LOOKAHEAD: u32 = 1_000;

/// What scanning needs: the scan secret and the spend public key.
#[derive(Clone)]
pub(crate) struct ViewKeys {
    pub scan: SecretKey,
    pub spend: PublicKey,
}

/// A stealth address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StealthAddress {
    pub scan: PublicKey,
    pub spend: PublicKey,
}

fn tweak(scan: &SecretKey, index: u32) -> [u8; 32] {
    let mut preimage = index.to_le_bytes().to_vec();
    preimage.extend(scan.secret_bytes());
    hashed(tag::ADDRESS, &preimage)
}

impl ViewKeys {
    /// The wallet's MWEB keys from its BIP-39 seed: the scan secret, and the
    /// spend secret beside the view keys.
    pub(crate) fn from_seed(seed: &[u8]) -> Result<(Self, Zeroizing<[u8; 32]>), SendError> {
        let derive = |path: &str| -> Result<SecretKey, SendError> {
            let master = crate::derivation::bitcoin::ExtendedPrivateKey::master_from_seed(
                b"Bitcoin seed",
                seed,
            )?;
            let path = crate::derivation::bitcoin::parse_bip32_path(path)?;
            Ok(master.derive_path(secp256k1::SECP256K1, &path)?.private_key)
        };
        let scan = derive("m/1000'/0'")?;
        let spend = derive("m/1000'/1'")?;
        let spend_secret = Zeroizing::new(spend.secret_bytes());
        Ok((
            Self {
                scan,
                spend: primitives::public(&spend),
            },
            spend_secret,
        ))
    }

    pub(crate) fn address(&self, index: u32) -> Result<StealthAddress, SendError> {
        let tweak = primitives::secret(tweak(&self.scan, index))?;
        let spend = primitives::add(&self.spend, &primitives::public(&tweak))?;
        Ok(StealthAddress {
            scan: primitives::mul(&spend, &self.scan.secret_bytes())?,
            spend,
        })
    }

    /// The spend public key of every recognized address, by index.
    pub(crate) fn spend_keys(
        &self,
    ) -> Result<std::collections::HashMap<PublicKey, u32>, SendError> {
        (0..ADDRESS_LOOKAHEAD)
            .map(|index| Ok((self.address(index)?.spend, index)))
            .collect()
    }
}

/// The secret of address `index`'s spend key: `b + mᵢ`.
pub(crate) fn address_spend_secret(
    view: &ViewKeys,
    spend_secret: &[u8; 32],
    index: u32,
) -> Result<SecretKey, SendError> {
    primitives::add_secrets(
        &primitives::secret(*spend_secret)?,
        &primitives::secret(tweak(&view.scan, index))?,
    )
}

fn hrp(chain: Chain) -> Result<Hrp, SendError> {
    match chain {
        Chain::Litecoin => Ok(Hrp::parse_unchecked("ltcmweb")),
        Chain::LitecoinTestnet => Ok(Hrp::parse_unchecked("tmweb")),
        _ => Err(SendError::invalid("Not a Litecoin network")),
    }
}

impl StealthAddress {
    pub(crate) fn encode(&self, chain: Chain) -> Result<String, SendError> {
        let mut payload = self.scan.serialize().to_vec();
        payload.extend(self.spend.serialize());
        let hrp = hrp(chain)?;
        Ok(std::iter::once(Fe32::Q)
            .chain(payload.into_iter().bytes_to_fes())
            .with_checksum::<Bech32>(&hrp)
            .chars()
            .collect())
    }

    /// An MWEB address on `chain`'s network, or `None`.
    pub(crate) fn decode(chain: Chain, address: &str) -> Option<Self> {
        let expected = hrp(chain).ok()?;
        let checked = CheckedHrpstring::new::<Bech32>(address.trim()).ok()?;
        if checked.hrp() != expected {
            return None;
        }
        let mut fes = checked.fe32_iter::<std::vec::IntoIter<u8>>();
        if fes.next()? != Fe32::Q {
            return None;
        }
        let bytes: Vec<u8> = fes.fes_to_bytes().collect();
        if bytes.len() != 66 {
            return None;
        }
        let address_keys = Self {
            scan: PublicKey::from_slice(&bytes[..33]).ok()?,
            spend: PublicKey::from_slice(&bytes[33..]).ok()?,
        };
        // One spelling: lowercase, and padding bits zero.
        (address_keys.encode(chain).ok()? == address.trim().to_ascii_lowercase())
            .then_some(address_keys)
    }
}
