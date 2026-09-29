//! SUI transfer: resolve gas objects, build a local PTB, sign, execute.
use super::bcs;
use crate::api::sui_json_rpc::SuiClient;
use crate::send::keys::Ed25519Seed;
use base64::{Engine, engine::general_purpose::STANDARD};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct GasCoin {
    pub id: [u8; 32],
    pub version: u64,
    pub digest: [u8; 32],
    pub balance: u64,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PreparedSuiTransfer {
    sender: [u8; 32],
    pub(crate) bytes: Vec<u8>,
    pub(crate) objects: Vec<GasCoin>,
}

pub(crate) fn prepare_transfer(
    from: &str,
    to: &str,
    amount: u64,
    gas_budget: u64,
    gas_price: u64,
    coins: &[GasCoin],
) -> Result<PreparedSuiTransfer, String> {
    let sender = bcs::address(from)?;
    let to = bcs::address(to)?;
    if amount == 0 || gas_budget == 0 || gas_price == 0 || coins.is_empty() || coins.len() > 256 {
        return Err("invalid Sui amount or gas payment".into());
    }
    let mut seen = std::collections::HashSet::new();
    let total = coins.iter().try_fold(0u64, |sum, c| {
        if !seen.insert(c.id) {
            return Err("duplicate Sui gas object");
        }
        sum.checked_add(c.balance).ok_or("Sui gas balance overflow")
    })?;
    if total
        < amount
            .checked_add(gas_budget)
            .ok_or("Sui amount plus gas overflow")?
    {
        return Err("insufficient SUI for amount plus gas budget".into());
    }
    let mut bytes = vec![0, 0]; // TransactionData::V1, TransactionKind::ProgrammableTransaction
    bcs::uleb(2, &mut bytes); // inputs: Pure(amount), Pure(recipient)
    bytes.push(0);
    bcs::bytes(&amount.to_le_bytes(), &mut bytes);
    bytes.push(0);
    bcs::bytes(&to, &mut bytes);
    bcs::uleb(2, &mut bytes); // commands
    bytes.extend_from_slice(&[2, 0, 1, 1, 0, 0]); // SplitCoins(GasCoin, [Input(0)])
    bytes.extend_from_slice(&[1, 1, 3, 0, 0, 0, 0, 1, 1, 0]); // TransferObjects([NestedResult(0,0)], Input(1))
    bytes.extend_from_slice(&sender);
    bcs::uleb(coins.len(), &mut bytes);
    for coin in coins {
        bytes.extend_from_slice(&coin.id);
        bytes.extend_from_slice(&coin.version.to_le_bytes());
        bcs::bytes(&coin.digest, &mut bytes);
    }
    bytes.extend_from_slice(&sender); // gas owner
    bytes.extend_from_slice(&gas_price.to_le_bytes());
    bytes.extend_from_slice(&gas_budget.to_le_bytes());
    bytes.push(0); // TransactionExpiration::None
    Ok(PreparedSuiTransfer {
        sender,
        bytes,
        objects: coins.to_vec(),
    })
}
impl PreparedSuiTransfer {
    pub(crate) fn sign(self, key: &Ed25519Seed) -> Result<(String, String), String> {
        let public = key.public_key();
        let expected = blake2b_simd::Params::new()
            .hash_length(32)
            .to_state()
            .update(&[0])
            .update(&public)
            .finalize();
        if self.sender != expected.as_bytes() {
            return Err("Sui sender does not match signing seed".into());
        }
        let digest = blake2b_simd::Params::new()
            .hash_length(32)
            .to_state()
            .update(&[0, 0, 0])
            .update(&self.bytes)
            .finalize();
        let mut signature = vec![0];
        signature.extend_from_slice(&key.sign(digest.as_bytes()));
        signature.extend_from_slice(&public);
        Ok((STANDARD.encode(self.bytes), STANDARD.encode(signature)))
    }
}

pub(crate) async fn prepare_native_transfer(
    client: &SuiClient,
    from: &str,
    to: &str,
    mist: u64,
    gas_budget: u64,
) -> Result<PreparedSuiTransfer, String> {
    bcs::address(from)?;
    bcs::address(to)?;
    if mist == 0 || gas_budget == 0 {
        return Err("Sui amount and gas budget must be positive".into());
    }
    let required = mist
        .checked_add(gas_budget)
        .ok_or("Sui amount plus gas overflow")?;
    let gas_price = client.fetch_reference_gas_price().await?;
    let mut coins = Vec::new();
    let mut cursor: Option<String> = None;
    let mut total = 0u64;
    let mut cursors = std::collections::HashSet::new();
    loop {
        let page = client.fetch_sui_coins_page(from, cursor.as_deref()).await?;
        for coin in page.coins {
            total = total
                .checked_add(coin.balance)
                .ok_or("Sui coin balance overflow")?;
            coins.push(GasCoin {
                id: bcs::address(&coin.object_id)?,
                version: coin.version,
                digest: coin.digest,
                balance: coin.balance,
            });
            if total >= required || coins.len() == 256 {
                break;
            }
        }
        if total >= required || coins.len() == 256 {
            break;
        }
        let Some(next) = page.next_cursor else {
            break;
        };
        if !cursors.insert(next.clone()) {
            return Err("repeated Sui coin cursor".into());
        }
        cursor = Some(next);
    }
    prepare_transfer(from, to, mist, gas_budget, gas_price, &coins)
}
