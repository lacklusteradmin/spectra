//! Signing a plain-text message to prove an address, and checking such a
//! signature, in each network's own scheme.
//!
//! Every scheme here binds the text to the purpose: Bitcoin's signed-message
//! magic and BIP-322's tagged hash, EIP-191's and TIP-191's prefixes, Sui's
//! personal-message intent and Substrate's `<Bytes>` wrapping, so a signature
//! over a message can never authorise a transaction. Solana signs the bytes
//! as they are, so a message that reads as a Solana transaction is refused;
//! EIP-712 typed data, which can authorise a token permit, is refused on EVM
//! networks rather than signed as text. Stellar, Cardano, Kaspa and Monero
//! sign in their own schemes (`message_schemes.rs`). Networks with no
//! plain-message standard a wallet would check (Aptos, NEAR and TON, whose
//! formats bind a dapp's domain and nonce, XRP, ICP, Decred and Zcash) have
//! no scheme here.

use bitcoin::hashes::Hash as _;
use secp256k1::{Message, Secp256k1, SecretKey};
use sha2::{Digest, Sha256};

use crate::derivation::types::BitcoinScriptType;
use crate::registry::Chain;
use crate::send::error::SendError;

/// How a network's wallets sign and check a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum MessageScheme {
    /// The Bitcoin family's `signmessage` for P2PKH addresses: a recoverable
    /// ECDSA signature over the network's signed-message magic, in base64.
    SignedMessage,
    /// BIP-322 simple signatures for Bitcoin's SegWit and Taproot addresses:
    /// the witness of a virtual spend, in base64.
    Bip322,
    /// EIP-191 `personal_sign`: a recoverable signature over the Ethereum
    /// signed-message prefix, in hex.
    PersonalSign,
    /// TIP-191, Tron's `signMessageV2`: as EIP-191 with Tron's prefix.
    TronSignedMessage,
    /// Solana's `signMessage`: Ed25519 over the message bytes, in base58.
    SolanaMessage,
    /// Sui's personal message: Ed25519 over the intent-prefixed digest, as
    /// Sui's serialized signature in base64.
    SuiPersonalMessage,
    /// Substrate's `signRaw`: sr25519 over the message wrapped in `<Bytes>`,
    /// in hex.
    SubstrateBytes,
    /// SEP-53: Ed25519 over SHA-256 of Stellar's signed-message prefix and
    /// the message, in base64.
    StellarSignedMessage,
    /// CIP-8 as CIP-30's `signData` returns it: a COSE_Sign1 naming the
    /// address and the COSE_Key, in a `{"signature", "key"}` object.
    CardanoDataSignature,
    /// Kaspa's personal message: Schnorr over a keyed Blake2b hash, in hex.
    KaspaPersonalMessage,
    /// Monero's `SigV2`: wallet2's spend-key signature.
    MoneroSignature,
}

