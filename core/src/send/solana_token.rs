//! Solana token transfers under the mint's Token-2022 rules: one
//! `TransferChecked` from the owner's associated account, stating the
//! transfer fee the program withholds and carrying the extra accounts a
//! transfer hook's validation account lists. Every account the transfer
//! touches is checked before anything is signed.

use crate::api::solana_json_rpc::{SolanaClient, TransferTokenAccount};
use crate::derivation::solana::decode_b58_32;
use crate::send::error::SendError;
use crate::send::solana::{
    ASSOCIATED_TOKEN_PROGRAM_ID, compile_message, derive_associated_token_account,
    find_program_address, token_transfer_data,
};
use serde::{Deserialize, Serialize};

/// Slots before an epoch ends within which a fee that changes with the next
/// epoch is refused. A blockhash lives for 150 blocks, and a transaction that
/// lands after the change states the wrong fee and fails.
const FEE_EPOCH_MARGIN_SLOTS: u64 = 300;

/// One token transfer as reviewed, and what its message is compiled from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparedSolanaTokenTransfer {
    pub mint: String,
    /// The token program that owns the mint.
    pub program: String,
    pub decimals: u8,
    /// What leaves the owner's account, in the token's smallest units.
    pub amount: u64,
    /// The Token-2022 transfer fee withheld from `amount`, which the
    /// transfer states and the program checks again when it executes.
    pub fee: Option<u64>,
    pub hook: Option<PreparedTransferHook>,
}

/// A Token-2022 transfer hook: the program the token program calls, the
/// validation account that lists what it needs, and those accounts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparedTransferHook {
    pub program: String,
    pub validation_account: String,
    pub accounts: Vec<HookAccount>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct HookAccount {
    pub address: String,
    pub writable: bool,
}

fn b58(key: &[u8; 32]) -> String {
    bs58::encode(key).into_string()
}

impl PreparedSolanaTokenTransfer {
    /// Read the mint and every account the transfer touches, and refuse what
    /// the network would refuse or what would not arrive as reviewed.
    pub(crate) async fn plan(
        client: &SolanaClient,
        owner: &[u8; 32],
        recipient: &[u8; 32],
        mint: &str,
        decimals: u8,
        amount: u64,
    ) -> Result<Self, SendError> {
        let transfer_mint = client.fetch_transfer_mint(mint).await?;
        if decimals != transfer_mint.decimals {
            return Err(SendError::invalid("SPL decimals changed; review again"));
        }
        if amount == 0 {
            return Err(SendError::invalid("Amount must be greater than zero"));
        }
        let mint_key = decode_b58_32(mint)?;
        let program = transfer_mint.program;
        let source = derive_associated_token_account(owner, &mint_key, &program)?;
        let destination = derive_associated_token_account(recipient, &mint_key, &program)?;
        let belongs = |account: &TransferTokenAccount, holder: &[u8; 32]| {
            account.program == program && account.mint == mint && account.owner == b58(holder)
        };
        match client.fetch_transfer_token_account(&b58(&source)).await? {
            Some(account) if !belongs(&account, owner) => {
                return Err(SendError::invalid(
                    "The sending token account belongs to another owner or token",
                ));
            }
            Some(account) if account.frozen => {
                return Err(SendError::invalid(
                    "Your token account is frozen by the token's issuer",
                ));
            }
            Some(account) if account.amount >= amount => {}
            _ => {
                return Err(SendError::InsufficientFunds(
                    "Insufficient token balance".into(),
                ));
            }
        }
        match client
            .fetch_transfer_token_account(&b58(&destination))
            .await?
        {
            Some(account) if !belongs(&account, recipient) => {
                return Err(SendError::invalid(
                    "The recipient's token account belongs to another owner or token",
                ));
            }
            Some(account) if account.frozen => {
                return Err(SendError::invalid(
                    "The recipient's token account is frozen by the token's issuer",
                ));
            }
            Some(account) if account.requires_memo => {
                return Err(SendError::invalid(
                    "The recipient's token account accepts only transfers with a memo",
                ));
            }
            Some(account) if account.refuses_public_credits => {
                return Err(SendError::invalid(
                    "The recipient's token account accepts only confidential transfers",
                ));
            }
            Some(_) => {}
            None if transfer_mint.default_frozen => {
                return Err(SendError::invalid(
                    "The recipient has no account for this token, and the account a transfer creates starts frozen",
                ));
            }
            None => {}
        }
        let fee = match transfer_mint.transfer_fee {
            None => None,
            Some(config) => {
                let epoch = client.fetch_epoch_info().await?;
                let fee = config.at(epoch.epoch).fee(amount);
                if config.at(epoch.epoch.saturating_add(1)).fee(amount) != fee
                    && epoch.slots_in_epoch - epoch.slot_index <= FEE_EPOCH_MARGIN_SLOTS
                {
                    return Err(SendError::invalid(
                        "The token's transfer fee changes in the epoch about to start; try again once it has",
                    ));
                }
                Some(fee)
            }
        };
        let hook = match transfer_mint.transfer_hook {
            None => None,
            Some(hook_program) => Some(
                resolve_transfer_hook(
                    client,
                    &hook_program,
                    [source, mint_key, destination, *owner],
                    amount,
                )
                .await?,
            ),
        };
        Ok(Self {
            mint: mint.to_string(),
            program: b58(&program),
            decimals,
            amount,
            fee,
            hook,
        })
    }

