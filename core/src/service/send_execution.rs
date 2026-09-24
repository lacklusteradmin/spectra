use super::*;
use zeroize::Zeroizing;

/// Validate before keys or network reads; token precision is checked again
/// after metadata resolution. Native precision belongs to the registry.
pub(super) fn validate_execution_amount(
    chain: Chain,
    request: &crate::send::SendExecutionRequest,
) -> Result<(), SpectraBridgeError> {
    if !chain.is_evm() && request.evm_overrides.is_some() {
        return Err("EVM overrides require an EVM network".into());
    }
    if let Some(overrides) = &request.evm_overrides {
        overrides.resolve(chain)?;
    }
    if request.fee_sat == Some(0) {
        return Err("Fee must be positive".into());
    }
    if chain.mainnet_counterpart() == Chain::Ton {
        crate::derivation::ton::parse_ton_address(&request.to_address)
            .and_then(|a| a.for_network(chain.is_testnet()))
            .map_err(|message| SpectraBridgeError::InvalidInput { message })?;
    }
    if let Some(fee) = request.fee_amount {
        crate::send::payload::fee_units(fee, u32::from(chain.native_decimals()))?;
    }
    if let Some(budget) = request.gas_budget {
        crate::send::payload::fee_units(budget, u32::from(chain.native_decimals()))?;
    }
    if let Some(rate) = request.fee_rate_svb {
        crate::send::payload::fee_units(rate, 8)?;
    }
    let decimals = if request.contract_address.is_none() {
        u32::from(chain.native_decimals())
    } else {
        request
            .amount_str
            .trim()
            .split_once('.')
            .map_or(0, |(_, f)| f.len() as u32)
    };
    let raw = crate::send::amount_input::parse_raw_amount(&request.amount_str, decimals)?;
    if raw == 0 && (!chain.is_evm() || request.contract_address.is_some()) {
        return Err(SpectraBridgeError::InvalidInput {
            message: "amount must be positive".into(),
        });
    }
    Ok(())
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Derive key material, build the chain-specific payload, sign, and
    /// broadcast in a single call.
    ///
    /// The caller selects a stored wallet. Core resolves and validates its
    /// signing identity; callers cannot supply a competing chain name or key.
    pub async fn execute_send(
        &self,
        request: crate::send::SendExecutionRequest,
    ) -> Result<crate::send::SendExecutionResult, SpectraBridgeError> {
        let service = self.clone();
        // The submission and its record finish even if the UI task is cancelled.
        tokio::spawn(async move { service.execute_send_owned(request, None).await })
            .await
            .map_err(|e| SpectraBridgeError::from(e.to_string()))?
    }
}
impl WalletService {
    pub(super) async fn execute_confirmed_send(
        &self,
        request: crate::send::SendExecutionRequest,
        sender: String,
        automatic_nonce: bool,
    ) -> Result<crate::send::SendExecutionResult, SpectraBridgeError> {
        let service = self.clone();
        tokio::spawn(async move {
            service
                .execute_send_owned(request, Some((sender, automatic_nonce)))
                .await
        })
        .await
        .map_err(|e| SpectraBridgeError::from(e.to_string()))?
    }

