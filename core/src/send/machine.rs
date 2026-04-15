//! `SendStateMachine` — pure-Rust event-driven state machine for the send flow.
//!
//! The machine is exposed to Swift as a UniFFI object. Swift drives it by
//! calling `apply_event(event_json)` which returns a `(new_state_json,
//! [effect_json])` pair. Effects are fire-and-forget instructions Swift must
//! execute (e.g. "fetch fee preview", "submit transaction", "show alert").
//!
//! ## State diagram (simplified)
//!
//! ```text
//! Idle ──[SetAsset]──► AssetSelected ──[SetAddress]──► AddressEntered
//!                                                            │
//!                                              [SetAmount]   │
//!                                                            ▼
//!                                                    AmountEntered
//!                                                            │
//!                                           [RequestFeePreview]
//!                                                            ▼
//!                                                    FetchingFee ──[FeeReady]──► Confirming
//!                                                                                    │
//!                                                                      [Confirm]     │
//!                                                                                    ▼
//!                                                                            Submitting
//!                                                                                │
//!                                                             [TxSuccess/TxError] │
//!                                                                                ▼
//!                                                                          Done / Error
//! ```
//!
//! Any state can transition back to `Idle` via the `Reset` event.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;

// ----------------------------------------------------------------
// State
// ----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SendState {
    Idle,
    AssetSelected {
        chain_id: u32,
        symbol: String,
        contract: Option<String>,
        decimals: u8,
    },
    AddressEntered {
        chain_id: u32,
        symbol: String,
        contract: Option<String>,
        decimals: u8,
        destination: String,
    },
    AmountEntered {
        chain_id: u32,
        symbol: String,
        contract: Option<String>,
        decimals: u8,
        destination: String,
        amount_display: String,
    },
    FetchingFee {
        chain_id: u32,
        symbol: String,
        contract: Option<String>,
        decimals: u8,
        destination: String,
        amount_display: String,
    },
    Confirming {
        chain_id: u32,
        symbol: String,
        contract: Option<String>,
        decimals: u8,
        destination: String,
        amount_display: String,
        fee_display: String,
        fee_raw: String,
    },
    Submitting {
        chain_id: u32,
        symbol: String,
        destination: String,
        amount_display: String,
    },
    Success {
        txid: String,
        chain_id: u32,
        symbol: String,
        destination: String,
        amount_display: String,
    },
    Error {
        message: String,
        recoverable: bool,
    },
}

// ----------------------------------------------------------------
// Events
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SendEvent {
    /// Select the asset to send.
    SetAsset {
        chain_id: u32,
        symbol: String,
        contract: Option<String>,
        decimals: u8,
    },
    /// Enter a destination address (already validated by Swift).
    SetAddress { destination: String },
    /// Enter an amount (human-readable display string, validated by Swift).
    SetAmount { amount_display: String },
    /// Ask Rust to drive a fee fetch (Swift must execute the `FetchFee` effect).
    RequestFeePreview,
    /// Fee fetch completed successfully.
    FeeReady { fee_display: String, fee_raw: String },
    /// Fee fetch failed.
    FeeFailed { reason: String },
    /// User confirmed — Swift must now execute the `SubmitTransaction` effect.
    Confirm,
    /// Transaction submitted successfully.
    TxSuccess { txid: String },
    /// Transaction submission failed.
    TxError { reason: String },
    /// Reset to `Idle` from any state.
    Reset,
}

// ----------------------------------------------------------------
// Effects (instructions back to Swift)
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SendEffect {
    /// Swift should call `WalletService.fetch_fee_preview` and then apply
    /// `FeeReady` or `FeeFailed`.
    FetchFeePreview {
        chain_id: u32,
        symbol: String,
        contract: Option<String>,
        destination: String,
        amount_display: String,
    },
    /// Swift should call `WalletService.sign_and_send` (or equivalent) and
    /// then apply `TxSuccess` or `TxError`.
    SubmitTransaction {
        chain_id: u32,
        symbol: String,
        contract: Option<String>,
        destination: String,
        amount_display: String,
        fee_raw: String,
    },
    /// Display an error to the user (non-recoverable send path).
    ShowError { message: String },
    /// Navigation: dismiss the send sheet (send completed).
    Dismiss,
}

// ----------------------------------------------------------------
// Transition engine
// ----------------------------------------------------------------