    /// An idempotent create of the recipient's associated account, then the
    /// transfer, with the hook's accounts after the transfer's own four.
    pub(crate) fn message(
        &self,
        owner: &[u8; 32],
        recipient: &[u8; 32],
        blockhash: &str,
    ) -> Result<Vec<u8>, SendError> {
        let mint = decode_b58_32(&self.mint)?;
        let program = decode_b58_32(&self.program)?;
        let source = derive_associated_token_account(owner, &mint, &program)?;
        let destination = derive_associated_token_account(recipient, &mint, &program)?;
        let mut metas = vec![
            (*owner, true),
            (destination, true),
            (source, true),
            (*recipient, false),
            (mint, false),
            ([0; 32], false),
            (program, false),
            (ASSOCIATED_TOKEN_PROGRAM_ID, false),
        ];
        let mut transfer = vec![2, 4, 1, 0];
        if let Some(hook) = &self.hook {
            let extra = hook
                .accounts
                .iter()
                .map(|account| Ok((decode_b58_32(&account.address)?, account.writable)))
                .chain([
                    decode_b58_32(&hook.program).map(|key| (key, false)),
                    decode_b58_32(&hook.validation_account).map(|key| (key, false)),
                ])
                .collect::<Result<Vec<_>, crate::derivation::error::DerivationError>>()?;
            for meta in extra {
                metas.push(meta);
                transfer.push(metas.len() - 1);
            }
        }
        compile_message(
            owner,
            &metas,
            &[
                (7, vec![0, 1, 3, 4, 5, 6], vec![1]),
                (
                    6,
                    transfer,
                    token_transfer_data(self.amount, self.decimals, self.fee),
                ),
            ],
            blockhash,
        )
    }

    /// What the token's rules do to this transfer, when they do anything: a
    /// fee withheld on the way, or a program run that can refuse it.
    pub(crate) fn terms(&self) -> Option<crate::send::stages::AssetTransferTerms> {
        let fee = self.fee.unwrap_or(0);
        if fee == 0 && self.hook.is_none() {
            return None;
        }
        let units =
            |amount: u64| crate::decimal::from_units(u128::from(amount), self.decimals.into());
        Some(crate::send::stages::AssetTransferTerms {
            debited: units(self.amount),
            received: units(self.amount.saturating_sub(fee)),
            fee: units(fee),
            hook_program: self.hook.as_ref().map(|hook| hook.program.clone()),
            carried_native: None,
            recipient_registration: None,
        })
    }

    /// The same transfer read again: the same fee, hook accounts and token,
    /// whatever epoch it was computed in.
    pub(crate) fn same_transfer(&self, other: &Self) -> bool {
        self == other
    }
}

// ── Transfer hooks
//
// spl-transfer-hook-interface's offchain `add_extra_account_metas_for_execute`
// and spl-tlv-account-resolution's `ExtraAccountMeta::resolve`: the hook's
// validation account holds, under the `Execute` discriminator, a list of
// account descriptions resolved against the `Execute` instruction's accounts
// (source, mint, destination, authority, validation account, then each extra
// account as it resolves) and its data (discriminator, then the amount).

/// The first eight bytes of sha256("spl-transfer-hook-interface:execute").
fn execute_discriminator() -> [u8; 8] {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(b"spl-transfer-hook-interface:execute");
    hash[..8].try_into().expect("eight bytes")
}

