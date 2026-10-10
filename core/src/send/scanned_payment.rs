//! Payment codes both ways: what a scanned one asks for — whom to pay, and
//! the amount and memo it names — and the one a receiving wallet shows to
//! ask for an amount, written so this reader reads it back unchanged.
//!
//! A QR code is rarely a bare address. Wallets encode BIP-21 and its
//! descendants — `bitcoin:bc1q…?amount=0.1`, EIP-681's `ethereum:0x…@1`,
//! Solana Pay, SEP-7's `web+stellar:pay?destination=…`, `ton://transfer/EQ…`
//! — and a scanner that keeps only the address makes the payer retype the
//! amount and, worse, an exchange's deposit tag. What the code names is read
//! against the network and the asset being sent, and refused when it asks
//! for something else: a token's code is not a payment in the network's
//! coin, and a code pinned to another EVM chain is not a payment on this one.

use crate::SpectraBridgeError;
use crate::registry::{Chain, PaymentMemoKind, PaymentUriFormat};
use crate::send::flow::normalize_address;
use crate::send::payment_memo::PaymentMemo;
use crate::validation::address::{AddressValidationRequest, validate_address};

/// A scanned code read for the composer.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ScannedPayment {
    /// The recipient, in the form the store and the signer keep.
    pub address: String,
    /// The amount the code asks for, a decimal in whole units of the asset
    /// being sent. Absent when the code names none, or names it in a coin
    /// other than the one selected.
    pub amount: Option<String>,
    /// The destination tag or memo the code asks for, where the network
    /// takes one.
    pub memo: Option<PaymentMemo>,
}

/// Read `payload` as a payment on `chain` of the asset `token_contract`
/// names (the network's coin when `None`).
///
/// The chain is required and there is no fallback: there is no address
/// without a chain to judge it against, and the camera's string never reaches
/// the send field unchecked.
#[uniffi::export]
pub fn read_scanned_payment(
    chain: Chain,
    token_contract: Option<String>,
    payload: String,
) -> Result<ScannedPayment, SpectraBridgeError> {
    let code = PaymentCode::parse(&payload);
    let request = code.request(chain)?;
    let address = recipient(chain, &request)?;
    let selected = crate::tokens::normalize_token_identifier(token_contract, chain);
    let amount = match (&request.token, &selected) {
        (Some(asked), Some(selected))
            if crate::tokens::normalize_token_identifier(Some(asked.clone()), chain).as_ref()
                == Some(selected) =>
        {
            request.amount
        }
        (Some(_), _) => {
            return Err(SpectraBridgeError::invalid(
                "The scanned code asks to be paid in another asset than the one selected.",
            ));
        }
        // The amount is in the network's coin; it says nothing of a token.
        (None, Some(_)) => None,
        (None, None) => request.amount,
    };
    Ok(ScannedPayment {
        address,
        amount,
        memo: memo(chain, &code)?,
    })
}

/// The address alone in a scanned code, for a field that takes an address
/// rather than a payment — a contact, an NFT's or a pool's recipient. The
/// code's amount, memo and asset are not this field's to apply, so they are
/// neither read nor refused; its network still must be `chain`.
#[uniffi::export]
pub fn read_scanned_address(chain: Chain, payload: String) -> Result<String, SpectraBridgeError> {
    recipient(chain, &PaymentCode::parse(&payload).request(chain)?)
}

/// The first candidate `chain` accepts, in the form the store keeps, once the
/// code's pinned network — if it names one — is `chain`.
fn recipient(chain: Chain, request: &PaymentRequest) -> Result<String, SpectraBridgeError> {
    let address = request
        .recipients
        .iter()
        .map(|candidate| normalize_address(chain, candidate))
        .find(|normalized| is_valid(chain, normalized))
        .ok_or_else(|| {
            SpectraBridgeError::refused(
                "The scanned code holds no %@ address.",
                [chain.chain_display_name()],
            )
        })?;
    if let Some(pinned) = request.evm_chain_id
        && chain.evm_chain_id().ok() != Some(pinned)
    {
        return Err(SpectraBridgeError::refused(
            "The scanned code asks for a payment on another network than %@.",
            [chain.chain_display_name()],
        ));
    }
    Ok(address)
}

/// Whether a receiving wallet on `chain` can ask for an amount in a code
/// other wallets read: the networks with a payment URI format.
#[uniffi::export]
pub fn payment_requests_supported(chain: Chain) -> bool {
    chain.payment_uri_format().is_some()
}

