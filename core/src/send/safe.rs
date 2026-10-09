//! Safe (safe-global, formerly Gnosis Safe) multisig accounts on EVM
//! networks: a proxy contract whose owners sign a `SafeTx` (EIP-712) and
//! whose `execTransaction` runs it once a threshold of them has.
//!
//! Only a proxy Safe's own factories deploy, delegating to an official
//! singleton of a supported version, is accepted: its owners, threshold and
//! nonce are what the contract says, and what it runs is what they signed.
//! Signatures are verified here before they count: an ECDSA signature over
//! the transaction's hash (`v` 27 or 28) or over its `eth_sign` digest (`v`
//! 31 or 32), recovered to an owner. Approved-hash (`v` 1) and contract
//! (`v` 0) signatures only the Safe itself can check, so they are refused.
//!
//! Signed transactions travel as data, in the shape Safe's own SDK holds
//! one: the Safe, its network, its version, the `SafeTransactionData` and
//! the signatures by signer. Safe's hosted Transaction Service, which needs
//! an API key, is never asked.

use num_bigint::BigUint;
use serde::{Deserialize, Serialize};

use crate::derivation::evm::keccak256;
use crate::send::error::SendError;

pub(crate) type EvmAddress = [u8; 20];

/// The Safe versions accepted, each with its singletons: `Safe` and
/// `SafeL2`, at every address `safe-deployments` 1.37.63 lists for them
/// (canonical, eip155 and zkSync deployments). Each address is a CREATE2
/// address committing to the official bytecode, so code found there on any
/// network is that code.
const SINGLETONS: [(&str, &str); 10] = [
    ("1.3.0", "d9db270c1b5e3bd161e8c8503c55ceabee709552"),
    ("1.3.0", "69f4d1788e39c87893c980c06edf4b7f686e2938"),
    ("1.3.0", "b00ce5cccdef57e539ddced01df43a13855d9910"),
    ("1.3.0", "3e5c63644e683549055b9be8653de26e0b4cd36e"),
    ("1.3.0", "fb1bffc9d739b8d520daf37df666da4c687191ea"),
    ("1.3.0", "1727c2c531cf966f902e5927b98490fdfb3b2b70"),
    ("1.4.1", "41675c099f32341bf84bfc5382af534df5c7461a"),
    ("1.4.1", "c35f063962328ac65ced5d4c3fc5def8dec68dfa"),
    ("1.4.1", "29fcb43b46531bca003ddc8fcb67ffe91900c762"),
    ("1.4.1", "610fca2e0279fa1f8c00c8c2f71df522ad469380"),
];

/// The runtime code of the proxies the 1.3.0 and 1.4.1 factories deploy,
/// by hash: Keccak-256 of the code on an EVM network, and zkSync's own
/// bytecode hash (`SafeProxy` as protocol-kit 6.1.2 lists it) on zkSync
/// Era. A proxy from an older factory, upgraded or not, is none of these.
const PROXY_CODE_HASHES: [&str; 4] = [
    "b89c1b3bdf2cf8827818646bce9a8f6e372885f8c55e5c07acbd307cb133b000",
    "d7d408ebcd99b2b70be43e20253d6d92a8ea8fab29bd3be7f55b10032331fb4c",
    "0100004124426fb9ebb25e27d670c068e52f9ba631bd383279a188be47e3f86d",
    "0100003b6cfa15bd7d1cae1c9c022074524d7785d34859ad0576d8fab4305d4f",
];

/// `keccak256("EIP712Domain(uint256 chainId,address verifyingContract)")`,
/// the domain of every Safe from 1.3.0.
const DOMAIN_TYPEHASH: &str = "47e79534a245952e8b16893a336b85a3d9ea9fa8c573f3d803afb92a79469218";
/// `keccak256("SafeTx(address to,uint256 value,bytes data,uint8 operation,
/// uint256 safeTxGas,uint256 baseGas,uint256 gasPrice,address gasToken,
/// address refundReceiver,uint256 nonce)")`.
const SAFE_TX_TYPEHASH: &str = "bb8310d486368db6bd6f849402fdd73ad53d316b5a4b2644ad6efe0f941286d8";
/// `execTransaction(address,uint256,bytes,uint8,uint256,uint256,uint256,
/// address,address,bytes)`.
const SEL_EXEC_TRANSACTION: [u8; 4] = [0x6a, 0x76, 0x12, 0x02];

/// The version an official singleton runs, or `None` for any other address.
pub(crate) fn official_singleton(singleton: &EvmAddress) -> Option<&'static str> {
    let hex = hex::encode(singleton);
    SINGLETONS
        .iter()
        .find(|(_, address)| *address == hex)
        .map(|(version, _)| *version)
}