/// The hook program's account listing what it needs for `mint`.
pub(crate) fn extra_account_metas_address(
    mint: &[u8; 32],
    program: &[u8; 32],
) -> Result<[u8; 32], SendError> {
    find_program_address(&[b"extra-account-metas", mint], program)
}

/// One `ExtraAccountMeta` as stored: 35 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExtraAccountMeta {
    discriminator: u8,
    address_config: [u8; 32],
    is_signer: bool,
    is_writable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Seed {
    Literal(Vec<u8>),
    InstructionData { index: u8, length: u8 },
    AccountKey { index: u8 },
    AccountData { account: u8, offset: u8, length: u8 },
}

fn malformed() -> SendError {
    SendError::invalid("The token's transfer hook lists accounts Spectra cannot read")
}

/// The `Execute` entry of a validation account's TLV data.
fn extra_account_metas(data: &[u8]) -> Result<Vec<ExtraAccountMeta>, SendError> {
    let execute = execute_discriminator();
    let mut rest = data;
    while rest.len() >= 12 {
        let (discriminator, tail) = rest.split_at(8);
        if discriminator.iter().all(|byte| *byte == 0) {
            break;
        }
        let length = u32::from_le_bytes(tail[..4].try_into().expect("four bytes")) as usize;
        let value = tail[4..].get(..length).ok_or_else(malformed)?;
        if discriminator == execute {
            let count = u32::from_le_bytes(
                value
                    .get(..4)
                    .ok_or_else(malformed)?
                    .try_into()
                    .expect("four bytes"),
            ) as usize;
            let entries = &value[4..];
            if entries.len() < count.checked_mul(35).ok_or_else(malformed)? {
                return Err(malformed());
            }
            return Ok(entries
                .as_chunks::<35>()
                .0
                .iter()
                .take(count)
                .map(|entry| ExtraAccountMeta {
                    discriminator: entry[0],
                    address_config: entry[1..33].try_into().expect("32 bytes"),
                    is_signer: entry[33] != 0,
                    is_writable: entry[34] != 0,
                })
                .collect());
        }
        rest = &tail[4 + length..];
    }
    Err(malformed())
}

/// Seeds packed into an address config, up to the first empty one.
fn seeds(config: &[u8; 32]) -> Result<Vec<Seed>, SendError> {
    let mut seeds = Vec::new();
    let mut at = 0;
    while at < config.len() {
        let rest = &config[at..];
        let (seed, size) = match rest[0] {
            0 => break,
            1 => {
                let length = *rest.get(1).ok_or_else(malformed)? as usize;
                let bytes = rest.get(2..2 + length).ok_or_else(malformed)?;
                (Seed::Literal(bytes.to_vec()), 2 + length)
            }
            2 => match rest.get(1..3) {
                Some(&[index, length]) => (Seed::InstructionData { index, length }, 3),
                _ => return Err(malformed()),
            },
            3 => (
                Seed::AccountKey {
                    index: *rest.get(1).ok_or_else(malformed)?,
                },
                2,
            ),
            4 => match rest.get(1..4) {
                Some(&[account, offset, length]) => (
                    Seed::AccountData {
                        account,
                        offset,
                        length,
                    },
                    4,
                ),
                _ => return Err(malformed()),
            },
            _ => return Err(malformed()),
        };
        seeds.push(seed);
        at += size;
    }
    Ok(seeds)
}

/// The `Execute` instruction's accounts as they resolve, with each one's
/// data read when a seed first needs it.
struct ExecuteAccounts<'a> {
    client: &'a SolanaClient,
    keys: Vec<([u8; 32], bool)>,
    data: Vec<Option<Option<Vec<u8>>>>,
}

impl ExecuteAccounts<'_> {
    fn key(&self, index: u8) -> Result<[u8; 32], SendError> {
        self.keys
            .get(usize::from(index))
            .map(|(key, _)| *key)
            .ok_or_else(malformed)
    }

    async fn data(&mut self, index: u8) -> Result<&[u8], SendError> {
        let index = usize::from(index);
        let key = self.keys.get(index).ok_or_else(malformed)?.0;
        if self.data[index].is_none() {
            self.data[index] = Some(
                self.client
                    .fetch_account_data(&b58(&key))
                    .await?
                    .map(|(_, data)| data),
            );
        }
        self.data[index]
            .as_ref()
            .and_then(Option::as_deref)
            .ok_or_else(|| {
                SendError::invalid(
                    "The token's transfer hook needs an account that does not exist yet",
                )
            })
    }

    fn push(&mut self, key: [u8; 32], writable: bool) {
        self.keys.push((key, writable));
        self.data.push(None);
    }
}

