//! Network-specific addresses for raw signing keys and seeds, including Cardano extended keys.
use super::types::DerivationResult;
use crate::{error::SpectraBridgeError, registry::Chain};

pub(super) fn derive(
    chain: Chain,
    key_hex: &str,
    want_address: bool,
    want_public_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    // A Hedera key is stored in the DER that names its scheme.
    if chain.is_hedera() {
        let wants = super::types::Wants {
            address: want_address,
            public_key: want_public_key,
            private_key: false,
        };
        return Ok(super::hedera::derive_from_private_key(key_hex, wants)?);
    }
    let bytes = zeroize::Zeroizing::new(hex::decode(key_hex)?);
    if chain.mainnet_counterpart() == Chain::Cardano {
        let key: &[u8; 64] = bytes.as_slice().try_into().map_err(|_| {
            SpectraBridgeError::invalid(
                "Cardano requires a 64-byte extended private key (kL || kR)",
            )
        })?;
        let public = super::ed25519_extended::public_key(key)?;
        return Ok(DerivationResult {
            address: if want_address {
                Some(super::cardano::derive_cardano_shelley_enterprise_address(
                    &public,
                    !chain.is_testnet(),
                )?)
            } else {
                None
            },
            public_key_hex: want_public_key.then(|| hex::encode(public)),
            private_key_hex: None,
        });
    }
    if chain.is_starknet() {
        let private = super::starknet::private_key(&bytes)?;
        let public = super::starknet::public_key(&private);
        return Ok(DerivationResult {
            address: if want_address {
                Some(super::starknet::felt_hex(
                    &super::starknet::account_address(chain, &public)?,
                ))
            } else {
                None
            },
            public_key_hex: want_public_key.then(|| hex::encode(public.to_bytes_be())),
            private_key_hex: None,
        });
    }
    let seed: &[u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| SpectraBridgeError::invalid("Private key must be exactly 32 bytes"))?;
    let family = chain.mainnet_counterpart();
    let public = match family {
        _ if chain.is_substrate() => Some(
            schnorrkel::MiniSecretKey::from_bytes(seed)
                .map_err(SpectraBridgeError::failure)?
                .expand_to_keypair(schnorrkel::ExpansionMode::Ed25519)
                .public
                .to_bytes(),
        ),
        Chain::Solana
        | Chain::Stellar
        | Chain::Sui
        | Chain::Aptos
        | Chain::Ton
        | Chain::Near
        | Chain::Algorand
        | Chain::Tezos => Some(
            ed25519_dalek::SigningKey::from_bytes(seed)
                .verifying_key()
                .to_bytes(),
        ),
        _ => None,
    };
    if let Some(public) = public {
        let address = if want_address {
            Some(match family {
                Chain::Solana => bs58::encode(public).into_string(),
                Chain::Stellar => super::stellar::address_from_public_key(&public),
                Chain::Sui => super::sui::address_from_public_key(&public),
                Chain::Aptos => super::aptos::address_from_public_key(&public),
                Chain::Ton => super::ton::TonWalletVersion::default().address(&public, chain)?,
                Chain::Near => hex::encode(public),
                Chain::Algorand => super::algorand::address_from_public_key(&public),
                Chain::Tezos => super::tezos::address_from_public_key(&public),
                _ if chain.is_substrate() => super::primitives::encode_ss58(
                    &public,
                    chain.ss58_prefix().expect("a Substrate network"),
                ),
                _ => unreachable!("public seed key only exists for these protocols"),
            })
        } else {
            None
        };
        return Ok(DerivationResult {
            address,
            public_key_hex: want_public_key.then(|| hex::encode(public)),
            private_key_hex: None,
        });
    }
    let secret = secp256k1::SecretKey::from_slice(&bytes).map_err(SpectraBridgeError::failure)?;
    let key = secp256k1::PublicKey::from_secret_key(&secp256k1::Secp256k1::new(), &secret);
    let address = if want_address {
        Some(match chain.mainnet_counterpart() {
            Chain::Tron => super::tron::address_from_public_key(&key),
            Chain::Icp => hex::encode(super::icp::account_from_principal(&super::icp::principal(
                &key,
            ))),
            Chain::Xrp => super::xrp::address_from_public_key(&key)?,
            _ if chain.is_cosmos() => super::cosmos::address_from_public_key(chain, &key)?,
            Chain::Filecoin => super::filecoin::address_from_public_key(chain, &key)?,
            _ => {
                let path = super::path::default_path_from_catalog(chain)?;
                chain
                    .encode_discovery_address(&key, super::dispatch::script_type_for_path(&path))?
            }
        })
    } else {
        None
    };
    Ok(DerivationResult {
        address,
        // A Filecoin key is written as the uncompressed point its address
        // hashes.
        public_key_hex: want_public_key.then(|| match chain.mainnet_counterpart() {
            Chain::Filecoin => hex::encode(key.serialize_uncompressed()),
            _ => hex::encode(key.serialize()),
        }),
        private_key_hex: None,
    })
}
