// Pure-logic helpers backing the send flow: address validation and
// normalization, EVM chain context, send-preview flattening, risk evaluation.
//
// Every function here is a pure transform with no platform dependencies.

use crate::registry::Chain;
use crate::send::preview_types::*;
use crate::validation::address::{validate_address, AddressValidationRequest};

#[derive(Debug, Clone, uniffi::Record)]
pub struct EvmReceiptClassification {
    pub is_confirmed: bool,
    pub is_failed: bool,
    pub block_number: Option<i64>,
    pub cost: Option<crate::store::EvmReceiptCost>,
}

/// Address-format kind for a chain *display name*.
///
/// Thin wrapper over [`Chain::address_validation_kind`]; the mapping itself
/// lives in the registry.
pub(crate) fn chain_kind(chain_name: &str) -> Option<&'static str> {
    Chain::from_display_name(chain_name).map(Chain::address_validation_kind)
}

#[uniffi::export]
pub fn is_valid_send_address(chain_name: String, address: String) -> bool {
    let Some(kind) = chain_kind(&chain_name) else {
        return false;
    };
    // Normalized first, because the authoritative path already does.
    // `AddAddressBookEntry` normalizes and *then* validates, while every
    // caller of this validated the raw string — and on Sui the two disagree:
    // a 64-hex address typed without its `0x` prefix is invalid raw and valid
    // once `AddressNormalization::LowercaseHexPrefixed` has added the prefix.
    // So the composer refused an address the store would have accepted.
    //
    // Answering about the normalized form makes the two orders agree by
    // construction, and it is the form that gets stored and sent either way.
    validate_address(AddressValidationRequest {
        kind: kind.to_string(),
        value: normalize_address(&chain_name, &address),
    })
    .is_valid
}

pub(crate) fn normalize_address(chain_name: &str, address: &str) -> String {
    use crate::registry::AddressNormalization;
    let t = address.trim();
    let Some(chain) = crate::registry::Chain::from_display_name(chain_name) else {
        return t.to_string();
    };
    match chain.address_normalization() {
        AddressNormalization::None => t.to_string(),
        AddressNormalization::Lowercase => t.to_lowercase(),
        AddressNormalization::LowercaseHexPrefixed => {
            let l = t.to_lowercase();
            if l.starts_with("0x") {
                l
            } else {
                format!("0x{l}")
            }
        }
    }
}

#[uniffi::export]
pub fn normalized_send_address(chain_name: String, address: String) -> String {
    normalize_address(&chain_name, &address)
}

/// The address a scanned payment payload yields on `chain_name`, or `None`.
///
/// A QR code is rarely a bare address. Wallets encode BIP-21 and its
/// descendants — `bitcoin:bc1q…?amount=0.1`, `ethereum:0x…@1/transfer`,
/// `ton://transfer/EQ…` — so a scanner that hands the whole payload to the
/// composer fills the address field with a URI. The payload is reduced to the
/// substrings that could be an address and the first one this chain accepts is
/// returned, normalized the way the store and the signer normalize.
///
/// The chain is required and there is no fallback. This lived in the iOS
/// scanner, which returned the first candidate *unvalidated* when no asset was
/// selected — putting an unchecked string from a camera straight into the send
/// field. There is no address without a chain to judge it against.
#[uniffi::export]
pub fn scanned_send_address(chain_name: String, payload: String) -> Option<String> {
    let kind = chain_kind(&chain_name)?;
    scanned_address_candidates(&payload)
        .into_iter()
        .map(|candidate| normalize_address(&chain_name, &candidate))
        .find(|normalized| {
            validate_address(AddressValidationRequest {
                kind: kind.to_string(),
                value: normalized.clone(),
            })
            .is_valid
        })
}

/// The substrings of a scanned payload that could be an address, most literal
/// first.
///
/// Deliberately a candidate list rather than a URI grammar: the payload comes
/// from a camera, the schemes differ per chain, and refusing everything that
/// does not parse as one grammar refuses more real codes than it prevents.
/// Every candidate is validated against the chain before it is used, which is
/// what makes a loose split safe — a fragment that is not an address on this
/// chain cannot survive the filter in [`scanned_send_address`].
fn scanned_address_candidates(payload: &str) -> Vec<String> {
    fn push(candidates: &mut Vec<String>, value: &str) {
        let value = value.trim();
        if !value.is_empty() && !candidates.iter().any(|c| c == value) {
            candidates.push(value.to_string());
        }
    }

    let trimmed = payload.trim();
    let mut candidates = Vec::new();
    if trimmed.is_empty() {
        return candidates;
    }
    push(&mut candidates, trimmed);
    // `?amount=`/`#` carry the request, not the address.
    let without_query = trimmed.split(['?', '#']).next().unwrap_or(trimmed);
    push(&mut candidates, without_query);
    // `scheme:address`, and `scheme://host/path` for the chains that use one.
    if let Some((_, rest)) = without_query.split_once(':') {
        let rest = rest.trim_start_matches('/');
        push(&mut candidates, rest);
        // `ton://transfer/EQ…` puts the address in a path segment, and
        // `ethereum:0x…/transfer` puts a function name after it.
        for segment in rest.split('/') {
            push(&mut candidates, segment);
        }
    }
    // `ethereum:0x…@1` pins an EIP-155 chain id onto the address.
    for pinned in candidates
        .iter()
        .filter_map(|c| c.split_once('@').map(|(address, _)| address.to_string()))
        .collect::<Vec<_>>()
    {
        push(&mut candidates, &pinned);
    }
    candidates
}

/// Heuristic: does the trimmed input look like an ENS name (`foo.eth`, no
/// spaces, not an 0x-prefixed hex address)?
///
/// Not exported: whether a name is looked up at all is
/// `Chain::resolves_ens_names`, and `WalletService::resolve_send_destination`
/// asks both questions together. A caller that could only ask this one had to
/// supply the other half itself.
pub(crate) fn is_ens_name_candidate(value: &str) -> bool {
    let normalized = value.trim().to_lowercase();
    normalized.ends_with(".eth") && !normalized.contains(' ') && !normalized.starts_with("0x")
}

/// The send-preview for the chain currently being composed.
///
/// One variant per preview record shape. The caller picks the variant, so no
/// chain-name matching happens on this path and only the relevant preview
/// crosses the FFI.
#[derive(Debug, Clone, serde::Serialize, uniffi::Enum)]
pub enum SendPreview {
    /// Bitcoin, Bitcoin Cash, Bitcoin SV and Litecoin share one preview shape.
    Utxo {
        preview: BitcoinSendPreview,
    },
    Dogecoin {
        preview: DogecoinSendPreview,
    },
    Ethereum {
        preview: EvmSendPreview,
    },
    Tron {
        preview: TronSendPreview,
    },
    Solana {
        preview: SolanaSendPreview,
    },
    Xrp {
        preview: XrpSendPreview,
    },
    Stellar {
        preview: StellarSendPreview,
    },
    Monero {
        preview: MoneroSendPreview,
    },
    Cardano {
        preview: CardanoSendPreview,
    },
    Sui {
        preview: SuiSendPreview,
    },
    Aptos {
        preview: AptosSendPreview,
    },
    Ton {
        preview: TonSendPreview,
    },
    Icp {
        preview: IcpSendPreview,
    },
    Near {
        preview: NearSendPreview,
    },
    Polkadot {
        preview: PolkadotSendPreview,
    },
    /// Substrate like Polkadot, and the same record — but its own tag, because
    /// the front end keys its preview slots on this and a Bittensor preview
    /// filed under Polkadot would be shown for the wrong chain.
    Bittensor {
        preview: PolkadotSendPreview,
    },
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, serde::Serialize, uniffi::Record)]
pub struct SendPreviewDetailsCore {
    pub spendableBalance: Option<f64>,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: Option<f64>,
}