/// zkSync's hash of a contract's bytecode: SHA-256 with its first four
/// bytes replaced by the version (1), a zero and the length in 32-byte
/// words.
fn zksync_bytecode_hash(code: &[u8]) -> Option<[u8; 32]> {
    use sha2::Digest;
    if code.is_empty() || !code.len().is_multiple_of(32) {
        return None;
    }
    let words = u16::try_from(code.len() / 32).ok()?;
    let mut hash: [u8; 32] = sha2::Sha256::digest(code).into();
    hash[0] = 1;
    hash[1] = 0;
    hash[2..4].copy_from_slice(&words.to_be_bytes());
    Some(hash)
}

/// Whether `code` is one of the official proxies' runtime code.
pub(crate) fn is_official_proxy(code: &[u8]) -> bool {
    let keccak = hex::encode(keccak256(code));
    let zksync = zksync_bytecode_hash(code).map(hex::encode);
    PROXY_CODE_HASHES
        .iter()
        .any(|hash| *hash == keccak || zksync.as_deref() == Some(*hash))
}

/// What a Safe's proxy and singleton say about it, as `api::evm_safe`
/// reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SafeState {
    pub code: Vec<u8>,
    pub singleton: EvmAddress,
    pub version: String,
    pub owners: Vec<EvmAddress>,
    pub threshold: u64,
    pub nonce: u64,
    pub modules: Vec<EvmAddress>,
    /// More modules are enabled than one page lists.
    pub more_modules: bool,
    pub guard: Option<EvmAddress>,
    pub balance_wei: u128,
}

/// The policy a Safe enforces, once its proxy and singleton are official.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SafePolicy {
    pub version: String,
    /// Lowercase `0x…` addresses, as the Safe lists them.
    pub owners: Vec<String>,
    pub threshold: u64,
}

impl SafeState {
    /// The Safe's policy, refused unless the account is an official proxy
    /// delegating to an official singleton of a supported version, which
    /// reports that version and a threshold its owners can meet.
    pub(crate) fn policy(&self) -> Result<SafePolicy, SendError> {
        if self.code.is_empty() {
            return Err(SendError::invalid(
                "This address holds no contract: it is not a Safe.",
            ));
        }
        if !is_official_proxy(&self.code) {
            return Err(SendError::invalid(
                "This contract is not an official Safe proxy of version 1.3.0 or 1.4.1.",
            ));
        }
        let version = official_singleton(&self.singleton).ok_or_else(|| {
            SendError::invalid(
                "This Safe runs a singleton that is not an official Safe 1.3.0 or 1.4.1 deployment.",
            )
        })?;
        if self.version != version {
            return Err(SendError::invalid(
                "The Safe reports another version than its singleton's.",
            ));
        }
        if self.threshold == 0 || self.threshold > self.owners.len() as u64 {
            return Err(SendError::invalid(
                "The Safe's threshold is not one its owners can meet.",
            ));
        }
        Ok(SafePolicy {
            version: version.to_string(),
            owners: self.owners.iter().map(address_text).collect(),
            threshold: self.threshold,
        })
    }

    /// What can move or block the Safe's funds beside its owners: each
    /// enabled module and a guard.
    pub(crate) fn warnings(&self) -> Vec<crate::LocalizableMessage> {
        let mut warnings: Vec<crate::LocalizableMessage> = self
            .modules
            .iter()
            .map(|module| {
                crate::LocalizableMessage::new(
                    "Module %@ can move this Safe's funds without its owners' signatures.",
                    [address_text(module)],
                )
            })
            .collect();
        if self.more_modules {
            warnings.push("More modules are enabled than were read.".into());
        }
        if let Some(guard) = self.guard {
            warnings.push(crate::LocalizableMessage::new(
                "Guard %@ checks every transaction and can block it.",
                [address_text(&guard)],
            ));
        }
        warnings
    }
}

pub(crate) fn address_text(address: &EvmAddress) -> String {
    format!("0x{}", hex::encode(address))
}

pub(crate) fn parse_address(text: &str) -> Result<EvmAddress, SendError> {
    let digits = text.trim().strip_prefix("0x").unwrap_or(text.trim());
    hex::decode(digits)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| SendError::invalid("Not an EVM address."))
}

/// A whole number as Safe's SDK writes one: a decimal string, or a JSON
/// number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum Number {
    Text(String),
    Integer(u64),
}

