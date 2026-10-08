//! Stellar credit assets: a payment of one, and the trustline an account
//! needs to hold one. Each is planned from one verified node's latest
//! ledger, refusing before anything is signed what the network would
//! refuse: a missing or unauthorized trustline on either side, a recipient
//! line without room, and balances short of the amount or the fee.

use crate::api::horizon::HorizonClient;
use crate::api::stellar_asset::StellarAsset;
use crate::send::error::SendError;
use serde::{Deserialize, Serialize};

/// A payment of a credit asset, as reviewed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparedStellarAssetPayment {
    pub sequence: u64,
    pub fee_stroops: u64,
    /// `CODE:ISSUER`.
    pub asset: String,
    pub amount_stroops: i64,
}

/// Setting a trustline's limit: the largest to hold an asset, zero to
/// remove an empty line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparedStellarChangeTrust {
    pub sequence: u64,
    pub fee_stroops: u64,
    /// `CODE:ISSUER`.
    pub asset: String,
    pub limit_stroops: i64,
}

fn refused(message: &'static str) -> SendError {
    SendError::Invalid(message.into())
}

impl PreparedStellarAssetPayment {
    pub(crate) async fn plan(
        client: &HorizonClient,
        chain: crate::registry::Chain,
        sender: &str,
        recipient: &str,
        asset: &str,
        amount_stroops: i64,
        fee_stroops: u64,
    ) -> Result<Self, SendError> {
        let asset = StellarAsset::parse(asset)?;
        if amount_stroops <= 0 {
            return Err(refused("Amount must be greater than zero"));
        }
        if sender == asset.issuer {
            return Err(refused(
                "This wallet issues the asset; Spectra sends only assets a wallet holds",
            ));
        }
        if sender == recipient {
            return Err(refused("A token payment goes to another account"));
        }
        let state = client
            .fetch_asset_state(chain, sender, &asset, Some(recipient))
            .await?;
        let holder = state
            .holder
            .ok_or_else(|| refused("This wallet's account does not exist on the network yet"))?;
        let line = holder
            .trustline(&asset)
            .ok_or_else(|| refused("This wallet has no trustline for the asset"))?;
        if !line.authorized {
            return Err(refused(
                "The asset's issuer has not authorized this wallet to hold it",
            ));
        }
        if line.balance.saturating_sub(line.selling_liabilities) < amount_stroops {
            return Err(SendError::InsufficientFunds(
                "Insufficient token balance".into(),
            ));
        }
        let spendable_xlm = holder
            .native_stroops
            .saturating_sub(holder.native_selling_liabilities)
            .saturating_sub(holder.minimum_balance(state.base_reserve) as i64);
        if spendable_xlm < fee_stroops as i64 {
            return Err(SendError::InsufficientFunds(
                "Insufficient XLM for the network fee".into(),
            ));
        }
        let destination = state.destination.flatten().ok_or_else(|| {
            refused("The recipient's account does not exist, and an asset payment cannot create it")
        })?;
        if recipient != asset.issuer {
            let line = destination
                .trustline(&asset)
                .ok_or_else(|| refused("The recipient has no trustline for the asset"))?;
            if !line.authorized {
                return Err(refused(
                    "The asset's issuer has not authorized the recipient to hold it",
                ));
            }
            let room = line
                .limit
                .saturating_sub(line.balance)
                .saturating_sub(line.buying_liabilities);
            if room < amount_stroops {
                return Err(refused(
                    "The recipient's trustline limit leaves no room for this amount",
                ));
            }
        }
        Ok(Self {
            sequence: holder
                .sequence
                .checked_add(1)
                .ok_or_else(|| refused("Sequence exhausted"))?,
            fee_stroops,
            asset: asset.identifier(),
            amount_stroops,
        })
    }

    pub(crate) fn asset(&self) -> Result<StellarAsset, SendError> {
        Ok(StellarAsset::parse(&self.asset)?)
    }

    /// The same payment read again, whatever the account's sequence.
    pub(crate) fn same_payment(&self, other: &Self) -> bool {
        Self {
            sequence: other.sequence,
            ..self.clone()
        } == *other
    }
}

impl PreparedStellarChangeTrust {
    /// A trustline of the largest limit, so the account can hold `asset`;
    /// or, with `remove`, a limit of zero on an empty line, which removes
    /// it. Returns the base reserve the line locks or frees.
    pub(crate) async fn plan(
        client: &HorizonClient,
        chain: crate::registry::Chain,
        holder: &str,
        asset: &str,
        remove: bool,
        fee_stroops: u64,
    ) -> Result<(Self, u64), SendError> {
        let asset = StellarAsset::parse(asset)?;
        if holder == asset.issuer {
            return Err(refused("A trust line is to another account"));
        }
        let state = client
            .fetch_asset_state(chain, holder, &asset, None)
            .await?;
        let account = state
            .holder
            .ok_or_else(|| refused("This wallet's account does not exist on the network yet"))?;
        let available = account
            .native_stroops
            .saturating_sub(account.native_selling_liabilities);
        let limit_stroops = if remove {
            let line = account
                .trustline(&asset)
                .ok_or_else(|| refused("This wallet has no trustline for the asset"))?;
            if line.balance != 0 {
                return Err(refused("The trust line still holds the token"));
            }
            if line.buying_liabilities != 0 {
                return Err(refused("Open offers still use this trustline"));
            }
            0
        } else {
            if account.trustline(&asset).is_some() {
                return Err(refused("This wallet already trusts the token"));
            }
            if !state.issuer_exists {
                return Err(refused("The asset's issuer does not exist on this network"));
            }
            // One more subentry raises the minimum balance by a base reserve.
            let minimum = account.minimum_balance(state.base_reserve) + state.base_reserve;
            if available < (minimum + fee_stroops) as i64 {
                return Err(SendError::InsufficientFunds(
                    "Insufficient XLM for the trustline's reserve".into(),
                ));
            }
            crate::send::stellar::MAX_TRUST_LIMIT
        };
        if available.saturating_sub(account.minimum_balance(state.base_reserve) as i64)
            < fee_stroops as i64
        {
            return Err(SendError::InsufficientFunds(
                "Insufficient XLM for the network fee".into(),
            ));
        }
        Ok((
            Self {
                sequence: account
                    .sequence
                    .checked_add(1)
                    .ok_or_else(|| refused("Sequence exhausted"))?,
                fee_stroops,
                asset: asset.identifier(),
                limit_stroops,
            },
            state.base_reserve,
        ))
    }

    pub(crate) fn asset(&self) -> Result<StellarAsset, SendError> {
        Ok(StellarAsset::parse(&self.asset)?)
    }

    pub(crate) fn removes(&self) -> bool {
        self.limit_stroops == 0
    }

    /// The same trustline change read again, whatever the sequence.
    pub(crate) fn same_change(&self, other: &Self) -> bool {
        Self {
            sequence: other.sequence,
            ..self.clone()
        } == *other
    }
}

#[cfg(test)]
#[path = "tests/stellar_issued.rs"]
mod tests;