pub(crate) fn compute_send_preview_details(
    preview: Option<SendPreview>,
    coin_amount: f64,
) -> Option<SendPreviewDetailsCore> {
    let preview = preview?;

    // Which fields each preview shape contributes. The seventh value is an
    // estimated network fee, present only for UTXO chains; it backs the
    // `coin_amount - fee` fallback applied below when a preview reports no
    // spendable balance or max-sendable of its own.
    //
    // Several shapes carry fields this deliberately drops (Tron and friends
    // populate `estimatedTransactionBytes`, but the send sheet does not show
    // byte counts for account-model chains). That selection is preserved
    // exactly as it was.
    let (spendable, fee_rate, tx_bytes, input_count, uses_change, max_sendable, est_fee) =
        match preview {
            SendPreview::Utxo { preview: p } => (
                p.spendableBalance,
                p.feeRateDescription,
                p.estimatedTransactionBytes,
                p.selectedInputCount,
                p.usesChangeOutput,
                p.maxSendable,
                Some(p.estimatedNetworkFee),
            ),
            SendPreview::Dogecoin { preview: p } => (
                Some(p.spendableBalance),
                p.feeRateDescription,
                Some(p.estimatedTransactionBytes),
                Some(p.selectedInputCount),
                Some(p.usesChangeOutput),
                Some(p.maxSendable),
                None,
            ),
            SendPreview::Ethereum { preview: p } => (
                p.spendableBalance,
                p.feeRateDescription,
                None,
                None,
                None,
                p.maxSendable,
                None,
            ),
            SendPreview::Polkadot { preview: p } => (
                Some(p.spendableBalance),
                p.feeRateDescription,
                p.estimatedTransactionBytes,
                None,
                None,
                Some(p.maxSendable),
                None,
            ),
            SendPreview::Bittensor { preview: p } => (
                Some(p.spendableBalance),
                p.feeRateDescription,
                p.estimatedTransactionBytes,
                None,
                None,
                Some(p.maxSendable),
                None,
            ),
            // Account-model chains: balance, fee description and max sendable.
            SendPreview::Tron { preview: p } => {
                simple(p.spendableBalance, p.feeRateDescription, p.maxSendable)
            }
            SendPreview::Solana { preview: p } => {
                simple(p.spendableBalance, p.feeRateDescription, p.maxSendable)
            }
            SendPreview::Xrp { preview: p } => {
                simple(p.spendableBalance, p.feeRateDescription, p.maxSendable)
            }
            SendPreview::Stellar { preview: p } => {
                simple(p.spendableBalance, p.feeRateDescription, p.maxSendable)
            }
            SendPreview::Monero { preview: p } => {
                simple(p.spendableBalance, p.feeRateDescription, p.maxSendable)
            }
            SendPreview::Cardano { preview: p } => {
                simple(p.spendableBalance, p.feeRateDescription, p.maxSendable)
            }
            SendPreview::Sui { preview: p } => {
                simple(p.spendableBalance, p.feeRateDescription, p.maxSendable)
            }
            SendPreview::Aptos { preview: p } => {
                simple(p.spendableBalance, p.feeRateDescription, p.maxSendable)
            }
            SendPreview::Ton { preview: p } => {
                simple(p.spendableBalance, p.feeRateDescription, p.maxSendable)
            }
            SendPreview::Icp { preview: p } => {
                simple(p.spendableBalance, p.feeRateDescription, p.maxSendable)
            }
            SendPreview::Near { preview: p } => {
                simple(p.spendableBalance, p.feeRateDescription, p.maxSendable)
            }
        };

    let fallback = est_fee.map(|fee| (coin_amount - fee).max(0.0));
    Some(SendPreviewDetailsCore {
        spendableBalance: spendable.or(fallback),
        feeRateDescription: fee_rate,
        estimatedTransactionBytes: tx_bytes,
        selectedInputCount: input_count,
        usesChangeOutput: uses_change,
        maxSendable: max_sendable.or(fallback),
    })
}

