//! Transparent-only Zcash construction. Never asks a server to construct or sign.
use super::zcash::{expiry_height, sign_transaction, ZcashNetworkUpgrade};
use crate::{fetch::blockbook::BlockbookClient, registry::Chain};
use serde::{Deserialize, Serialize};

type Input = (String, u32, u64, Vec<u8>);
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PreparedZcashTransaction {
    pub inputs: Vec<Input>,
    pub outputs: Vec<(Vec<u8>, u64)>,
    pub sender: String,
    pub fee: u64,
    pub expiry_height: u32,
    pub upgrade: ZcashNetworkUpgrade,
}
impl PreparedZcashTransaction {
    pub fn sign(&self, key: &[u8]) -> Result<(Vec<u8>, String), String> {
        sign_transaction(
            &self.inputs,
            &self.outputs,
            self.expiry_height,
            key,
            self.upgrade,
        )
    }
}

fn address_script(address: &str, chain: Chain) -> Result<Vec<u8>, String> {
    let raw = bs58::decode(address)
        .with_check(None)
        .into_vec()
        .map_err(|e| e.to_string())?;
    if raw.len() != 22 {
        return Err("Invalid Zcash transparent address".into());
    }
    let (pkh, sh) = match chain {
        Chain::Zcash => ([0x1c, 0xb8], [0x1c, 0xbd]),
        Chain::ZcashTestnet => ([0x1d, 0x25], [0x1c, 0xba]),
        _ => return Err("Not a Zcash network".into()),
    };
    if raw[..2] == pkh {
        Ok(super::bitcoin_wire::p2pkh_script(
            raw[2..].try_into().unwrap(),
        ))
    } else if raw[..2] == sh {
        let mut script = vec![0xa9, 0x14];
        script.extend(&raw[2..]);
        script.push(0x87);
        Ok(script)
    } else {
        Err("Zcash address is on the wrong network".into())
    }
}

#[derive(Deserialize)]
struct Status {
    backend: Backend,
}
#[derive(Deserialize)]
struct Backend {
    blocks: u32,
    consensus: Consensus,
}
#[derive(Deserialize)]
struct Consensus {
    chaintip: String,
    nextblock: String,
}

impl BlockbookClient {
    /// Bind the actual backend to the selected genesis, height and known consensus schedule.
    /// No silent NU5 fallback when older Blockbook versions omit consensus information.
    pub(crate) async fn zcash_context(&self) -> Result<(u32, u32), String> {
        let expected_genesis = self.chain.zcash_genesis()?;
        let genesis: serde_json::Value = self.get("/api/v2/block-index/0").await?;
        if genesis["blockHash"].as_str() != Some(expected_genesis) {
            return Err("Zcash endpoint is on the wrong network".into());
        }
        let status: Status = self.get("/api/v2").await?;
        let height = status.backend.blocks;
        let branch = self
            .chain
            .zcash_consensus_branch(height.checked_add(1).ok_or("Height overflow")?)?;
        let parse = |s: &str| -> Result<u32, String> {
            if s.len() != 8 {
                return Err("Invalid Zcash consensus branch".into());
            }
            u32::from_str_radix(s, 16).map_err(|e| e.to_string())
        };
        if parse(&status.backend.consensus.nextblock)? != branch
            || parse(&status.backend.consensus.chaintip)?
                != self.chain.zcash_consensus_branch(height)?
        {
            return Err(
                "Zcash consensus upgrade is unsupported or inconsistent; update before sending"
                    .into(),
            );
        }
        Ok((height, branch))
    }

    pub(crate) async fn prepare_zcash(
        &self,
        sender: &str,
        recipient: &str,
        amount: u64,
        fee: Option<u64>,
    ) -> Result<PreparedZcashTransaction, String> {
        let sender_script = address_script(sender, self.chain)?;
        if sender_script.len() != 25 {
            return Err("Zcash sender must be P2PKH".into());
        }
        let recipient_script = address_script(recipient, self.chain)?;
        if amount < 546 {
            return Err("Zcash output is below the transparent dust threshold".into());
        }
        let (height, branch) = self.zcash_context().await?;
        let inputs: Vec<Input> = self
            .fetch_utxos(sender)
            .await?
            .into_iter()
            .filter(|u| u.confirmations > 0)
            .map(|u| (u.txid, u.vout, u.value_sat, sender_script.clone()))
            .collect();
        if inputs.is_empty() {
            return Err("No confirmed spendable Zcash inputs".into());
        }
        let mut seen = std::collections::HashSet::new();
        for u in &inputs {
            super::bitcoin_wire::decode_txid_le(&u.0)?;
            if !seen.insert((&u.0, u.1)) {
                return Err("Duplicate Zcash input".into());
            }
        }
        let total = inputs
            .iter()
            .try_fold(0_u64, |sum, input| sum.checked_add(input.2))
            .ok_or("Zcash input value overflow")?;
        if total > 21_000_000 * 100_000_000 || amount > total {
            return Err("Zcash amount exceeds the available money range".into());
        }
        // ZIP-317: standard P2PKH inputs are 150 logical bytes, outputs 34.
        // Allow two outputs when choosing the fee; grace actions are two.
        let conventional = 5_000_u64
            .checked_mul(u64::try_from(inputs.len().max(2)).map_err(|_| "Too many inputs")?)
            .ok_or("Fee overflow")?;
        let mut fee = fee.unwrap_or(conventional);
        if fee < conventional {
            return Err("Zcash fee is below the ZIP-317 conventional fee".into());
        }
        let change = super::accounting::checked_change(inputs.iter().map(|u| u.2), amount, fee)?;
        let mut outputs = vec![(recipient_script, amount)];
        if change >= 546 {
            outputs.push((sender_script, change));
        } else {
            fee = fee.checked_add(change).ok_or("Fee overflow")?;
        }
        let mut expiry = expiry_height(u64::from(height))?;
        // Never let the reviewed transaction span a scheduled branch change.
        while self.chain.zcash_consensus_branch(expiry)? != branch {
            expiry -= 1;
        }
        Ok(PreparedZcashTransaction {
            inputs,
            outputs,
            sender: sender.into(),
            fee,
            expiry_height: expiry,
            upgrade: ZcashNetworkUpgrade {
                version_group_id: 0x26a7_270a,
                consensus_branch_id: branch,
            },
        })
    }

    pub(crate) async fn validate_zcash_prepared(
        &self,
        prepared: &PreparedZcashTransaction,
    ) -> Result<(), String> {
        let (height, branch) = self.zcash_context().await?;
        if height >= prepared.expiry_height || branch != prepared.upgrade.consensus_branch_id {
            return Err(
                "Zcash transaction expired or consensus changed; build and review again".into(),
            );
        }
        let current = self.fetch_utxos(&prepared.sender).await?;
        for u in &prepared.inputs {
            if !current.iter().any(|c| {
                c.txid == u.0 && c.vout == u.1 && c.value_sat == u.2 && c.confirmations > 0
            }) {
                return Err("Zcash input changed or was spent; build and review again".into());
            }
        }
        Ok(())
    }
}
