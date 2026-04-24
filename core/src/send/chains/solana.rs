//! Solana send: native SOL transfer + SPL TransferChecked (with idempotent ATA
//! create). Ed25519 signing + sendTransaction RPC broadcast.

use base64::Engine as _;
use serde_json::json;

use crate::derivation::chains::solana::decode_b58_32;
use crate::fetch::chains::solana::{SolanaClient, SolanaSendResult};

impl SolanaClient {
    /// Sign and broadcast a native SOL transfer.
    pub async fn sign_and_broadcast(
        &self,
        from_pubkey_bytes: &[u8; 32],
        to_address: &str,
        lamports: u64,
        private_key_bytes: &[u8; 64],
    ) -> Result<SolanaSendResult, String> {
        let blockhash = self.fetch_recent_blockhash().await?;
        let to_pubkey = bs58::decode(to_address)
            .into_vec()
            .map_err(|e| format!("invalid to address: {e}"))?;
        if to_pubkey.len() != 32 {
            return Err(format!("invalid to pubkey length: {}", to_pubkey.len()));
        }
        let to_pubkey: [u8; 32] = to_pubkey.try_into().unwrap();

        let raw_tx = build_sol_transfer(
            from_pubkey_bytes,
            &to_pubkey,
            lamports,
            &blockhash,
            private_key_bytes,
        )?;

        let encoded = base64::engine::general_purpose::STANDARD.encode(&raw_tx);
        let result = self
            .call(
                "sendTransaction",
                json!([encoded.clone(), {"encoding": "base64", "preflightCommitment": "confirmed"}]),
            )
            .await?;
        let signature = result
            .as_str()
            .ok_or("sendTransaction: expected string")?
            .to_string();
        Ok(SolanaSendResult { signature, signed_tx_base64: encoded })
    }

    /// Sign and broadcast an SPL token transfer. Derives the source and
    /// destination associated token accounts; if the destination ATA does
    /// not exist yet the transaction prepends a Create-Idempotent
    /// instruction so it is materialized in the same atomic tx.
    #[allow(clippy::too_many_arguments)]
    pub async fn sign_and_broadcast_spl(
        &self,
        from_owner_pubkey: &[u8; 32],
        to_owner_b58: &str,
        mint_b58: &str,
        amount_raw: u64,
        decimals: u8,
        private_key_bytes: &[u8; 64],
    ) -> Result<SolanaSendResult, String> {
        let to_owner = decode_b58_32(to_owner_b58)?;
        let mint = decode_b58_32(mint_b58)?;

        let source_ata = derive_associated_token_account(from_owner_pubkey, &mint)?;
        let dest_ata = derive_associated_token_account(&to_owner, &mint)?;

        let source_ata_b58 = bs58::encode(&source_ata).into_string();
        let dest_ata_b58 = bs58::encode(&dest_ata).into_string();

        // Destination ATA may not exist yet; we always emit the idempotent
        // create instruction so it's a no-op if the account already exists.
        let _dest_exists = self.account_exists(&dest_ata_b58).await.unwrap_or(false);

        let blockhash = self.fetch_recent_blockhash().await?;
        let raw_tx = build_spl_transfer_checked(
            from_owner_pubkey,
            &to_owner,
            &mint,
            &source_ata,
            &dest_ata,
            amount_raw,
            decimals,
            &blockhash,
            private_key_bytes,
        )?;

        let encoded = base64::engine::general_purpose::STANDARD.encode(&raw_tx);
        let result = self
            .call(
                "sendTransaction",
                json!([encoded.clone(), {"encoding": "base64", "preflightCommitment": "confirmed"}]),
            )
            .await?;
        let signature = result
            .as_str()
            .ok_or("sendTransaction: expected string")?
            .to_string();
        let _ = source_ata_b58;
        Ok(SolanaSendResult { signature, signed_tx_base64: encoded })
    }

    /// Broadcast an already-signed transaction given as a base64 string.
    pub async fn broadcast_raw(
        &self,
        signed_tx_base64: &str,
    ) -> Result<SolanaSendResult, String> {
        let result = self
            .call(
                "sendTransaction",
                json!([signed_tx_base64, {"encoding": "base64", "preflightCommitment": "confirmed"}]),
            )
            .await?;
        let signature = result
            .as_str()
            .ok_or("sendTransaction: expected string")?
            .to_string();
        Ok(SolanaSendResult {
            signature,
            signed_tx_base64: signed_tx_base64.to_string(),
        })
    }
}

// ----------------------------------------------------------------
// Transaction builder
// ----------------------------------------------------------------