/// Field selection shared by the account-model chains.
#[allow(clippy::type_complexity)]
fn simple(
    spendable: f64,
    fee_rate: Option<String>,
    max_sendable: f64,
) -> (
    Option<f64>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<bool>,
    Option<f64>,
    Option<f64>,
) {
    (
        Some(spendable),
        fee_rate,
        None,
        None,
        None,
        Some(max_sendable),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utxo_preview() -> BitcoinSendPreview {
        BitcoinSendPreview {
            estimatedNetworkFee: 0.5,
            feeRateDescription: Some("12 sat/vB".to_string()),
            spendableBalance: None,
            estimatedTransactionBytes: Some(226),
            selectedInputCount: Some(2),
            usesChangeOutput: Some(true),
            maxSendable: None,
            ..Default::default()
        }
    }

    /// UTXO chains report a network fee, and a preview that gives no spendable
    /// balance or max-sendable falls back to `amount - fee`.
    #[test]
    fn utxo_preview_falls_back_to_amount_minus_fee() {
        let d = compute_send_preview_details(
            Some(SendPreview::Utxo {
                preview: utxo_preview(),
            }),
            2.0,
        )
        .expect("details");
        assert_eq!(d.spendableBalance, Some(1.5));
        assert_eq!(d.maxSendable, Some(1.5));
        assert_eq!(d.estimatedTransactionBytes, Some(226));
        assert_eq!(d.selectedInputCount, Some(2));
        assert_eq!(d.usesChangeOutput, Some(true));
        assert_eq!(d.feeRateDescription.as_deref(), Some("12 sat/vB"));
    }

    /// The fallback never goes negative.
    #[test]
    fn utxo_fallback_clamps_at_zero() {
        let d = compute_send_preview_details(
            Some(SendPreview::Utxo {
                preview: utxo_preview(),
            }),
            0.1,
        )
        .expect("details");
        assert_eq!(d.spendableBalance, Some(0.0));
    }

    /// A preview's own values win over the fallback.
    #[test]
    fn utxo_preview_values_take_precedence_over_the_fallback() {
        let mut preview = utxo_preview();
        preview.spendableBalance = Some(9.0);
        preview.maxSendable = Some(8.0);
        let d = compute_send_preview_details(Some(SendPreview::Utxo { preview }), 2.0)
            .expect("details");
        assert_eq!(d.spendableBalance, Some(9.0));
        assert_eq!(d.maxSendable, Some(8.0));
    }

    /// Account-model chains contribute balance, fee text and max sendable, and
    /// deliberately drop the byte/input/change fields their record also carries.
    #[test]
    fn account_model_previews_drop_utxo_only_fields() {
        let d = compute_send_preview_details(
            Some(SendPreview::Tron {
                preview: TronSendPreview {
                    spendableBalance: 100.0,
                    feeRateDescription: Some("1 TRX".to_string()),
                    estimatedTransactionBytes: Some(300),
                    selectedInputCount: Some(1),
                    usesChangeOutput: Some(true),
                    maxSendable: 99.0,
                    ..Default::default()
                },
            }),
            100.0,
        )
        .expect("details");
        assert_eq!(d.spendableBalance, Some(100.0));
        assert_eq!(d.maxSendable, Some(99.0));
        assert_eq!(d.estimatedTransactionBytes, None);
        assert_eq!(d.selectedInputCount, None);
        assert_eq!(d.usesChangeOutput, None);
    }

    /// Polkadot is the one account-model chain that does surface byte size.
    #[test]
    fn polkadot_preview_keeps_transaction_bytes() {
        let d = compute_send_preview_details(
            Some(SendPreview::Polkadot {
                preview: PolkadotSendPreview {
                    spendableBalance: 10.0,
                    feeRateDescription: None,
                    estimatedTransactionBytes: Some(144),
                    maxSendable: 9.0,
                    ..Default::default()
                },
            }),
            10.0,
        )
        .expect("details");
        assert_eq!(d.estimatedTransactionBytes, Some(144));
        assert_eq!(d.selectedInputCount, None);
    }

    #[test]
    fn absent_preview_yields_no_details() {
        assert!(compute_send_preview_details(None, 1.0).is_none());
    }

    #[test]
    fn ens_candidate_positive() {
        assert!(is_ens_name_candidate("vitalik.eth"));
        assert!(is_ens_name_candidate("  Foo.ETH  "));
    }

    #[test]
    fn ens_candidate_negative() {
        assert!(!is_ens_name_candidate("0xabc.eth"));
        assert!(!is_ens_name_candidate("foo .eth"));
        assert!(!is_ens_name_candidate("foo.com"));
    }
}

// ── FFI: high-risk send evaluation ──────────────────────────────────────────

/// A chain_name + address pair used in the high-risk send evaluation.
#[derive(Debug, Clone)]
pub struct HighRiskChainAddress {
    pub chain_name: String,
    pub address: String,
}

/// Typed input for high-risk send evaluation.
#[derive(Debug, Clone)]
pub struct HighRiskSendRequest {
    pub chain_name: String,
    pub symbol: String,
    pub amount: f64,
    pub holding_amount: f64,
    pub destination_address: String,
    pub destination_input: String,
    pub used_ens_resolution: bool,
    pub wallet_family_name: String,
    pub address_book_entries: Vec<HighRiskChainAddress>,
    pub tx_addresses: Vec<HighRiskChainAddress>,
}

/// A reason a send looks risky. Front ends word each one; which ones exist,
/// and what each carries, are core's.
///
/// Was a record with a free-string `code` and five optional fields any code
/// might or might not fill. The app switched on the string with a `default`
/// that returned nothing, so a warning core added later would have vanished
/// from the confirmation sheet without a compiler error or a test failing —
/// on the screen whose job is to show every reason to stop. An enum makes a
/// new reason a compile error on every front end that has not worded it.
///
/// The serialized form keeps `code` beside each variant's fields, which is
/// what `spectra send quote` prints.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum HighRiskSendWarning {
    /// The destination does not parse as an address on `chain`.
    InvalidFormat { chain: String },
    /// Neither the address book nor this wallet's history has sent here.
    NewAddress,
    /// An ENS `name` was resolved to `address`.
    EnsResolved { name: String, address: String },
    /// The send is `percent` of the `symbol` holding, at 25% or more.
    LargeSend { percent: u64, symbol: String },
    /// An EVM `chain`, and a destination shaped like another family's address.
    NonEvmOnEvm { chain: String },
    /// An ENS name on an EVM `chain` whose names do not resolve through ENS.
    EnsOffEthereum { chain: String },
    /// A chain that is not EVM, and a destination shaped like an EVM address.
    EthOnUtxo { chain: String },
    /// A destination shaped like another chain's address on `chain`.
    ///
    /// `non_tron`, `non_solana`, `non_xrp` and `non_monero` before: four codes
    /// for one reason, each worded with its chain's name baked in.
    ForeignAddressFormat { chain: String },
    /// The holding's chain is not the wallet's.
    ChainMismatch,
}

impl HighRiskSendWarning {
    /// The serialized `code`, for tests and logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidFormat { .. } => "invalid_format",
            Self::NewAddress => "new_address",
            Self::EnsResolved { .. } => "ens_resolved",
            Self::LargeSend { .. } => "large_send",
            Self::NonEvmOnEvm { .. } => "non_evm_on_evm",
            Self::EnsOffEthereum { .. } => "ens_off_ethereum",
            Self::EthOnUtxo { .. } => "eth_on_utxo",
            Self::ForeignAddressFormat { .. } => "foreign_address_format",
            Self::ChainMismatch => "chain_mismatch",
        }
    }
}

