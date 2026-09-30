pub mod amount_input;
pub mod ethereum;
mod evm_overrides;
pub mod flow;
pub mod keys;

pub mod payload;
pub mod preview_decode;
pub mod preview_types;
pub mod stages;
pub mod transfer;
pub mod verification;

// Per-chain write-path: build / sign / broadcast transaction methods.
pub(crate) mod accounting;
pub mod aptos;
#[cfg(test)]
mod audit_tests;
mod bcs;
pub mod bitcoin;
pub mod bitcoin_cash;
pub mod bitcoin_gold;
pub mod bitcoin_sv;
pub(crate) mod bitcoin_wire;
pub mod bittensor;
pub mod cardano;
pub mod dash;
pub mod decred;
pub mod dogecoin;
pub mod evm;
pub mod icp;
pub(crate) mod icp_stages;
pub mod kaspa;
pub mod litecoin;
pub(crate) mod monero_local;
pub mod mweb;
pub mod near;
pub mod polkadot;
pub mod solana;
pub mod stellar;
pub mod substrate;
pub mod sui;
pub mod ton;
pub mod tron;
pub mod xrp;
pub mod zcash;
pub(crate) mod zcash_stages;

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

pub use transfer::{SendAsset, SendAssetKind, SendTokenIdentity};

/// Whether a send can be made, with what core resolved to make it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SendPreflight {
    pub chain: crate::registry::Chain,
    pub symbol: String,
    pub normalized_destination_address: String,
    /// The amount as typed, trimmed: an exact decimal, never a float.
    pub amount: String,
    /// The token this send moves, or `None` for a native asset. A token core
    /// cannot identify is refused rather than sent with a guessed scale.
    pub token_contract_address: Option<String>,
    pub token_decimals: Option<u32>,
    /// The gas-asset balance a token send needs before it can land, where the
    /// chain has no fee estimate for that path — see
    /// `Chain::token_send_gas_reserve`. `None` for a native send.
    pub token_send_gas_reserve: Option<String>,
}

/// Unified request for `WalletService::execute_send`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct SendExecutionRequest {
    /// Spectra chain ID string (e.g. "bitcoin", "ethereum").
    pub chain_id: crate::registry::Chain,
    /// Core-owned wallet whose stored signing identity is used.
    pub wallet_id: String,
    /// Required only for a password-sealed wallet. No seed or raw key crosses here.
    pub password: Option<String>,
    /// Destination/recipient address.
    pub to_address: String,
    /// Exact decimal input, validated and converted to integer units in core.
    pub amount_str: String,
    // ── Token-specific ──────────────────────────────────────────────────
    /// Contract/mint address for token sends (ERC-20, SPL, TRC-20, NEP-141).
    pub contract_address: Option<String>,
    /// Token decimals for raw-unit conversion.
    pub token_decimals: Option<u32>,
    // ── Chain-specific optional fields ───────────────────────────────────
    /// Fee rate as an exact decimal: sat/vB for Bitcoin, DOGE per kB for
    /// Dogecoin.
    pub fee_rate_svb: Option<String>,
    /// UTXO fee in satoshis (BCH, BSV, LTC, DOGE).
    pub fee_sat: Option<u64>,
    /// Sui gas budget in SUI, as an exact decimal.
    pub gas_budget: Option<String>,
    /// Cardano fee in ADA, as an exact decimal.
    pub fee_amount: Option<String>,
    /// EVM overrides (nonce, custom gas fees). Typed; Rust assembles the
    /// payload fragment internally — no JSON shuttle from Swift.
    pub evm_overrides: Option<crate::send::ethereum::EvmSendOverridesInput>,
    /// Monero priority level.
    pub monero_priority: Option<u32>,
    /// Sign the transaction and stop, without putting it on the chain.
    ///
    /// Everything a send does except the irreversible step: the stored
    /// identity is resolved, the amount and fees are converted, the live nonce
    /// or UTXO set is read, the transaction is built and signed, and the raw
    /// payload comes back. It is how the send path is exercised end to end
    /// without moving funds — see "Known open items". The EVM builder already
    /// had this behind `EvmSendOverridesInput::sign_only`; the Bitcoin builder
    /// had it and the execution path hard-coded it to `false`.
    #[uniffi(default = false)]
    pub sign_only: bool,
}

