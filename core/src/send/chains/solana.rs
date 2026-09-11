//! Solana send: native SOL transfer + SPL TransferChecked (with idempotent ATA
//! create). Ed25519 signing + sendTransaction RPC broadcast.

use crate::send::keys::Ed25519Seed;
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
        private_key_bytes: &Ed25519Seed,
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

        self.broadcast_raw(&base64::engine::general_purpose::STANDARD.encode(&raw_tx))
            .await
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
        private_key_bytes: &Ed25519Seed,
    ) -> Result<SolanaSendResult, String> {
        let to_owner = decode_b58_32(to_owner_b58)?;
        let mint = decode_b58_32(mint_b58)?;

        let source_ata = derive_associated_token_account(from_owner_pubkey, &mint)?;
        let dest_ata = derive_associated_token_account(&to_owner, &mint)?;

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

        self.broadcast_raw(&base64::engine::general_purpose::STANDARD.encode(&raw_tx))
            .await
    }

    /// Broadcast an already-signed transaction given as a base64 string.
    pub async fn broadcast_raw(&self, signed_tx_base64: &str) -> Result<SolanaSendResult, String> {
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

// ── Transaction builder

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
    private_key: &Ed25519Seed,
) -> Result<Vec<u8>, String> {
    let mut data = 2u32.to_le_bytes().to_vec();
    data.extend_from_slice(&lamports.to_le_bytes());
    compile_and_sign(
        from,
        &[(*from, true), (*to, true), ([0; 32], false)],
        &[(2, vec![0, 1], data)],
        recent_blockhash_b58,
        private_key,
    )
}

// ── SPL helpers: ATA derivation and SPL Transfer transaction builder

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
    private_key: &Ed25519Seed,
) -> Result<Vec<u8>, String> {
    let mut data = vec![12];
    data.extend_from_slice(&amount_raw.to_le_bytes());
    data.push(decimals);
    compile_and_sign(
        from_owner,
        &[
            (*from_owner, true),
            (*dest_ata, true),
            (*source_ata, true),
            (*to_owner, false),
            (*mint, false),
            ([0; 32], false),
            (SPL_TOKEN_PROGRAM_ID, false),
            (ASSOCIATED_TOKEN_PROGRAM_ID, false),
        ],
        &[
            (7, vec![0, 1, 3, 4, 5, 6], vec![1]),
            (6, vec![2, 4, 1, 0], data),
        ],
        recent_blockhash_b58,
        private_key,
    )
}

/// Compile account identities once across all instructions. Aliases merge
/// writable privileges, then every instruction is remapped to the unique keys.
/// These transfer instructions have exactly one signer: the fee payer.
fn compile_and_sign(
    payer: &[u8; 32],
    account_metas: &[([u8; 32], bool)],
    instructions: &[(usize, Vec<usize>, Vec<u8>)],
    blockhash: &str,
    key: &Ed25519Seed,
) -> Result<Vec<u8>, String> {
    key.require_public_key(payer)?;
    let blockhash = decode_b58_32(blockhash)?;
    let mut accounts = vec![(*payer, true)];
    for (pubkey, writable) in account_metas {
        if let Some(existing) = accounts.iter_mut().find(|a| a.0 == *pubkey) {
            existing.1 |= writable;
        } else {
            accounts.push((*pubkey, *writable));
        }
    }
    // Stable sort: signer first, followed by writable and readonly unsigned.
    accounts.sort_by_key(|(pubkey, writable)| (pubkey != payer, !writable));
    let readonly = accounts.iter().filter(|a| !a.1).count();
    let mut msg = vec![1, 0, readonly as u8];
    msg.extend(compact_u16(accounts.len()));
    for (pubkey, _) in &accounts {
        msg.extend(pubkey);
    }
    msg.extend(blockhash);
    msg.extend(compact_u16(instructions.len()));
    let index = |original: usize| -> u8 {
        accounts
            .iter()
            .position(|a| a.0 == account_metas[original].0)
            .expect("registered account") as u8
    };
    for (program, metas, data) in instructions {
        msg.push(index(*program));
        msg.extend(compact_u16(metas.len()));
        msg.extend(metas.iter().map(|i| index(*i)));
        msg.extend(compact_u16(data.len()));
        msg.extend(data);
    }
    let mut tx = vec![1];
    tx.extend(key.sign(&msg));
    tx.extend(msg);
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