/// Typed high-risk send evaluation.
///
/// Not exported: `WalletService::high_risk_send_reasons` is the entry point,
/// because the address book and the send history this reads are core's.
pub fn evaluate_high_risk_send_reasons(request: HighRiskSendRequest) -> Vec<HighRiskSendWarning> {
    let chain_name = &request.chain_name;
    let mut warnings: Vec<HighRiskSendWarning> = Vec::new();

    // 1. Address format validation.
    //
    // Asked of the normalized address, because that is the form the store
    // keeps and the send transmits, and `is_valid_send_address` — the other
    // caller of this same question — already asks it that way. This asked the
    // raw string: on Sui a 64-hex address typed without its `0x` is valid once
    // `AddressNormalization::LowercaseHexPrefixed` has added the prefix, so
    // the composer accepted it, the store accepted it, and this stood beside
    // them calling it `invalid_format`. One question, one form, one answer.
    if !is_valid_send_address(chain_name.clone(), request.destination_address.clone()) {
        warnings.push(HighRiskSendWarning::InvalidFormat {
            chain: chain_name.clone(),
        });
    }

    // The chain's own normalization is the comparison form, and nothing more
    // is applied on top of it. A blanket `to_lowercase()` stood here: correct
    // for the chains whose rule is already `Lowercase`, and wrong for every
    // chain whose rule is `None` — "case and shape are significant", as
    // `AddressNormalization` puts it. On Bitcoin and Solana two base58 strings
    // differing only in case are two different addresses, so folding them
    // together let a lookalike of an address in the book pass as one already
    // seen, and the `new_address` warning — the one that catches a swapped
    // destination — did not fire.
    let norm_dest = normalize_address(chain_name, &request.destination_address);

    // 2. New address detection.
    let has_address_book = request.address_book_entries.iter().any(|e| {
        e.chain_name == *chain_name && normalize_address(chain_name, &e.address) == norm_dest
    });
    let has_tx_history = request.tx_addresses.iter().any(|e| {
        e.chain_name == *chain_name && normalize_address(chain_name, &e.address) == norm_dest
    });
    if !has_address_book && !has_tx_history {
        warnings.push(HighRiskSendWarning::NewAddress);
    }

    // 3. ENS resolution warning.
    if request.used_ens_resolution {
        warnings.push(HighRiskSendWarning::EnsResolved {
            name: request.destination_input.clone(),
            address: request.destination_address.clone(),
        });
    }

    // 4. Large send percentage (≥25 % of holding balance).
    if request.holding_amount > 0.0 {
        let ratio = request.amount / request.holding_amount;
        if ratio >= 0.25 {
            let pct = (ratio * 100.0).round() as u64;
            warnings.push(HighRiskSendWarning::LargeSend {
                percent: pct,
                symbol: request.symbol.clone(),
            });
        }
    }

    // 5-10. Cross-chain prefix mismatch checks.
    let lowered = request.destination_input.to_lowercase();
    let chain = crate::registry::Chain::from_display_name(chain_name);
    // Membership is the registry's. Seven of the twenty-three EVM mainnets were
    // named here, and this gate decides whether the EVM destination checks run
    // at all — a name list here silently means "no warning" for whichever
    // chains it forgets.
    let is_evm = chain.is_some_and(|c| c.is_evm());
    // ENS resolves on Ethereum; anywhere else the resolved address is worth a
    // second look.
    let is_ens_foreign_chain = is_evm
        && chain.is_some_and(|c| c.mainnet_counterpart() != crate::registry::Chain::Ethereum);
    let is_ens_candidate = is_ens_name_candidate(&lowered);

    if is_evm {
        let looks_non_evm = lowered.starts_with("bc1")
            || lowered.starts_with("tb1")
            || lowered.starts_with("ltc1")
            || lowered.starts_with("bnb1")
            || lowered.starts_with('t')
            || lowered.starts_with('d')
            || lowered.starts_with('a');
        if looks_non_evm {
            warnings.push(HighRiskSendWarning::NonEvmOnEvm {
                chain: chain_name.clone(),
            });
        }
        if is_ens_foreign_chain && is_ens_candidate {
            warnings.push(HighRiskSendWarning::EnsOffEthereum {
                chain: chain_name.clone(),
            });
        }
    } else if crate::registry::Chain::from_display_name(chain_name)
        .is_some_and(|c| c.flags_evm_address_as_wrong_chain())
    {
        if lowered.starts_with("0x") || is_ens_candidate {
            warnings.push(HighRiskSendWarning::EthOnUtxo {
                chain: chain_name.clone(),
            });
        }
    } else {
        let foreign = match chain_name.as_str() {
            "Tron" => lowered.starts_with("0x") || lowered.starts_with("bc1"),
            "Solana" => {
                lowered.starts_with("0x")
                    || lowered.starts_with("bc1")
                    || lowered.starts_with("ltc1")
                    || lowered.starts_with('t')
            }
            "XRP Ledger" => {
                lowered.starts_with("0x") || lowered.starts_with("bc1") || lowered.starts_with('t')
            }
            "Monero" => {
                lowered.starts_with("0x") || lowered.starts_with("bc1") || lowered.starts_with('r')
            }
            _ => false,
        };
        if foreign {
            warnings.push(HighRiskSendWarning::ForeignAddressFormat {
                chain: chain_name.clone(),
            });
        }
    }

    // 11. Wallet-chain context mismatch.
    if !request.wallet_family_name.is_empty() && request.wallet_family_name != *chain_name {
        warnings.push(HighRiskSendWarning::ChainMismatch);
    }

    warnings
}

// ── Chain predicates ──────────────────────────────────────────────

use crate::SpectraBridgeError;

// Per-chain static config for the Litecoin/Dogecoin/Solana/XRP/Monero/Sui/Aptos
// branch of Swift's destination-risk probe: display chain name and balance
// label for messages.

// Maps Swift's BroadcastEntry payload format → (chain_id, result_field, wrap_key,
// extract_field). Returns an error for unknown formats.

#[derive(Debug, Clone, uniffi::Record)]
pub struct RebroadcastDispatch {
    pub chain_id: String,
    pub result_field: String,
    pub wrap_key: Option<String>,
    pub extract_field: Option<String>,
}

pub fn rebroadcast_dispatch_for_format(
    format: String,
) -> Result<RebroadcastDispatch, SpectraBridgeError> {
    // Keep chain IDs aligned with SpectraChainID in Swift.
    // 0 bitcoin, 1 bitcoin_cash, 2 bitcoin_sv, 3 litecoin, 4 dogecoin,
    // 5 ethereum, 6 tron, 7 solana, 8 xrp, 9 stellar, 10 monero,
    // 11 cardano, 12 sui, 13 aptos, 14 ton, 15 icp, 16 near, 17 polkadot
    let entry: Option<RebroadcastDispatch> = match format.as_str() {
        "bitcoin.raw_hex" => Some(RebroadcastDispatch {
            chain_id: "bitcoin".into(),
            result_field: "txid".into(),
            wrap_key: None,
            extract_field: None,
        }),
        "bitcoin_cash.raw_hex" => Some(RebroadcastDispatch {
            chain_id: "bitcoin-cash".into(),
            result_field: "txid".into(),
            wrap_key: None,
            extract_field: None,
        }),
        "bitcoin_sv.raw_hex" => Some(RebroadcastDispatch {
            chain_id: "bitcoin-sv".into(),
            result_field: "txid".into(),
            wrap_key: None,
            extract_field: None,
        }),
        "litecoin.raw_hex" => Some(RebroadcastDispatch {
            chain_id: "litecoin".into(),
            result_field: "txid".into(),
            wrap_key: None,
            extract_field: None,
        }),
        "dogecoin.raw_hex" => Some(RebroadcastDispatch {
            chain_id: "dogecoin".into(),
            result_field: "txid".into(),
            wrap_key: None,
            extract_field: None,
        }),
        "tron.signed_json" => Some(RebroadcastDispatch {
            chain_id: "tron".into(),
            result_field: "txid".into(),
            wrap_key: None,
            extract_field: None,
        }),
        "solana.base64" => Some(RebroadcastDispatch {
            chain_id: "solana".into(),
            result_field: "signature".into(),
            wrap_key: None,
            extract_field: None,
        }),
        "xrp.blob_hex" => Some(RebroadcastDispatch {
            chain_id: "xrp".into(),
            result_field: "txid".into(),
            wrap_key: Some("tx_blob_hex".into()),
            extract_field: None,
        }),
        "stellar.xdr" => Some(RebroadcastDispatch {
            chain_id: "stellar".into(),
            result_field: "txid".into(),
            wrap_key: Some("signed_xdr_b64".into()),
            extract_field: None,
        }),
        "cardano.cbor_hex" => Some(RebroadcastDispatch {
            chain_id: "cardano".into(),
            result_field: "txid".into(),
            wrap_key: Some("cbor_hex".into()),
            extract_field: None,
        }),
        "near.base64" => Some(RebroadcastDispatch {
            chain_id: "near".into(),
            result_field: "txid".into(),
            wrap_key: Some("signed_tx_b64".into()),
            extract_field: None,
        }),
        "polkadot.extrinsic_hex" => Some(RebroadcastDispatch {
            chain_id: "polkadot".into(),
            result_field: "txid".into(),
            wrap_key: Some("extrinsic_hex".into()),
            extract_field: None,
        }),
        "aptos.signed_json" => Some(RebroadcastDispatch {
            chain_id: "aptos".into(),
            result_field: "txid".into(),
            wrap_key: Some("signed_body_json".into()),
            extract_field: None,
        }),
        "ton.boc" => Some(RebroadcastDispatch {
            chain_id: "ton".into(),
            result_field: "message_hash".into(),
            wrap_key: Some("boc_b64".into()),
            extract_field: None,
        }),
        "bitcoin.rust_json" => Some(RebroadcastDispatch {
            chain_id: "bitcoin".into(),
            result_field: "txid".into(),
            wrap_key: None,
            extract_field: Some("raw_tx_hex".into()),
        }),
        "bitcoin_cash.rust_json" => Some(RebroadcastDispatch {
            chain_id: "bitcoin-cash".into(),
            result_field: "txid".into(),
            wrap_key: None,
            extract_field: Some("raw_tx_hex".into()),
        }),
        "bitcoin_sv.rust_json" => Some(RebroadcastDispatch {
            chain_id: "bitcoin-sv".into(),
            result_field: "txid".into(),
            wrap_key: None,
            extract_field: Some("raw_tx_hex".into()),
        }),
        "litecoin.rust_json" => Some(RebroadcastDispatch {
            chain_id: "litecoin".into(),
            result_field: "txid".into(),
            wrap_key: None,
            extract_field: Some("raw_tx_hex".into()),
        }),
        "dogecoin.rust_json" => Some(RebroadcastDispatch {
            chain_id: "dogecoin".into(),
            result_field: "txid".into(),
            wrap_key: None,
            extract_field: Some("raw_tx_hex".into()),
        }),
        "solana.rust_json" => Some(RebroadcastDispatch {
            chain_id: "solana".into(),
            result_field: "signature".into(),
            wrap_key: None,
            extract_field: Some("signed_tx_base64".into()),
        }),
        "tron.rust_json" => Some(RebroadcastDispatch {
            chain_id: "tron".into(),
            result_field: "txid".into(),
            wrap_key: None,
            extract_field: Some("signed_tx_json".into()),
        }),
        "xrp.rust_json" => Some(RebroadcastDispatch {
            chain_id: "xrp".into(),
            result_field: "txid".into(),
            wrap_key: None,
            extract_field: None,
        }),
        "stellar.rust_json" => Some(RebroadcastDispatch {
            chain_id: "stellar".into(),
            result_field: "txid".into(),
            wrap_key: None,
            extract_field: None,
        }),
        "cardano.rust_json" => Some(RebroadcastDispatch {
            chain_id: "cardano".into(),
            result_field: "txid".into(),
            wrap_key: None,
            extract_field: None,
        }),
        "polkadot.rust_json" => Some(RebroadcastDispatch {
            chain_id: "polkadot".into(),
            result_field: "txid".into(),
            wrap_key: None,
            extract_field: None,
        }),
        "sui.rust_json" => Some(RebroadcastDispatch {
            chain_id: "sui".into(),
            result_field: "digest".into(),
            wrap_key: None,
            extract_field: None,
        }),
        "aptos.rust_json" => Some(RebroadcastDispatch {
            chain_id: "aptos".into(),
            result_field: "txid".into(),
            wrap_key: None,
            extract_field: None,
        }),
        "ton.rust_json" => Some(RebroadcastDispatch {
            chain_id: "ton".into(),
            result_field: "message_hash".into(),
            wrap_key: None,
            extract_field: None,
        }),
        "near.rust_json" => Some(RebroadcastDispatch {
            chain_id: "near".into(),
            result_field: "txid".into(),
            wrap_key: None,
            extract_field: None,
        }),
        _ => None,
    };
    entry.ok_or_else(|| {
        SpectraBridgeError::from("Rebroadcast is not supported for this transaction format yet.")
    })
}