fn transition(state: &SendState, event: SendEvent) -> (SendState, Vec<SendEffect>) {
    match (state, event) {
        // Reset from anywhere.
        (_, SendEvent::Reset) => (SendState::Idle, vec![]),

        // Select asset.
        (SendState::Idle, SendEvent::SetAsset { chain_id, symbol, contract, decimals })
        | (SendState::AssetSelected { .. }, SendEvent::SetAsset { chain_id, symbol, contract, decimals })
        | (SendState::Error { .. }, SendEvent::SetAsset { chain_id, symbol, contract, decimals }) => (
            SendState::AssetSelected { chain_id, symbol, contract, decimals },
            vec![],
        ),

        // Enter address.
        (
            SendState::AssetSelected { chain_id, symbol, contract, decimals },
            SendEvent::SetAddress { destination },
        ) => (
            SendState::AddressEntered {
                chain_id: *chain_id,
                symbol: symbol.clone(),
                contract: contract.clone(),
                decimals: *decimals,
                destination,
            },
            vec![],
        ),
        (
            SendState::AddressEntered { chain_id, symbol, contract, decimals, .. },
            SendEvent::SetAddress { destination },
        ) => (
            SendState::AddressEntered {
                chain_id: *chain_id,
                symbol: symbol.clone(),
                contract: contract.clone(),
                decimals: *decimals,
                destination,
            },
            vec![],
        ),

        // Enter amount.
        (
            SendState::AddressEntered { chain_id, symbol, contract, decimals, destination },
            SendEvent::SetAmount { amount_display },
        )
        | (
            SendState::AmountEntered { chain_id, symbol, contract, decimals, destination, .. },
            SendEvent::SetAmount { amount_display },
        ) => (
            SendState::AmountEntered {
                chain_id: *chain_id,
                symbol: symbol.clone(),
                contract: contract.clone(),
                decimals: *decimals,
                destination: destination.clone(),
                amount_display,
            },
            vec![],
        ),

        // Request fee preview.
        (
            SendState::AmountEntered { chain_id, symbol, contract, decimals, destination, amount_display },
            SendEvent::RequestFeePreview,
        ) => {
            let effect = SendEffect::FetchFeePreview {
                chain_id: *chain_id,
                symbol: symbol.clone(),
                contract: contract.clone(),
                destination: destination.clone(),
                amount_display: amount_display.clone(),
            };
            (
                SendState::FetchingFee {
                    chain_id: *chain_id,
                    symbol: symbol.clone(),
                    contract: contract.clone(),
                    decimals: *decimals,
                    destination: destination.clone(),
                    amount_display: amount_display.clone(),
                },
                vec![effect],
            )
        }

        // Fee ready.
        (
            SendState::FetchingFee { chain_id, symbol, contract, decimals, destination, amount_display },
            SendEvent::FeeReady { fee_display, fee_raw },
        ) => (
            SendState::Confirming {
                chain_id: *chain_id,
                symbol: symbol.clone(),
                contract: contract.clone(),
                decimals: *decimals,
                destination: destination.clone(),
                amount_display: amount_display.clone(),
                fee_display,
                fee_raw,
            },
            vec![],
        ),

        // Fee failed — return to AmountEntered with error.
        (
            SendState::FetchingFee { chain_id, symbol, contract, decimals, destination, amount_display },
            SendEvent::FeeFailed { reason },
        ) => (
            SendState::AmountEntered {
                chain_id: *chain_id,
                symbol: symbol.clone(),
                contract: contract.clone(),
                decimals: *decimals,
                destination: destination.clone(),
                amount_display: amount_display.clone(),
            },
            vec![SendEffect::ShowError { message: format!("Fee estimate failed: {reason}") }],
        ),

        // Confirm — kick off submit.
        (
            SendState::Confirming { chain_id, symbol, contract, destination, amount_display, fee_raw, .. },
            SendEvent::Confirm,
        ) => {
            let effect = SendEffect::SubmitTransaction {
                chain_id: *chain_id,
                symbol: symbol.clone(),
                contract: contract.clone(),
                destination: destination.clone(),
                amount_display: amount_display.clone(),
                fee_raw: fee_raw.clone(),
            };
            (
                SendState::Submitting {
                    chain_id: *chain_id,
                    symbol: symbol.clone(),
                    destination: destination.clone(),
                    amount_display: amount_display.clone(),
                },
                vec![effect],
            )
        }

        // Transaction submitted successfully.
        (
            SendState::Submitting { chain_id, symbol, destination, amount_display },
            SendEvent::TxSuccess { txid },
        ) => (
            SendState::Success {
                txid,
                chain_id: *chain_id,
                symbol: symbol.clone(),
                destination: destination.clone(),
                amount_display: amount_display.clone(),
            },
            vec![SendEffect::Dismiss],
        ),

        // Transaction failed.
        (SendState::Submitting { .. }, SendEvent::TxError { reason }) => (
            SendState::Error { message: reason, recoverable: true },
            vec![],
        ),

        // Unhandled combination — no-op.
        (state, _) => (state.clone(), vec![]),
    }
}

// ----------------------------------------------------------------
// UniFFI-exported object
// ----------------------------------------------------------------

/// Drives the send-flow state machine. Swift holds one per active send sheet.
#[derive(uniffi::Object)]
pub struct SendStateMachine {
    state: Mutex<SendState>,
}

#[uniffi::export]
impl SendStateMachine {
    #[uniffi::constructor]
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            state: Mutex::new(SendState::Idle),
        })
    }

    /// Apply `event_json` (a serialized `SendEvent`) and return JSON:
    /// `{"state": <new_state>, "effects": [<effect>, ...]}`.
    pub fn apply_event(&self, event_json: String) -> Result<String, crate::SpectraBridgeError> {
        let event: SendEvent = serde_json::from_str(&event_json)
            .map_err(|e| crate::SpectraBridgeError::from(format!("bad event json: {e}")))?;

        let new_state;
        let effects;
        {
            let guard = self.state.lock().map_err(|_| crate::SpectraBridgeError::from("state lock poisoned"))?;
            (new_state, effects) = transition(&guard, event);
        }
        {
            let mut guard = self.state.lock().map_err(|_| crate::SpectraBridgeError::from("state lock poisoned"))?;
            *guard = new_state.clone();
        }

        Ok(serde_json::json!({
            "state": new_state,
            "effects": effects,
        }).to_string())
    }

    /// Return the current state as JSON.
    pub fn current_state_json(&self) -> Result<String, crate::SpectraBridgeError> {
        let guard = self.state.lock().map_err(|_| crate::SpectraBridgeError::from("state lock poisoned"))?;
        serde_json::to_string(&*guard).map_err(crate::SpectraBridgeError::from)
    }

    /// Reset to `Idle` unconditionally.
    pub fn reset(&self) {
        if let Ok(mut guard) = self.state.lock() {
            *guard = SendState::Idle;
        }
    }
}
