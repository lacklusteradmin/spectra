//! The account protocols' staged sends: which protocol prepares, revalidates,
//! signs and expires a transaction. Each protocol's own rules are in its
//! `send_stage_<protocol>` sibling (NEAR's in `send_near`); the UTXO family's
//! are in `send_stage_utxo`, `send_stage_litecoin` and `send_stage_peercoin`.
use super::*;
use crate::api::endpoint::EndpointApi;
use crate::api::error::ApiError;
use crate::send::payload::{PreparedSubmission, SignedPayload};
use crate::send::payment_memo::PaymentMemo;
use crate::send::stages::*;

/// What preparation reads before a protocol builds: the endpoints that
/// verify the network, and the amount at the asset's precision. Each protocol
/// narrows `amount` to its own wire type: [`protocol_u64`] where that is one
/// `u64` of smallest units, while TRC-20 and jettons carry wider words.
pub(super) struct StagedAmount {
    pub eps: Arc<Vec<String>>,
    pub decimals: u32,
    pub amount: u128,
}

/// `amount` as the one `u64` of smallest units a protocol's transfer carries,
/// refused where it does not fit.
pub(super) fn protocol_u64(amount: u128) -> Result<u64, SpectraBridgeError> {
    u64::try_from(amount).map_err(|_| SpectraBridgeError::failure("Amount exceeds protocol range"))
}

impl WalletService {
    /// Verify the persisted transaction against current chain state without
    /// substituting new fields. Used before signing and before its first send.
    pub(super) async fn validate_prepared_protocol_state(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        match &stored.prepared {
            PreparedPayload::Xrp { .. }
            | PreparedPayload::XrpIssuedPayment(_)
            | PreparedPayload::XrpTrustSet(_)
            | PreparedPayload::XrpAccountDelete { .. } => {
                self.validate_xrp_state(chain, stored).await?
            }
            PreparedPayload::Stellar { .. }
            | PreparedPayload::StellarAssetPayment(_)
            | PreparedPayload::StellarChangeTrust(_)
            | PreparedPayload::StellarAccountMerge { .. } => {
                self.validate_stellar_state(chain, stored).await?
            }
            PreparedPayload::Evm(_) => self.validate_evm_state(chain, stored).await?,
            PreparedPayload::Ton { .. } => self.validate_ton_state(chain, stored).await?,
            PreparedPayload::Tron(_) => self.validate_tron_state(chain, stored).await?,
            PreparedPayload::Aptos(_) => self.validate_aptos_state(chain, stored).await?,
            PreparedPayload::Cosmos(_) => self.validate_cosmos_state(chain, stored).await?,
            PreparedPayload::Algorand(_) => self.validate_algorand_state(chain, stored).await?,
            PreparedPayload::Vechain(_) => self.validate_vechain_state(chain, stored).await?,
            PreparedPayload::Filecoin(_) => self.validate_filecoin_state(chain, stored).await?,
            PreparedPayload::Tezos(_) => self.validate_tezos_state(chain, stored).await?,
            PreparedPayload::Starknet(_) => self.validate_starknet_state(chain, stored).await?,
            PreparedPayload::Hedera(_) => self.validate_hedera_state(chain, stored).await?,
            PreparedPayload::Sui(_) | PreparedPayload::SuiMerge(_) => {
                self.validate_sui_state(chain, stored).await?
            }
            PreparedPayload::Solana(_) | PreparedPayload::SolanaAccountClosure(_) => {
                self.validate_solana_state(chain, stored).await?
            }
            PreparedPayload::Near { .. }
            | PreparedPayload::NearFunctionCall(_)
            | PreparedPayload::NearDeleteKey(_) => self.validate_near_state(stored).await?,
            _ => {}
        }
        Ok(())
    }

    /// Refuse a payment with no destination tag or memo to an account that
    /// asks for one — XRP's `lsfRequireDestTag`, Stellar's SEP-29
    /// `config.memo_required` — read from a verified node. An exchange
    /// cannot tell whose deposit such a payment is, and the network takes
    /// it all the same.
    pub(super) async fn require_payment_memo(
        &self,
        chain: Chain,
        destination: &str,
        memo: Option<&PaymentMemo>,
    ) -> Result<(), SpectraBridgeError> {
        if memo.is_some() || chain.payment_memo_kinds().is_empty() {
            return Ok(());
        }
        let endpoints = self
            .endpoints_for(chain, &[EndpointCapability::Verification])
            .await;
        let refusal = match chain.mainnet_counterpart() {
            Chain::Xrp
                if XrplClient::new(endpoints.clone())
                    .requires_destination_tag(chain, destination)
                    .await? =>
            {
                "%@ requires a destination tag. Add the one the recipient gave you."
            }
            Chain::Stellar
                if HorizonClient::new(endpoints)
                    .fetch_memo_required(chain, destination)
                    .await? =>
            {
                "%@ requires a memo. Add the one the recipient gave you."
            }
            _ => return Ok(()),
        };
        Err(SpectraBridgeError::refused(refusal, [destination]))
    }