// ─── Rebroadcast prepared payload ────────────────────────────────────────────
// Fuses the dispatch-table lookup with the payload shape transformation so Swift
// never has to build JSON objects or scrape fields for rebroadcast. Handles:
//   • sui.signed_json — remap {txBytesBase64, signatureBase64} → {tx_bytes_b64, sig_b64}
//   • extract_field branch — pull named field value out of a wallet-produced JSON
//   • wrap_key branch — wrap raw payload string under a single JSON key
//   • otherwise — pass payload through unchanged

#[derive(Debug, Clone, uniffi::Record)]
pub struct PreparedBroadcastPayload {
    pub chain_id: String,
    pub broadcast_payload: String,
    pub result_field: String,
}

pub fn rebroadcast_prepare_payload(
    format: String,
    raw_payload: String,
) -> Result<PreparedBroadcastPayload, SpectraBridgeError> {
    if format == "sui.signed_json" {
        let remapped = sui_signed_json_remap(&raw_payload).unwrap_or_else(|| raw_payload.clone());
        return Ok(PreparedBroadcastPayload {
            chain_id: "sui".into(),
            broadcast_payload: remapped,
            result_field: "digest".to_string(),
        });
    }
    let dispatch = rebroadcast_dispatch_for_format(format)?;
    let broadcast_payload = if let Some(extract_field) = dispatch.extract_field.as_ref() {
        crate::send::preview_decode::extract_json_string_field(
            raw_payload.clone(),
            extract_field.clone(),
        )
    } else if let Some(wrap_key) = dispatch.wrap_key.as_ref() {
        let mut map = serde_json::Map::new();
        map.insert(
            wrap_key.clone(),
            serde_json::Value::String(raw_payload.clone()),
        );
        serde_json::to_string(&serde_json::Value::Object(map)).unwrap_or(raw_payload)
    } else {
        raw_payload
    };
    Ok(PreparedBroadcastPayload {
        chain_id: dispatch.chain_id,
        broadcast_payload,
        result_field: dispatch.result_field,
    })
}

fn sui_signed_json_remap(raw: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let obj = v.as_object()?;
    let tx = obj.get("txBytesBase64")?.as_str()?;
    let sig = obj.get("signatureBase64")?.as_str()?;
    let remapped = serde_json::json!({ "tx_bytes_b64": tx, "sig_b64": sig });
    serde_json::to_string(&remapped).ok()
}

/// Returns the canonical "raw" derivation-chain name for a given chain row.
/// Testnets share their mainnet counterpart's derivation engine, so e.g.
/// `"Ethereum Sepolia"` returns `"Ethereum"`. The Chain enum is the source
/// of truth for that mapping.
/// Not exported: it is a column of `chain_identities` now.
pub fn seed_derivation_chain_raw(chain: crate::registry::Chain) -> Option<String> {
    if chain.is_testnet() {
        return Some(chain.chain_display_name().to_string());
    }
    let mainnet = chain.mainnet_counterpart();
    // Some EVM L1/L2/sidechains (BNB Chain, Optimism, etc.) reuse Ethereum's
    // derivation path; the historical raw-name table preserved that
    // collapsing. Mirror it here.
    let raw = match mainnet {
        crate::registry::Chain::BnbChain => "Ethereum",
        c => c.chain_display_name(),
    };
    Some(raw.to_string())
}

// Lifted from Swift `evmHasContractCode`: a nonempty `eth_getCode` result
// (anything other than "0x" or "0x0") indicates deployed bytecode.

pub fn evm_has_contract_code(code: String) -> bool {
    let trimmed = code.trim();
    !trimmed.is_empty()
        && !trimmed.eq_ignore_ascii_case("0x")
        && !trimmed.eq_ignore_ascii_case("0x0")
}

// When preparing a speed-up / cancel replacement, Swift bumps existing custom
// fees by 20% with a 0.1 gwei floor (or falls back to defaults if either input
// is missing / blank). Returns formatted strings (3 decimals) the way Swift
// renders them into the composer fields.

