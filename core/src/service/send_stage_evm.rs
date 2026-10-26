//! EVM: building, revalidating, signing and expiring its staged
//! transactions.
use super::*;
use crate::send::stages::*;

impl WalletService {
    /// Recheck a prepared EVM transaction against the chain.
    pub(super) async fn validate_evm_state(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        let PreparedPayload::Evm(prepared) = &stored.prepared else {
            return Ok(());
        };
        let client = EvmClient::new(
            self.endpoints_for(chain, &[EndpointCapability::Verification])
                .await,
            chain.evm_chain_id()?,
        );
        let nonce = if stored
            .request
            .evm_overrides
            .as_ref()
            .and_then(|overrides| overrides.nonce)
            .is_some()
        {
            client.fetch_latest_nonce(&stored.view.sender).await?
        } else {
            client.fetch_nonce(&stored.view.sender).await?
        };
        if prepared.chain_id != chain.evm_chain_id()? || nonce > prepared.nonce {
            return Err(SpectraBridgeError::failure(
                "Prepared nonce or network is stale; build and review again",
            ));
        }
        self.validate_evm_fee_budget(chain, prepared).await?;
        self.validate_evm_funds(chain, &stored.view.sender, prepared)
            .await?;
        self.validate_nft_transfer_state(stored).await?;
        Ok(())
    }
}
