//! SUI transfer: resolve gas objects, build a local PTB, sign, execute.
use super::bcs;
use crate::fetch::chains::sui::{SuiClient, SuiSendResult};
use crate::send::keys::Ed25519Seed;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};

pub(crate) struct GasCoin {
    pub id: [u8; 32],
    pub version: u64,
    pub digest: [u8; 32],
    pub balance: u64,
}
pub(crate) struct PreparedSuiTransfer {
    sender: [u8; 32],
    bytes: Vec<u8>,
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
    Ok(PreparedSuiTransfer { sender, bytes })
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
impl SuiClient {
    pub async fn sign_and_send(
        &self,
        from: &str,
        to: &str,
        mist: u64,
        gas_budget: u64,
        key: &Ed25519Seed,
        public: &[u8; 32],
    ) -> Result<SuiSendResult, String> {
        key.require_public_key(public)?;
        bcs::address(from)?;
        bcs::address(to)?;
        if mist == 0 || gas_budget == 0 {
            return Err("Sui amount and gas budget must be positive".into());
        }
        let required = mist
            .checked_add(gas_budget)
            .ok_or("Sui amount plus gas overflow")?;
        let price = self.call("suix_getReferenceGasPrice", json!([])).await?;
        let gas_price = price
            .as_str()
            .and_then(|s| s.parse().ok())
            .ok_or("missing Sui reference gas price")?;
        let mut coins = Vec::new();
        let mut cursor = Value::Null;
        let mut total = 0u64;
        let mut cursors = std::collections::HashSet::new();
        loop {
            let page = self
                .call("suix_getCoins", json!([from, "0x2::sui::SUI", cursor, 50]))
                .await?;
            for row in page["data"].as_array().ok_or("missing Sui coins")? {
                let balance = row["balance"]
                    .as_str()
                    .and_then(|s| s.parse().ok())
                    .ok_or("invalid Sui coin balance")?;
                let id = bcs::address(row["coinObjectId"].as_str().ok_or("missing Sui coin id")?)?;
                let version = row["version"]
                    .as_str()
                    .and_then(|s| s.parse().ok())
                    .ok_or("missing Sui coin version")?;
                let digest = bs58::decode(row["digest"].as_str().ok_or("missing Sui coin digest")?)
                    .into_vec()
                    .map_err(|_| "invalid Sui coin digest")?
                    .try_into()
                    .map_err(|_| "Sui digest must be 32 bytes")?;
                total = total
                    .checked_add(balance)
                    .ok_or("Sui coin balance overflow")?;
                coins.push(GasCoin {
                    id,
                    version,
                    digest,
                    balance,
                });
                if total >= required || coins.len() == 256 {
                    break;
                }
            }
            if total >= required
                || coins.len() == 256
                || page["hasNextPage"].as_bool() == Some(false)
            {
                break;
            }
            cursor = page
                .get("nextCursor")
                .filter(|v| v.is_string())
                .cloned()
                .ok_or("missing Sui coin cursor")?;
            if !cursors.insert(cursor.to_string()) {
                return Err("repeated Sui coin cursor".into());
            }
        }
        let prepared = prepare_transfer(from, to, mist, gas_budget, gas_price, &coins)?;
        let (bytes, signature) = prepared.sign(key)?;
        self.execute_signed_tx(&bytes, &signature).await
    }
    pub async fn execute_signed_tx(
        &self,
        tx_bytes_b64: &str,
        sig_b64: &str,
    ) -> Result<SuiSendResult, String> {
        crate::send::payload::before_submission(
            serde_json::json!({"tx_bytes_b64":tx_bytes_b64,"sig_b64":sig_b64}).to_string(),
            "digest",
            None,
            None,
        )
        .await?;
        let result = self
            .call(
                "sui_executeTransactionBlock",
                json!([tx_bytes_b64,[sig_b64],{"showEffects":true},"WaitForLocalExecution"]),
            )
            .await?;
        if result
            .pointer("/effects/status/status")
            .and_then(Value::as_str)
            != Some("success")
        {
            return Err(format!("Sui execution did not succeed: {result}"));
        }
        let digest = result["digest"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("missing Sui transaction digest")?
            .to_string();
        Ok(SuiSendResult {
            digest,
            tx_bytes_b64: tx_bytes_b64.into(),
            sig_b64: sig_b64.into(),
        })
    }
}
