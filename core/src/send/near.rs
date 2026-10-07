//! NEAR send: BORSH-encoded Transfer, FunctionCall and DeleteKey
//! transaction builders and Ed25519 signer.

use crate::send::error::SendError;

pub(crate) fn nep141_transfer_args(receiver: &str, amount: u128) -> Result<Vec<u8>, SendError> {
    Ok(serde_json::to_vec(
        &serde_json::json!({"receiver_id":receiver,"amount":amount.to_string()}),
    )?)
}

/// NEAR identifies the unsigned Borsh transaction, excluding Signature's
/// Ed25519 discriminator and 64 signature bytes.
pub(crate) fn signed_transaction_hash(signed: &[u8]) -> Result<String, SendError> {
    use sha2::{Digest, Sha256};
    let end = signed
        .len()
        .checked_sub(65)
        .filter(|end| *end > 0)
        .ok_or_else(|| SendError::invalid("Invalid signed NEAR transaction"))?;
    if signed[end] != 0 {
        return Err(SendError::invalid("Invalid NEAR Ed25519 signature"));
    }
    Ok(bs58::encode(Sha256::digest(&signed[..end])).into_string())
}

// ── NEAR transaction builder (BORSH)

/// Build a signed NEAR Transfer transaction.
pub fn build_near_transfer_tx(
    signer_id: &str,
    public_key: &[u8; 32],
    nonce: u64,
    receiver_id: &str,
    yocto_amount: u128,
    block_hash: &[u8; 32],
    private_key: &[u8; 32],
) -> Result<Vec<u8>, SendError> {
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};

    let tx = borsh_encode_transfer(
        signer_id,
        public_key,
        nonce,
        receiver_id,
        yocto_amount,
        block_hash,
    );

    // Hash the transaction for signing.
    let tx_hash: [u8; 32] = Sha256::digest(&tx).into();

    let signing_key = SigningKey::from_bytes(private_key);
    if signing_key.verifying_key().as_bytes() != public_key {
        return Err(SendError::Invalid(
            "NEAR: public key does not match signer".into(),
        ));
    }
    let signature = signing_key.sign(&tx_hash);

    // SignedTransaction = Transaction || Signature
    // Signature in NEAR is: [key_type(1)] + [sig(64)]
    let mut signed = tx;
    signed.push(0); // key type = ED25519
    signed.extend_from_slice(signature.to_bytes().as_ref());

    Ok(signed)
}

/// BORSH-encode a NEAR Transfer transaction.
fn borsh_encode_transfer(
    signer_id: &str,
    public_key: &[u8; 32],
    nonce: u64,
    receiver_id: &str,
    yocto_amount: u128,
    block_hash: &[u8; 32],
) -> Vec<u8> {
    let mut out = Vec::new();

    // signer_id: string (u32 len + bytes)
    borsh_string(&mut out, signer_id);
    // public_key: key_type(u8) + bytes(32)
    out.push(0); // ED25519
    out.extend_from_slice(public_key);
    // nonce: u64
    out.extend_from_slice(&nonce.to_le_bytes());
    // receiver_id: string
    borsh_string(&mut out, receiver_id);
    // block_hash: [u8; 32]
    out.extend_from_slice(block_hash);
    // actions: array (u32 len)
    out.extend_from_slice(&1u32.to_le_bytes());
    // Action::Transfer = variant 3
    out.push(3u8);
    // Transfer.deposit: u128
    out.extend_from_slice(&yocto_amount.to_le_bytes());

    out
}

/// Build a signed NEAR FunctionCall transaction (used for NEP-141 transfers).
#[allow(clippy::too_many_arguments)]
pub fn build_near_function_call_tx(
    signer_id: &str,
    public_key: &[u8; 32],
    nonce: u64,
    receiver_id: &str,
    method_name: &str,
    args: &[u8],
    gas: u64,
    deposit: u128,
    block_hash: &[u8; 32],
    private_key: &[u8; 32],
) -> Result<Vec<u8>, SendError> {
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};

    let tx = borsh_encode_function_call(
        signer_id,
        public_key,
        nonce,
        receiver_id,
        method_name,
        args,
        gas,
        deposit,
        block_hash,
    );

    let tx_hash: [u8; 32] = Sha256::digest(&tx).into();
    let signing_key = SigningKey::from_bytes(private_key);
    if signing_key.verifying_key().as_bytes() != public_key {
        return Err(SendError::Invalid(
            "NEAR: public key does not match signer".into(),
        ));
    }
    let signature = signing_key.sign(&tx_hash);

    // SignedTransaction = Transaction || Signature (key_type(1) + sig(64))
    let mut signed = tx;
    signed.push(0); // ED25519
    signed.extend_from_slice(signature.to_bytes().as_ref());
    Ok(signed)
}