    pub(super) async fn prepare_staged_protocol(
        &self,
        chain: Chain,
        request: &mut crate::send::SendExecutionRequest,
        sender: &str,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        let eps = self
            .endpoints_for(chain, &[EndpointCapability::Verification])
            .await;
        let decimals = if let Some(contract) = &request.contract_address {
            let decimals = u32::from(
                self.token_decimals(chain, contract, MetadataRead::Fresh)
                    .await?,
            );
            if request.token_decimals.is_some_and(|d| d != decimals) {
                return Err(SpectraBridgeError::failure(
                    "Token decimals changed; review again",
                ));
            }
            request.token_decimals = Some(decimals);
            decimals
        } else {
            u32::from(chain.native_decimals())
        };
        let amount = crate::amount::input::parse_raw_amount(&request.amount_str, decimals)?;
        self.require_payment_memo(chain, &request.to_address, request.memo.as_ref())
            .await?;
        let staged = StagedAmount {
            eps,
            decimals,
            amount,
        };
        if chain.is_cosmos() {
            return self.prepare_cosmos(chain, request, sender, staged).await;
        }
        Ok(match chain.mainnet_counterpart() {
            Chain::Xrp => self.prepare_xrp(chain, request, sender, staged).await?,
            Chain::Stellar => self.prepare_stellar(chain, request, sender, staged).await?,
            Chain::Ton => self.prepare_ton(chain, request, sender, staged).await?,
            Chain::Tron => self.prepare_tron(chain, request, sender, staged).await?,
            Chain::Aptos => self.prepare_aptos(chain, request, sender, staged).await?,
            Chain::Algorand => {
                self.prepare_algorand(chain, request, sender, staged)
                    .await?
            }
            Chain::Vechain => self.prepare_vechain(chain, request, sender, staged).await?,
            Chain::Starknet => {
                self.prepare_starknet(chain, request, sender, staged)
                    .await?
            }
            Chain::Sui => self.prepare_sui(chain, request, sender, staged).await?,
            Chain::Solana => self.prepare_solana(chain, request, sender, staged).await?,
            Chain::Cardano => self.prepare_cardano(chain, request, sender, staged).await?,
            _ if chain.is_substrate() => {
                self.prepare_substrate(chain, request, sender, staged)
                    .await?
            }
            Chain::Icp => self.prepare_icp(chain, request, sender, staged).await?,
            Chain::Near => self.prepare_near(chain, request, sender, staged).await?,
            Chain::Filecoin => {
                self.prepare_filecoin(chain, request, sender, staged)
                    .await?
            }
            Chain::Tezos => self.prepare_tezos(chain, request, sender, staged).await?,
            Chain::Hedera => self.prepare_hedera(chain, request, sender, staged).await?,
            Chain::Bitcoin
            | Chain::Dogecoin
            | Chain::BitcoinSV
            | Chain::BitcoinCash
            | Chain::BitcoinGold
            | Chain::Dash
            | Chain::Zcash
            | Chain::Decred
            | Chain::Kaspa
            | Chain::Qtum
            | Chain::FractalBitcoin
            | Chain::Ecash
            | Chain::Digibyte => {
                self.prepare_account_transfer(chain, request, sender, protocol_u64(amount)?)
                    .await?
            }
            Chain::Litecoin => {
                self.prepare_litecoin(chain, request, sender, protocol_u64(amount)?)
                    .await?
            }
            Chain::Peercoin => {
                self.prepare_peercoin(chain, request, sender, protocol_u64(amount)?)
                    .await?
            }
            Chain::Monero => {
                PreparedPayload::Monero(self.prepare_monero(request, protocol_u64(amount)?).await?)
            }
            _ => {
                return Err(SpectraBridgeError::failure(
                    "Transparent preparation unavailable for this protocol",
                ));
            }
        })
    }