impl Number {
    fn value(&self) -> Result<BigUint, SendError> {
        let invalid = || SendError::invalid("A Safe transaction's numbers are whole numbers.");
        match self {
            Self::Integer(value) => Ok(BigUint::from(*value)),
            Self::Text(text) if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => {
                text.parse().map_err(|_| invalid())
            }
            Self::Text(_) => Err(invalid()),
        }
    }
}

/// `SafeTransactionData`, field for field as protocol-kit names them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SafeTransactionData {
    pub to: String,
    value: Number,
    pub data: String,
    pub operation: u8,
    safe_tx_gas: Number,
    base_gas: Number,
    gas_price: Number,
    gas_token: String,
    refund_receiver: String,
    nonce: Number,
}

/// One owner's signature, as `0x…` 65 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SafeSignatureData {
    pub signer: String,
    pub data: String,
}

/// A Safe transaction and its signatures, as one owner hands them to the
/// next.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SafeSessionData {
    pub safe: String,
    pub chain_id: u64,
    pub version: String,
    pub transaction: SafeTransactionData,
    pub signatures: Vec<SafeSignatureData>,
}

/// A Safe transaction's fields, each read and range-checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SafeTx {
    pub to: EvmAddress,
    pub value: BigUint,
    pub data: Vec<u8>,
    pub operation: u8,
    pub safe_tx_gas: BigUint,
    pub base_gas: BigUint,
    pub gas_price: BigUint,
    pub gas_token: EvmAddress,
    pub refund_receiver: EvmAddress,
    pub nonce: u64,
}

fn below_2_256(value: BigUint) -> Result<BigUint, SendError> {
    if value.bits() > 256 {
        return Err(SendError::invalid(
            "A Safe transaction's number is past 2^256.",
        ));
    }
    Ok(value)
}

fn hex_bytes(text: &str) -> Result<Vec<u8>, SendError> {
    let digits = text.trim().strip_prefix("0x").unwrap_or(text.trim());
    hex::decode(digits).map_err(|_| SendError::invalid("Not hex data."))
}

impl SafeTransactionData {
    pub(crate) fn parse(&self) -> Result<SafeTx, SendError> {
        if self.operation > 1 {
            return Err(SendError::invalid(
                "A Safe transaction is a call (0) or a delegate call (1).",
            ));
        }
        let nonce = self.nonce.value()?;
        Ok(SafeTx {
            to: parse_address(&self.to)?,
            value: below_2_256(self.value.value()?)?,
            data: hex_bytes(&self.data)?,
            operation: self.operation,
            safe_tx_gas: below_2_256(self.safe_tx_gas.value()?)?,
            base_gas: below_2_256(self.base_gas.value()?)?,
            gas_price: below_2_256(self.gas_price.value()?)?,
            gas_token: parse_address(&self.gas_token)?,
            refund_receiver: parse_address(&self.refund_receiver)?,
            nonce: u64::try_from(&nonce)
                .map_err(|_| SendError::invalid("The Safe nonce is out of range."))?,
        })
    }
}

impl SafeTx {
    /// A plain call: `value` wei and `data` to `to`, no refund, at `nonce`.
    pub(crate) fn call(to: EvmAddress, value: u128, data: Vec<u8>, nonce: u64) -> Self {
        Self {
            to,
            value: BigUint::from(value),
            data,
            operation: 0,
            safe_tx_gas: BigUint::ZERO,
            base_gas: BigUint::ZERO,
            gas_price: BigUint::ZERO,
            gas_token: [0; 20],
            refund_receiver: [0; 20],
            nonce,
        }
    }

    pub(crate) fn data(&self) -> SafeTransactionData {
        SafeTransactionData {
            to: address_text(&self.to),
            value: Number::Text(self.value.to_string()),
            data: format!("0x{}", hex::encode(&self.data)),
            operation: self.operation,
            safe_tx_gas: Number::Text(self.safe_tx_gas.to_string()),
            base_gas: Number::Text(self.base_gas.to_string()),
            gas_price: Number::Text(self.gas_price.to_string()),
            gas_token: address_text(&self.gas_token),
            refund_receiver: address_text(&self.refund_receiver),
            nonce: Number::Integer(self.nonce),
        }
    }

    /// The Safe's domain separator on `chain_id`.
    pub(crate) fn domain_separator(chain_id: u64, safe: &EvmAddress) -> [u8; 32] {
        let mut encoded = Vec::with_capacity(96);
        encoded.extend(hex::decode(DOMAIN_TYPEHASH).expect("constant"));
        encoded.extend(uint_word(&BigUint::from(chain_id)));
        encoded.extend(address_word(safe));
        keccak256(&encoded)
    }