#[derive(Debug, Clone, uniffi::Record)]
pub struct EvmReplacementFeeBump {
    pub max_fee_gwei: String,
    pub priority_fee_gwei: String,
}

pub fn evm_replacement_fee_bump(
    existing_max_fee_gwei: Option<String>,
    existing_priority_fee_gwei: Option<String>,
    default_max_fee_gwei: f64,
    default_priority_fee_gwei: f64,
) -> EvmReplacementFeeBump {
    let parse = |s: Option<&str>| -> Option<f64> {
        s.and_then(|v| {
            let trimmed = v.trim();
            if trimmed.is_empty() {
                None
            } else {
                trimmed.parse::<f64>().ok()
            }
        })
    };
    let have_max = parse(existing_max_fee_gwei.as_deref());
    let have_pri = parse(existing_priority_fee_gwei.as_deref());
    if have_max.is_none() || have_pri.is_none() {
        return EvmReplacementFeeBump {
            max_fee_gwei: format!("{:.1}", default_max_fee_gwei),
            priority_fee_gwei: format!("{:.1}", default_priority_fee_gwei),
        };
    }
    let bumped_max = (have_max.unwrap() * 1.2).max(0.1);
    let bumped_pri = (have_pri.unwrap() * 1.2).max(0.1);
    EvmReplacementFeeBump {
        max_fee_gwei: format!("{:.3}", bumped_max),
        priority_fee_gwei: format!("{:.3}", bumped_pri),
    }
}

#[cfg(test)]
mod flow_helpers_tests {
    use super::*;

    /// `chain_kind` used to carry its own display-name table, and that table
    /// omitted 22 mainnet chains — Base, Polygon, Zcash, Kaspa, Dash and the
    /// newer EVM rollups among them. `chain_kind` returned `None` for each, so
    /// `is_valid_send_address` rejected *every* address on those chains and the
    /// send flow could not be completed at all. It now reads the registry.
    #[test]
    fn send_validation_covers_the_chains_the_old_table_omitted() {
        let evm = "0x9858EfFD232B4033E47d90003D41EC34EcaEda94";
        let previously_broken_evm = [
            "Base",
            "Polygon",
            "Linea",
            "Scroll",
            "Blast",
            "Mantle",
            "Sei",
            "Celo",
            "Cronos",
            "opBNB",
            "zkSync Era",
            "Sonic",
            "Berachain",
            "Unichain",
            "Ink",
            "X Layer",
        ];
        for chain in previously_broken_evm {
            assert_eq!(
                chain_kind(chain),
                Some("evm"),
                "{chain} must resolve to the EVM validator"
            );
            assert!(
                is_valid_send_address(chain.to_string(), evm.to_string()),
                "{chain} must accept a valid EVM address"
            );
        }

        // Non-EVM chains the old table also missed.
        for (chain, kind) in [
            ("Zcash", "zcash"),
            ("Bitcoin Gold", "bitcoinGold"),
            ("Decred", "decred"),
            ("Kaspa", "kaspa"),
            ("Dash", "dash"),
            ("Bittensor", "bittensor"),
        ] {
            assert_eq!(chain_kind(chain), Some(kind), "{chain} kind");
        }

        // Still rejects what it should.
        assert_eq!(chain_kind("Not A Chain"), None);
        assert!(!is_valid_send_address(
            "Polygon".to_string(),
            "not-an-address".to_string()
        ));
    }

    #[test]
    fn rebroadcast_dispatch_btc() {
        let d = rebroadcast_dispatch_for_format("bitcoin.raw_hex".to_string()).unwrap();
        assert_eq!(d.chain_id, "bitcoin");
        assert_eq!(d.result_field, "txid");
    }

    #[test]
    fn rebroadcast_dispatch_unknown_errors() {
        assert!(rebroadcast_dispatch_for_format("nope".to_string()).is_err());
    }

    #[test]
    fn evm_has_contract_code_variants() {
        assert!(!evm_has_contract_code("0x".to_string()));
        assert!(!evm_has_contract_code("0X0".to_string()));
        assert!(!evm_has_contract_code("   0x ".to_string()));
        assert!(!evm_has_contract_code(String::new()));
        assert!(evm_has_contract_code("0x60806040".to_string()));
    }

    #[test]
    fn evm_bump_defaults_when_blank() {
        let r = evm_replacement_fee_bump(None, Some(" ".to_string()), 4.0, 2.0);
        assert_eq!(r.max_fee_gwei, "4.0");
        assert_eq!(r.priority_fee_gwei, "2.0");
    }

    #[test]
    fn evm_bump_scales_existing() {
        let r =
            evm_replacement_fee_bump(Some("5.0".to_string()), Some("2.5".to_string()), 4.0, 2.0);
        assert_eq!(r.max_fee_gwei, "6.000");
        assert_eq!(r.priority_fee_gwei, "3.000");
    }

    #[test]
    fn prepare_payload_sui_signed_json_remap() {
        let raw = r#"{"txBytesBase64":"AAAA","signatureBase64":"BBBB"}"#;
        let p = rebroadcast_prepare_payload("sui.signed_json".into(), raw.into()).unwrap();
        assert_eq!(p.chain_id, "sui");
        assert_eq!(p.result_field, "digest");
        let parsed: serde_json::Value = serde_json::from_str(&p.broadcast_payload).unwrap();
        assert_eq!(parsed["tx_bytes_b64"], "AAAA");
        assert_eq!(parsed["sig_b64"], "BBBB");
    }

    #[test]
    fn prepare_payload_sui_malformed_passthrough() {
        let raw = "not json";
        let p = rebroadcast_prepare_payload("sui.signed_json".into(), raw.into()).unwrap();
        assert_eq!(p.broadcast_payload, raw);
    }

    #[test]
    fn prepare_payload_wrap_key() {
        let p = rebroadcast_prepare_payload("xrp.blob_hex".into(), "deadbeef".into()).unwrap();
        assert_eq!(p.chain_id, "xrp");
        assert_eq!(p.result_field, "txid");
        let parsed: serde_json::Value = serde_json::from_str(&p.broadcast_payload).unwrap();
        assert_eq!(parsed["tx_blob_hex"], "deadbeef");
    }

    #[test]
    fn prepare_payload_extract_field() {
        let raw = r#"{"raw_tx_hex":"ff00","other":"x"}"#;
        let p = rebroadcast_prepare_payload("bitcoin.rust_json".into(), raw.into()).unwrap();
        assert_eq!(p.chain_id, "bitcoin");
        assert_eq!(p.broadcast_payload, "ff00");
    }

    #[test]
    fn prepare_payload_passthrough() {
        let p = rebroadcast_prepare_payload("bitcoin.raw_hex".into(), "abcd".into()).unwrap();
        assert_eq!(p.broadcast_payload, "abcd");
    }

    #[test]
    fn prepare_payload_unknown_errors() {
        assert!(rebroadcast_prepare_payload("nope".into(), "x".into()).is_err());
    }

    #[test]
    fn evm_bump_respects_floor() {
        let r =
            evm_replacement_fee_bump(Some("0.01".to_string()), Some("0.01".to_string()), 4.0, 2.0);
        assert_eq!(r.max_fee_gwei, "0.100");
        assert_eq!(r.priority_fee_gwei, "0.100");
    }
}