    pub(super) async fn sign_staged_protocol(
        &self,
        chain: Chain,
        stored: &StoredSend,
        signer: &super::send_identity::ResolvedSendIdentity,
    ) -> Result<PreparedSubmission, SpectraBridgeError> {
        let (payload, hash) = match &stored.prepared {
            PreparedPayload::Xrp { .. }
            | PreparedPayload::XrpIssuedPayment(_)
            | PreparedPayload::XrpTrustSet(_)
            | PreparedPayload::XrpAccountDelete { .. } => {
                self.sign_staged_xrp(chain, stored, signer).await?
            }
            PreparedPayload::Stellar { .. }
            | PreparedPayload::StellarAssetPayment(_)
            | PreparedPayload::StellarChangeTrust(_)
            | PreparedPayload::StellarAccountMerge { .. } => {
                self.sign_staged_stellar(chain, stored, signer).await?
            }
            PreparedPayload::Evm(_) => {
                return Err(SpectraBridgeError::failure(
                    "EVM signing uses its typed signer",
                ));
            }
            PreparedPayload::Ton { .. } => self.sign_staged_ton(chain, stored, signer).await?,
            PreparedPayload::Tron(_) => self.sign_staged_tron(stored, signer).await?,
            PreparedPayload::Aptos(_) => self.sign_staged_aptos(stored, signer).await?,
            PreparedPayload::Cosmos(_) => self.sign_staged_cosmos(chain, stored, signer).await?,
            PreparedPayload::Algorand(_) => {
                self.sign_staged_algorand(chain, stored, signer).await?
            }
            PreparedPayload::Vechain(_) => self.sign_staged_vechain(chain, stored, signer).await?,
            PreparedPayload::Filecoin(_) => {
                self.sign_staged_filecoin(chain, stored, signer).await?
            }
            PreparedPayload::Tezos(_) => self.sign_staged_tezos(chain, stored, signer).await?,
            PreparedPayload::Starknet(_) => {
                self.sign_staged_starknet(chain, stored, signer).await?
            }
            PreparedPayload::Hedera(_) => self.sign_staged_hedera(chain, stored, signer).await?,
            PreparedPayload::Sui(_) | PreparedPayload::SuiMerge(_) => {
                self.sign_staged_sui(stored, signer).await?
            }
            PreparedPayload::Solana(_) | PreparedPayload::SolanaAccountClosure(_) => {
                self.sign_staged_solana(chain, stored, signer).await?
            }
            PreparedPayload::Cardano(_) => self.sign_staged_cardano(chain, stored, signer).await?,
            PreparedPayload::Substrate(_) => {
                self.sign_staged_substrate(chain, stored, signer).await?
            }
            PreparedPayload::Icp(_) | PreparedPayload::IcpStaking(_) => {
                self.sign_staged_icp(stored, signer).await?
            }
            PreparedPayload::Near { .. }
            | PreparedPayload::NearFunctionCall(_)
            | PreparedPayload::NearDeleteKey(_) => {
                self.sign_staged_near(chain, stored, signer).await?
            }
            // Signed with the shielded account's key, which the seed
            // derives: `sign_send` does that before this is reached.
            PreparedPayload::ZcashShielded(_) | PreparedPayload::LitecoinMweb(_) => {
                return Err(SpectraBridgeError::failure(
                    "A shielded transaction is signed with the shielded account's key",
                ));
            }
            PreparedPayload::Monero(p) => {
                let (raw, hash) = self
                    .sign_monero(p, &stored.view.wallet_id, &signer.private_key_hex)
                    .await?;
                (SignedPayload::Hex(raw), Some(hash))
            }
            PreparedPayload::AccountTransfer(_) => {
                return self.sign_account_transfer(chain, stored, signer).await;
            }
            PreparedPayload::Litecoin(_) => {
                return self.sign_litecoin(chain, stored, signer).await;
            }
            PreparedPayload::LitecoinPegIn(_) => {
                return self.sign_litecoin_pegin(chain, stored, signer).await;
            }
            PreparedPayload::Peercoin(_) => return self.sign_peercoin(chain, stored, signer).await,
        };
        Ok(PreparedSubmission {
            payload,
            transaction_hash: hash,
            nonce: None,
        })
    }
}

impl WalletService {
    pub(super) async fn validate_broadcast_endpoint(
        &self,
        chain: Chain,
        endpoint: &str,
    ) -> Result<(), SpectraBridgeError> {
        let allowed = self
            .endpoints_for(chain, &[EndpointCapability::Broadcast])
            .await;
        if !allowed
            .iter()
            .any(|url| url.trim_end_matches('/') == endpoint.trim_end_matches('/'))
        {
            return Err(SpectraBridgeError::failure(
                "Endpoint does not support broadcasts on the selected network",
            ));
        }
        Ok(self.validate_endpoint_network(chain, endpoint).await?)
    }