    /// `keccak256(abi.encode(SAFE_TX_TYPEHASH, …))`.
    pub(crate) fn struct_hash(&self) -> [u8; 32] {
        let mut encoded = Vec::with_capacity(352);
        encoded.extend(hex::decode(SAFE_TX_TYPEHASH).expect("constant"));
        encoded.extend(address_word(&self.to));
        encoded.extend(uint_word(&self.value));
        encoded.extend(keccak256(&self.data));
        encoded.extend(uint_word(&BigUint::from(self.operation)));
        encoded.extend(uint_word(&self.safe_tx_gas));
        encoded.extend(uint_word(&self.base_gas));
        encoded.extend(uint_word(&self.gas_price));
        encoded.extend(address_word(&self.gas_token));
        encoded.extend(address_word(&self.refund_receiver));
        encoded.extend(uint_word(&BigUint::from(self.nonce)));
        keccak256(&encoded)
    }

    /// The hash the owners sign and `execTransaction` checks: EIP-712's
    /// `keccak256(0x1901 ‖ domainSeparator ‖ structHash)`.
    pub(crate) fn hash(&self, chain_id: u64, safe: &EvmAddress) -> [u8; 32] {
        let mut encoded = vec![0x19, 0x01];
        encoded.extend(Self::domain_separator(chain_id, safe));
        encoded.extend(self.struct_hash());
        keccak256(&encoded)
    }

    /// `execTransaction` with `signatures`, the owners' signatures in
    /// ascending owner order.
    pub(crate) fn exec_calldata(&self, signatures: &[u8]) -> Vec<u8> {
        let padded = |bytes: &[u8]| {
            let mut out = uint_word(&BigUint::from(bytes.len())).to_vec();
            out.extend(bytes);
            out.resize(32 + bytes.len().div_ceil(32) * 32, 0);
            out
        };
        let data = padded(&self.data);
        let mut out = SEL_EXEC_TRANSACTION.to_vec();
        out.extend(address_word(&self.to));
        out.extend(uint_word(&self.value));
        out.extend(uint_word(&BigUint::from(10u32 * 32)));
        out.extend(uint_word(&BigUint::from(self.operation)));
        out.extend(uint_word(&self.safe_tx_gas));
        out.extend(uint_word(&self.base_gas));
        out.extend(uint_word(&self.gas_price));
        out.extend(address_word(&self.gas_token));
        out.extend(address_word(&self.refund_receiver));
        out.extend(uint_word(&BigUint::from(10 * 32 + data.len())));
        out.extend(data);
        out.extend(padded(signatures));
        out
    }
}

fn uint_word(value: &BigUint) -> [u8; 32] {
    let bytes = value.to_bytes_be();
    let mut word = [0; 32];
    word[32 - bytes.len()..].copy_from_slice(&bytes);
    word
}

fn address_word(address: &EvmAddress) -> [u8; 32] {
    let mut word = [0; 32];
    word[12..].copy_from_slice(address);
    word
}

/// The owner a 65-byte signature of `hash` recovers to: plain ECDSA over
/// the hash (`v` 27 or 28) or over its `eth_sign` digest (`v` 31 or 32).
pub(crate) fn recover_signer(hash: &[u8; 32], signature: &[u8]) -> Result<EvmAddress, SendError> {
    use secp256k1::ecdsa::{RecoverableSignature, RecoveryId};
    let [compact @ .., v] = signature else {
        return Err(SendError::invalid("A Safe signature is 65 bytes."));
    };
    if signature.len() != 65 {
        return Err(SendError::invalid("A Safe signature is 65 bytes."));
    }
    let (digest, recovery) = match *v {
        27 | 28 => (*hash, v - 27),
        31 | 32 => {
            let mut prefixed = b"\x19Ethereum Signed Message:\n32".to_vec();
            prefixed.extend(hash);
            (keccak256(&prefixed), v - 31)
        }
        0 | 1 => {
            return Err(SendError::invalid(
                "Contract and approved-hash signatures only the Safe can check; sign with an owner's key.",
            ));
        }
        _ => return Err(SendError::invalid("Not a Safe signature.")),
    };
    let id = RecoveryId::from_i32(i32::from(recovery)).map_err(SendError::invalid)?;
    let signature = RecoverableSignature::from_compact(compact, id)
        .map_err(|_| SendError::invalid("Not a Safe signature."))?;
    let key = secp256k1::Secp256k1::verification_only()
        .recover_ecdsa(&secp256k1::Message::from_digest(digest), &signature)
        .map_err(|_| SendError::invalid("The signature recovers to no key."))?;
    let mut address = [0; 20];
    address.copy_from_slice(&keccak256(&key.serialize_uncompressed()[1..])[12..]);
    Ok(address)
}