async fn resolve_pda(
    accounts: &mut ExecuteAccounts<'_>,
    seeds: &[Seed],
    instruction: &[u8],
    program: &[u8; 32],
) -> Result<[u8; 32], SendError> {
    let mut parts: Vec<Vec<u8>> = Vec::new();
    for seed in seeds {
        parts.push(match seed {
            Seed::Literal(bytes) => bytes.clone(),
            Seed::InstructionData { index, length } => instruction
                .get(usize::from(*index)..usize::from(*index) + usize::from(*length))
                .ok_or_else(malformed)?
                .to_vec(),
            Seed::AccountKey { index } => accounts.key(*index)?.to_vec(),
            Seed::AccountData {
                account,
                offset,
                length,
            } => accounts
                .data(*account)
                .await?
                .get(usize::from(*offset)..usize::from(*offset) + usize::from(*length))
                .ok_or_else(malformed)?
                .to_vec(),
        });
    }
    let parts: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
    find_program_address(&parts, program)
}

/// Resolve the hook's extra accounts, refusing any it would have sign.
async fn resolve_transfer_hook(
    client: &SolanaClient,
    program: &[u8; 32],
    [source, mint, destination, owner]: [[u8; 32]; 4],
    amount: u64,
) -> Result<PreparedTransferHook, SendError> {
    let validation = extra_account_metas_address(&mint, program)?;
    let (validation_owner, data) = client
        .fetch_account_data(&b58(&validation))
        .await?
        .ok_or_else(|| {
            SendError::invalid("The token's transfer hook has no account listing what it needs")
        })?;
    if validation_owner != *program {
        return Err(malformed());
    }
    let metas = extra_account_metas(&data)?;
    let mut instruction = execute_discriminator().to_vec();
    instruction.extend(amount.to_le_bytes());
    let mut accounts = ExecuteAccounts {
        client,
        keys: Vec::new(),
        data: Vec::new(),
    };
    for key in [source, mint, destination, owner, validation] {
        accounts.push(key, false);
    }
    accounts.data[4] = Some(Some(data));
    for meta in metas {
        if meta.is_signer {
            return Err(SendError::invalid(
                "The token's transfer hook asks for a signature Spectra does not give",
            ));
        }
        let key = match meta.discriminator {
            0 => meta.address_config,
            1 => {
                resolve_pda(
                    &mut accounts,
                    &seeds(&meta.address_config)?,
                    &instruction,
                    program,
                )
                .await?
            }
            2 => match meta.address_config[..3] {
                [1, index, ..] => instruction
                    .get(usize::from(index)..usize::from(index) + 32)
                    .ok_or_else(malformed)?
                    .try_into()
                    .expect("32 bytes"),
                [2, account, offset] => accounts
                    .data(account)
                    .await?
                    .get(usize::from(offset)..usize::from(offset) + 32)
                    .ok_or_else(malformed)?
                    .try_into()
                    .expect("32 bytes"),
                _ => return Err(malformed()),
            },
            external if external >= 0x80 => {
                let owner_program = accounts.key(external - 0x80)?;
                resolve_pda(
                    &mut accounts,
                    &seeds(&meta.address_config)?,
                    &instruction,
                    &owner_program,
                )
                .await?
            }
            _ => return Err(malformed()),
        };
        // An account the instruction already holds read-only stays
        // read-only, as the token program passes it to the hook.
        let writable = meta.is_writable
            && accounts
                .keys
                .iter()
                .filter(|(existing, _)| *existing == key)
                .map(|(_, writable)| *writable)
                .reduce(|a, b| a || b)
                .unwrap_or(true);
        accounts.push(key, writable);
    }
    Ok(PreparedTransferHook {
        program: b58(program),
        validation_account: b58(&validation),
        accounts: accounts.keys[5..]
            .iter()
            .map(|(key, writable)| HookAccount {
                address: b58(key),
                writable: *writable,
            })
            .collect(),
    })
}

#[cfg(test)]
#[path = "tests/solana_token.rs"]
mod tests;