/// The payment code asking for `amount` of `chain`'s coin, and `memo`, at
/// `address`: what the receive page encodes in its QR code and shares.
///
/// The address must be `chain`'s and the amount a positive decimal within
/// the coin's precision; a memo must be one the network's payments carry. A
/// network with no format wallets read is refused rather than given one
/// only Spectra reads.
#[uniffi::export]
pub fn payment_request_uri(
    chain: Chain,
    address: String,
    amount: Option<String>,
    memo: Option<PaymentMemo>,
) -> Result<String, SpectraBridgeError> {
    let format = chain.payment_uri_format().ok_or_else(|| {
        SpectraBridgeError::refused(
            "%@ has no payment request format other wallets read. Share the address and the amount instead.",
            [chain.chain_display_name()],
        )
    })?;
    let address = normalize_address(chain, &address);
    if !is_valid(chain, &address) {
        return Err(SpectraBridgeError::refused(
            "That is not a %@ address.",
            [chain.chain_display_name()],
        ));
    }
    let decimals = u32::from(chain.native_decimals());
    let units = match amount
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        None => None,
        Some(text) => Some(
            crate::send::amount_input::parse_raw_amount(text, decimals)
                .ok()
                .filter(|units| *units > 0)
                .ok_or_else(|| {
                    SpectraBridgeError::refused(
                        "Enter an amount of %@ above zero, to its precision.",
                        [chain.coin_symbol()],
                    )
                })?,
        ),
    };
    let whole = units.map(|units| crate::decimal::from_units(units, decimals));
    let memo = memo.map(|memo| memo.validated(chain)).transpose()?;
    let mut params: Vec<(&str, String)> = Vec::new();
    let head = match format {
        PaymentUriFormat::Bip21(scheme) => {
            params.extend(whole.map(|whole| ("amount", whole)));
            // A CashAddr address carries its scheme already.
            if address.starts_with(&format!("{scheme}:")) {
                address.clone()
            } else {
                format!("{scheme}:{address}")
            }
        }
        PaymentUriFormat::Eip681 => {
            params.extend(units.map(|units| ("value", units.to_string())));
            let id = chain.evm_chain_id().map_err(SpectraBridgeError::failure)?;
            format!("ethereum:{address}@{id}")
        }
        PaymentUriFormat::SolanaPay => {
            params.extend(whole.map(|whole| ("amount", whole)));
            format!("solana:{address}")
        }
        PaymentUriFormat::Xrpl => {
            params.extend(whole.map(|whole| ("amount", whole)));
            format!("ripple:{address}")
        }
        PaymentUriFormat::Sep7 => {
            params.push(("destination", address.clone()));
            params.extend(whole.map(|whole| ("amount", whole)));
            "web+stellar:pay".to_string()
        }
        PaymentUriFormat::MoneroUri => {
            params.extend(whole.map(|whole| ("tx_amount", whole)));
            format!("monero:{address}")
        }
        PaymentUriFormat::TonTransfer => {
            params.extend(units.map(|units| ("amount", units.to_string())));
            format!("ton://transfer/{address}")
        }
    };
    match (&memo, format) {
        (None, _) => {}
        (Some(memo), PaymentUriFormat::Xrpl) => params.push(("dt", memo.value.clone())),
        (Some(memo), PaymentUriFormat::Sep7) => {
            params.push(("memo", memo.value.clone()));
            params.push((
                "memo_type",
                match memo.kind {
                    PaymentMemoKind::MemoId => "MEMO_ID",
                    _ => "MEMO_TEXT",
                }
                .to_string(),
            ));
        }
        (Some(_), _) => {
            return Err(SpectraBridgeError::refused(
                "A %@ payment request carries no memo.",
                [chain.chain_display_name()],
            ));
        }
    }
    if params.is_empty() {
        return Ok(head);
    }
    let query = params
        .iter()
        .map(|(key, value)| format!("{key}={}", percent_encoded(value)))
        .collect::<Vec<_>>()
        .join("&");
    Ok(format!("{head}?{query}"))
}