/// The scheme `address` on `chain` signs messages in, or `None`. Reads only
/// the address, so a page can say whether a wallet signs before any secret
/// is opened.
pub fn scheme_for(chain: Chain, address: &str) -> Option<MessageScheme> {
    let family = chain.mainnet_counterpart();
    if chain.is_evm() {
        return Some(MessageScheme::PersonalSign);
    }
    match family {
        Chain::Tron => Some(MessageScheme::TronSignedMessage),
        Chain::Solana => Some(MessageScheme::SolanaMessage),
        Chain::Sui => Some(MessageScheme::SuiPersonalMessage),
        Chain::Polkadot | Chain::Bittensor => Some(MessageScheme::SubstrateBytes),
        Chain::Stellar => Some(MessageScheme::StellarSignedMessage),
        Chain::Cardano => {
            schemes::cardano::signs(address).then_some(MessageScheme::CardanoDataSignature)
        }
        Chain::Kaspa => {
            schemes::kaspa::signs(chain, address).then_some(MessageScheme::KaspaPersonalMessage)
        }
        Chain::Monero => schemes::monero::signs(address).then_some(MessageScheme::MoneroSignature),
        _ if magic(chain).is_some() => {
            if is_p2pkh(chain, address) {
                Some(MessageScheme::SignedMessage)
            } else if chain.accepts_account_xpub() && bitcoin_script(chain, address).is_some() {
                Some(MessageScheme::Bip322)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// The signed-message magic each network's own node prefixes.
fn magic(chain: Chain) -> Option<&'static str> {
    Some(match chain.mainnet_counterpart() {
        Chain::Bitcoin | Chain::BitcoinCash | Chain::BitcoinSV => "Bitcoin Signed Message:\n",
        Chain::Litecoin => "Litecoin Signed Message:\n",
        Chain::Dogecoin => "Dogecoin Signed Message:\n",
        Chain::Dash => "DarkCoin Signed Message:\n",
        Chain::BitcoinGold => "Bitcoin Gold Signed Message:\n",
        Chain::Peercoin => "Peercoin Signed Message:\n",
        _ => return None,
    })
}

/// The Base58Check version a P2PKH address on `chain` carries, read off the
/// chain's own encoding of a key.
fn p2pkh_version(chain: Chain) -> Option<Vec<u8>> {
    let generator =
        secp256k1::PublicKey::from_secret_key_global(&SecretKey::from_slice(&[1; 32]).ok()?);
    let address = chain
        .encode_discovery_address(&generator, BitcoinScriptType::P2pkh)
        .ok()?;
    let payload = bs58::decode(address).with_check(None).into_vec().ok()?;
    Some(payload[..payload.len().checked_sub(20)?].to_vec())
}

fn is_p2pkh(chain: Chain, address: &str) -> bool {
    let Some(version) = p2pkh_version(chain) else {
        return false;
    };
    bs58::decode(address)
        .with_check(None)
        .into_vec()
        .is_ok_and(|payload| payload.len() == version.len() + 20 && payload.starts_with(&version))
}

/// The output script of a Bitcoin address on `chain`'s network.
fn bitcoin_script(chain: Chain, address: &str) -> Option<bitcoin::ScriptBuf> {
    let network = if chain.is_testnet() {
        bitcoin::Network::Testnet
    } else {
        bitcoin::Network::Bitcoin
    };
    let address: bitcoin::Address<bitcoin::address::NetworkUnchecked> = address.parse().ok()?;
    let address = address.require_network(network).ok()?;
    let script = address.script_pubkey();
    (script.is_p2wpkh() || script.is_p2sh() || script.is_p2tr()).then_some(script)
}

/// Refuse a message the scheme would sign as something else.
fn check_is_message(chain: Chain, message: &str) -> Result<(), SendError> {
    if chain.is_evm() && is_typed_data(message) {
        return Err(SendError::Invalid(
            "This is EIP-712 typed data, which can authorise spending. Spectra signs only plain messages."
                .into(),
        ));
    }
    if chain.mainnet_counterpart() == Chain::Solana && is_solana_transaction(message.as_bytes()) {
        return Err(SendError::Invalid(
            "This message reads as a Solana transaction. Spectra signs only plain messages.".into(),
        ));
    }
    Ok(())
}

/// EIP-712 typed data: a JSON object naming its domain, types and primary
/// type.
fn is_typed_data(message: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(message.trim()).is_ok_and(|value| {
        ["domain", "types", "primaryType"]
            .iter()
            .all(|key| value.get(key).is_some())
    })
}

/// Whether `bytes` are exactly a Solana transaction message, legacy or
/// versioned, which a signature over them would authorise.
fn is_solana_transaction(bytes: &[u8]) -> bool {
    fn compact(bytes: &[u8], at: &mut usize) -> Option<usize> {
        let mut value = 0usize;
        for shift in [0, 7, 14] {
            let byte = *bytes.get(*at)?;
            *at += 1;
            value |= usize::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }
    fn skip(bytes: &[u8], at: &mut usize, count: usize) -> Option<()> {
        *at = at.checked_add(count)?;
        (*at <= bytes.len()).then_some(())
    }
    let parse = || -> Option<usize> {
        let mut at = 0;
        let versioned = *bytes.first()? & 0x80 != 0;
        if versioned {
            at += 1;
        }
        skip(bytes, &mut at, 3)?;
        let keys = compact(bytes, &mut at)?;
        skip(bytes, &mut at, keys.checked_mul(32)?)?;
        skip(bytes, &mut at, 32)?;
        for _ in 0..compact(bytes, &mut at)? {
            skip(bytes, &mut at, 1)?;
            let accounts = compact(bytes, &mut at)?;
            skip(bytes, &mut at, accounts)?;
            let data = compact(bytes, &mut at)?;
            skip(bytes, &mut at, data)?;
        }
        if versioned {
            for _ in 0..compact(bytes, &mut at)? {
                skip(bytes, &mut at, 32)?;
                let writable = compact(bytes, &mut at)?;
                skip(bytes, &mut at, writable)?;
                let readonly = compact(bytes, &mut at)?;
                skip(bytes, &mut at, readonly)?;
            }
        }
        Some(at)
    };
    parse() == Some(bytes.len())
}

/// Sign `message` with the key behind `address`, in `chain`'s scheme.
/// `key_hex` is the 32-byte secret (an Ed25519 seed, an sr25519 mini
/// secret or a secp256k1 scalar) the wallet signs transactions with.
pub(crate) fn sign_message(
    chain: Chain,
    address: &str,
    key_hex: &str,
    message: &str,
) -> Result<(MessageScheme, String), SendError> {
    let scheme = scheme_for(chain, address).ok_or_else(|| {
        SendError::Invalid("This network has no message-signing standard.".into())
    })?;
    check_is_message(chain, message)?;
    let key = zeroize::Zeroizing::new(
        hex::decode(key_hex.trim_start_matches("0x"))
            .map_err(|_| SendError::Invalid("signing key is not hex".into()))?,
    );
    // Cardano signs with the whole extended key, kL ‖ kR.
    if scheme == MessageScheme::CardanoDataSignature {
        let extended: zeroize::Zeroizing<[u8; 64]> = zeroize::Zeroizing::new(
            key.get(..64)
                .and_then(|key| key.try_into().ok())
                .ok_or_else(|| SendError::Invalid("Cardano key must be 64 bytes".into()))?,
        );
        return Ok((scheme, schemes::cardano::sign(&extended, address, message)?));
    }
    let seed: zeroize::Zeroizing<[u8; 32]> = zeroize::Zeroizing::new(
        key.get(..32)
            .and_then(|seed| seed.try_into().ok())
            .ok_or_else(|| SendError::Invalid("signing key must be 32 bytes".into()))?,
    );
    let signature = match scheme {
        MessageScheme::SignedMessage => {
            let secret = secret_key(&seed)?;
            let digest = signed_message_digest(magic(chain).unwrap_or_default(), message);
            let (recovery, compact) = Secp256k1::signing_only()
                .sign_ecdsa_recoverable(&Message::from_digest(digest), &secret)
                .serialize_compact();
            // Header: 27 + recovery id, + 4 for a compressed key.
            let mut bytes = vec![31 + recovery.to_i32() as u8];
            bytes.extend_from_slice(&compact);
            base64_encode(&bytes)
        }
        MessageScheme::Bip322 => bip322::sign(chain, address, &secret_key(&seed)?, message)?,
        MessageScheme::PersonalSign | MessageScheme::TronSignedMessage => {
            let secret = secret_key(&seed)?;
            let digest = prefixed_keccak(scheme, message);
            let (recovery, compact) = Secp256k1::signing_only()
                .sign_ecdsa_recoverable(&Message::from_digest(digest), &secret)
                .serialize_compact();
            let mut bytes = compact.to_vec();
            bytes.push(27 + recovery.to_i32() as u8);
            format!("0x{}", hex::encode(bytes))
        }
        MessageScheme::SolanaMessage => {
            use ed25519_dalek::Signer as _;
            let key = ed25519_dalek::SigningKey::from_bytes(&seed);
            bs58::encode(key.sign(message.as_bytes()).to_bytes()).into_string()
        }
        MessageScheme::SuiPersonalMessage => {
            use ed25519_dalek::Signer as _;
            let key = ed25519_dalek::SigningKey::from_bytes(&seed);
            let signature = key.sign(&sui_personal_digest(message));
            let mut bytes = vec![0x00];
            bytes.extend_from_slice(&signature.to_bytes());
            bytes.extend_from_slice(key.verifying_key().as_bytes());
            base64_encode(&bytes)
        }
        MessageScheme::SubstrateBytes => {
            let pair = schnorrkel::MiniSecretKey::from_bytes(&*seed)
                .map_err(|e| SendError::Invalid(format!("sr25519 key: {e}").into()))?
                .expand_to_keypair(schnorrkel::ExpansionMode::Ed25519);
            let signature = pair
                .sign(schnorrkel::signing_context(b"substrate").bytes(&substrate_wrapped(message)));
            format!("0x{}", hex::encode(signature.to_bytes()))
        }
        MessageScheme::StellarSignedMessage => schemes::stellar::sign(&seed, message),
        MessageScheme::KaspaPersonalMessage => schemes::kaspa::sign(&seed, message)?,
        // A Monero key is the spend key and then the view key.
        MessageScheme::MoneroSignature => schemes::monero::sign(&seed, address, message)?,
        MessageScheme::CardanoDataSignature => unreachable!("signed with the extended key above"),
    };
    Ok((scheme, signature))
}

/// Whether `signature` is `address`'s over `message`, in `chain`'s scheme.
/// A signature that does not decode is not one.
#[uniffi::export]
pub fn verify_message(chain: Chain, address: String, message: String, signature: String) -> bool {
    let address = address.trim();
    let signature = signature.trim();
    let Some(scheme) = scheme_for(chain, address) else {
        return false;
    };
    match scheme {
        MessageScheme::SignedMessage => {
            let digest = signed_message_digest(magic(chain).unwrap_or_default(), &message);
            let Some(bytes) = base64_decode(signature).filter(|bytes| bytes.len() == 65) else {
                return false;
            };
            let Some(key) = recover(digest, bytes[0].wrapping_sub(27) & 3, &bytes[1..65]) else {
                return false;
            };
            // A header for an uncompressed key names another address.
            (31..=34).contains(&bytes[0])
                && chain
                    .encode_discovery_address(&key, BitcoinScriptType::P2pkh)
                    .is_ok_and(|derived| derived == address)
        }
        MessageScheme::Bip322 => bip322::verify(chain, address, &message, signature),
        MessageScheme::PersonalSign | MessageScheme::TronSignedMessage => {
            let digest = prefixed_keccak(scheme, &message);
            let Some(bytes) = hex::decode(signature.trim_start_matches("0x"))
                .ok()
                .filter(|bytes| bytes.len() == 65)
            else {
                return false;
            };
            let recovery = match bytes[64] {
                v @ (27 | 28) => v - 27,
                v @ (0 | 1) => v,
                _ => return false,
            };
            let Some(key) = recover(digest, recovery, &bytes[..64]) else {
                return false;
            };
            let derived = if scheme == MessageScheme::PersonalSign {
                let hash = crate::derivation::evm::keccak256(&key.serialize_uncompressed()[1..]);
                format!("0x{}", hex::encode(&hash[12..]))
            } else {
                crate::derivation::tron::address_from_public_key(&key)
            };
            if scheme == MessageScheme::PersonalSign {
                derived.eq_ignore_ascii_case(address)
            } else {
                derived == address
            }
        }
        MessageScheme::SolanaMessage => {
            let Some(public) = bs58::decode(address).into_vec().ok() else {
                return false;
            };
            let Some(bytes) = bs58::decode(signature).into_vec().ok() else {
                return false;
            };
            ed25519_verify(&public, message.as_bytes(), &bytes)
        }
        MessageScheme::SuiPersonalMessage => {
            let Some(bytes) = base64_decode(signature).filter(|bytes| bytes.len() == 97) else {
                return false;
            };
            bytes[0] == 0x00
                && crate::derivation::sui::address_from_public_key(
                    bytes[65..97].try_into().unwrap_or(&[0; 32]),
                ) == address
                && ed25519_verify(
                    &bytes[65..97],
                    &sui_personal_digest(&message),
                    &bytes[1..65],
                )
        }
        MessageScheme::SubstrateBytes => {
            // The prefix Spectra encodes the network's addresses with.
            let prefix = if chain == Chain::Polkadot { 0 } else { 42 };
            let Ok((_, public)) = crate::derivation::primitives::decode_ss58(address, Some(prefix))
            else {
                return false;
            };
            let Some(bytes) = hex::decode(signature.trim_start_matches("0x")).ok() else {
                return false;
            };
            let (Ok(public), Ok(signature)) = (
                schnorrkel::PublicKey::from_bytes(&public),
                schnorrkel::Signature::from_bytes(&bytes),
            ) else {
                return false;
            };
            public
                .verify(
                    schnorrkel::signing_context(b"substrate").bytes(&substrate_wrapped(&message)),
                    &signature,
                )
                .is_ok()
        }
        MessageScheme::StellarSignedMessage => {
            schemes::stellar::verify(address, &message, signature)
        }
        MessageScheme::CardanoDataSignature => {
            schemes::cardano::verify(address, &message, signature)
        }
        MessageScheme::KaspaPersonalMessage => {
            schemes::kaspa::verify(chain, address, &message, signature)
        }
        MessageScheme::MoneroSignature => schemes::monero::verify(address, &message, signature),
    }
}

#[path = "message_schemes.rs"]
mod schemes;

fn secret_key(seed: &[u8; 32]) -> Result<SecretKey, SendError> {
    SecretKey::from_slice(seed)
        .map_err(|e| SendError::Invalid(format!("secp256k1 key: {e}").into()))
}

fn recover(digest: [u8; 32], recovery: u8, compact: &[u8]) -> Option<secp256k1::PublicKey> {
    let id = secp256k1::ecdsa::RecoveryId::from_i32(i32::from(recovery)).ok()?;
    let signature = secp256k1::ecdsa::RecoverableSignature::from_compact(compact, id).ok()?;
    Secp256k1::verification_only()
        .recover_ecdsa(&Message::from_digest(digest), &signature)
        .ok()
}

fn ed25519_verify(public: &[u8], message: &[u8], signature: &[u8]) -> bool {
    let (Ok(public), Ok(signature)) = (
        <[u8; 32]>::try_from(public),
        <[u8; 64]>::try_from(signature),
    ) else {
        return false;
    };
    ed25519_dalek::VerifyingKey::from_bytes(&public).is_ok_and(|key| {
        key.verify_strict(message, &ed25519_dalek::Signature::from_bytes(&signature))
            .is_ok()
    })
}

/// SHA-256d of the magic and the message, each after its varint length.
fn signed_message_digest(magic: &str, message: &str) -> [u8; 32] {
    use crate::send::bitcoin_wire::{dsha256, varint};
    let mut data = varint(magic.len());
    data.extend_from_slice(magic.as_bytes());
    data.extend(varint(message.len()));
    data.extend_from_slice(message.as_bytes());
    dsha256(&data)
}

/// Keccak-256 of the scheme's prefix, the message's decimal length and the
/// message.
fn prefixed_keccak(scheme: MessageScheme, message: &str) -> [u8; 32] {
    use sha3::{Digest as _, Keccak256};
    let prefix = if scheme == MessageScheme::TronSignedMessage {
        "\x19TRON Signed Message:\n"
    } else {
        "\x19Ethereum Signed Message:\n"
    };
    let mut hasher = Keccak256::new();
    hasher.update(prefix.as_bytes());
    hasher.update(message.len().to_string().as_bytes());
    hasher.update(message.as_bytes());
    hasher.finalize().into()
}

/// Blake2b-256 of the personal-message intent and the message as BCS bytes.
fn sui_personal_digest(message: &str) -> [u8; 32] {
    use blake2::digest::{Update, VariableOutput};
    let mut data = vec![3, 0, 0];
    let mut length = message.len();
    loop {
        let byte = (length & 0x7f) as u8;
        length >>= 7;
        if length == 0 {
            data.push(byte);
            break;
        }
        data.push(byte | 0x80);
    }
    data.extend_from_slice(message.as_bytes());
    let mut hasher = blake2::Blake2bVar::new(32).expect("32 is a valid Blake2b length");
    hasher.update(&data);
    let mut digest = [0u8; 32];
    hasher
        .finalize_variable(&mut digest)
        .expect("the buffer is the requested length");
    digest
}

fn substrate_wrapped(message: &str) -> Vec<u8> {
    [b"<Bytes>".as_slice(), message.as_bytes(), b"</Bytes>"].concat()
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.decode(text).ok()
}

/// BIP-322 simple signatures: a virtual transaction spends an output to the
/// address, committing to the message, and the signature is the witness of
/// a second transaction spending it.
mod bip322 {
    use super::*;
    use bitcoin::absolute::LockTime;
    use bitcoin::sighash::{EcdsaSighashType, Prevouts, SighashCache, TapSighashType};
    use bitcoin::transaction::Version;
    use bitcoin::{Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness};

    /// The tagged hash of the message.
    fn message_hash(message: &str) -> [u8; 32] {
        let tag = Sha256::digest(b"BIP0322-signed-message");
        let mut hasher = Sha256::new();
        hasher.update(tag);
        hasher.update(tag);
        hasher.update(message.as_bytes());
        hasher.finalize().into()
    }

    /// The transaction to sign, spending the virtual output to `script`.
    fn to_sign(script: &ScriptBuf, message: &str) -> Transaction {
        let mut script_sig = vec![0x00, 0x20];
        script_sig.extend_from_slice(&message_hash(message));
        let to_spend = Transaction {
            version: Version(0),
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint {
                    txid: bitcoin::Txid::all_zeros(),
                    vout: 0xFFFF_FFFF,
                },
                script_sig: ScriptBuf::from_bytes(script_sig),
                sequence: Sequence(0),
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::ZERO,
                script_pubkey: script.clone(),
            }],
        };
        Transaction {
            version: Version(0),
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint {
                    txid: to_spend.compute_txid(),
                    vout: 0,
                },
                script_sig: ScriptBuf::new(),
                sequence: Sequence(0),
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::ZERO,
                script_pubkey: ScriptBuf::from_bytes(vec![0x6a]),
            }],
        }
    }

    /// The witness as the simple signature serializes it.
    fn encode_witness(witness: &Witness) -> String {
        base64_encode(&bitcoin::consensus::serialize(witness))
    }

    pub(super) fn sign(
        chain: Chain,
        address: &str,
        secret: &SecretKey,
        message: &str,
    ) -> Result<String, SendError> {
        let script = bitcoin_script(chain, address)
            .ok_or_else(|| SendError::Invalid("not a SegWit or Taproot address".into()))?;
        let secp = Secp256k1::new();
        let public = bitcoin::PublicKey::new(secret.public_key(&secp));
        let mut tx = to_sign(&script, message);
        let prevouts = [TxOut {
            value: Amount::ZERO,
            script_pubkey: script.clone(),
        }];
        let witness = if script.is_p2tr() {
            use bitcoin::key::TapTweak as _;
            let tweaked =
                bitcoin::key::Keypair::from_secret_key(&secp, secret).tap_tweak(&secp, None);
            if ScriptBuf::new_p2tr_tweaked(
                tweaked
                    .to_keypair()
                    .x_only_public_key()
                    .0
                    .dangerous_assume_tweaked(),
            ) != script
            {
                return Err(SendError::Invalid(
                    "the key does not own this address".into(),
                ));
            }
            let sighash = SighashCache::new(&tx)
                .taproot_key_spend_signature_hash(
                    0,
                    &Prevouts::All(&prevouts),
                    TapSighashType::Default,
                )
                .map_err(|e| SendError::Internal(format!("sighash: {e}")))?;
            let signature = secp.sign_schnorr_no_aux_rand(
                &Message::from_digest(sighash.to_byte_array()),
                &tweaked.to_keypair(),
            );
            Witness::from_slice(&[signature.as_ref().to_vec()])
        } else {
            let wpkh = ScriptBuf::new_p2wpkh(
                &public
                    .wpubkey_hash()
                    .map_err(|e| SendError::Invalid(format!("key: {e}").into()))?,
            );
            let owns = if script.is_p2sh() {
                wpkh.to_p2sh() == script
            } else {
                wpkh == script
            };
            if !owns {
                return Err(SendError::Invalid(
                    "the key does not own this address".into(),
                ));
            }
            if script.is_p2sh() {
                let mut script_sig = vec![wpkh.len() as u8];
                script_sig.extend_from_slice(wpkh.as_bytes());
                tx.input[0].script_sig = ScriptBuf::from_bytes(script_sig);
            }
            let sighash = SighashCache::new(&tx)
                .p2wpkh_signature_hash(0, &wpkh, Amount::ZERO, EcdsaSighashType::All)
                .map_err(|e| SendError::Internal(format!("sighash: {e}")))?;
            let mut signature = secp
                .sign_ecdsa(&Message::from_digest(sighash.to_byte_array()), secret)
                .serialize_der()
                .to_vec();
            signature.push(EcdsaSighashType::All as u8);
            Witness::from_slice(&[signature, public.to_bytes()])
        };
        Ok(encode_witness(&witness))
    }

    pub(super) fn verify(chain: Chain, address: &str, message: &str, signature: &str) -> bool {
        let Some(script) = bitcoin_script(chain, address) else {
            return false;
        };
        let Some(witness) = base64_decode(signature)
            .and_then(|bytes| bitcoin::consensus::deserialize::<Witness>(&bytes).ok())
        else {
            return false;
        };
        let secp = Secp256k1::verification_only();
        let tx = to_sign(&script, message);
        if script.is_p2tr() {
            let Some(item) = witness.nth(0).filter(|_| witness.len() == 1) else {
                return false;
            };
            let (bytes, sighash_type) = match item.len() {
                64 => (item, TapSighashType::Default),
                65 => match TapSighashType::from_consensus_u8(item[64]) {
                    Ok(kind) if kind != TapSighashType::Default => (&item[..64], kind),
                    _ => return false,
                },
                _ => return false,
            };
            let prevouts = [TxOut {
                value: Amount::ZERO,
                script_pubkey: script.clone(),
            }];
            let Ok(sighash) = SighashCache::new(&tx).taproot_key_spend_signature_hash(
                0,
                &Prevouts::All(&prevouts),
                sighash_type,
            ) else {
                return false;
            };
            let (Ok(key), Ok(signature)) = (
                secp256k1::XOnlyPublicKey::from_slice(&script.as_bytes()[2..34]),
                secp256k1::schnorr::Signature::from_slice(bytes),
            ) else {
                return false;
            };
            return secp
                .verify_schnorr(
                    &signature,
                    &Message::from_digest(sighash.to_byte_array()),
                    &key,
                )
                .is_ok();
        }
        let (Some(signature), Some(public)) = (witness.nth(0), witness.nth(1)) else {
            return false;
        };
        let Ok(public) = bitcoin::PublicKey::from_slice(public) else {
            return false;
        };
        let Ok(hash) = public.wpubkey_hash() else {
            return false;
        };
        let wpkh = ScriptBuf::new_p2wpkh(&hash);
        let owns = if script.is_p2sh() {
            wpkh.to_p2sh() == script
        } else {
            wpkh == script
        };
        let Some((&sighash_byte, der)) = signature.split_last() else {
            return false;
        };
        let Ok(sighash_type) = EcdsaSighashType::from_standard(u32::from(sighash_byte)) else {
            return false;
        };
        let Ok(sighash) =
            SighashCache::new(&tx).p2wpkh_signature_hash(0, &wpkh, Amount::ZERO, sighash_type)
        else {
            return false;
        };
        let Ok(signature) = secp256k1::ecdsa::Signature::from_der(der) else {
            return false;
        };
        owns && witness.len() == 2
            && secp
                .verify_ecdsa(
                    &Message::from_digest(sighash.to_byte_array()),
                    &signature,
                    &public.inner,
                )
                .is_ok()
    }
}

#[cfg(test)]
#[path = "tests/message.rs"]
mod tests;