    async fn execute_send_owned(
        &self,
        mut request: crate::send::SendExecutionRequest,
        confirmation: Option<(String, bool)>,
    ) -> Result<crate::send::SendExecutionResult, SpectraBridgeError> {
        // Compose the very same durable operations exposed to CLI and Swift.
        let _execution = self.send_execute_lock.lock().await;
        let sign_only = request.wants_sign_only();
        let password = request.password.take().map(Zeroizing::new);
        let chain = chain_for_id(&request.chain_id)?;
        validate_execution_amount(chain, &request)?;
        if let Some(overrides) = &request.evm_overrides {
            overrides.resolve(chain)?;
        }
        if let Some((sender, automatic_nonce)) = confirmation {
            let state = self.app_state().await;
            let current = state
                .wallets
                .iter()
                .find(|w| w.id == request.wallet_id)
                .and_then(|w| w.address_on(chain))
                .ok_or("Sending wallet removed")?;
            if crate::send::flow::normalize_address(chain.str_id(), current)
                != crate::send::flow::normalize_address(chain.str_id(), &sender)
            {
                return Err("Sending identity changed; review again".into());
            }
            if chain.is_evm()
                && automatic_nonce
                && request
                    .evm_overrides
                    .as_ref()
                    .and_then(|o| o.nonce)
                    .and_then(|n| u64::try_from(n).ok())
                    != Some(self.next_send_nonce(chain, &sender).await?)
            {
                return Err("Send nonce changed; review again".into());
            }
        }
        let prepared = self.build_send(request).await?;
        let signed = self
            .sign_send(
                prepared.id.clone(),
                prepared.review_digest,
                password.as_ref().map(|p| p.to_string()),
            )
            .await?;
        let stored = self.load_send_artifact(signed.id.clone()).await?;
        let submission = stored.submission.ok_or("Signed content missing")?;
        let completed = if sign_only {
            signed
        } else {
            let endpoints = self.send_endpoints(chain.str_id().into()).await?;
            let completed = self.broadcast_send(signed.id, endpoints).await?;
            if !completed
                .attempts
                .iter()
                .any(|a| a.outcome == crate::send::stages::SubmissionOutcome::Accepted)
            {
                return Err(format!(
                    "Submission not accepted or uncertain; inspect transaction {} before retrying",
                    completed.id
                )
                .into());
            }
            completed
        };
        let hash = completed
            .attempts
            .iter()
            .find_map(|a| a.transaction_hash.clone())
            .or(completed.transaction_hash.clone())
            .unwrap_or_default();
        let evm = match stored.prepared {
            crate::send::stages::PreparedPayload::Evm(p) => {
                Some(crate::send::ethereum::EvmSendDetails {
                    txid: hash.clone(),
                    raw_tx_hex: submission.payload.clone(),
                    nonce: i64::try_from(p.nonce).map_err(|_| "Nonce exceeds supported range")?,
                    gas_limit: i64::try_from(p.gas_limit)
                        .map_err(|_| "Gas limit exceeds supported range")?,
                })
            }
            _ => None,
        };
        Ok(crate::send::SendExecutionResult {
            protocol_result_json: serde_json::to_string(&completed)?,
            transaction_hash: hash,
            payload_format: if chain.is_evm() {
                "evm.raw_hex".into()
            } else {
                "core.submission_json".into()
            },
            signed_payload: sign_only.then(|| {
                if chain.is_evm() {
                    submission.payload.clone()
                } else {
                    serde_json::to_string(&submission).expect("submission is serializable")
                }
            }),
            evm,
        })
    }
}

/// The chain a send is signed for: the network the wallet is on, within the
/// family the request named.
///
/// A wallet with no record of its own follows the app's selection. A request
/// naming a different family — which `resolve_send_identity` refuses anyway —
/// keeps the requested chain, so this cannot move a send onto another chain.
pub(crate) fn send_chain_for(
    state: &crate::store::state::CoreAppState,
    wallet_id: &str,
    requested: Chain,
) -> Result<Chain, SpectraBridgeError> {
    let wallet = state
        .wallets
        .iter()
        .find(|w| w.id.eq_ignore_ascii_case(wallet_id))
        .ok_or_else(|| SpectraBridgeError::InvalidInput {
            message: "send wallet does not exist".into(),
        })?;
    let selected = wallet
        .chain()
        .ok_or("wallet has an invalid network identity")?;
    if selected.mainnet_counterpart() == requested.mainnet_counterpart() && selected != requested {
        return Err(
            "selected asset network differs from wallet network; select an asset on that network"
                .into(),
        );
    }
    Ok(requested)
}

impl WalletService {
    /// None means no metadata reader for this family; provider failures are errors.
    pub(super) async fn token_contract_decimals(
        &self,
        chain: Chain,
        contract: &str,
    ) -> Result<Option<u32>, SpectraBridgeError> {
        let endpoints = self.endpoints_for(chain.str_id(), &["token-balance"]).await;
        if chain.is_evm() {
            let client = crate::fetch::evm::EvmClient::new(endpoints, chain.evm_chain_id()?);
            return Ok(Some(u32::from(
                client.fetch_erc20_metadata(contract).await?.decimals,
            )));
        }
        if chain.mainnet_counterpart() == Chain::Tron {
            let client = crate::fetch::tron::TronClient::new(endpoints);
            return Ok(Some(u32::from(
                client.fetch_trc20_metadata(contract).await?.decimals,
            )));
        }
        if chain.mainnet_counterpart() == Chain::Solana {
            let client = crate::fetch::solana::SolanaClient::new(endpoints);
            return Ok(Some(u32::from(
                client.fetch_transfer_mint(contract).await?.1,
            )));
        }
        if chain.mainnet_counterpart() == Chain::Near {
            let client = crate::fetch::near::NearClient::new(endpoints);
            return Ok(Some(u32::from(
                client.fetch_ft_metadata(contract).await?.decimals,
            )));
        }
        Ok(None)
    }
}

#[cfg(test)]
#[path = "send_execution_audit_tests.rs"]
mod audit_execution_tests;
#[cfg(test)]
#[path = "send_execution_tests.rs"]
mod tests;
