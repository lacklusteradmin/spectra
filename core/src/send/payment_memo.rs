//! A payment's destination tag or memo: what tells a recipient that shares
//! one account among many — an exchange's deposit address — whose deposit
//! it is. XRP Ledger payments carry a `DestinationTag`, Stellar transactions
//! a text or ID memo, TON messages a text comment, and Cosmos and Hedera
//! transactions a text memo. A send takes one, binds it into the review
//! digest, and is refused without one when the destination asks for it:
//! XRP's `lsfRequireDestTag`, Stellar's SEP-29 `config.memo_required`. TON,
//! the Cosmos SDK and Hedera have no such flag.

use crate::registry::{Chain, PaymentMemoKind};
use crate::send::error::SendError;
use serde::{Deserialize, Serialize};

/// The longest TON comment a payment carries, in bytes: what one cell holds
/// after the 32-bit op, so the message stays within the network's static fee.
const TON_COMMENT_BYTES: usize = 123;

/// A destination tag or memo, of a kind the payment's network takes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Record)]
pub struct PaymentMemo {
    pub kind: PaymentMemoKind,
    /// The tag or ID in decimal, or the text exactly as it is sent.
    pub value: String,
}

/// A Stellar memo as the transaction encodes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StellarMemo<'a> {
    Text(&'a str),
    Id(u64),
}

/// The memo kinds a payment on `chain` can carry, the first the default:
/// what a send composer offers. Empty where payments carry none.
#[uniffi::export]
pub fn payment_memo_kinds(chain: Chain) -> Vec<PaymentMemoKind> {
    chain.payment_memo_kinds().to_vec()
}

/// `value` as a canonical decimal of `T`, or `message`.
fn number<T: std::str::FromStr + ToString>(
    value: &str,
    message: &'static str,
) -> Result<String, SendError> {
    let value = value.trim();
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SendError::invalid(message));
    }
    value
        .parse::<T>()
        .map(|n| n.to_string())
        .map_err(|_| SendError::invalid(message))
}

impl PaymentMemo {
    /// This memo as a payment on `chain` sends it: of a kind the network
    /// takes, a number in range written in canonical decimal, a text within
    /// the protocol's length.
    pub(crate) fn validated(&self, chain: Chain) -> Result<Self, SendError> {
        if !chain.payment_memo_kinds().contains(&self.kind) {
            return Err(SendError::Invalid(crate::error::LocalizableMessage::new(
                "A %@ payment carries no such destination tag or memo.",
                [chain.chain_display_name()],
            )));
        }
        let value = match self.kind {
            PaymentMemoKind::DestinationTag => number::<u32>(
                &self.value,
                "A destination tag is a whole number from 0 to 4294967295.",
            )?,
            PaymentMemoKind::MemoId => number::<u64>(
                &self.value,
                "A memo ID is a whole number from 0 to 18446744073709551615.",
            )?,
            PaymentMemoKind::MemoText => {
                let limit = chain.memo_text_limit().unwrap_or(0);
                if self.value.is_empty() || self.value.len() > limit {
                    return Err(SendError::Invalid(crate::error::LocalizableMessage::new(
                        "A text memo is 1 to %@ bytes of text.",
                        [limit.to_string()],
                    )));
                }
                // A Hedera memo with a zero byte is refused by the network.
                if chain.is_hedera() && self.value.contains('\0') {
                    return Err(SendError::invalid("A text memo holds no zero byte."));
                }
                self.value.clone()
            }
            PaymentMemoKind::Comment => {
                if self.value.is_empty() || self.value.len() > TON_COMMENT_BYTES {
                    return Err(SendError::invalid("A comment is 1 to 123 bytes of text."));
                }
                self.value.clone()
            }
        };
        Ok(Self {
            kind: self.kind,
            value,
        })
    }

    /// The XRP `DestinationTag` `memo` names, if any.
    pub(crate) fn destination_tag(memo: Option<&Self>) -> Result<Option<u32>, SendError> {
        match memo {
            None => Ok(None),
            Some(Self {
                kind: PaymentMemoKind::DestinationTag,
                value,
            }) => value
                .parse()
                .map(Some)
                .map_err(|_| SendError::invalid("Invalid destination tag")),
            Some(_) => Err(SendError::invalid(
                "An XRP payment carries a destination tag, not a memo",
            )),
        }
    }