/// Build a signed Solana legacy transaction for a native SOL transfer.
///
/// Wire format (legacy):
///   compact_u16(num_sigs) || sig[0..64] || message_bytes
///
/// Message:
///   [header: 3 bytes] [compact_u16(num_accounts)] [accounts..] [blockhash: 32]
///   [compact_u16(num_instructions)] [instruction: program_id_idx | compact_u16(accounts) | compact_u16(data)]
pub fn build_sol_transfer(
    from: &[u8; 32],
    to: &[u8; 32],
    lamports: u64,
    recent_blockhash_b58: &str,
    private_key: &[u8; 64],
) -> Result<Vec<u8>, String> {
    use ed25519_dalek::{Signer, SigningKey};

    let blockhash_bytes = bs58::decode(recent_blockhash_b58)
        .into_vec()
        .map_err(|e| format!("invalid blockhash: {e}"))?;
    if blockhash_bytes.len() != 32 {
        return Err("blockhash must be 32 bytes".to_string());
    }
    let blockhash: [u8; 32] = blockhash_bytes.try_into().unwrap();

    // System program ID (all zeros except last byte = 0).
    let system_program: [u8; 32] = [0u8; 32];

    // Accounts: [from (signer+writable), to (writable), system_program]
    // Header: [num_required_signatures=1, num_readonly_signed=0, num_readonly_unsigned=1]
    let header = [1u8, 0u8, 1u8];

    // Build message.
    let mut msg = Vec::new();
    msg.extend_from_slice(&header);
    // Account list (3 accounts).
    msg.extend_from_slice(&compact_u16(3));
    msg.extend_from_slice(from);
    msg.extend_from_slice(to);
    msg.extend_from_slice(&system_program);
    // Recent blockhash.
    msg.extend_from_slice(&blockhash);
    // Instructions (1).
    msg.extend_from_slice(&compact_u16(1));
    // Instruction: program id index = 2 (system program).
    msg.push(2u8);
    // Account indices: [0 (from), 1 (to)].
    msg.extend_from_slice(&compact_u16(2));
    msg.push(0u8); // from index
    msg.push(1u8); // to index
    // Data: SystemInstruction::Transfer = [2,0,0,0] + lamports as le u64
    let mut data = Vec::new();
    data.extend_from_slice(&2u32.to_le_bytes()); // Transfer instruction
    data.extend_from_slice(&lamports.to_le_bytes());
    msg.extend_from_slice(&compact_u16(data.len()));
    msg.extend_from_slice(&data);

    // Sign.
    let signing_key =
        SigningKey::from_bytes(&private_key[..32].try_into().map_err(|_| "privkey too short")?);
    let signature = signing_key.sign(&msg);

    // Serialize: compact_u16(1) || sig || message
    let mut tx = Vec::new();
    tx.extend_from_slice(&compact_u16(1)); // 1 signature
    tx.extend_from_slice(signature.to_bytes().as_ref());
    tx.extend_from_slice(&msg);

    Ok(tx)
}

// ----------------------------------------------------------------
// SPL helpers: ATA derivation and SPL Transfer transaction builder
// ----------------------------------------------------------------

/// SPL Token program id (decoded base58).
pub const SPL_TOKEN_PROGRAM_ID: [u8; 32] = [
    6, 221, 246, 225, 215, 101, 161, 147, 217, 203, 225, 70, 206, 235, 121, 172, 28, 180, 133, 237,
    95, 91, 55, 145, 58, 140, 245, 133, 126, 255, 0, 169,
];

/// Associated Token Account program id (decoded base58).
pub const ASSOCIATED_TOKEN_PROGRAM_ID: [u8; 32] = [
    140, 151, 37, 143, 78, 36, 137, 241, 187, 61, 16, 41, 20, 142, 13, 131, 11, 90, 19, 153, 218,
    255, 16, 132, 4, 142, 123, 216, 219, 233, 248, 89,
];

/// Derive the Associated Token Account for a (wallet, mint) pair.
///
/// PDA seeds = [wallet, TOKEN_PROGRAM_ID, mint], program = ASSOCIATED_TOKEN_PROGRAM_ID.
pub fn derive_associated_token_account(
    wallet: &[u8; 32],
    mint: &[u8; 32],
) -> Result<[u8; 32], String> {
    use sha2::{Digest, Sha256};
    let seeds: [&[u8]; 3] = [wallet, &SPL_TOKEN_PROGRAM_ID, mint];
    // Brute-force the bump seed from 255 down until we find an off-curve point.
    for bump in (0u8..=255u8).rev() {
        let mut h = Sha256::new();
        for s in seeds.iter() {
            h.update(s);
        }
        h.update([bump]);
        h.update(ASSOCIATED_TOKEN_PROGRAM_ID);
        h.update(b"ProgramDerivedAddress");
        let digest: [u8; 32] = h.finalize().into();
        if is_off_curve(&digest) {
            return Ok(digest);
        }
    }
    Err("failed to find PDA bump".to_string())
}