/// Whether a send is addressed to a private extension-block output, which the
/// composer badges.
///
/// The app decided it by comparing the chain's name with "Litecoin" and the
/// address with two prefixes — the rule `extra_output_overhead_bytes` already
/// held in core, restated beside a string.
#[uniffi::export]
pub fn is_extension_block_send_destination(chain_name: String, destination: String) -> bool {
    crate::registry::Chain::from_display_name(&chain_name)
        .is_some_and(|chain| chain.is_extension_block_destination(&destination))
}

/// Extra transaction bytes a destination costs beyond a plain output, by chain.
///
/// Not exported: the preview core builds prices these bytes itself. The front
/// end fetched the number to do that arithmetic on its side.
pub fn extra_output_overhead_bytes(chain_name: String, destination: String) -> u64 {
    crate::registry::Chain::from_display_name(&chain_name)
        .map(|c| c.extra_output_overhead_bytes(&destination))
        .unwrap_or(0)
}

#[cfg(test)]
mod validating_and_normalising_cannot_disagree {
    use super::{
        evaluate_high_risk_send_reasons, is_valid_send_address, normalize_address,
        HighRiskSendRequest,
    };
    use crate::registry::Chain;

    fn high_risk_codes(chain_name: &str, destination: &str) -> Vec<String> {
        evaluate_high_risk_send_reasons(HighRiskSendRequest {
            chain_name: chain_name.to_string(),
            symbol: "SUI".to_string(),
            amount: 1.0,
            holding_amount: 1000.0,
            destination_address: destination.to_string(),
            destination_input: destination.to_string(),
            used_ens_resolution: false,
            wallet_family_name: chain_name.to_string(),
            address_book_entries: vec![],
            tx_addresses: vec![],
        })
        .into_iter()
        .map(|warning| warning.code().to_string())
        .collect()
    }

    /// The third caller of the same question.
    ///
    /// The fix above landed in `is_valid_send_address` and the high-risk check
    /// kept validating the raw string, so a Sui address typed without its `0x`
    /// was accepted by the composer, accepted by the store, and called
    /// `invalid_format` by the warning sheet at the same time. Both orders,
    /// one answer — including here.
    #[test]
    fn the_high_risk_check_asks_the_same_question_the_composer_does() {
        let bare = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        for form in [bare.to_string(), normalize_address("Sui", bare)] {
            assert!(is_valid_send_address("Sui".into(), form.clone()));
            assert!(
                !high_risk_codes("Sui", &form).contains(&"invalid_format".to_string()),
                "{form} validates but was flagged invalid_format"
            );
        }
    }

    /// Case folding is the chain's decision, not this comparison's.
    ///
    /// A blanket `to_lowercase()` sat on top of `normalize_address` here. On
    /// EVM that is a no-op — the rule is already `Lowercase` — but on a chain
    /// whose rule is `None` it makes two different base58 addresses compare
    /// equal, so an address that merely *looks* like one in the book passed as
    /// already-seen and the `new_address` warning did not fire.
    #[test]
    fn a_case_sensitive_chain_does_not_fold_a_lookalike_into_a_known_address() {
        use super::{HighRiskChainAddress, HighRiskSendRequest};

        // A real Solana address, and the same string with one letter recased —
        // a different address, and base58 says so.
        let known = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
        let lookalike = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWm";
        assert_ne!(known, lookalike);

        let codes = |destination: &str| {
            evaluate_high_risk_send_reasons(HighRiskSendRequest {
                chain_name: "Solana".to_string(),
                symbol: "SOL".to_string(),
                amount: 1.0,
                holding_amount: 1000.0,
                destination_address: destination.to_string(),
                destination_input: destination.to_string(),
                used_ens_resolution: false,
                wallet_family_name: "Solana".to_string(),
                address_book_entries: vec![HighRiskChainAddress {
                    chain_name: "Solana".to_string(),
                    address: known.to_string(),
                }],
                tx_addresses: vec![],
            })
            .into_iter()
            .map(|warning| warning.code().to_string())
            .collect::<Vec<_>>()
        };

        assert!(
            !codes(known).contains(&"new_address".to_string()),
            "the address in the book is not a new address"
        );
        assert!(
            codes(lookalike).contains(&"new_address".to_string()),
            "a different address that differs only in case is still a new address"
        );
    }

    /// The chains that *do* fold case keep folding it: this is the registry's
    /// rule being applied, not case sensitivity being imposed everywhere.
    #[test]
    fn an_evm_address_still_matches_the_book_in_any_case() {
        use super::{HighRiskChainAddress, HighRiskSendRequest};

        let stored = "0xAbCdEf0123456789AbCdEf0123456789AbCdEf01";
        let typed = stored.to_uppercase().replace("0X", "0x");
        assert_ne!(stored, typed);

        let codes = evaluate_high_risk_send_reasons(HighRiskSendRequest {
            chain_name: "Ethereum".to_string(),
            symbol: "ETH".to_string(),
            amount: 1.0,
            holding_amount: 1000.0,
            destination_address: typed.clone(),
            destination_input: typed,
            used_ens_resolution: false,
            wallet_family_name: "Ethereum".to_string(),
            address_book_entries: vec![HighRiskChainAddress {
                chain_name: "Ethereum".to_string(),
                address: stored.to_string(),
            }],
            tx_addresses: vec![],
        })
        .into_iter()
        .map(|warning| warning.code().to_string())
        .collect::<Vec<_>>();

        assert!(
            !codes.contains(&"new_address".to_string()),
            "EVM folds case, so this is the address already in the book: {codes:?}"
        );
    }

    /// It still says so when the address really is malformed.
    #[test]
    fn a_malformed_destination_is_still_flagged() {
        assert!(high_risk_codes("Sui", "definitely-not-an-address")
            .contains(&"invalid_format".to_string()));
    }

    /// The order a caller happens to use must not change the answer.
    ///
    /// `AddAddressBookEntry` normalizes and then validates; every Swift caller
    /// validated the raw string. On Sui those disagreed — a 64-hex address
    /// typed without its `0x` prefix is invalid raw and valid once
    /// `LowercaseHexPrefixed` has added the prefix — so the composer and the
    /// address book refused what the store would have accepted.
    #[test]
    fn a_sui_address_without_its_prefix_is_accepted_either_way() {
        let bare = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let normalized = normalize_address("Sui", bare);
        assert!(normalized.starts_with("0x"), "{normalized}");
        assert!(is_valid_send_address("Sui".into(), bare.to_string()));
        assert!(is_valid_send_address("Sui".into(), normalized));
    }

    /// Every chain, both orders, one answer. Whitespace and case are part of
    /// what normalizing settles, so they are in the input here.
    #[test]
    fn no_chain_answers_differently_before_and_after_normalising() {
        for chain in Chain::mainnets() {
            let name = chain.chain_display_name().to_string();
            for sample in [
                "  0x742d35Cc6634C0532925a3b844Bc454e4438f44e  ",
                "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
                "not-an-address",
                "",
            ] {
                let direct = is_valid_send_address(name.clone(), sample.to_string());
                let after = is_valid_send_address(name.clone(), normalize_address(&name, sample));
                assert_eq!(
                    direct, after,
                    "{name} answers {direct} for {sample:?} and {after} once normalized"
                );
            }
        }
    }
}