impl SendExecutionRequest {
    /// Whether this asks to sign and stop, by either route.
    ///
    /// One function rather than the expression written out wherever the
    /// question came up: the refusal read both routes and the result field
    /// read only `sign_only`, so a caller asking through the EVM overrides got
    /// a signed transaction and a `None` where it should be. Four readers, one
    /// answer, and they cannot drift apart again.
    pub(crate) fn wants_sign_only(&self) -> bool {
        self.sign_only
            || self
                .evm_overrides
                .as_ref()
                .and_then(|input| input.sign_only)
                .unwrap_or(false)
    }

    pub(crate) fn zeroize_sensitive_fields(&mut self) {
        if let Some(password) = &mut self.password {
            password.zeroize();
        }
    }
}

/// Result from `WalletService::execute_send`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct SendExecutionResult {
    /// Serialized protocol result, including protocol metadata and signed payload.
    /// Swift must not parse this for business state; add typed fields here
    /// when new send-result values are needed.
    pub protocol_result_json: String,
    /// Extracted transaction hash/ID.
    pub transaction_hash: String,
    /// Payload format key (e.g. "bitcoin.rust_json").
    pub payload_format: String,
    /// EVM-specific details (nonce, raw_tx_hex, gas_limit), converted directly
    /// from the protocol result. `None` for non-EVM chains.
    pub evm: Option<crate::send::ethereum::EvmSendDetails>,
    /// The signed transaction, hex-encoded, when `sign_only` was asked for.
    ///
    /// A typed field rather than something to dig out of `protocol_result_json`:
    /// that is an opaque per-chain blob, and a caller reading it for business
    /// state is what this record's own comment warns against.
    pub signed_payload: Option<String>,
}

/// Why a send cannot land, once the fee is counted.
///
/// The preflight already refuses `amount > available_balance`.
/// That is half the question: the fee comes out of the chain's gas asset,
/// which for a token send is a different balance entirely, and the half that
/// knew about it lived in Swift.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum SendAffordability {
    Unavailable,
    /// Amount and fee both fit.
    Affordable,
    /// A native send: amount plus fee is more than the wallet holds.
    AmountPlusFeeExceedsBalance {
        symbol: String,
        required: String,
    },
    /// A token send: the amount alone is more than the token balance.
    AmountExceedsBalance {
        symbol: String,
    },
    /// A token send whose amount fits, with too little gas asset for the fee.
    FeeExceedsGasBalance {
        gas_symbol: String,
        fee: String,
        chain_id: crate::registry::Chain,
    },
}

/// What the caller knows. Everything else — whether this is the chain's own
/// asset, what that asset is called, how many decimals a fee is quoted to —
/// is read from the registry here.
#[derive(Debug, Clone, uniffi::Record)]
pub struct SendAffordabilityInput {
    pub is_native: bool,
    pub chain_id: crate::registry::Chain,
    /// The asset being sent.
    pub symbol: String,
    /// Exact decimals, all of them: this decides whether funds leave.
    pub amount: String,
    pub network_fee: String,
    /// What the wallet holds of the asset being sent.
    pub holding_balance: String,
    /// What it holds of the chain's gas asset. Ignored for a native send,
    /// where that is `holding_balance`.
    pub gas_balance: Option<String>,
}

/// Can this send land?
///
/// Naming the chain is enough: whether the asset is native, its symbol and
/// the fee are registry facts.
///
/// The verdict is an enum, not a sentence: the wording is localized in the
/// front end's own bundle.
#[uniffi::export]
pub fn send_affordability(input: SendAffordabilityInput) -> SendAffordability {
    if !input.is_native && input.gas_balance.is_none() {
        return SendAffordability::Unavailable;
    }
    let gas_symbol = input.chain_id.coin_symbol().to_string();
    use crate::decimal::{add, compare};
    use std::cmp::Ordering::Greater;
    // An amount that is not a decimal cannot be judged, and is not sent.
    let exceeds = |a: &str, b: &str| compare(a, b).is_none_or(|o| o == Greater);

    // A chain whose gas asset we cannot name is not a chain we can judge a
    // token send on, so treat the send as native: that path needs no gas
    // symbol and still refuses amount + fee over the balance.
    let is_native = input.is_native;

    if is_native {
        let Some(total) = add(&input.amount, &input.network_fee) else {
            return SendAffordability::Unavailable;
        };
        if exceeds(&total, &input.holding_balance) {
            return SendAffordability::AmountPlusFeeExceedsBalance {
                symbol: input.symbol,
                required: total,
            };
        }
        return SendAffordability::Affordable;
    }

    if exceeds(&input.amount, &input.holding_balance) {
        return SendAffordability::AmountExceedsBalance {
            symbol: input.symbol,
        };
    }
    match input.gas_balance {
        Some(gas_balance) if exceeds(&input.network_fee, &gas_balance) => {
            SendAffordability::FeeExceedsGasBalance {
                gas_symbol,
                fee: crate::decimal::canonical(&input.network_fee).unwrap_or(input.network_fee),
                chain_id: input.chain_id,
            }
        }
        _ => SendAffordability::Affordable,
    }
}