/// A query value with everything but the unreserved characters escaped.
fn percent_encoded(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn is_valid(chain: Chain, address: &str) -> bool {
    validate_address(AddressValidationRequest {
        kind: chain.address_validation_kind().to_string(),
        value: address.to_string(),
    })
    .is_valid
}

/// A payload split into its parts, without yet judging any of them.
struct PaymentCode {
    /// The payload, trimmed.
    whole: String,
    /// Lowercased, when the payload has one.
    scheme: Option<String>,
    /// What follows the scheme, up to the query: an address, or EIP-681's
    /// `address@chain/function`, or SEP-7's operation.
    target: String,
    /// The query's parameters, percent-decoded, in order.
    params: Vec<(String, String)>,
}

/// What a code asks for once its scheme has been read.
struct PaymentRequest {
    /// The substrings that could be the recipient, most literal first.
    recipients: Vec<String>,
    /// The token the code asks to be paid in, as the code spells it.
    token: Option<String>,
    /// EIP-155 chain id an EIP-681 code is pinned to.
    evm_chain_id: Option<u64>,
    /// The amount, already a whole-unit decimal.
    amount: Option<String>,
}

impl PaymentCode {
    fn parse(payload: &str) -> Self {
        let whole = payload.trim().to_string();
        let (head, query) = match whole.split_once('?') {
            Some((head, query)) => (head, query.split('#').next().unwrap_or(query)),
            None => (whole.split('#').next().unwrap_or(&whole), ""),
        };
        let (scheme, target) = match head.split_once(':') {
            // `scheme:` is letters, digits, `+`, `-` and `.`; anything else
            // before a colon is not a scheme (a TON raw address `0:abc…`).
            Some((scheme, rest))
                if scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                    && scheme
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) =>
            {
                (
                    Some(scheme.to_ascii_lowercase()),
                    rest.trim_start_matches('/'),
                )
            }
            _ => (None, head),
        };
        let params = query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .filter_map(|(key, value)| Some((percent_decoded(key)?, percent_decoded(value)?)))
            .collect();
        let target = target.to_string();
        Self {
            whole,
            scheme,
            target,
            params,
        }
    }

    /// The first parameter of any of `names`.
    fn param(&self, names: &[&str]) -> Option<&str> {
        self.params
            .iter()
            .find(|(key, _)| names.contains(&key.as_str()))
            .map(|(_, value)| value.trim())
            .filter(|value| !value.is_empty())
    }

    fn request(&self, chain: Chain) -> Result<PaymentRequest, SpectraBridgeError> {
        match self.scheme.as_deref() {
            Some("ethereum") => self.eip681(chain),
            Some("web+stellar") => Ok(PaymentRequest {
                recipients: self
                    .param(&["destination"])
                    .map(str::to_string)
                    .into_iter()
                    .collect(),
                token: self
                    .param(&["asset_code"])
                    .zip(self.param(&["asset_issuer"]))
                    .map(|(code, issuer)| format!("{code}:{issuer}")),
                evm_chain_id: None,
                amount: self.whole_units(&["amount"])?,
            }),
            // TON's `amount` is in nanotons.
            Some("ton") => Ok(PaymentRequest {
                recipients: self.candidates(),
                token: self.param(&["jetton"]).map(str::to_string),
                evm_chain_id: None,
                amount: self.base_units(&["amount"], chain)?,
            }),
            _ => Ok(PaymentRequest {
                recipients: self.candidates(),
                // Solana Pay names an SPL token by its mint.
                token: self.param(&["spl-token"]).map(str::to_string),
                evm_chain_id: None,
                // Monero's `tx_amount`; BIP-21's and the rest's `amount`.
                amount: self.whole_units(&["amount", "tx_amount"])?,
            }),
        }
    }

    /// EIP-681: `ethereum:[pay-]<address>[@<chain id>][/<function>]?…`. A
    /// plain payment's `value` is in wei. A token `transfer` is addressed to
    /// the token's contract, and pays the `address` parameter — reading the
    /// contract as the recipient would send the funds to the token itself.
    fn eip681(&self, chain: Chain) -> Result<PaymentRequest, SpectraBridgeError> {
        let target = self.target.strip_prefix("pay-").unwrap_or(&self.target);
        let (contract_and_chain, function) = match target.split_once('/') {
            Some((head, function)) => (head, Some(function.trim_end_matches('/'))),
            None => (target, None),
        };
        let (contract, evm_chain_id) = match contract_and_chain.split_once('@') {
            Some((address, id)) => (
                address,
                Some(id.trim().parse::<u64>().map_err(|_| {
                    SpectraBridgeError::invalid(
                        "The scanned code names a network Spectra cannot read.",
                    )
                })?),
            ),
            None => (contract_and_chain, None),
        };
        match function {
            None | Some("") => Ok(PaymentRequest {
                recipients: vec![contract.to_string()],
                token: None,
                evm_chain_id,
                amount: self.base_units(&["value"], chain)?,
            }),
            Some("transfer") => Ok(PaymentRequest {
                recipients: self
                    .param(&["address"])
                    .map(str::to_string)
                    .into_iter()
                    .collect(),
                token: Some(contract.to_string()),
                evm_chain_id,
                // `uint256` counts the token's base units, whose decimals
                // the code does not carry.
                amount: None,
            }),
            Some(_) => Err(SpectraBridgeError::invalid(
                "The scanned code asks for a contract call, not a payment.",
            )),
        }
    }

    /// The substrings of the payload that could be an address, most literal
    /// first.
    ///
    /// Deliberately a candidate list rather than one grammar per scheme: the
    /// payload comes from a camera and the schemes differ per chain, so
    /// refusing what does not parse as one grammar refuses more real codes
    /// than it prevents. Every candidate is validated against the chain
    /// before it is used, which is what makes a loose split safe.
    fn candidates(&self) -> Vec<String> {
        fn push(candidates: &mut Vec<String>, value: &str) {
            let value = value.trim();
            if !value.is_empty() && !candidates.iter().any(|c| c == value) {
                candidates.push(value.to_string());
            }
        }
        let mut candidates = Vec::new();
        if self.whole.is_empty() {
            return candidates;
        }
        push(&mut candidates, &self.whole);
        push(&mut candidates, &self.target);
        // `ton://transfer/EQ…` puts the address in a path segment.
        for segment in self.target.split('/') {
            push(&mut candidates, segment);
        }
        candidates
    }

    /// A whole-unit decimal amount: BIP-21's `amount=0.1`.
    fn whole_units(&self, names: &[&str]) -> Result<Option<String>, SpectraBridgeError> {
        let Some(text) = self.param(names) else {
            return Ok(None);
        };
        // At its own precision: the asset's is checked where it is typed.
        let places = text
            .split_once('.')
            .map_or(0, |(_, fraction)| fraction.len());
        let units = u32::try_from(places)
            .ok()
            .and_then(|places| crate::send::amount_input::parse_raw_amount(text, places).ok())
            .ok_or_else(unreadable_amount)?;
        Ok((units > 0).then(|| text.to_string()))
    }

    /// An amount in the network coin's base units — wei, nanotons — as a
    /// whole-unit decimal. EIP-681 writes large values as `2.014e18`.
    fn base_units(
        &self,
        names: &[&str],
        chain: Chain,
    ) -> Result<Option<String>, SpectraBridgeError> {
        let Some(text) = self.param(names) else {
            return Ok(None);
        };
        let units = integer_with_exponent(text).ok_or_else(unreadable_amount)?;
        Ok((units > 0).then(|| crate::decimal::from_units(units, chain.native_decimals().into())))
    }
}