/// Only a quote for the selected asset can populate its amount field. Generic
/// simple-chain previews quote the native gas asset even for token holdings.
pub(crate) fn quoted_send_amount(
    preview: Option<SendPreview>,
    chain_name: String,
    is_native: bool,
    token_decimals: Option<u32>,
    percentage: u32,
) -> Option<String> {
    let chain = Chain::from_display_name(&chain_name)?;
    let preview = preview?;
    let decimals = if is_native {
        u32::from(chain.native_decimals())
    } else {
        match &preview {
            SendPreview::Ethereum { .. } if chain.is_evm() => {}
            SendPreview::Tron { .. } if chain == Chain::Tron => {}
            _ => return None,
        }
        token_decimals?
    };
    // No fallback to a caller's portfolio balance: a missing maximum is a
    // missing quote, not permission to offer the whole holding.
    let maximum = compute_send_preview_details(Some(preview), f64::NAN)?.maxSendable?;
    crate::send::amount_input::send_amount_shortcut(maximum, decimals, percentage)
}

#[cfg(test)]
mod shortcut_preview_tests {
    use super::*;
    #[test]
    fn no_quote_and_gas_coin_quotes_cannot_fill_token_amounts() {
        assert!(quoted_send_amount(None, "Bitcoin".into(), true, None, 100).is_none());
        let preview = SendPreview::Solana {
            preview: SolanaSendPreview {
                maxSendable: 12.0,
                ..Default::default()
            },
        };
        assert!(quoted_send_amount(Some(preview), "Solana".into(), false, Some(6), 100).is_none());
        let preview = SendPreview::Ethereum {
            preview: EvmSendPreview {
                maxSendable: Some(4.2),
                ..Default::default()
            },
        };
        assert_eq!(
            quoted_send_amount(Some(preview), "Ethereum".into(), false, Some(6), 100).as_deref(),
            Some("4.199999")
        );
    }
}

#[cfg(test)]
mod scanned_payload_tests {
    use super::*;

    const BTC: &str = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq";
    const EVM: &str = "0x9858EfFD232B4033E47d90003D41EC34EcaEda94";

    /// The shapes a wallet actually puts in a QR code. Every one of these was
    /// pasted into the send field verbatim before the scanner asked core.
    #[test]
    fn payment_uris_reduce_to_the_address_they_carry() {
        let cases = [
            ("Bitcoin", BTC.to_string(), BTC),
            ("Bitcoin", format!("bitcoin:{BTC}"), BTC),
            (
                "Bitcoin",
                format!("bitcoin:{BTC}?amount=0.1&label=Shop"),
                BTC,
            ),
            ("Bitcoin", format!("  {BTC}  "), BTC),
            // EIP-681, with and without the chain-id pin and the function.
            ("Ethereum", format!("ethereum:{EVM}"), EVM),
            ("Ethereum", format!("ethereum:{EVM}@1"), EVM),
            (
                "Ethereum",
                format!("ethereum:{EVM}@1/transfer?value=1"),
                EVM,
            ),
            // A scheme that puts the address in a path segment.
            ("Ethereum", format!("wc://x/{EVM}"), EVM),
        ];
        for (chain, payload, expected) in cases {
            assert_eq!(
                scanned_send_address(chain.into(), payload.clone()).as_deref(),
                Some(normalize_address(chain, expected)).as_deref(),
                "{chain} did not read {payload}"
            );
        }
    }

    /// The returned address is the stored form, not the scanned one. The
    /// scanner lowercased EVM addresses itself and left every other chain's
    /// normalization — Sui's and Aptos's missing `0x`, NEAR's and ICP's case —
    /// to whoever read the field next.
    #[test]
    fn the_address_comes_back_in_the_form_the_store_keeps() {
        assert_eq!(
            scanned_send_address("Ethereum".into(), format!("ethereum:{EVM}")).as_deref(),
            Some(EVM.to_lowercase().as_str())
        );
        let bare_sui = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        assert_eq!(
            scanned_send_address("Sui".into(), format!("sui:{bare_sui}")).as_deref(),
            Some(format!("0x{bare_sui}").as_str())
        );
    }

    /// Nothing that is not an address on the named chain survives, and the
    /// chain is not optional: the iOS caller returned the first candidate
    /// unvalidated when no asset was selected.
    #[test]
    fn a_payload_with_no_address_for_this_chain_yields_nothing() {
        for payload in [
            "",
            "   ",
            "not-an-address",
            "https://example.com/pay?to=someone",
            // A real address, on the wrong chain.
            BTC,
        ] {
            assert_eq!(
                scanned_send_address("Ethereum".into(), payload.into()),
                None,
                "Ethereum accepted {payload}"
            );
        }
        assert_eq!(
            scanned_send_address("Not A Chain".into(), BTC.into()),
            None,
            "an unknown chain must judge nothing"
        );
    }

    /// The whole payload is tried first, so a bare address is never split, and
    /// a later candidate is only reached because the earlier ones failed.
    #[test]
    fn candidates_run_from_the_most_literal_to_the_least() {
        let candidates = scanned_address_candidates(&format!("bitcoin:{BTC}?amount=1"));
        assert_eq!(
            candidates.first().map(String::as_str),
            Some(format!("bitcoin:{BTC}?amount=1").as_str())
        );
        assert!(candidates.iter().any(|c| c == BTC));
        // No empties and no duplicates, whatever the punctuation.
        let noisy = scanned_address_candidates("ton://transfer//EQ1/");
        assert!(noisy.iter().all(|c| !c.trim().is_empty()));
        let mut unique = noisy.clone();
        unique.dedup();
        assert_eq!(unique.len(), noisy.len());
    }
}

#[cfg(test)]
mod high_risk_warning_shape {
    use super::{evaluate_high_risk_send_reasons, HighRiskSendRequest, HighRiskSendWarning};

    fn warnings(chain_name: &str, destination: &str) -> Vec<HighRiskSendWarning> {
        evaluate_high_risk_send_reasons(HighRiskSendRequest {
            chain_name: chain_name.to_string(),
            symbol: "X".to_string(),
            amount: 1.0,
            holding_amount: 1000.0,
            destination_address: destination.to_string(),
            destination_input: destination.to_string(),
            used_ens_resolution: false,
            wallet_family_name: chain_name.to_string(),
            address_book_entries: vec![],
            tx_addresses: vec![],
        })
    }

    /// Four chain-named codes were one reason. It is one variant now, and the
    /// chain it was raised on travels with it rather than being in its name.
    #[test]
    fn a_foreign_address_is_one_reason_that_names_its_chain() {
        for chain in ["Tron", "Solana", "XRP Ledger", "Monero"] {
            assert!(
                warnings(chain, "0x1111111111111111111111111111111111111111").contains(
                    &HighRiskSendWarning::ForeignAddressFormat {
                        chain: chain.to_string()
                    }
                ),
                "{chain} did not flag an EVM-shaped destination"
            );
        }
    }

    /// The code stays in the serialized form beside the variant's fields.
    #[test]
    fn a_warning_serializes_with_its_code() {
        let warning = HighRiskSendWarning::LargeSend {
            percent: 40,
            symbol: "BTC".to_string(),
        };
        assert_eq!(
            serde_json::to_value(&warning).unwrap(),
            serde_json::json!({"code": "large_send", "percent": 40, "symbol": "BTC"})
        );
        assert_eq!(warning.code(), "large_send");
    }
}