#[allow(clippy::too_many_arguments)]
fn borsh_encode_function_call(
    signer_id: &str,
    public_key: &[u8; 32],
    nonce: u64,
    receiver_id: &str,
    method_name: &str,
    args: &[u8],
    gas: u64,
    deposit: u128,
    block_hash: &[u8; 32],
) -> Vec<u8> {
    let mut out = Vec::new();

    // signer_id: string
    borsh_string(&mut out, signer_id);
    // public_key: key_type(u8) + bytes(32)
    out.push(0); // ED25519
    out.extend_from_slice(public_key);
    // nonce: u64
    out.extend_from_slice(&nonce.to_le_bytes());
    // receiver_id: string
    borsh_string(&mut out, receiver_id);
    // block_hash: [u8; 32]
    out.extend_from_slice(block_hash);
    // actions: array (u32 len)
    out.extend_from_slice(&1u32.to_le_bytes());
    // Action::FunctionCall = variant 2
    out.push(2u8);
    // method_name: string
    borsh_string(&mut out, method_name);
    // args: Vec<u8> (u32 len + bytes)
    out.extend_from_slice(&(args.len() as u32).to_le_bytes());
    out.extend_from_slice(args);
    // gas: u64
    out.extend_from_slice(&gas.to_le_bytes());
    // deposit: u128
    out.extend_from_slice(&deposit.to_le_bytes());

    out
}

fn borsh_string(out: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PreparedNearFunctionCall {
    pub signer: String,
    pub public_key: [u8; 32],
    pub nonce: u64,
    pub receiver: String,
    pub method: String,
    pub args: Vec<u8>,
    pub gas: u64,
    pub deposit: String,
    pub block_hash: [u8; 32],
    pub message: Vec<u8>,
    pub fee_budget: String,
}
impl PreparedNearFunctionCall {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        signer: &str,
        public_key: [u8; 32],
        nonce: u64,
        receiver: &str,
        method: &str,
        args: Vec<u8>,
        gas: u64,
        deposit: u128,
        block_hash: [u8; 32],
    ) -> Self {
        let message = borsh_encode_function_call(
            signer,
            &public_key,
            nonce,
            receiver,
            method,
            &args,
            gas,
            deposit,
            &block_hash,
        );
        Self {
            signer: signer.into(),
            public_key,
            nonce,
            receiver: receiver.into(),
            method: method.into(),
            args,
            gas,
            deposit: deposit.to_string(),
            block_hash,
            message,
            fee_budget: "0".into(),
        }
    }
    pub(crate) fn sign(
        &self,
        key: &crate::send::keys::Ed25519Seed,
    ) -> Result<(Vec<u8>, String), SendError> {
        use sha2::{Digest, Sha256};
        key.require_public_key(&self.public_key)?;
        let expected = borsh_encode_function_call(
            &self.signer,
            &self.public_key,
            self.nonce,
            &self.receiver,
            &self.method,
            &self.args,
            self.gas,
            self.deposit.parse::<u128>().map_err(SendError::invalid)?,
            &self.block_hash,
        );
        if expected != self.message {
            return Err(SendError::invalid(
                "NEAR staking fields differ from reviewed message",
            ));
        }
        let digest = Sha256::digest(&self.message);
        let mut raw = self.message.clone();
        raw.push(0);
        raw.extend(key.sign(&digest));
        Ok((raw, bs58::encode(digest).into_string()))
    }
}

/// A transaction deleting one of the signer's access keys: its own receiver,
/// one `DeleteKey` action.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PreparedNearDeleteKey {
    pub signer: String,
    pub public_key: [u8; 32],
    pub nonce: u64,
    pub deleted_key: [u8; 32],
    pub block_hash: [u8; 32],
    pub message: Vec<u8>,
    /// The most the transaction costs, in yoctoNEAR.
    pub fee_budget: String,
}

impl PreparedNearDeleteKey {
    pub(crate) fn prepare(
        signer: &str,
        public_key: [u8; 32],
        nonce: u64,
        deleted_key: [u8; 32],
        block_hash: [u8; 32],
        fee_budget: u128,
    ) -> Result<Self, SendError> {
        if deleted_key == public_key {
            return Err(SendError::invalid(
                "NEAR: a transaction cannot delete the key that signs it",
            ));
        }
        Ok(Self {
            signer: signer.into(),
            public_key,
            nonce,
            deleted_key,
            block_hash,
            message: borsh_encode_delete_key(signer, &public_key, nonce, &deleted_key, &block_hash),
            fee_budget: fee_budget.to_string(),
        })
    }