fn unreadable_amount() -> SpectraBridgeError {
    SpectraBridgeError::invalid("The scanned code's amount is not a number Spectra can read.")
}

/// `1500`, `1.5e3` or `15e2` as the integer it writes; `None` for a
/// fraction, a sign or anything else.
fn integer_with_exponent(text: &str) -> Option<u128> {
    let (mantissa, exponent) = match text.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, exponent.parse::<u32>().ok()?),
        None => (text, 0),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let fraction = fraction.trim_end_matches('0');
    let shift = exponent.checked_sub(u32::try_from(fraction.len()).ok()?)?;
    let digits =
        crate::send::amount_input::parse_raw_amount(&format!("{whole}{fraction}"), 0).ok()?;
    digits.checked_mul(10u128.checked_pow(shift)?)
}

/// `%XX` escapes and `+` for a space, as a query writes them; `None` when the
/// escapes do not decode to UTF-8.
fn percent_decoded(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let hex = text.get(index + 1..index + 3)?;
                decoded.push(u8::from_str_radix(hex, 16).ok()?);
                index += 3;
            }
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded).ok()
}

/// The tag or memo the code asks for, on a network whose payments carry one:
/// XRP's `dt`, SEP-7's `memo` with its `memo_type`. A memo of a kind Spectra
/// cannot send is refused rather than dropped, since a deposit without it is
/// lost to the account that shares the address.
fn memo(chain: Chain, code: &PaymentCode) -> Result<Option<PaymentMemo>, SpectraBridgeError> {
    let memo = match chain.mainnet_counterpart() {
        Chain::Xrp => code.param(&["dt", "tag"]).map(|value| PaymentMemo {
            kind: PaymentMemoKind::DestinationTag,
            value: value.to_string(),
        }),
        Chain::Stellar => match code.param(&["memo"]) {
            None => None,
            Some(value) => {
                let kind = match code.param(&["memo_type"]) {
                    None | Some("MEMO_TEXT") => PaymentMemoKind::MemoText,
                    Some("MEMO_ID") => PaymentMemoKind::MemoId,
                    Some(other) => {
                        return Err(SpectraBridgeError::refused(
                            "The scanned code asks for a %@ memo, which Spectra cannot send.",
                            [other],
                        ));
                    }
                };
                Some(PaymentMemo {
                    kind,
                    value: value.to_string(),
                })
            }
        },
        _ => None,
    };
    memo.map(|memo| memo.validated(chain))
        .transpose()
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BTC: &str = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq";
    const EVM: &str = "0x9858EfFD232B4033E47d90003D41EC34EcaEda94";
    const TOKEN: &str = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
    const XRP: &str = "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh";

    fn read(chain: Chain, payload: &str) -> Result<ScannedPayment, SpectraBridgeError> {
        read_scanned_payment(chain, None, payload.to_string())
    }

    /// The shapes a wallet actually puts in a QR code.
    #[test]
    fn payment_uris_reduce_to_the_address_they_carry() {
        let cases = [
            (Chain::Bitcoin, BTC.to_string(), BTC),
            (Chain::Bitcoin, format!("bitcoin:{BTC}"), BTC),
            (
                Chain::Bitcoin,
                format!("bitcoin:{BTC}?amount=0.1&label=Shop"),
                BTC,
            ),
            (Chain::Bitcoin, format!("  {BTC}  "), BTC),
            (Chain::Ethereum, format!("ethereum:{EVM}"), EVM),
            (Chain::Ethereum, format!("ethereum:pay-{EVM}@1"), EVM),
            (Chain::Ethereum, format!("wc://x/{EVM}"), EVM),
        ];
        for (chain, payload, expected) in cases {
            assert_eq!(
                read(chain, &payload).map(|p| p.address).ok(),
                Some(normalize_address(chain, expected)),
                "{chain} did not read {payload}"
            );
        }
    }

    #[test]
    fn peercoin_payment_uris_validate_the_selected_network() {
        for (chain, other, address) in [
            (
                Chain::Peercoin,
                Chain::PeercoinTestnet,
                "PDFtxCFhnxaZk8JBYxazZvRdLr2GdVrLzm",
            ),
            (
                Chain::PeercoinTestnet,
                Chain::Peercoin,
                "mkBg6GwqZ4XdYQ72vTEqiwfgb6T6WRSDm5",
            ),
        ] {
            let payload = format!("peercoin:{address}?amount=1.123456&label=Peercoin");
            assert_eq!(read(chain, &payload).unwrap().address, address);
            assert!(read(other, &payload).is_err());
        }
    }

    /// The returned address is the stored form, not the scanned one.
    #[test]
    fn the_address_comes_back_in_the_form_the_store_keeps() {
        assert_eq!(
            read(Chain::Ethereum, &format!("ethereum:{EVM}"))
                .unwrap()
                .address,
            EVM.to_lowercase()
        );
        let bare_sui = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        assert_eq!(
            read(Chain::Sui, &format!("sui:{bare_sui}"))
                .unwrap()
                .address,
            format!("0x{bare_sui}")
        );
    }

    /// Nothing that is not an address on the named chain survives.
    #[test]
    fn a_payload_with_no_address_for_this_chain_is_refused() {
        for payload in [
            "",
            "   ",
            "not-an-address",
            "https://example.com/pay?to=someone",
            BTC,
        ] {
            assert!(
                read(Chain::Ethereum, payload).is_err(),
                "Ethereum accepted {payload}"
            );
        }
    }

    #[test]
    fn the_amount_comes_back_in_whole_units() {
        let bitcoin = read(
            Chain::Bitcoin,
            &format!("bitcoin:{BTC}?amount=0.1&label=Shop"),
        )
        .unwrap();
        assert_eq!(bitcoin.amount.as_deref(), Some("0.1"));
        // EIP-681 counts wei, and writes large values with an exponent.
        for value in ["1500000000000000000", "1.5e18", "15e17"] {
            let ether = read(Chain::Ethereum, &format!("ethereum:{EVM}@1?value={value}")).unwrap();
            assert_eq!(ether.amount.as_deref(), Some("1.5"), "value={value}");
        }
        assert!(read(Chain::Ethereum, &format!("ethereum:{EVM}?value=1.5")).is_err());
        assert!(read(Chain::Bitcoin, &format!("bitcoin:{BTC}?amount=-1")).is_err());
        // Zero asks for nothing.
        assert_eq!(
            read(Chain::Bitcoin, &format!("bitcoin:{BTC}?amount=0"))
                .unwrap()
                .amount,
            None
        );
    }

    /// A token transfer is addressed to the token's contract and pays its
    /// `address` parameter. Reading the contract as the recipient sent the
    /// funds to the token itself.
    #[test]
    fn an_eip681_token_transfer_pays_its_address_parameter() {
        let payload = format!("ethereum:{TOKEN}@1/transfer?address={EVM}&uint256=1000000");
        let paid =
            read_scanned_payment(Chain::Ethereum, Some(TOKEN.to_lowercase()), payload.clone())
                .unwrap();
        assert_eq!(paid.address, EVM.to_lowercase());
        assert_eq!(paid.amount, None);
        // The network's coin is not the token the code asks for.
        assert!(read(Chain::Ethereum, &payload).is_err());
        // A contract call that is not a transfer is not a payment.
        assert!(
            read(
                Chain::Ethereum,
                &format!("ethereum:{TOKEN}/approve?address={EVM}")
            )
            .is_err()
        );
    }

    #[test]
    fn a_code_pinned_to_another_evm_chain_is_refused() {
        assert!(read(Chain::Ethereum, &format!("ethereum:{EVM}@137")).is_err());
        assert!(read(Chain::Ethereum, &format!("ethereum:{EVM}@1")).is_ok());
    }

    /// A coin amount says nothing of a token: the address is kept and the
    /// amount left for the payer.
    #[test]
    fn a_coin_amount_is_not_a_token_amount() {
        let paid = read_scanned_payment(
            Chain::Ethereum,
            Some(TOKEN.into()),
            format!("ethereum:{EVM}?value=1e18"),
        )
        .unwrap();
        assert_eq!(paid.address, EVM.to_lowercase());
        assert_eq!(paid.amount, None);
    }

    #[test]
    fn a_destination_tag_or_memo_comes_with_the_address() {
        let xrp = read(Chain::Xrp, &format!("ripple:{XRP}?amount=25&dt=123456")).unwrap();
        assert_eq!(xrp.amount.as_deref(), Some("25"));
        assert_eq!(
            xrp.memo,
            Some(PaymentMemo {
                kind: PaymentMemoKind::DestinationTag,
                value: "123456".into()
            })
        );
        assert!(read(Chain::Xrp, &format!("ripple:{XRP}?dt=tag")).is_err());

        let stellar = "GDRXE2BQUC3AZNPVFSCEZ76NJ3WWL25FYFK6RGZGIEKWE4SOOHSUJUJ6";
        let text = read(
            Chain::Stellar,
            &format!("web+stellar:pay?destination={stellar}&amount=12.5&memo=order%20%2342"),
        )
        .unwrap();
        assert_eq!(text.address, stellar);
        assert_eq!(text.amount.as_deref(), Some("12.5"));
        assert_eq!(
            text.memo,
            Some(PaymentMemo {
                kind: PaymentMemoKind::MemoText,
                value: "order #42".into()
            })
        );
        let id = read(
            Chain::Stellar,
            &format!("web+stellar:pay?destination={stellar}&memo=9001&memo_type=MEMO_ID"),
        )
        .unwrap();
        assert_eq!(id.memo.map(|m| m.kind), Some(PaymentMemoKind::MemoId));
        assert!(
            read(
                Chain::Stellar,
                &format!("web+stellar:pay?destination={stellar}&memo=AAAA&memo_type=MEMO_HASH"),
            )
            .is_err()
        );
        // A network without memos ignores the parameter.
        assert_eq!(
            read(Chain::Bitcoin, &format!("bitcoin:{BTC}?memo=x"))
                .unwrap()
                .memo,
            None
        );
    }

    /// A field that takes an address takes a token transfer's recipient, not
    /// its contract, and leaves the amount alone; another network is still
    /// refused.
    #[test]
    fn an_address_field_reads_the_recipient_alone() {
        let transfer = format!("ethereum:{TOKEN}@1/transfer?address={EVM}&uint256=1000000");
        assert_eq!(
            read_scanned_address(Chain::Ethereum, transfer).unwrap(),
            EVM.to_lowercase()
        );
        assert_eq!(
            read_scanned_address(Chain::Bitcoin, format!("bitcoin:{BTC}?amount=1")).unwrap(),
            BTC
        );
        assert!(read_scanned_address(Chain::Ethereum, format!("ethereum:{EVM}@137")).is_err());
        assert!(read_scanned_address(Chain::Ethereum, BTC.into()).is_err());
    }

    /// Every request the receive page can show reads back through the
    /// scanner as the address, amount and memo it was made from.
    #[test]
    fn a_payment_request_reads_back_as_itself() {
        let stellar = "GDRXE2BQUC3AZNPVFSCEZ76NJ3WWL25FYFK6RGZGIEKWE4SOOHSUJUJ6";
        let cases: Vec<(Chain, &str, &str, Option<PaymentMemo>)> = vec![
            (Chain::Bitcoin, BTC, "0.015", None),
            (Chain::Ethereum, EVM, "1.5", None),
            (Chain::Base, EVM, "0.25", None),
            (
                Chain::Xrp,
                XRP,
                "25",
                Some(PaymentMemo {
                    kind: PaymentMemoKind::DestinationTag,
                    value: "123".into(),
                }),
            ),
            (
                Chain::Stellar,
                stellar,
                "12.5",
                Some(PaymentMemo {
                    kind: PaymentMemoKind::MemoText,
                    value: "order #42".into(),
                }),
            ),
            (
                Chain::Stellar,
                stellar,
                "1",
                Some(PaymentMemo {
                    kind: PaymentMemoKind::MemoId,
                    value: "9001".into(),
                }),
            ),
        ];
        for (chain, address, amount, memo) in cases {
            let uri = payment_request_uri(chain, address.into(), Some(amount.into()), memo.clone())
                .unwrap();
            let read = read_scanned_payment(chain, None, uri.clone()).unwrap();
            assert_eq!(read.address, normalize_address(chain, address), "{uri}");
            assert_eq!(read.amount.as_deref(), Some(amount), "{uri}");
            assert_eq!(read.memo, memo, "{uri}");
        }
        // Without an amount the code is the address in the network's scheme.
        assert_eq!(
            payment_request_uri(Chain::Bitcoin, BTC.into(), None, None).unwrap(),
            format!("bitcoin:{BTC}")
        );
    }

    #[test]
    fn a_payment_request_refuses_what_it_cannot_say() {
        // No format other wallets read.
        assert!(!payment_requests_supported(Chain::Tron));
        assert!(payment_request_uri(Chain::Tron, "TQ".into(), Some("1".into()), None).is_err());
        // Not this network's address, no amount above zero, past precision.
        assert!(payment_request_uri(Chain::Ethereum, BTC.into(), Some("1".into()), None).is_err());
        assert!(payment_request_uri(Chain::Bitcoin, BTC.into(), Some("0".into()), None).is_err());
        assert!(
            payment_request_uri(Chain::Bitcoin, BTC.into(), Some("0.000000001".into()), None)
                .is_err()
        );
        // A memo where payments carry none.
        let tag = PaymentMemo {
            kind: PaymentMemoKind::DestinationTag,
            value: "1".into(),
        };
        assert!(
            payment_request_uri(Chain::Bitcoin, BTC.into(), Some("1".into()), Some(tag)).is_err()
        );
    }

    #[test]
    fn integers_with_exponents() {
        assert_eq!(integer_with_exponent("1500"), Some(1500));
        assert_eq!(integer_with_exponent("1.5e3"), Some(1500));
        assert_eq!(integer_with_exponent("1.50e1"), Some(15));
        assert_eq!(integer_with_exponent("1.55e1"), None);
        assert_eq!(integer_with_exponent("-1"), None);
        assert_eq!(integer_with_exponent("1e40"), None);
    }
}