/// Sign `hash` as an owner: ECDSA, `v` 27 or 28, as protocol-kit's
/// EIP-712 signatures are.
pub(crate) fn sign(hash: &[u8; 32], private_key: &[u8]) -> Result<Vec<u8>, SendError> {
    let secret = secp256k1::SecretKey::from_slice(private_key).map_err(SendError::invalid)?;
    let (recovery, compact) = secp256k1::Secp256k1::signing_only()
        .sign_ecdsa_recoverable(&secp256k1::Message::from_digest(*hash), &secret)
        .serialize_compact();
    let mut signature = compact.to_vec();
    signature.push(27 + recovery.to_i32() as u8);
    Ok(signature)
}

/// A session reviewed against the Safe's policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SafeReview {
    pub tx: SafeTx,
    pub hash: [u8; 32],
    /// The owners whose valid signature the session carries, ascending,
    /// each with its signature.
    pub signatures: Vec<(EvmAddress, Vec<u8>)>,
}

impl SafeReview {
    pub(crate) fn complete(&self, policy: &SafePolicy) -> bool {
        self.signatures.len() as u64 >= policy.threshold
    }

    /// The threshold's signatures, ascending by owner, as `execTransaction`
    /// reads them.
    pub(crate) fn signature_bytes(&self, policy: &SafePolicy) -> Vec<u8> {
        self.signatures
            .iter()
            .take(policy.threshold as usize)
            .flat_map(|(_, signature)| signature.clone())
            .collect()
    }
}

/// `data` as the Safe `safe` on `chain_id` under `policy`: refused for
/// another Safe, network or version, a delegate call, a signature that is
/// not a valid one by an owner, or an owner twice.
pub(crate) fn review(
    policy: &SafePolicy,
    safe: &EvmAddress,
    chain_id: u64,
    data: &SafeSessionData,
) -> Result<SafeReview, SendError> {
    if parse_address(&data.safe)? != *safe || data.chain_id != chain_id {
        return Err(SendError::invalid(
            "This Safe transaction is another Safe's or another network's.",
        ));
    }
    if data.version != policy.version {
        return Err(SendError::invalid(
            "This Safe transaction names another Safe version than the Safe runs.",
        ));
    }
    let tx = data.transaction.parse()?;
    if tx.operation != 0 {
        return Err(SendError::invalid(
            "A delegate call runs other code as the Safe itself; Spectra does not sign one.",
        ));
    }
    let hash = tx.hash(chain_id, safe);
    let owners: Vec<EvmAddress> = policy
        .owners
        .iter()
        .map(|owner| parse_address(owner))
        .collect::<Result<_, _>>()?;
    let mut signatures: Vec<(EvmAddress, Vec<u8>)> = Vec::new();
    for entry in &data.signatures {
        let bytes = hex_bytes(&entry.data)?;
        let signer = recover_signer(&hash, &bytes)?;
        if signer != parse_address(&entry.signer)? {
            return Err(SendError::refused(
                "A signature named %@ is not that owner's signature of this transaction.",
                [entry.signer.as_str()],
            ));
        }
        if !owners.contains(&signer) {
            return Err(SendError::refused(
                "%@ signed, but is not one of the Safe's owners.",
                [address_text(&signer)],
            ));
        }
        if signatures.iter().any(|(known, _)| *known == signer) {
            return Err(SendError::refused(
                "%@ signed twice.",
                [address_text(&signer)],
            ));
        }
        signatures.push((signer, bytes));
    }
    signatures.sort_by_key(|(owner, _)| *owner);
    Ok(SafeReview {
        tx,
        hash,
        signatures,
    })
}

/// The session's data after `review`, its signatures in owner order.
pub(crate) fn session_data(
    safe: &EvmAddress,
    chain_id: u64,
    policy: &SafePolicy,
    tx: &SafeTx,
    signatures: &[(EvmAddress, Vec<u8>)],
) -> SafeSessionData {
    SafeSessionData {
        safe: address_text(safe),
        chain_id,
        version: policy.version.clone(),
        transaction: tx.data(),
        signatures: signatures
            .iter()
            .map(|(signer, signature)| SafeSignatureData {
                signer: address_text(signer),
                data: format!("0x{}", hex::encode(signature)),
            })
            .collect(),
    }
}

#[cfg(test)]
#[path = "tests/safe.rs"]
mod tests;
