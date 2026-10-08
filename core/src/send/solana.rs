//! Solana send: native SOL transfer, token transfers (planned under the
//! mint's Token-2022 rules in `solana_token`), message compilation and
//! Ed25519 signing.

use crate::send::error::SendError;

use crate::send::keys::Ed25519Seed;

use crate::api::solana_json_rpc::SolanaClient;
use crate::derivation::solana::decode_b58_32;

// ── Transaction builder

/// Build a signed Solana legacy transaction for a native SOL transfer.
///
/// Wire format (legacy):
///   compact_u16(num_sigs) || sig[0..64] || message_bytes
///
/// Message:
///   [header: 3 bytes] [compact_u16(num_accounts)] [accounts..] [blockhash: 32]
///   [compact_u16(num_instructions)] [instruction: program_id_idx | compact_u16(accounts) | compact_u16(data)]
#[cfg(test)]
pub fn build_sol_transfer(
    from: &[u8; 32],
    to: &[u8; 32],
    lamports: u64,
    recent_blockhash_b58: &str,
    private_key: &Ed25519Seed,
) -> Result<Vec<u8>, SendError> {
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
    token_program: &[u8; 32],
) -> Result<[u8; 32], SendError> {
    find_program_address(&[wallet, token_program, mint], &ASSOCIATED_TOKEN_PROGRAM_ID)
}