/// An ed25519 point is "off-curve" if CompressedEdwardsY::decompress returns None.
/// PDAs are valid only when the resulting point is off-curve (so they cannot
/// coincide with a real pubkey).
fn is_off_curve(bytes: &[u8; 32]) -> bool {
    use curve25519_dalek::edwards::CompressedEdwardsY;
    CompressedEdwardsY::from_slice(bytes)
        .ok()
        .and_then(|p| p.decompress())
        .is_none()
}

/// Build a signed Solana legacy transaction that
///   1. Issues an Associated Token Account Create-Idempotent instruction
///      so the destination ATA is materialized if needed,
///   2. Issues an SPL Token `TransferChecked` instruction for the transfer.
#[allow(clippy::too_many_arguments)]
pub fn build_spl_transfer_checked(
    from_owner: &[u8; 32],
    to_owner: &[u8; 32],
    mint: &[u8; 32],
    source_ata: &[u8; 32],
    dest_ata: &[u8; 32],
    amount_raw: u64,
    decimals: u8,
    recent_blockhash_b58: &str,
    private_key: &[u8; 64],
) -> Result<Vec<u8>, String> {
    use ed25519_dalek::{Signer, SigningKey};

    let blockhash_bytes = bs58::decode(recent_blockhash_b58)
        .into_vec()
        .map_err(|e| format!("invalid blockhash: {e}"))?;
    if blockhash_bytes.len() != 32 {
        return Err("blockhash must be 32 bytes".to_string());
    }
    let blockhash: [u8; 32] = blockhash_bytes.try_into().unwrap();

    // Account list order (all distinct pubkeys used anywhere).
    let system_program: [u8; 32] = [0u8; 32];
    let accounts: [&[u8; 32]; 8] = [
        from_owner,
        dest_ata,
        source_ata,
        to_owner,
        mint,
        &system_program,
        &SPL_TOKEN_PROGRAM_ID,
        &ASSOCIATED_TOKEN_PROGRAM_ID,
    ];

    let header = [1u8, 0u8, 5u8];

    let mut msg = Vec::new();
    msg.extend_from_slice(&header);
    msg.extend_from_slice(&compact_u16(accounts.len()));
    for a in &accounts {
        msg.extend_from_slice(a.as_ref());
    }
    msg.extend_from_slice(&blockhash);

    // 2 instructions.
    msg.extend_from_slice(&compact_u16(2));

    // -- Instruction 1: CreateIdempotent on ATA program --------------------
    msg.push(7u8); // program id index (ata program)
    let ata_accts: [u8; 6] = [0, 1, 3, 4, 5, 6];
    msg.extend_from_slice(&compact_u16(ata_accts.len()));
    msg.extend_from_slice(&ata_accts);
    let ata_data: [u8; 1] = [1u8];
    msg.extend_from_slice(&compact_u16(ata_data.len()));
    msg.extend_from_slice(&ata_data);

    // -- Instruction 2: SPL TransferChecked ------------------------------
    msg.push(6u8); // program id index (spl token)
    let xfer_accts: [u8; 4] = [2, 4, 1, 0];
    msg.extend_from_slice(&compact_u16(xfer_accts.len()));
    msg.extend_from_slice(&xfer_accts);
    let mut xfer_data = Vec::with_capacity(1 + 8 + 1);
    xfer_data.push(12u8); // TransferChecked
    xfer_data.extend_from_slice(&amount_raw.to_le_bytes());
    xfer_data.push(decimals);
    msg.extend_from_slice(&compact_u16(xfer_data.len()));
    msg.extend_from_slice(&xfer_data);

    // Sign the message bytes.
    let signing_key =
        SigningKey::from_bytes(&private_key[..32].try_into().map_err(|_| "privkey too short")?);
    let signature = signing_key.sign(&msg);

    let mut tx = Vec::new();
    tx.extend_from_slice(&compact_u16(1)); // 1 signature
    tx.extend_from_slice(signature.to_bytes().as_ref());
    tx.extend_from_slice(&msg);
    Ok(tx)
}

/// Solana compact-u16 encoding.
fn compact_u16(val: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut v = val as u16;
    loop {
        let mut byte = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if v == 0 {
            break;
        }
    }
    out
}
