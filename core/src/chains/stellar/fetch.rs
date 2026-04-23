//! Stellar fetch paths (Horizon): native balance, per-asset balance, sequence,
//! base fee, and payments history.

use super::{
    HorizonAccount, HorizonFeeStats, HorizonPayments, StellarAssetBalance, StellarBalance,
    StellarClient, StellarHistoryEntry,
};

impl StellarClient {
    pub async fn fetch_balance(&self, address: &str) -> Result<StellarBalance, String> {
        let account: HorizonAccount = self.get(&format!("/accounts/{address}")).await?;
        let native = account
            .balances
            .iter()
            .find(|b| b.asset_type == "native")
            .ok_or("no native balance")?;
        // Stellar balances are decimal strings (e.g. "100.0000000")
        let stroops = parse_stellar_amount(&native.balance)?;
        Ok(StellarBalance {
            stroops,
            xlm_display: native.balance.clone(),
        })
    }

    /// Fetch a custom (issued) asset balance. `asset_code` is the alphanumeric
    /// asset code (e.g. "USDC"); `asset_issuer` is the G... issuer account.
    /// If the account has no trustline to this asset, returns a zero balance.
    pub async fn fetch_asset_balance(
        &self,
        address: &str,
        asset_code: &str,
        asset_issuer: &str,
    ) -> Result<StellarAssetBalance, String> {
        let account: HorizonAccount = self.get(&format!("/accounts/{address}")).await?;
        let entry = account.balances.iter().find(|b| {
            b.asset_type != "native" && b.asset_code == asset_code && b.asset_issuer == asset_issuer
        });
        match entry {
            Some(b) => {
                let stroops = parse_stellar_amount(&b.balance)?;
                Ok(StellarAssetBalance {
                    asset_code: asset_code.to_string(),
                    asset_issuer: asset_issuer.to_string(),
                    amount_stroops: stroops,
                    amount_display: b.balance.clone(),
                })
            }
            None => Ok(StellarAssetBalance {
                asset_code: asset_code.to_string(),
                asset_issuer: asset_issuer.to_string(),
                amount_stroops: 0,
                amount_display: "0.0000000".to_string(),
            }),
        }
    }

    pub async fn fetch_sequence(&self, address: &str) -> Result<u64, String> {
        let account: HorizonAccount = self.get(&format!("/accounts/{address}")).await?;
        account
            .sequence
            .parse::<u64>()
            .map_err(|e| format!("sequence parse: {e}"))
    }

    pub async fn fetch_base_fee(&self) -> Result<u64, String> {
        let stats: HorizonFeeStats = self.get("/fee_stats").await?;
        Ok(stats.fee_charged.mode.parse::<u64>().unwrap_or(100))
    }

    pub async fn fetch_history(&self, address: &str) -> Result<Vec<StellarHistoryEntry>, String> {
        let payments: HorizonPayments = self
            .get(&format!(
                "/accounts/{address}/payments?limit=50&order=desc&include_failed=false"
            ))
            .await?;
        Ok(payments
            .embedded
            .records
            .into_iter()
            .filter(|r| r.op_type == "payment" || r.op_type == "create_account")
            .map(|r| {
                let amount_stroops = parse_stellar_amount(&r.amount).unwrap_or(0);
                let is_incoming = r.to == address;
                StellarHistoryEntry {
                    txid: r.transaction_hash,
                    ledger: 0,
                    timestamp: r.created_at,
                    from: r.from,
                    to: r.to,
                    amount_stroops,
                    fee_charged: 0,
                    is_incoming,
                }
            })
            .collect())
    }
}

pub(super) fn parse_stellar_amount(s: &str) -> Result<i64, String> {
    // "100.0000000" -> stroops
    let parts: Vec<&str> = s.splitn(2, '.').collect();
    let whole: i64 = parts[0].parse().map_err(|e| format!("amount parse: {e}"))?;
    let frac_str = parts.get(1).copied().unwrap_or("0");
    let frac_padded = format!("{:0<7}", frac_str);
    let frac: i64 = frac_padded[..7].parse().unwrap_or(0);
    Ok(whole * 10_000_000 + frac)
}