/// Can this send be made? `asset` is `None` when the holding is missing or on
/// a chain core does not know.
pub fn validate_send_preflight(
    wallet_found: bool,
    asset: Option<&SendAsset>,
    available_balance: &str,
    destination_address: &str,
    amount_input: &str,
) -> Result<SendPreflight, String> {
    if !wallet_found {
        return Err("Select a wallet".to_string());
    }
    let asset = asset.ok_or_else(|| "Select an asset".to_string())?;
    if !asset.is_sendable() {
        return Err(format!("{} transfers are not enabled yet.", asset.symbol));
    }

    let normalized_destination_address = destination_address.trim().to_string();
    if normalized_destination_address.is_empty() {
        return Err("Enter a destination address".to_string());
    }

    let amount_input = amount_input.trim();
    let exact = crate::decimal::canonical(amount_input)
        .ok_or_else(|| "Enter a valid amount".to_string())?;
    if !asset.allows_zero_amount() && crate::decimal::is_zero(&exact) {
        return Err("Enter a valid amount".to_string());
    }
    if crate::decimal::compare(&exact, available_balance)
        .is_none_or(|o| o == std::cmp::Ordering::Greater)
    {
        return Err("Amount exceeds the available balance".to_string());
    }

    let token = asset.token();
    Ok(SendPreflight {
        chain: asset.chain,
        symbol: asset.symbol.clone(),
        normalized_destination_address,
        amount: amount_input.to_string(),
        token_contract_address: token.map(|token| token.contract.clone()),
        token_decimals: token.map(|token| token.decimals),
        // Only a token send needs it: the native asset pays its own fee out
        // of the amount, which the balance check above already covers.
        token_send_gas_reserve: token
            .and(asset.chain.token_send_gas_reserve())
            .map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        SendAsset, SendAssetKind, SendExecutionRequest, SendTokenIdentity, validate_send_preflight,
    };
    use crate::registry::Chain;

    /// Every chain sends its native asset, so every chain can name a fee: its
    /// own preview path, a shared-path shape, or a fallback.
    #[test]
    fn every_chain_has_a_send_preview() {
        for chain in Chain::all() {
            assert!(
                chain.has_send_preview(),
                "{} sends but the screen would say it has no network preview",
                chain.str_id()
            );
        }
    }

    /// Every EVM chain folds its addresses the same way, mainnets and testnets
    /// alike. Seven of twenty-three were named before, so an address book entry
    /// on the other sixteen kept whatever case was typed.
    #[test]
    fn every_evm_chain_lowercases_its_addresses() {
        use crate::registry::Chain;
        use crate::send::flow::normalize_address;

        let mixed = "0xAbCdEf0123456789AbCdEf0123456789AbCdEf01";
        for chain in Chain::all().filter(|c| c.is_evm()) {
            assert_eq!(
                normalize_address(chain, mixed),
                mixed.to_lowercase(),
                "{} left a mixed-case address as typed",
                chain.str_id()
            );
        }
    }

    /// And a chain whose addresses are case-significant is left alone. Folding
    /// a Bitcoin or Solana address destroys it.
    #[test]
    fn case_significant_chains_are_untouched() {
        use crate::registry::{AddressNormalization, Chain};
        use crate::send::flow::normalize_address;

        for chain in Chain::all() {
            if chain.address_normalization() != AddressNormalization::None {
                continue;
            }
            let sample = "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2";
            assert_eq!(
                normalize_address(chain, sample),
                sample,
                "{} altered an address whose case is significant",
                chain.str_id()
            );
        }
    }

    /// Every EVM chain runs the EVM destination checks, and every one that is
    /// not Ethereum flags an ENS name.
    #[test]
    fn every_evm_chain_runs_the_evm_destination_checks() {
        use crate::registry::Chain;
        use crate::send::flow::{HighRiskSendRequest, evaluate_high_risk_send_reasons};

        let request = |chain: Chain, destination: &str| HighRiskSendRequest {
            chain_id: chain,
            symbol: chain.coin_symbol().to_string(),
            amount: "1".into(),
            holding_amount: "100".into(),
            destination_address: destination.to_string(),
            destination_input: destination.to_string(),
            used_ens_resolution: false,
            wallet_chain_id: chain,
            address_book_entries: Vec::new(),
            tx_addresses: Vec::new(),
        };
        let codes = |w: Vec<crate::send::flow::HighRiskSendWarning>| -> Vec<String> {
            w.into_iter().map(|w| w.code().to_string()).collect()
        };

        for chain in Chain::all().filter(|c| c.is_evm()) {
            // A native SegWit address is not an EVM address on any chain.
            let bitcoin_address = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq";
            assert!(
                codes(evaluate_high_risk_send_reasons(request(
                    chain,
                    bitcoin_address
                )))
                .contains(&"non_evm_on_evm".to_string()),
                "{} accepted a Bitcoin address with no warning",
                chain.str_id()
            );

            let ens = "vitalik.eth";
            let raised = codes(evaluate_high_risk_send_reasons(request(chain, ens)));
            let expected = chain.mainnet_counterpart() != Chain::Ethereum;
            assert_eq!(
                raised.contains(&"ens_off_ethereum".to_string()),
                expected,
                "{} handled an ENS name wrongly",
                chain.str_id()
            );
        }
    }

    /// Every EVM chain can name the asset its history is denominated in.
    #[test]
    fn every_evm_chain_names_its_native_asset() {
        use crate::registry::Chain;

        for chain in Chain::all().filter(|c| c.is_evm()) {
            let asset = crate::fetch::history_decode::history_evm_native_asset(chain)
                .unwrap_or_else(|| panic!("{} has no native asset", chain.str_id()));
            assert_eq!(asset.symbol, chain.entry().gas_token_symbol);
            assert!(
                !asset.symbol.is_empty(),
                "{} has no native symbol",
                chain.str_id()
            );
            assert!(!asset.asset_display_name.is_empty());
        }
        // And a chain that is not EVM is still refused.
        assert!(
            crate::fetch::history_decode::history_evm_native_asset(crate::registry::Chain::Bitcoin)
                .is_none()
        );
    }

    /// The MWEB overhead belongs to Litecoin and to MWEB destinations only.
    #[test]
    fn only_litecoin_mweb_destinations_cost_extra_bytes() {
        use crate::registry::Chain;

        let mweb = "ltcmweb1qq0000000000000000000000000000000000000000000000000000000";
        assert_eq!(Chain::Litecoin.extra_output_overhead_bytes(mweb), 1017);
        // A testnet MWEB address on the same chain family, and the mainnet
        // prefix on the testnet chain.
        assert_eq!(
            Chain::LitecoinTestnet.extra_output_overhead_bytes("tmweb1qq"),
            1017
        );
        // A plain Litecoin address costs nothing extra.
        assert_eq!(
            Chain::Litecoin
                .extra_output_overhead_bytes("ltc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"),
            0
        );
        // The composer's badge asks the same rule.
        assert!(crate::send::flow::is_extension_block_send_destination(
            crate::registry::Chain::Litecoin,
            mweb.into()
        ));
        assert!(!crate::send::flow::is_extension_block_send_destination(
            crate::registry::Chain::Bitcoin,
            mweb.into()
        ));
        // And no other chain charges it, whatever the destination looks like.
        for chain in Chain::all().filter(|c| c.mainnet_counterpart() != Chain::Litecoin) {
            assert_eq!(
                chain.extra_output_overhead_bytes(mweb),
                0,
                "{} charged Litecoin's MWEB overhead",
                chain.str_id()
            );
        }
    }

    fn asset(chain: Chain, symbol: &str, kind: SendAssetKind) -> SendAsset {
        SendAsset {
            chain,
            symbol: symbol.into(),
            kind,
        }
    }

    #[test]
    fn rejects_zero_amount_for_non_evm_native_sends() {
        let btc = asset(Chain::Bitcoin, "BTC", SendAssetKind::Native);
        let error = validate_send_preflight(true, Some(&btc), "1", "bc1qdestination", "0")
            .expect_err("bitcoin zero-value sends should be rejected in preflight");
        assert_eq!(error, "Enter a valid amount");
    }

    #[test]
    fn preserves_zero_amount_for_native_evm_preflight() {
        let eth = asset(Chain::Ethereum, "ETH", SendAssetKind::Native);
        let plan = validate_send_preflight(true, Some(&eth), "1", "0xabc", "0")
            .expect("native EVM zero-value sends remain allowed");
        assert_eq!(plan.amount, "0");
    }

    /// A token nothing tracks is refused, whatever chain it is on. The EVM
    /// and Tron routes used to let one through with no contract to send.
    #[test]
    fn an_untracked_token_is_refused() {
        let usdt = asset(Chain::Ethereum, "USDT", SendAssetKind::UntrackedToken);
        let error = validate_send_preflight(true, Some(&usdt), "10", "0xabc", "1")
            .expect_err("an untracked token has no contract to send");
        assert_eq!(error, "USDT transfers are not enabled yet.");
    }

    /// The gas floor a NEP-141 send has to clear is a fact about NEAR, so core
    /// states it — and only where it applies: a native NEAR send pays its fee
    /// out of the amount, and every other chain estimates one.
    #[test]
    fn a_near_token_send_carries_the_chains_gas_floor() {
        let preflight = |asset: &SendAsset, destination: &str| {
            validate_send_preflight(true, Some(asset), "10", destination, "1")
                .expect("a sendable asset")
        };
        let token = |contract: &str, decimals| {
            SendAssetKind::Token(SendTokenIdentity {
                contract: contract.into(),
                decimals,
            })
        };

        let usdc = asset(Chain::Near, "USDC", token("usdc.near", 6));
        assert_eq!(
            preflight(&usdc, "receiver.near")
                .token_send_gas_reserve
                .as_deref(),
            Some("0.001")
        );
        let near = asset(Chain::Near, "NEAR", SendAssetKind::Native);
        assert_eq!(
            preflight(&near, "receiver.near").token_send_gas_reserve,
            None
        );

        let usdt = asset(
            Chain::Tron,
            "USDT",
            token("TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t", 6),
        );
        let tron = preflight(&usdt, "TLa2f6VPqDgRE67v1736s7bJ8Ray5wYjU7");
        assert_eq!(tron.token_send_gas_reserve, None);
        assert_eq!(tron.token_decimals, Some(6));
    }

    #[test]
    fn send_execution_request_scrubs_secret_fields() {
        let mut request = SendExecutionRequest {
            chain_id: crate::registry::Chain::Ethereum,
            wallet_id: "w".into(),
            password: Some("password".into()),
            to_address: "0xto".into(),
            amount_str: "1".into(),
            contract_address: None,
            token_decimals: None,
            fee_rate_svb: None,
            fee_sat: None,
            gas_budget: None,
            fee_amount: None,
            evm_overrides: None,
            monero_priority: None,
            sign_only: false,
        };
        request.zeroize_sensitive_fields();
        assert_eq!(request.password.as_deref(), Some(""));
    }
}

// ── FFI surface ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod the_generic_submit_chains_can_name_a_fee {
    /// Zcash, Bitcoin Gold, Decred, Kaspa and Dash take the shared submit path
    /// with no shared-path preview, so their fee comes from the fallback.
    #[test]
    fn each_has_a_fee_fallback() {
        use crate::registry::Chain;
        for chain in [
            Chain::Zcash,
            Chain::BitcoinGold,
            Chain::Decred,
            Chain::Kaspa,
            Chain::Dash,
        ] {
            let name = chain.str_id();
            assert!(
                chain.uses_generic_send_submit(),
                "{name} must take the shared submit path"
            );
            assert!(
                chain.send_execution_shape().fee_fallback.is_some(),
                "{name} needs a fee fallback"
            );
            assert!(chain.simple_preview_chain().is_none(), "{name}");
        }
    }
}

