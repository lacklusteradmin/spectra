//! XRP Ledger issued currencies: a payment of one, and the trust line an
//! account needs to hold one. Each is planned from one verified node's
//! validated ledger, refusing before anything is signed what the network
//! would refuse: a missing, frozen, unauthorized or full trust line on
//! either side, an issuer that does not let its currency ripple between
//! holders, and a balance short of the amount and the issuer's transfer fee.

use crate::api::xrpl_amount::{IouValue, XrplIssue};
use crate::api::xrpl_json_rpc::{XrplClient, XrplIssuer};
use crate::send::error::SendError;
use crate::send::xrp::IssuedAmount;
use serde::{Deserialize, Serialize};

/// The transfer rate that charges nothing.
const NO_FEE: u32 = 1_000_000_000;

/// The largest limit a trust line takes, which wallets set to accept any
/// amount.
pub(crate) const MAX_TRUST_LIMIT: &str = "9999999999999999e80";

/// A payment of an issued currency, as reviewed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparedXrpIssuedPayment {
    pub sequence: u32,
    pub fee_drops: u64,
    /// `CODE.rIssuer`.
    pub asset: String,
    /// What arrives, an exact decimal.
    pub amount: String,
    /// The most the sender spends where the issuer's transfer rate takes a
    /// share: the amount times the rate, rounded up.
    pub send_max: Option<String>,
    /// The issuer's rate in billionths; 1 000 000 000 takes nothing.
    pub transfer_rate: u32,
}

/// Setting a trust line's limit: the largest to hold an issued currency,
/// zero to remove an empty line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparedXrpTrustSet {
    pub sequence: u32,
    pub fee_drops: u64,
    /// `CODE.rIssuer`.
    pub asset: String,
    /// An exact decimal.
    pub limit: String,
}

fn refused(message: &'static str) -> SendError {
    SendError::Invalid(message.into())
}

fn issued(asset: &str, value: &str) -> Result<IssuedAmount, SendError> {
    Ok(IssuedAmount {
        issue: XrplIssue::parse(asset)?,
        value: IouValue::from_decimal(value)
            .ok_or_else(|| refused("Invalid XRP Ledger token amount"))?,
    })
}

impl PreparedXrpIssuedPayment {
    pub(crate) async fn plan(
        client: &XrplClient,
        chain: crate::registry::Chain,
        sender: &str,
        recipient: &str,
        asset: &str,
        amount: &str,
        fee_drops: u64,
    ) -> Result<Self, SendError> {
        let issue = XrplIssue::parse(asset)?;
        let value = IouValue::from_decimal(amount)
            .filter(|value| !value.is_zero())
            .ok_or_else(|| {
                refused(
                    "An XRP Ledger token amount is positive, with at most 16 significant digits",
                )
            })?;
        if sender == issue.issuer {
            return Err(refused(
                "This wallet issues the token; Spectra sends only tokens a wallet holds",
            ));
        }
        if sender == recipient {
            return Err(refused("A token payment goes to another account"));
        }
        let state = client
            .fetch_issue_state(chain, sender, &issue, Some(recipient))
            .await?;
        let (xrp_drops, _) = state
            .holder
            .ok_or_else(|| refused("This wallet's account does not exist on the network yet"))?;
        let issuer = state
            .issuer
            .ok_or_else(|| refused("The token's issuer does not exist on this network"))?;
        let line = state
            .holder_line
            .ok_or_else(|| refused("This wallet has no trust line for the token"))?;
        let to_issuer = recipient == issue.issuer;
        if line.deep_frozen
            || (!to_issuer && (line.frozen || issuer.flags & XrplIssuer::GLOBAL_FREEZE != 0))
        {
            return Err(refused("The token's issuer has frozen it for this wallet"));
        }
        let requires_auth = issuer.flags & XrplIssuer::REQUIRE_AUTH != 0;
        if requires_auth && !line.peer_authorized {
            return Err(refused(
                "The token's issuer has not authorized this wallet to hold it",
            ));
        }
        let (_, destination_line) = state.destination.flatten().ok_or_else(|| {
            refused("The recipient's account does not exist, and a token payment cannot create it")
        })?;
        if state.deposit_authorized == Some(false) {
            return Err(refused(
                "The recipient accepts payments only from accounts it has authorized",
            ));
        }
        if !to_issuer {
            let destination = destination_line
                .ok_or_else(|| refused("The recipient has no trust line for the token"))?;
            if destination.deep_frozen {
                return Err(refused(
                    "The token's issuer has frozen the recipient's trust line",
                ));
            }
            if requires_auth && !destination.peer_authorized {
                return Err(refused(
                    "The token's issuer has not authorized the recipient to hold it",
                ));
            }
            if !IouValue::sum_fits(&value, &destination.balance, &destination.limit) {
                return Err(refused(
                    "The recipient's trust line limit leaves no room for this amount",
                ));
            }
            // Between two holders the currency ripples through its issuer,
            // which must allow it on one of the two lines.
            if line.no_ripple_peer && destination.no_ripple_peer {
                return Err(refused(
                    "The token's issuer does not let it move between holders",
                ));
            }
        }
        let rate = if to_issuer {
            NO_FEE
        } else {
            issuer.transfer_rate
        };
        let cost = value.times_rate_up(rate).ok_or_else(|| {
            refused("The amount and the issuer's fee exceed what the ledger holds")
        })?;
        if line.balance < cost {
            return Err(SendError::InsufficientFunds(
                "Insufficient token balance".into(),
            ));
        }
        if xrp_drops < fee_drops {
            return Err(SendError::InsufficientFunds(
                "Insufficient XRP for the network fee".into(),
            ));
        }
        crate::send::xrp::validate_drops(u128::from(fee_drops))?;
        Ok(Self {
            sequence: client.fetch_sequence(sender).await?,
            fee_drops,
            asset: issue.identifier(),
            amount: value.to_decimal(),
            send_max: (rate != NO_FEE).then(|| cost.to_decimal()),
            transfer_rate: rate,
        })
    }