    /// The Stellar memo `memo` names, if any.
    pub(crate) fn stellar(memo: Option<&Self>) -> Result<Option<StellarMemo<'_>>, SendError> {
        match memo {
            None => Ok(None),
            Some(Self {
                kind: PaymentMemoKind::MemoText,
                value,
            }) if !value.is_empty()
                && Chain::Stellar
                    .memo_text_limit()
                    .is_some_and(|limit| value.len() <= limit) =>
            {
                Ok(Some(StellarMemo::Text(value)))
            }
            Some(Self {
                kind: PaymentMemoKind::MemoId,
                value,
            }) => value
                .parse()
                .map(|id| Some(StellarMemo::Id(id)))
                .map_err(|_| SendError::invalid("Invalid memo ID")),
            Some(_) => Err(SendError::invalid(
                "A Stellar payment carries a text or ID memo",
            )),
        }
    }

    /// The text memo a Cosmos or Hedera transaction on `chain` carries for
    /// `memo`, empty when it names none.
    pub(crate) fn text(memo: Option<&Self>, chain: Chain) -> Result<&str, SendError> {
        match memo {
            None => Ok(""),
            Some(Self {
                kind: PaymentMemoKind::MemoText,
                value,
            }) if !value.is_empty()
                && !(chain.is_hedera() && value.contains('\0'))
                && chain
                    .memo_text_limit()
                    .is_some_and(|limit| value.len() <= limit) =>
            {
                Ok(value)
            }
            Some(_) => Err(SendError::invalid("This transaction carries a text memo")),
        }
    }

    /// The TON comment `memo` names, if any.
    pub(crate) fn ton_comment(memo: Option<&Self>) -> Result<Option<&str>, SendError> {
        match memo {
            None => Ok(None),
            Some(Self {
                kind: PaymentMemoKind::Comment,
                value,
            }) if !value.is_empty() && value.len() <= TON_COMMENT_BYTES => Ok(Some(value)),
            Some(_) => Err(SendError::invalid("A TON payment carries a text comment")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memo(kind: PaymentMemoKind, value: &str) -> PaymentMemo {
        PaymentMemo {
            kind,
            value: value.into(),
        }
    }

    #[test]
    fn each_network_takes_its_own_kinds_in_range() {
        use PaymentMemoKind::*;
        for (chain, kind, value, canonical) in [
            (Chain::Xrp, DestinationTag, " 0042 ", "42"),
            (
                Chain::XrpTestnet,
                DestinationTag,
                "4294967295",
                "4294967295",
            ),
            (
                Chain::Stellar,
                MemoId,
                "18446744073709551615",
                "18446744073709551615",
            ),
            (Chain::StellarTestnet, MemoText, " deposit ", " deposit "),
            (Chain::Stellar, MemoText, &"x".repeat(28), &"x".repeat(28)),
            (Chain::Ton, Comment, "104729", "104729"),
            (
                Chain::TonTestnet,
                Comment,
                &"x".repeat(123),
                &"x".repeat(123),
            ),
            (Chain::CosmosHub, MemoText, "104857600", "104857600"),
            (
                Chain::CosmosHub,
                MemoText,
                &"x".repeat(512),
                &"x".repeat(512),
            ),
            (
                Chain::Injective,
                MemoText,
                &"x".repeat(256),
                &"x".repeat(256),
            ),
            (Chain::Hedera, MemoText, "104857600", "104857600"),
            (Chain::Hedera, MemoText, &"é".repeat(50), &"é".repeat(50)),
        ] {
            assert_eq!(
                memo(kind, value).validated(chain).unwrap(),
                memo(kind, canonical),
                "{chain} {kind:?} {value}"
            );
        }
        for (chain, kind, value) in [
            (Chain::Xrp, DestinationTag, "4294967296"),
            (Chain::Xrp, DestinationTag, "-1"),
            (Chain::Xrp, DestinationTag, ""),
            (Chain::Xrp, DestinationTag, "1e3"),
            (Chain::Xrp, MemoText, "deposit"),
            (Chain::Stellar, DestinationTag, "1"),
            (Chain::Stellar, MemoId, "18446744073709551616"),
            (Chain::Stellar, MemoText, ""),
            // 28 bytes is the limit, not 28 characters.
            (Chain::Stellar, MemoText, &"é".repeat(15)),
            (Chain::Ethereum, MemoText, "deposit"),
            (Chain::Bitcoin, DestinationTag, "1"),
            (Chain::Ton, Comment, ""),
            // 123 bytes is the limit, not 123 characters.
            (Chain::Ton, Comment, &"é".repeat(62)),
            (Chain::Ton, MemoText, "deposit"),
            (Chain::Stellar, Comment, "deposit"),
            // A Cosmos memo is text, up to the network's own limit in bytes.
            (Chain::CosmosHub, MemoText, ""),
            (Chain::CosmosHub, MemoText, &"x".repeat(513)),
            (Chain::Injective, MemoText, &"x".repeat(257)),
            (Chain::Celestia, MemoId, "1"),
            (Chain::Osmosis, Comment, "deposit"),
            // A Hedera memo is text, up to 100 bytes, without a zero byte.
            (Chain::Hedera, MemoText, ""),
            (Chain::Hedera, MemoText, &"x".repeat(101)),
            (Chain::Hedera, MemoText, "dep\0osit"),
            (Chain::Hedera, MemoId, "1"),
            (Chain::Hedera, DestinationTag, "1"),
        ] {
            assert!(
                memo(kind, value).validated(chain).is_err(),
                "{chain} {kind:?} {value}"
            );
        }
    }

    #[test]
    fn only_xrp_stellar_ton_cosmos_and_hedera_payments_carry_one() {
        for chain in Chain::all() {
            let kinds = payment_memo_kinds(chain);
            match chain.mainnet_counterpart() {
                cosmos if cosmos.is_cosmos() => assert_eq!(kinds, [PaymentMemoKind::MemoText]),
                Chain::Xrp => assert_eq!(kinds, [PaymentMemoKind::DestinationTag]),
                Chain::Stellar => {
                    assert_eq!(kinds, [PaymentMemoKind::MemoText, PaymentMemoKind::MemoId])
                }
                Chain::Ton => assert_eq!(kinds, [PaymentMemoKind::Comment]),
                Chain::Hedera => assert_eq!(kinds, [PaymentMemoKind::MemoText]),
                _ => assert!(kinds.is_empty(), "{chain}"),
            }
        }
    }
}
