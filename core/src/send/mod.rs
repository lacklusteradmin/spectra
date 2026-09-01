pub mod amount_input;
pub mod ethereum;
pub mod flow;

pub mod payload;
pub mod preview_decode;
pub mod preview_types;
pub mod transfer;
pub mod verification;

// Per-chain write-path: build / sign / broadcast transaction methods.
pub mod chains;

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SendAssetRoutingInput {
    pub chain_name: String,
    pub symbol: String,
    pub is_evm_chain: bool,
    pub supports_solana_send_coin: bool,
    #[serde(default)]
    pub supports_near_token_send: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SendAssetRoutingPlan {
    pub preview_kind: Option<String>,
    pub submit_kind: Option<String>,
    pub native_evm_symbol: Option<String>,
    pub is_native_evm_asset: bool,
    pub allows_zero_amount: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SendSubmitPreflightRequest {
    pub wallet_found: bool,
    pub asset_found: bool,
    pub destination_address: String,
    pub amount_input: String,
    pub available_balance: f64,
    pub asset: Option<SendAssetRoutingInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SendSubmitPreflightPlan {
    pub submit_kind: String,
    pub preview_kind: Option<String>,
    pub normalized_destination_address: String,
    pub amount: f64,
    /// Original trimmed user input string — carries exact decimal representation
    /// through to `SendExecutionRequest.amount_str` so raw-unit conversion never
    /// touches f64.
    pub amount_str: String,
    pub chain_name: String,
    pub symbol: String,
    pub native_evm_symbol: Option<String>,
    pub is_native_evm_asset: bool,
    pub allows_zero_amount: bool,
    /// Whether this send takes the shared submit path — see
    /// `Chain::uses_generic_send_submit`. Decided here rather than by the
    /// caller because NEAR qualifies for its native asset and not for a token
    /// on it, which is a question about the asset and not only the chain.
    pub uses_generic_submit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TransferRequest {
    pub chain_name: String,
    pub from_address: String,
    pub to_address: String,
    pub amount: String,
    pub asset_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TransferPlan {
    pub chain_name: String,
    pub estimated_fee: String,
    pub signing_payload_hex: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SignedTransfer {
    pub chain_name: String,
    pub raw_transaction_hex: String,
    pub txid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastReceipt {
    pub chain_name: String,
    pub txid: String,
    pub source_id: String,
}

/// Unified request for `WalletService::execute_send`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct SendExecutionRequest {
    /// Spectra chain ID string (e.g. "bitcoin", "ethereum").
    pub chain_id: String,
    /// Chain display name used to select the derivation function ("Bitcoin", "Ethereum", …).
    pub chain_name: String,
    /// BIP-32/SLIP-10 derivation path (e.g. "m/84'/0'/0'/0/0").
    pub derivation_path: String,
    /// Seed phrase for HD derivation (mutually exclusive with `private_key_hex`).
    pub seed_phrase: Option<String>,
    /// Raw private key hex for non-HD wallets (mutually exclusive with `seed_phrase`).
    pub private_key_hex: Option<String>,
    /// Source/sender address.
    pub from_address: String,
    /// Destination/recipient address.
    pub to_address: String,
    /// Human-scale amount (e.g. 0.5 BTC, 1.0 ETH).
    pub amount: f64,
    /// Exact decimal string from the user's input (e.g. "0.1", "1.5"). When
    /// present, raw-unit conversion uses pure string arithmetic via
    /// `decimal_str_to_raw_units` — avoiding the f64 precision loss that
    /// occurs when `amount` is used directly. Always set this from
    /// `SendSubmitPreflightPlan.amount_str`; `None` falls back to the f64 path.
    pub amount_str: Option<String>,
    // ── Token-specific ──────────────────────────────────────────────────
    /// Contract/mint address for token sends (ERC-20, SPL, TRC-20, NEP-141).
    pub contract_address: Option<String>,
    /// Token decimals for raw-unit conversion.
    pub token_decimals: Option<u32>,
    // ── Chain-specific optional fields ───────────────────────────────────
    /// BTC fee rate in sat/vB.
    pub fee_rate_svb: Option<f64>,
    /// UTXO fee in satoshis (BCH, BSV, LTC, DOGE).
    pub fee_sat: Option<u64>,
    /// Sui gas budget in SUI.
    pub gas_budget: Option<f64>,
    /// Cardano fee in ADA.
    pub fee_amount: Option<f64>,
    /// EVM overrides (nonce, custom gas fees). Typed; Rust assembles the
    /// payload fragment internally — no JSON shuttle from Swift.
    pub evm_overrides: Option<crate::send::ethereum::EvmSendOverridesInput>,
    /// Monero priority level.
    pub monero_priority: Option<u32>,
    /// Power-user derivation overrides (passphrase, hmac key, script type, etc.).
    pub derivation_overrides: Option<crate::store::wallet_domain::CoreWalletDerivationOverrides>,
}

impl SendExecutionRequest {
    pub(crate) fn zeroize_sensitive_fields(&mut self) {
        if let Some(value) = &mut self.seed_phrase {
            value.zeroize();
        }
        if let Some(value) = &mut self.private_key_hex {
            value.zeroize();
        }
        if let Some(overrides) = &mut self.derivation_overrides {
            overrides.zeroize_sensitive_fields();
        }
    }
}

/// Result from `WalletService::execute_send`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct SendExecutionResult {
    /// Opaque chain payload used only for persisted rebroadcast support.
    /// Swift must not parse this for business state; add typed fields here
    /// when new send-result values are needed.
    pub rebroadcast_payload: String,
    /// Extracted transaction hash/ID.
    pub transaction_hash: String,
    /// Payload format key (e.g. "bitcoin.rust_json").
    pub payload_format: String,
    /// Decoded EVM-specific result (nonce, raw_tx_hex, gas_limit). Populated
    /// when the chain is EVM; `None` for non-EVM chains. Lets Swift skip the
    /// `decode_evm_send_result(json:)` round-trip.
    pub evm: Option<crate::send::ethereum::EvmSendResultDecoded>,
}

pub trait TransferPlanner: Send + Sync {
    fn build_plan(&self, request: &TransferRequest) -> Result<TransferPlan, String>;
}

pub trait TransactionBroadcaster: Send + Sync {
    fn broadcast(&self, signed_transfer: &SignedTransfer) -> Result<BroadcastReceipt, String>;
}

pub fn route_send_asset(input: &SendAssetRoutingInput) -> SendAssetRoutingPlan {
    let submit_kind = match (input.chain_name.as_str(), input.symbol.as_str()) {
        ("Bitcoin", "BTC") => Some("bitcoin"),
        ("Bitcoin Cash", "BCH") => Some("bitcoinCash"),
        ("Bitcoin SV", "BSV") => Some("bitcoinSV"),
        ("Litecoin", "LTC") => Some("litecoin"),
        ("Dogecoin", "DOGE") => Some("dogecoin"),
        ("Tron", "TRX") | ("Tron", "USDT") => Some("tron"),
        ("XRP Ledger", "XRP") => Some("xrp"),
        ("Stellar", "XLM") => Some("stellar"),
        ("Monero", "XMR") => Some("monero"),
        ("Cardano", "ADA") => Some("cardano"),
        ("Sui", "SUI") => Some("sui"),
        ("Aptos", "APT") => Some("aptos"),
        ("TON", "TON") => Some("ton"),
        ("Internet Computer", "ICP") => Some("icp"),
        ("NEAR", "NEAR") => Some("near"),
        ("Polkadot", "DOT") => Some("polkadot"),
        // Five chains whose send was written, wired into `execute_send`, and
        // then never named here — so `submit_kind` was `None` and the
        // preflight answered "transfers are not enabled yet" for all of them.
        // `core/src/send/chains/{zcash,bitcoin_gold,decred,kaspa,dash}.rs` are
        // 179 to 426 lines each.
        ("Zcash", "ZEC") => Some("zcash"),
        ("Bitcoin Gold", "BTG") => Some("bitcoin-gold"),
        ("Decred", "DCR") => Some("decred"),
        ("Kaspa", "KAS") => Some("kaspa"),
        ("Dash", "DASH") => Some("dash"),
        _ if input.is_evm_chain => Some("ethereum"),
        _ if input.supports_solana_send_coin => Some("solana"),
        _ if input.supports_near_token_send => Some("near"),
        _ => None,
    }
    .map(str::to_string);

    let native_evm_symbol = native_evm_symbol_for_chain(&input.chain_name);
    let is_native_evm_asset = native_evm_symbol
        .as_ref()
        .map(|symbol| input.symbol == symbol.as_str())
        .unwrap_or(false);

    SendAssetRoutingPlan {
        preview_kind: submit_kind.clone(),
        submit_kind,
        native_evm_symbol,
        is_native_evm_asset,
        allows_zero_amount: is_native_evm_asset,
    }
}

pub fn plan_send_submit_preflight(
    request: SendSubmitPreflightRequest,
) -> Result<SendSubmitPreflightPlan, String> {
    if !request.wallet_found {
        return Err("Select a wallet".to_string());
    }
    if !request.asset_found {
        return Err("Select an asset".to_string());
    }

    let asset = request.asset.ok_or_else(|| "Select an asset".to_string())?;
    let route = route_send_asset(&asset);
    let submit_kind = route
        .submit_kind
        .clone()
        .ok_or_else(|| format!("{} transfers are not enabled yet.", asset.symbol))?;

    let normalized_destination_address = request.destination_address.trim().to_string();
    if normalized_destination_address.is_empty() {
        return Err("Enter a destination address".to_string());
    }

    let amount_input = request.amount_input.trim();
    let amount = amount_input
        .parse::<f64>()
        .map_err(|_| "Enter a valid amount".to_string())?;

    if !amount.is_finite() || amount < 0.0 {
        return Err("Enter a valid amount".to_string());
    }

    if !route.allows_zero_amount && amount <= 0.0 {
        return Err("Enter a valid amount".to_string());
    }

    if amount > request.available_balance {
        return Err("Amount exceeds the available balance".to_string());
    }

    // NEAR is the one chain where this depends on the asset: its native send
    // is the shared shape and a token on it is not.
    let uses_generic_submit = crate::registry::Chain::from_display_name(&asset.chain_name)
        .is_some_and(|chain| {
            chain.uses_generic_send_submit() && asset.symbol == chain.coin_symbol()
        });

    Ok(SendSubmitPreflightPlan {
        submit_kind,
        preview_kind: route.preview_kind,
        normalized_destination_address,
        amount,
        amount_str: amount_input.to_string(),
        chain_name: asset.chain_name,
        symbol: asset.symbol,
        native_evm_symbol: route.native_evm_symbol,
        is_native_evm_asset: route.is_native_evm_asset,
        allows_zero_amount: route.allows_zero_amount,
        uses_generic_submit,
    })
}

/// The asset an EVM chain pays fees in, or `None` off the EVM family.
///
/// Seven chains were named here, so on the other sixteen EVM mainnets this
/// answered `None` — which made `is_native_evm_asset` false for the chain's own
/// gas token and `allows_zero_amount` false with it.
fn native_evm_symbol_for_chain(chain_name: &str) -> Option<String> {
    crate::registry::Chain::from_display_name(chain_name)
        .filter(|chain| chain.is_evm())
        .map(|chain| chain.coin_symbol().to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        plan_send_submit_preflight, route_send_asset, SendAssetRoutingInput,
        SendExecutionRequest, SendSubmitPreflightRequest,
    };

    /// Every chain the router sends down the shared preview path has a shape
    /// for it.
    ///
    /// Swift's dispatch used to name those eleven chains one arm at a time, and
    /// a twelfth entry in a `[String: SimpleChain]` table decided whether the
    /// call went out at all. Both are gone: the arm says "the chain this coin is
    /// on" and core derives the shape. What has to hold for that to be safe is
    /// this — a routing kind outside the seven with a preview path of their own
    /// is a chain `simple_preview_chain` answers for.
    #[test]
    fn every_shared_path_routing_kind_has_a_preview_shape() {
        use crate::registry::Chain;

        const DEDICATED: &[&str] = &[
            "bitcoin",
            "bitcoinCash",
            "bitcoinSV",
            "litecoin",
            "ethereum",
            "dogecoin",
            "tron",
        ];
        for chain in Chain::all().filter(|c| !c.is_testnet()) {
            let route = route_send_asset(&SendAssetRoutingInput {
                chain_name: chain.chain_display_name().to_string(),
                symbol: chain.coin_symbol().to_string(),
                is_evm_chain: chain.is_evm(),
                supports_solana_send_coin: false,
                supports_near_token_send: false,
            });
            let Some(kind) = route.preview_kind.as_deref() else { continue };
            if DEDICATED.contains(&kind) {
                continue;
            }
            // A chain that routes must be able to name a fee: through a
            // shared-path preview, or through the fallback the generic submit
            // uses when there is no preview to ask. Zcash, Bitcoin Gold,
            // Decred, Kaspa and Dash are the second kind — their sends were
            // written and wired into `execute_send`, and only this table was
            // missing them.
            assert!(
                chain.simple_preview_chain().is_some()
                    || chain.send_execution_shape().fee_fallback > 0.0,
                "{} routes as \"{kind}\" with neither a preview shape nor a fee fallback",
                chain.chain_display_name()
            );
        }
    }

    /// And nothing has a shape it cannot be routed to.
    #[test]
    fn every_preview_shape_belongs_to_a_chain_that_routes_there() {
        use crate::registry::Chain;

        for chain in Chain::all().filter(|c| !c.is_testnet()) {
            if chain.simple_preview_chain().is_none() {
                continue;
            }
            let route = route_send_asset(&SendAssetRoutingInput {
                chain_name: chain.chain_display_name().to_string(),
                symbol: chain.coin_symbol().to_string(),
                is_evm_chain: chain.is_evm(),
                // Native SOL has no `(chain, symbol)` arm of its own; it routes
                // through the token rule, which the service answers `true` for
                // the native coin. Stating that here is what keeps this test
                // about the shape mapping rather than about the token list.
                supports_solana_send_coin: chain == Chain::Solana,
                supports_near_token_send: false,
            });
            assert!(
                route.preview_kind.is_some(),
                "{} has a shared preview shape and routes nowhere",
                chain.chain_display_name()
            );
        }
    }

    #[test]
    fn routes_evm_native_assets_with_native_symbol_metadata() {
        let route = route_send_asset(&SendAssetRoutingInput {
            chain_name: "Avalanche".to_string(),
            symbol: "AVAX".to_string(),
            is_evm_chain: true,
            supports_solana_send_coin: false,
            supports_near_token_send: false,
        });

        assert_eq!(route.preview_kind.as_deref(), Some("ethereum"));
        assert_eq!(route.native_evm_symbol.as_deref(), Some("AVAX"));
        assert!(route.is_native_evm_asset);
        assert!(route.allows_zero_amount);
    }

    /// The preview and the submit are the same routing decision.
    ///
    /// They used to be two: a `plan_send_preview_routing` wrapper that read
    /// `route_send_asset().preview_kind` and threw the rest away, and a
    /// caller-side re-check for the submit branch. Asserting both fields of
    /// one route is what says they cannot drift.
    #[test]
    fn routes_supported_solana_assets_to_solana_preview_and_submit() {
        let route = route_send_asset(&SendAssetRoutingInput {
            chain_name: "Solana".to_string(),
            symbol: "USDC".to_string(),
            is_evm_chain: false,
            supports_solana_send_coin: true,
            supports_near_token_send: false,
        });

        assert_eq!(route.preview_kind.as_deref(), Some("solana"));
        assert_eq!(route.submit_kind.as_deref(), Some("solana"));

        // And an untracked mint routes nowhere, rather than to Solana.
        let untracked = route_send_asset(&SendAssetRoutingInput {
            chain_name: "Solana".to_string(),
            symbol: "USDC".to_string(),
            is_evm_chain: false,
            supports_solana_send_coin: false,
            supports_near_token_send: false,
        });
        assert_eq!(untracked.submit_kind, None);
    }

    /// The routing kinds are a closed set, and every chain that can send has
    /// one.
    ///
    /// `submitSend` switches on these strings. It used to re-derive the route
    /// from chain-name lists, so a kind renamed here would have changed
    /// nothing there; now a rename drops a chain straight into "not enabled
    /// yet" — silently, at the moment a user tries to send. This is the test
    /// that fails instead.
    #[test]
    fn every_sendable_chain_has_a_routing_kind_from_the_known_set() {
        use crate::registry::Chain;
        const KNOWN: &[&str] = &[
            "bitcoin",
            "bitcoinCash",
            "bitcoinSV",
            "litecoin",
            "dogecoin",
            "tron",
            "xrp",
            "stellar",
            "monero",
            "cardano",
            "sui",
            "aptos",
            "ton",
            "icp",
            "near",
            "polkadot",
            "ethereum",
            "solana",
                    "zcash",
            "bitcoin-gold",
            "decred",
            "kaspa",
            "dash",
];
        let mut unrouted = Vec::new();
        for chain in Chain::mainnets() {
            let symbol = chain.entry().gas_token_symbol.clone();
            let route = route_send_asset(&SendAssetRoutingInput {
                chain_name: chain.chain_display_name().to_string(),
                symbol,
                is_evm_chain: chain.is_evm(),
                supports_solana_send_coin: chain == Chain::Solana,
                supports_near_token_send: false,
            });
            match route.submit_kind.as_deref() {
                Some(kind) => assert!(
                    KNOWN.contains(&kind),
                    "{} routes to {kind:?}, which `submitSend` does not switch on",
                    chain.chain_display_name()
                ),
                None => unrouted.push(chain.chain_display_name()),
            }
        }
        // The chains with no send path are named, so adding one is a decision
        // rather than something that shows up as a dead branch.
        //
        // This list held six. Five of them — Zcash, Bitcoin Gold, Decred,
        // Kaspa and Dash — had complete send implementations under
        // `send/chains/` and arms in `service/send_execution.rs`; what they
        // did not have was a row in `route_send_asset`, so the preflight
        // refused before any of it ran. The list was recording the symptom,
        // not a decision.
        //
        // Bittensor is genuinely still out: its `execute_send` arm takes no
        // fee parameter, it has no shared-path preview, and the generic submit
        // needs a fee to validate the balance against. Giving it a fallback
        // would mean inventing a TAO fee, which is not this document's to
        // invent.
        assert_eq!(
            unrouted,
            vec!["Bittensor"],
            "the set of chains that cannot send changed"
        );
    }

    #[test]
    fn rejects_zero_amount_for_non_evm_native_sends() {
        let error = plan_send_submit_preflight(SendSubmitPreflightRequest {
            wallet_found: true,
            asset_found: true,
            destination_address: "bc1qdestination".to_string(),
            amount_input: "0".to_string(),
            available_balance: 1.0,
            asset: Some(SendAssetRoutingInput {
                chain_name: "Bitcoin".to_string(),
                symbol: "BTC".to_string(),
                is_evm_chain: false,
                supports_solana_send_coin: false,
                supports_near_token_send: false,
            }),
        })
        .expect_err("bitcoin zero-value sends should be rejected in preflight");

        assert_eq!(error, "Enter a valid amount");
    }

    #[test]
    fn preserves_zero_amount_for_native_evm_preflight() {
        let plan = plan_send_submit_preflight(SendSubmitPreflightRequest {
            wallet_found: true,
            asset_found: true,
            destination_address: "0xabc".to_string(),
            amount_input: "0".to_string(),
            available_balance: 1.0,
            asset: Some(SendAssetRoutingInput {
                chain_name: "Ethereum".to_string(),
                symbol: "ETH".to_string(),
                is_evm_chain: true,
                supports_solana_send_coin: false,
                supports_near_token_send: false,
            }),
        })
        .expect("native EVM zero-value sends remain allowed");

        assert_eq!(plan.submit_kind, "ethereum");
        assert_eq!(plan.amount, 0.0);
        assert!(plan.allows_zero_amount);
    }

    #[test]
    fn send_execution_request_scrubs_secret_fields() {
        let mut request = SendExecutionRequest {
            chain_id: "ethereum".to_string(),
            chain_name: "Ethereum".to_string(),
            derivation_path: "m/44'/60'/0'/0/0".to_string(),
            seed_phrase: Some("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".to_string()),
            private_key_hex: Some("0123456789abcdef".repeat(4)),
            from_address: "0xfrom".to_string(),
            to_address: "0xto".to_string(),
            amount: 1.0,
            amount_str: Some("1".to_string()),
            contract_address: None,
            token_decimals: None,
            fee_rate_svb: None,
            fee_sat: None,
            gas_budget: None,
            fee_amount: None,
            evm_overrides: None,
            monero_priority: None,
            derivation_overrides: Some(
                crate::store::wallet_domain::CoreWalletDerivationOverrides {
                    passphrase: Some("wallet passphrase".to_string()),
                    hmac_key: Some("custom hmac".to_string()),
                    salt_prefix: Some("mnemonic".to_string()),
                    ..Default::default()
                },
            ),
        };

        request.zeroize_sensitive_fields();

        assert_eq!(request.seed_phrase.as_deref(), Some(""));
        assert_eq!(request.private_key_hex.as_deref(), Some(""));
        let overrides = request.derivation_overrides.as_ref().expect("overrides");
        assert_eq!(overrides.passphrase.as_deref(), Some(""));
        assert_eq!(overrides.hmac_key.as_deref(), Some(""));
        assert_eq!(overrides.salt_prefix.as_deref(), Some(""));
    }
}

// ── FFI surface ─────────────────────────────────────────────────────────────





#[cfg(test)]
mod every_chain_with_a_send_implementation_can_route {
    /// A chain `execute_send` can broadcast is a chain the preflight routes.
    ///
    /// `route_send_asset` is a table of `(chain, symbol)` pairs, and five
    /// chains with complete send implementations were missing from it: Zcash,
    /// Bitcoin Gold, Decred, Kaspa and Dash. Each has a module under
    /// `send/chains/`, an arm in `service/send_execution.rs`, and derivation —
    /// and the preflight answered "transfers are not enabled yet" before any
    /// of it was reached.
    #[test]
    fn the_five_that_were_unroutable_now_route() {
        for (name, symbol, kind) in [
            ("Zcash", "ZEC", "zcash"),
            ("Bitcoin Gold", "BTG", "bitcoin-gold"),
            ("Decred", "DCR", "decred"),
            ("Kaspa", "KAS", "kaspa"),
            ("Dash", "DASH", "dash"),
        ] {
            let route = super::route_send_asset(&super::SendAssetRoutingInput {
                chain_name: name.to_string(),
                symbol: symbol.to_string(),
                is_evm_chain: false,
                supports_solana_send_coin: false,
                supports_near_token_send: false,
            });
            assert_eq!(route.submit_kind.as_deref(), Some(kind), "{name}");

            let chain = crate::registry::Chain::from_display_name(name).unwrap();
            assert!(chain.uses_generic_send_submit(), "{name} must take the shared submit path");
            // Without a fallback the generic submit refuses for want of a fee
            // estimate, and none of these has a shared-path preview.
            assert!(
                chain.send_execution_shape().fee_fallback > 0.0,
                "{name} needs a fee fallback"
            );
            assert!(chain.simple_preview_chain().is_none(), "{name}");
        }
    }
}


#[cfg(test)]
mod token_decimals_are_not_assumed {
    /// No chain's tokens all share one decimal count.
    ///
    /// Tron's send arm hardcoded six, which is USDT's and not BTT's, TUSD's,
    /// USD1's or USDD's — all eighteen. It never fired because
    /// `route_send_asset` lets only TRX and USDT reach it, so the guard against
    /// a 10^12 error was an unrelated restriction two files away.
    #[test]
    fn a_chain_can_host_tokens_of_different_decimals() {
        use std::collections::{HashMap, HashSet};
        let mut by_chain: HashMap<String, HashSet<u32>> = HashMap::new();
        for token in crate::tokens::list_tokens(String::new()) {
            by_chain
                .entry(token.chain.clone())
                .or_default()
                .insert(token.decimals);
        }
        let mixed: Vec<_> = by_chain
            .iter()
            .filter(|(_, d)| d.len() > 1)
            .map(|(c, d)| {
                let mut v: Vec<_> = d.iter().copied().collect();
                v.sort_unstable();
                (c.clone(), v)
            })
            .collect();
        assert!(
            !mixed.is_empty(),
            "if this ever holds, the assumption a caller could make is at least true"
        );
        let tron = by_chain.get("tron").expect("tron hosts tokens");
        assert!(
            tron.len() > 1,
            "tron's tokens are all {tron:?} decimals — the hardcoded 6 would have been harmless"
        );
    }
}