/// `Pubkey::find_program_address`: the first bump from 255 down whose
/// address is off the curve, so no key can sign for it. Seeds are at most 32
/// bytes each and, with the bump, at most 16.
pub(crate) fn find_program_address(
    seeds: &[&[u8]],
    program: &[u8; 32],
) -> Result<[u8; 32], SendError> {
    use sha2::{Digest, Sha256};
    if seeds.len() >= 16 || seeds.iter().any(|seed| seed.len() > 32) {
        return Err(SendError::invalid("Invalid Solana program address seeds"));
    }
    for bump in (0u8..=255u8).rev() {
        let mut h = Sha256::new();
        for s in seeds {
            h.update(s);
        }
        h.update([bump]);
        h.update(program);
        h.update(b"ProgramDerivedAddress");
        let digest: [u8; 32] = h.finalize().into();
        if is_off_curve(&digest) {
            return Ok(digest);
        }
    }
    Err(SendError::Internal("failed to find PDA bump".into()))
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
#[cfg(test)]
pub fn build_spl_transfer_checked(
    from_owner: &[u8; 32],
    to_owner: &[u8; 32],
    mint: &[u8; 32],
    source_ata: &[u8; 32],
    dest_ata: &[u8; 32],
    token_program: &[u8; 32],
    amount_raw: u64,
    decimals: u8,
    recent_blockhash_b58: &str,
    private_key: &Ed25519Seed,
) -> Result<Vec<u8>, SendError> {
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
            (*token_program, false),
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
#[cfg(test)]
fn compile_and_sign(
    payer: &[u8; 32],
    account_metas: &[([u8; 32], bool)],
    instructions: &[(usize, Vec<usize>, Vec<u8>)],
    blockhash: &str,
    key: &Ed25519Seed,
) -> Result<Vec<u8>, SendError> {
    PreparedSolanaTransaction {
        payer: *payer,
        blockhash: blockhash.into(),
        message: compile_message(payer, account_metas, instructions, blockhash)?,
        account_seed: None,
        network_fee: None,
        stake_rent: None,
        token: None,
    }
    .sign(key)
}

pub(crate) fn prepare_staking_data(
    owner: &str,
    target: &str,
    amount: u64,
    rent: u64,
    blockhash: &str,
    seed: Option<&str>,
    action: crate::staking::StakingAction,
) -> Result<PreparedSolanaTransaction, SendError> {
    use sha2::{Digest, Sha256};
    let payer = decode_b58_32(owner)?;
    let target = decode_b58_32(target)?;
    let program = decode_b58_32(
        crate::registry::Chain::Solana
            .solana_stake_program()
            .map_err(SendError::invalid)?,
    )?;
    let (metas, instructions) = match action {
        crate::staking::StakingAction::Stake => {
            let seed = seed
                .filter(|s| !s.is_empty() && s.len() <= 32)
                .ok_or_else(|| SendError::invalid("Missing Solana stake-account seed"))?;
            let account: [u8; 32] = Sha256::new()
                .chain_update(payer)
                .chain_update(seed.as_bytes())
                .chain_update(program)
                .finalize()
                .into();
            let mut create = 3u32.to_le_bytes().to_vec();
            create.extend(payer);
            create.extend((seed.len() as u64).to_le_bytes());
            create.extend(seed.as_bytes());
            create.extend(
                amount
                    .checked_add(rent)
                    .ok_or_else(|| SendError::invalid("Solana stake plus rent overflow"))?
                    .to_le_bytes(),
            );
            create.extend(200u64.to_le_bytes());
            create.extend(program);
            let mut initialize = 0u32.to_le_bytes().to_vec();
            initialize.extend(payer);
            initialize.extend(payer);
            initialize.extend([0; 48]);
            (
                vec![
                    (payer, true),
                    (account, true),
                    ([0; 32], false),
                    (program, false),
                    (target, false),
                ],
                vec![
                    (2, vec![0, 1], create),
                    (3, vec![1], initialize),
                    (3, vec![1, 4, 0], 2u32.to_le_bytes().to_vec()),
                ],
            )
        }
        crate::staking::StakingAction::Unstake => (
            vec![(payer, true), (target, true), (program, false)],
            vec![(2, vec![1, 0], 5u32.to_le_bytes().to_vec())],
        ),
        crate::staking::StakingAction::Withdraw => {
            if amount == 0 {
                return Err(SendError::invalid("Solana withdrawal must be positive"));
            }
            let mut data = 4u32.to_le_bytes().to_vec();
            data.extend(amount.to_le_bytes());
            (
                vec![(payer, true), (target, true), (program, false)],
                vec![(2, vec![1, 0, 0], data)],
            )
        }
        _ => return Err(SendError::invalid("Solana rewards compound into stake")),
    };
    Ok(PreparedSolanaTransaction {
        payer,
        blockhash: blockhash.into(),
        message: compile_message(&payer, &metas, &instructions, blockhash)?,
        account_seed: seed.map(str::to_string),
        network_fee: None,
        stake_rent: Some(rent),
        token: None,
    })
}

pub(crate) fn stake_account_address(owner: &str, seed: &str) -> Result<String, SendError> {
    use sha2::{Digest, Sha256};
    let program = decode_b58_32(
        crate::registry::Chain::Solana
            .solana_stake_program()
            .map_err(SendError::invalid)?,
    )?;
    Ok(bs58::encode(
        Sha256::new()
            .chain_update(decode_b58_32(owner)?)
            .chain_update(seed.as_bytes())
            .chain_update(program)
            .finalize(),
    )
    .into_string())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PreparedSolanaTransaction {
    pub payer: [u8; 32],
    pub blockhash: String,
    pub message: Vec<u8>,
    pub account_seed: Option<String>,
    pub network_fee: Option<u64>,
    pub stake_rent: Option<u64>,
    /// The token transfer the message is, checked again before signing.
    pub token: Option<super::solana_token::PreparedSolanaTokenTransfer>,
}
impl PreparedSolanaTransaction {
    pub fn sign(&self, key: &Ed25519Seed) -> Result<Vec<u8>, SendError> {
        key.require_public_key(&self.payer)?;
        let mut tx = vec![1];
        tx.extend(key.sign(&self.message));
        tx.extend(&self.message);
        Ok(tx)
    }
}

pub(crate) async fn prepare_transfer(
    client: &SolanaClient,
    from: &str,
    to: &str,
    amount: u64,
    token: Option<(&str, u8)>,
) -> Result<PreparedSolanaTransaction, SendError> {
    let payer = decode_b58_32(from)?;
    let recipient = decode_b58_32(to)?;
    let token = match token {
        Some((mint, decimals)) => Some(
            super::solana_token::PreparedSolanaTokenTransfer::plan(
                client, &payer, &recipient, mint, decimals, amount,
            )
            .await?,
        ),
        None => None,
    };
    let blockhash = client.fetch_recent_blockhash().await?;
    let message = match &token {
        Some(token) => token.message(&payer, &recipient, &blockhash)?,
        None => {
            let mut data = 2u32.to_le_bytes().to_vec();
            data.extend(amount.to_le_bytes());
            compile_message(
                &payer,
                &[(payer, true), (recipient, true), ([0; 32], false)],
                &[(2, vec![0, 1], data)],
                &blockhash,
            )?
        }
    };
    if let Some(hook) = token.as_ref().and_then(|token| token.hook.as_ref())
        && let Some(error) = client.simulate_message(&message).await?
    {
        return Err(SendError::Invalid(crate::LocalizableMessage::new(
            "The token's transfer hook program %@ refused this transfer: %@",
            [hook.program.as_str(), error.as_str()],
        )));
    }
    Ok(PreparedSolanaTransaction {
        payer,
        blockhash,
        message,
        account_seed: None,
        network_fee: None,
        stake_rent: None,
        token,
    })
}

/// `TransferChecked`, or on a mint with a transfer-fee extension
/// `TransferCheckedWithFee` stating the fee. Both take the same accounts:
/// source, mint, destination, owner.
///
/// Stating the fee is the point: Token-2022 recomputes it when the
/// transaction executes and fails the transfer on any mismatch, so a fee
/// changed after review cannot withhold more from the recipient than the
/// review said.
pub(crate) fn token_transfer_data(amount: u64, decimals: u8, fee: Option<u64>) -> Vec<u8> {
    // Token-2022 `TransferFeeExtension` (26), sub-instruction
    // `TransferCheckedWithFee` (1); the base `TransferChecked` is 12.
    let mut data = if fee.is_some() { vec![26, 1] } else { vec![12] };
    data.extend(amount.to_le_bytes());
    data.push(decimals);
    if let Some(fee) = fee {
        data.extend(fee.to_le_bytes());
    }
    data
}

/// The SPL Token and Token-2022 programs, which own token accounts.
pub(crate) const TOKEN_PROGRAMS: [&str; 2] = [
    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
    "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb",
];
/// The most token accounts one transaction closes, well inside a legacy
/// transaction's 1232 bytes.
pub(crate) const MAX_CLOSED_ACCOUNTS: usize = 20;

/// Empty token accounts closed into their owner, and what the message was
/// compiled from, so it can be compiled again and compared before signing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PreparedSolanaAccountClosure {
    /// Each closed account and the token program that owns it, base58.
    pub accounts: Vec<(String, String)>,
    pub transaction: PreparedSolanaTransaction,
}

impl PreparedSolanaAccountClosure {
    pub(crate) fn prepare(
        owner: &str,
        accounts: Vec<(String, String)>,
        blockhash: &str,
        network_fee: u64,
    ) -> Result<Self, SendError> {
        let message = close_accounts_message(owner, &accounts, blockhash)?;
        Ok(Self {
            transaction: PreparedSolanaTransaction {
                payer: decode_b58_32(owner)?,
                blockhash: blockhash.into(),
                message,
                account_seed: None,
                network_fee: Some(network_fee),
                stake_rent: None,
                token: None,
            },
            accounts,
        })
    }

    /// Whether the message is exactly these closures, from `owner`.
    pub(crate) fn is_exact(&self, owner: &str) -> bool {
        close_accounts_message(owner, &self.accounts, &self.transaction.blockhash)
            .is_ok_and(|message| message == self.transaction.message)
    }
}

/// One SPL `CloseAccount` (9) per account: the account, its rent's
/// destination (the owner) and the owner signing.
fn close_accounts_message(
    owner: &str,
    accounts: &[(String, String)],
    blockhash: &str,
) -> Result<Vec<u8>, SendError> {
    if accounts.is_empty() || accounts.len() > MAX_CLOSED_ACCOUNTS {
        return Err(SendError::invalid(
            "Invalid number of Solana accounts to close",
        ));
    }
    let payer = decode_b58_32(owner)?;
    let mut metas = vec![(payer, true)];
    let mut instructions = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (account, program) in accounts {
        if !TOKEN_PROGRAMS.contains(&program.as_str()) {
            return Err(SendError::invalid("Not a token program"));
        }
        let account = decode_b58_32(account)?;
        if account == payer || !seen.insert(account) {
            return Err(SendError::invalid("Invalid Solana account to close"));
        }
        metas.push((account, true));
        metas.push((decode_b58_32(program)?, false));
        instructions.push((metas.len() - 1, vec![metas.len() - 2, 0, 0], vec![9]));
    }
    compile_message(&payer, &metas, &instructions, blockhash)
}

pub(crate) fn compile_message(
    payer: &[u8; 32],
    account_metas: &[([u8; 32], bool)],
    instructions: &[(usize, Vec<usize>, Vec<u8>)],
    blockhash: &str,
) -> Result<Vec<u8>, SendError> {
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
    Ok(msg)
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

#[cfg(test)]
mod token_transfer_data_tests {
    use super::*;

    /// Closing an SPL and a Token-2022 account, as @solana/web3.js compiles
    /// and signs it.
    #[test]
    fn closing_token_accounts_matches_web3_js() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/solana-close-accounts.json"
        ))
        .unwrap();
        let text = |value: &serde_json::Value| value.as_str().unwrap().to_string();
        let accounts: Vec<(String, String)> = fixture["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| (text(&row["account"]), text(&row["program"])))
            .collect();
        let owner = text(&fixture["owner"]);
        let prepared = PreparedSolanaAccountClosure::prepare(
            &owner,
            accounts.clone(),
            &text(&fixture["blockhash"]),
            5000,
        )
        .unwrap();
        assert_eq!(
            hex::encode(&prepared.transaction.message),
            text(&fixture["message"])
        );
        assert!(prepared.is_exact(&owner));
        let key = Ed25519Seed::from_hex(&text(&fixture["seed"])).unwrap();
        assert_eq!(
            hex::encode(prepared.transaction.sign(&key).unwrap()),
            text(&fixture["signed"])
        );
        // Not a token program, the owner itself, or one account twice.
        let blockhash = text(&fixture["blockhash"]);
        let system = "11111111111111111111111111111111".to_string();
        assert!(
            PreparedSolanaAccountClosure::prepare(
                &owner,
                vec![(accounts[0].0.clone(), system)],
                &blockhash,
                0
            )
            .is_err()
        );
        assert!(
            PreparedSolanaAccountClosure::prepare(
                &owner,
                vec![(owner.clone(), accounts[0].1.clone())],
                &blockhash,
                0
            )
            .is_err()
        );
        assert!(
            PreparedSolanaAccountClosure::prepare(
                &owner,
                vec![accounts[0].clone(), accounts[0].clone()],
                &blockhash,
                0
            )
            .is_err()
        );
    }

    /// Byte layouts from spl-token-2022's `TokenInstruction::TransferChecked`
    /// and `TransferFeeInstruction::TransferCheckedWithFee` packing.
    #[test]
    fn transfer_checked_and_with_fee_layouts() {
        let amount = 0x0102_0304_0506_0708u64;
        let mut plain = vec![12];
        plain.extend([8, 7, 6, 5, 4, 3, 2, 1]);
        plain.push(6);
        assert_eq!(token_transfer_data(amount, 6, None), plain);
        let mut with_fee = vec![26, 1];
        with_fee.extend([8, 7, 6, 5, 4, 3, 2, 1]);
        with_fee.push(6);
        with_fee.extend([0x39, 0x30, 0, 0, 0, 0, 0, 0]);
        assert_eq!(token_transfer_data(amount, 6, Some(12345)), with_fee);
    }
}

#[cfg(test)]
mod associated_token_account_tests {
    use super::*;

    /// Independent @solana/spl-token 0.4.14 vectors, for the legacy Token
    /// program and Token-2022.
    #[test]
    fn associated_token_accounts_match_spl_token_vectors() {
        let key = |b58: &str| crate::derivation::solana::decode_b58_32(b58).unwrap();
        let owner = key("HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk");
        let mint = [0x44; 32];
        for (program, expected) in [
            (
                "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
                "FF2BjgeRK2LgK8Lj4wY2CTJrmJAKV5ZPCdHqfq1tJLGi",
            ),
            (
                "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb",
                "Hzvpgx8hB4wZewvsYXSedgrgSb4yNycQRhufYeMaKuRM",
            ),
        ] {
            let ata = derive_associated_token_account(&owner, &mint, &key(program)).unwrap();
            assert_eq!(bs58::encode(ata).into_string(), expected);
        }
    }
}