#[cfg(test)]
mod token_decimals_are_not_assumed {
    /// No chain's tokens all share one decimal count: Tron's USDT has six, and
    /// BTT, TUSD, USD1 and USDD have eighteen.
    #[test]
    fn a_chain_can_host_tokens_of_different_decimals() {
        use std::collections::{HashMap, HashSet};
        let mut by_chain: HashMap<crate::registry::Chain, HashSet<u32>> = HashMap::new();
        for token in crate::tokens::list_token_deployments(None) {
            by_chain
                .entry(token.chain_id)
                .or_default()
                .insert(token.decimals);
        }
        let mixed: Vec<_> = by_chain
            .iter()
            .filter(|(_, d)| d.len() > 1)
            .map(|(c, d)| {
                let mut v: Vec<_> = d.iter().copied().collect();
                v.sort_unstable();
                (*c, v)
            })
            .collect();
        assert!(
            !mixed.is_empty(),
            "if this ever holds, the assumption a caller could make is at least true"
        );
        let tron = by_chain
            .get(&crate::registry::Chain::Tron)
            .expect("tron hosts tokens");
        assert!(
            tron.len() > 1,
            "tron's tokens are all {tron:?} decimals — the hardcoded 6 would have been harmless"
        );
    }
}

#[cfg(test)]
mod affordability_reads_the_chain_rather_than_the_caller {
    use super::{SendAffordability, SendAffordabilityInput, send_affordability};

