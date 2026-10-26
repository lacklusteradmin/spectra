//! Cardano: building, revalidating, signing and expiring its staged
//! transactions.
use super::send_stage_protocols::StagedAmount;
use super::*;
use crate::api::error::ApiError;
use crate::send::payload::SignedPayload;
use crate::send::stages::*;

impl WalletService {
    pub(super) async fn prepare_cardano(
        &self,
        chain: Chain,
        request: &mut crate::send::SendExecutionRequest,
        sender: &str,
        staged: StagedAmount,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        let StagedAmount {
            eps, amount_u64, ..
        } = staged;
        let to = request.to_address.as_str();
        Ok(match chain.mainnet_counterpart() {
            Chain::Cardano => {
                let client = KoiosClient::new(eps);
                let transfer = match request.contract_address.as_deref() {
                    Some(asset) => crate::send::cardano::CardanoTransfer::Asset {
                        asset: crate::api::cardano_asset::CardanoAssetId::parse(asset)?,
                        quantity: amount_u64,
                    },
                    None => crate::send::cardano::CardanoTransfer::Ada(amount_u64),
                };
                let utxos = self.cardano_inputs(chain, sender).await?;
                PreparedPayload::Cardano(crate::send::cardano::PreparedCardanoTransaction::plan(
                    &utxos,
                    &client.fetch_protocol_params().await?,
                    sender,
                    to,
                    &transfer,
                    client
                        .fetch_latest_slot()
                        .await?
                        .checked_add(7200)
                        .ok_or_else(|| SpectraBridgeError::failure("Slot overflow"))?,
                    None,
                )?)
            }
            _ => {
                return Err(SpectraBridgeError::failure("Not a Cardano network"));
            }
        })
    }

    pub(super) async fn sign_staged_cardano(
        &self,
        chain: Chain,
        stored: &StoredSend,
        signer: &super::send_identity::ResolvedSendIdentity,
    ) -> Result<(SignedPayload, Option<String>), SpectraBridgeError> {
        let eps = self
            .endpoints_for(chain, &[EndpointCapability::Verification])
            .await;
        Ok(match &stored.prepared {
            PreparedPayload::Cardano(prepared) => {
                let client = KoiosClient::new(eps);
                if client.fetch_latest_slot().await? >= prepared.ttl {
                    return Err(SpectraBridgeError::failure(
                        "Cardano transaction expired; build and review again",
                    ));
                }
                // Every input still unspent with the value it was reviewed
                // with, and the fee and outputs still what the protocol asks.
                let current = self.cardano_inputs(chain, &stored.view.sender).await?;
                for input in &prepared.inputs {
                    if !current.contains(input) {
                        return Err(SpectraBridgeError::failure(
                            "Cardano input changed; build and review again",
                        ));
                    }
                }
                prepared
                    .clone()
                    .within_limits(&client.fetch_protocol_params().await?)?;
                let bytes = zeroize::Zeroizing::new(hex::decode(signer.private_key_hex.as_str())?);
                let key: &[u8; 64] = bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| SpectraBridgeError::failure("Invalid Cardano key"))?;
                let public = decode_hex_array::<32>(
                    signer
                        .public_key_hex
                        .as_deref()
                        .ok_or_else(|| SpectraBridgeError::failure("Missing Cardano public key"))?,
                    "Cardano public key",
                )?;
                let raw = prepared.sign(key, &public)?;
                (
                    SignedPayload::Hex(raw),
                    Some(hex::encode(prepared.transaction_hash()?)),
                )
            }
            _ => {
                return Err(SpectraBridgeError::failure("Not a Cardano transaction"));
            }
        })
    }

    /// Whether a signed Cardano transaction can no longer land.
    pub(super) async fn cardano_signature_expired(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<bool, SpectraBridgeError> {
        let endpoints = self
            .endpoints_for(chain, &[EndpointCapability::Verification])
            .await;
        Ok(match &stored.prepared {
            PreparedPayload::Cardano(prepared) => {
                KoiosClient::new(endpoints).fetch_latest_slot().await? >= prepared.ttl
            }
            _ => false,
        })
    }
}

impl WalletService {
    /// Every unspent output of `address`, with the assets each holds.
    pub(super) async fn cardano_inputs(
        &self,
        chain: Chain,
        address: &str,
    ) -> Result<Vec<crate::send::cardano::CardanoInput>, SpectraBridgeError> {
        KoiosClient::new(self.endpoints_for(chain, &[EndpointCapability::Utxo]).await)
            .fetch_utxos(address)
            .await?
            .into_iter()
            .map(|utxo| {
                Ok(crate::send::cardano::CardanoInput {
                    assets: utxo
                        .assets
                        .iter()
                        .map(|asset| {
                            Ok(crate::send::cardano::CardanoAssetAmount {
                                asset: asset.id()?.identifier(),
                                quantity: asset.amount()?,
                            })
                        })
                        .collect::<Result<_, ApiError>>()?,
                    tx_hash: utxo.tx_hash,
                    tx_index: utxo.tx_index,
                    lovelace: utxo.lovelace,
                })
            })
            .collect()
    }
}