    /// Refuse an endpoint that is not on `chain`'s network, asking the node
    /// itself where the protocol lets it say.
    pub(super) async fn validate_endpoint_network(
        &self,
        chain: Chain,
        endpoint: &str,
    ) -> Result<(), ApiError> {
        let api = self.endpoint_api(chain, endpoint).await;
        if crate::api::network::verify_network(chain, api, endpoint).await? {
            return Ok(());
        }
        // A network core cannot ask who it is: only a catalog endpoint, whose
        // network was checked when it was added, is trusted to be it.
        let known = crate::catalog::endpoints::catalog()
            .records
            .iter()
            .any(|r| {
                r.chain_id == chain
                    && chain.endpoint_apis().contains(&r.api)
                    && r.endpoint.trim_end_matches('/') == endpoint.trim_end_matches('/')
            });
        if !known {
            return Err(ApiError::invalid(
                "Endpoint network and broadcast capability cannot be verified",
            ));
        }
        Ok(())
    }
}

impl WalletService {
    /// Recheck expiry at submission time: a saved signature may outlive its block reference.
    /// Never rebuild or resign here, including after a lost response.
    pub(super) async fn validate_signed_expiry(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        let endpoints = self
            .endpoints_for(chain, &[EndpointCapability::Verification])
            .await;
        let expired = match &stored.prepared {
            PreparedPayload::Ton { .. } => self.ton_signature_expired(stored).await?,
            PreparedPayload::Tron(_) => self.tron_signature_expired(stored).await?,
            PreparedPayload::Aptos(_) => self.aptos_signature_expired(stored).await?,
            PreparedPayload::Cosmos(_) => self.cosmos_signature_expired(chain, stored).await?,
            PreparedPayload::Algorand(_) => self.algorand_signature_expired(chain, stored).await?,
            PreparedPayload::Vechain(_) => self.vechain_signature_expired(chain, stored).await?,
            PreparedPayload::Filecoin(_) => self.filecoin_signature_expired(chain, stored).await?,
            PreparedPayload::Tezos(_) => self.tezos_signature_expired(chain, stored).await?,
            PreparedPayload::Starknet(_) => self.starknet_signature_expired(chain, stored).await?,
            PreparedPayload::Hedera(_) => self.hedera_signature_expired(chain, stored).await?,
            PreparedPayload::Solana(_) | PreparedPayload::SolanaAccountClosure(_) => {
                self.solana_signature_expired(chain, stored).await?
            }
            PreparedPayload::Cardano(_) => self.cardano_signature_expired(chain, stored).await?,
            PreparedPayload::Substrate(_) => {
                self.substrate_signature_expired(chain, stored).await?
            }
            PreparedPayload::Icp(_) | PreparedPayload::IcpStaking(_) => {
                self.icp_signature_expired(stored).await?
            }
            PreparedPayload::Near { .. }
            | PreparedPayload::NearFunctionCall(_)
            | PreparedPayload::NearDeleteKey(_) => {
                self.near_signature_expired(chain, stored).await?
            }
            PreparedPayload::AccountTransfer(p) => match p.protocol {
                crate::send::account_utxo::AccountProtocol::Zcash {
                    expiry_height,
                    upgrade,
                } => {
                    let (height, branch) = BlockbookClient::new(endpoints, chain)
                        .zcash_context()
                        .await?;
                    height >= expiry_height || branch != upgrade.consensus_branch_id
                }
                _ => false,
            },
            PreparedPayload::ZcashShielded(_) => {
                let submission = stored
                    .submission
                    .as_ref()
                    .ok_or_else(|| SpectraBridgeError::failure("Missing signed transaction"))?;
                let expiry = super::zcash_shielded::zcash_expiry_height(
                    &chain.zcash_network()?,
                    submission.payload.raw_hex()?,
                )?;
                let lightwalletd = self
                    .api_endpoints(
                        chain,
                        EndpointApi::Lightwalletd,
                        &[EndpointCapability::History],
                    )
                    .await?;
                let tip = crate::api::lightwalletd::LightwalletdClient::new(Arc::new(lightwalletd))
                    .session(chain)
                    .await?
                    .tip;
                expiry != 0 && tip >= expiry
            }
            _ => false,
        };
        if expired {
            return Err(SpectraBridgeError::failure(
                "Signed transaction expired; inspect its on-chain status before building a new transaction",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/send_stage_protocols.rs"]
mod tests;