    pub(crate) fn sign(
        &self,
        key: &crate::send::keys::Ed25519Seed,
    ) -> Result<(Vec<u8>, String), SendError> {
        use sha2::{Digest, Sha256};
        key.require_public_key(&self.public_key)?;
        if self.deleted_key == self.public_key
            || borsh_encode_delete_key(
                &self.signer,
                &self.public_key,
                self.nonce,
                &self.deleted_key,
                &self.block_hash,
            ) != self.message
        {
            return Err(SendError::invalid(
                "NEAR key deletion differs from the reviewed message",
            ));
        }
        let digest = Sha256::digest(&self.message);
        let mut raw = self.message.clone();
        raw.push(0);
        raw.extend(key.sign(&digest));
        Ok((raw, bs58::encode(digest).into_string()))
    }
}

/// BORSH-encode a transaction deleting `deleted_key` from the signer's own
/// account.
fn borsh_encode_delete_key(
    signer_id: &str,
    public_key: &[u8; 32],
    nonce: u64,
    deleted_key: &[u8; 32],
    block_hash: &[u8; 32],
) -> Vec<u8> {
    let mut out = Vec::new();
    borsh_string(&mut out, signer_id);
    out.push(0); // ED25519
    out.extend_from_slice(public_key);
    out.extend_from_slice(&nonce.to_le_bytes());
    // The account deletes its own key: it is the receiver.
    borsh_string(&mut out, signer_id);
    out.extend_from_slice(block_hash);
    out.extend_from_slice(&1u32.to_le_bytes());
    // Action::DeleteKey = variant 6, then the key: type ED25519 + bytes.
    out.push(6u8);
    out.push(0);
    out.extend_from_slice(deleted_key);
    out
}

#[cfg(test)]
mod protocol_tests {
    use super::*;
    #[test]
    fn near_transactions_match_official_sdk_vectors() {
        let fixtures: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/protocol-transactions.json"
        ))
        .unwrap();
        let public: [u8; 32] = hex::decode(fixtures["public_key"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let transfer = build_near_transfer_tx(
            "alice.near",
            &public,
            42,
            "token.near",
            123456789,
            &[2; 32],
            &[1; 32],
        )
        .unwrap();
        let call = build_near_function_call_tx(
            "alice.near",
            &public,
            42,
            "token.near",
            "ft_transfer",
            br#"{"amount":"123456","receiver_id":"bob.near"}"#,
            30_000_000_000_000,
            1,
            &[2; 32],
            &[1; 32],
        )
        .unwrap();
        for (bytes, vector) in [transfer, call]
            .iter()
            .zip(fixtures["near"].as_array().unwrap())
        {
            assert_eq!(hex::encode(bytes), vector["signed_hex"].as_str().unwrap());
            assert_eq!(
                signed_transaction_hash(bytes).unwrap(),
                vector["hash"].as_str().unwrap()
            );
        }
        // DeleteKey as @near-js/transactions builds and signs it.
        let deletion: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/near-delete-key.json"))
                .unwrap();
        let text = |key: &str| deletion[key].as_str().unwrap().to_string();
        let bytes32 =
            |key: &str| -> [u8; 32] { hex::decode(text(key)).unwrap().try_into().unwrap() };
        let deleted: [u8; 32] = bs58::decode(text("deleted_key").strip_prefix("ed25519:").unwrap())
            .into_vec()
            .unwrap()
            .try_into()
            .unwrap();
        let prepared = PreparedNearDeleteKey::prepare(
            &text("signer"),
            bytes32("public_key"),
            deletion["nonce"].as_u64().unwrap(),
            deleted,
            bytes32("block_hash"),
            0,
        )
        .unwrap();
        let seed = crate::send::keys::Ed25519Seed::from_hex(&text("seed")).unwrap();
        let (raw, hash) = prepared.sign(&seed).unwrap();
        assert_eq!(hex::encode(&raw), text("signed_hex"));
        assert_eq!(hash, text("hash"));
        assert_eq!(signed_transaction_hash(&raw).unwrap(), text("hash"));
        // The signing key cannot delete itself.
        assert!(
            PreparedNearDeleteKey::prepare(
                "alice.near",
                bytes32("public_key"),
                1,
                bytes32("public_key"),
                [2; 32],
                0
            )
            .is_err()
        );
        assert!(
            build_near_transfer_tx(
                "alice.near",
                &[9; 32],
                42,
                "token.near",
                1,
                &[2; 32],
                &[1; 32]
            )
            .is_err()
        );
    }
}