    /// The same payment read again, whatever the account's sequence.
    pub(crate) fn same_payment(&self, other: &Self) -> bool {
        Self {
            sequence: other.sequence,
            ..self.clone()
        } == *other
    }

    pub(crate) fn amounts(&self) -> Result<(IssuedAmount, Option<IssuedAmount>), SendError> {
        Ok((
            issued(&self.asset, &self.amount)?,
            self.send_max
                .as_deref()
                .map(|send_max| issued(&self.asset, send_max))
                .transpose()?,
        ))
    }

    /// What the issuer's rate does to the payment, when it takes anything.
    pub(crate) fn terms(&self) -> Option<crate::send::stages::AssetTransferTerms> {
        let send_max = self.send_max.as_ref()?;
        let (paid, delivered) = (IouValue::parse(send_max)?, IouValue::parse(&self.amount)?);
        Some(crate::send::stages::AssetTransferTerms {
            debited: send_max.clone(),
            received: self.amount.clone(),
            fee: paid.minus(&delivered).1,
            hook_program: None,
            carried_native: None,
        })
    }
}

impl PreparedXrpTrustSet {
    /// A trust line of the largest limit, so the account can hold `asset`;
    /// or, with `remove`, a limit of zero on an empty line, which deletes it.
    pub(crate) async fn plan(
        client: &XrplClient,
        chain: crate::registry::Chain,
        holder: &str,
        asset: &str,
        remove: bool,
        fee_drops: u64,
    ) -> Result<(Self, u64), SendError> {
        let issue = XrplIssue::parse(asset)?;
        if holder == issue.issuer {
            return Err(refused("A trust line is to another account"));
        }
        let state = client
            .fetch_issue_state(chain, holder, &issue, None)
            .await?;
        let (xrp_drops, owner_count) = state
            .holder
            .ok_or_else(|| refused("This wallet's account does not exist on the network yet"))?;
        let issuer = state
            .issuer
            .ok_or_else(|| refused("The token's issuer does not exist on this network"))?;
        let limit = if remove {
            let line = state
                .holder_line
                .ok_or_else(|| refused("This wallet has no trust line for the token"))?;
            if !line.balance.is_zero() {
                return Err(refused("The trust line still holds the token"));
            }
            if !line.limit_peer.is_zero() {
                return Err(refused(
                    "The issuer extends its own trust on this line, so the line stays",
                ));
            }
            IouValue::ZERO
        } else {
            if state
                .holder_line
                .as_ref()
                .is_some_and(|line| !line.limit.is_zero())
            {
                return Err(refused("This wallet already trusts the token"));
            }
            if issuer.flags & XrplIssuer::DISALLOW_INCOMING_TRUSTLINE != 0 {
                return Err(refused("The token's issuer accepts no new trust lines"));
            }
            // rippled waives the reserve for an account's first two objects.
            let reserve = if owner_count < 2 {
                0
            } else {
                state.reserve_base + (owner_count + 1) * state.reserve_increment
            };
            if xrp_drops < reserve.max(fee_drops) {
                return Err(SendError::InsufficientFunds(
                    "Insufficient XRP for the trust line's reserve".into(),
                ));
            }
            IouValue::parse(MAX_TRUST_LIMIT).expect("the largest limit")
        };
        if xrp_drops < fee_drops {
            return Err(SendError::InsufficientFunds(
                "Insufficient XRP for the network fee".into(),
            ));
        }
        crate::send::xrp::validate_drops(u128::from(fee_drops))?;
        Ok((
            Self {
                sequence: client.fetch_sequence(holder).await?,
                fee_drops,
                asset: issue.identifier(),
                limit: limit.to_decimal(),
            },
            state.reserve_increment,
        ))
    }

    pub(crate) fn limit(&self) -> Result<IssuedAmount, SendError> {
        issued(&self.asset, &self.limit)
    }

    pub(crate) fn removes(&self) -> bool {
        crate::decimal::is_zero(&self.limit)
    }

    /// The same trust line change read again, whatever the sequence.
    pub(crate) fn same_change(&self, other: &Self) -> bool {
        Self {
            sequence: other.sequence,
            ..self.clone()
        } == *other
    }
}

#[cfg(test)]
#[path = "tests/xrp_issued.rs"]
mod tests;