    fn input(chain: crate::registry::Chain, symbol: &str) -> SendAffordabilityInput {
        SendAffordabilityInput {
            is_native: chain.coin_symbol() == symbol,
            chain_id: chain,
            symbol: symbol.to_string(),
            amount: "1".into(),
            network_fee: "0.5".into(),
            holding_balance: "1.2".into(),
            gas_balance: Some("0.1".into()),
        }
    }

    /// The four Swift call sites each decided "is this the chain's own asset"
    /// themselves, and spelled it four ways. Naming the chain is enough, and
    /// the answer comes from the same column a balance is denominated in.
    #[test]
    fn a_governance_token_is_not_the_asset_the_fee_comes_out_of() {
        // Arbitrum charges gas in ETH. A caller that took ARB for the native
        // asset would check the fee against the ARB balance — the same pair,
        // and the same mistake, the destination probe's five-symbol list made.
        assert_eq!(
            send_affordability(input(crate::registry::Chain::Arbitrum, "ARB")),
            SendAffordability::FeeExceedsGasBalance {
                gas_symbol: "ETH".to_string(),
                fee: "0.5".to_string(),
                chain_id: crate::registry::Chain::Arbitrum,
            }
        );
        // Tron's own asset, which its Swift caller matched with a literal.
        assert_eq!(
            send_affordability(input(crate::registry::Chain::Tron, "TRX")),
            SendAffordability::AmountPlusFeeExceedsBalance {
                symbol: "TRX".to_string(),
                required: "1.5".to_string(),
            }
        );
    }

    #[test]
    fn the_fee_is_quoted_to_the_chains_own_decimals() {
        let mut btc = input(crate::registry::Chain::Bitcoin, "BTC");
        btc.amount = "2".into();
        assert_eq!(
            send_affordability(btc),
            SendAffordability::AmountPlusFeeExceedsBalance {
                symbol: "BTC".to_string(),
                required: "2.5".to_string(),
            }
        );
    }

    #[test]
    fn a_token_send_is_refused_on_its_own_balance_before_the_fee_is_looked_at() {
        let mut over = input(crate::registry::Chain::Ethereum, "USDC");
        over.amount = "5".into();
        assert_eq!(
            send_affordability(over),
            SendAffordability::AmountExceedsBalance {
                symbol: "USDC".to_string()
            }
        );
    }

    #[test]
    fn both_fitting_is_affordable() {
        let mut ok = input(crate::registry::Chain::Ethereum, "USDC");
        ok.gas_balance = Some("2".into());
        assert_eq!(send_affordability(ok), SendAffordability::Affordable);

        let mut native = input(crate::registry::Chain::Ethereum, "ETH");
        native.holding_balance = "10".into();
        assert_eq!(send_affordability(native), SendAffordability::Affordable);
    }
}
