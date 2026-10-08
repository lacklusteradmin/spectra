//! A transparent transfer spent from a wallet account's addresses, as it is
//! reviewed: each input with the source address whose key signs it, the
//! exact outputs, the fee, and the facts its network's signer needs. Signing
//! changes nothing of it: the keys are each input's own, checked against the
//! script the input pays.

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::stages::UtxoPreparedInput;
use super::zcash::ZcashNetworkUpgrade;
use crate::registry::Chain;
use crate::send::error::SendError;

/// How a network serializes and signs an account transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum AccountProtocol {
    /// Bitcoin: version 2, every input signalling replace-by-fee, P2PKH,
    /// nested and native P2WPKH and Taproot key-path inputs.
    Bitcoin,
    /// Version-1 P2PKH under the legacy hash (Dogecoin, Dash) or, with a
    /// fork id, BIP143 with SIGHASH_FORKID (Bitcoin Cash, Bitcoin SV, Bitcoin
    /// Gold).
    LegacyP2pkh { fork_id: Option<u32> },
    /// Zcash's transparent V5 transaction, mined below `expiry_height` under
    /// the network upgrade it was reviewed in.
    Zcash {
        expiry_height: u32,
        upgrade: ZcashNetworkUpgrade,
    },
    /// Decred's regular-tree transaction.
    Decred,
    /// Kaspa's version-0 transaction of Schnorr P2PK inputs.
    Kaspa,
}

impl AccountProtocol {
    /// The protocol `chain`'s account transfers use, with what is fixed for
    /// the network; Zcash's expiry and upgrade come from the network at build.
    pub(crate) fn for_chain(chain: Chain) -> Result<Self, SendError> {
        Ok(match chain.mainnet_counterpart() {
            Chain::Bitcoin => Self::Bitcoin,
            Chain::Dogecoin | Chain::Dash => Self::LegacyP2pkh { fork_id: None },
            Chain::BitcoinCash | Chain::BitcoinSV | Chain::BitcoinGold => Self::LegacyP2pkh {
                fork_id: Some(chain.sighash_fork_id()?),
            },
            Chain::Decred => Self::Decred,
            Chain::Kaspa => Self::Kaspa,
            _ => {
                return Err(SendError::Invalid(
                    "This network has no account transfer protocol".into(),
                ));
            }
        })
    }
}

/// A reviewed transfer: the recipient's output first, then any change back
/// to the wallet's own address.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PreparedAccountTransfer {
    pub inputs: Vec<UtxoPreparedInput>,
    pub outputs: Vec<(Vec<u8>, u64)>,
    pub fee: u64,
    pub protocol: AccountProtocol,
}

/// What signing produced: the payload broadcast takes, and the transaction
/// hash where it is known before the network says.
pub(crate) struct SignedAccountTransfer {
    pub payload: String,
    pub transaction_hash: Option<String>,
}

impl PreparedAccountTransfer {
    /// The transfer's own arithmetic: inputs on distinct outpoints, each
    /// paying its source's script, and the inputs' total exactly the outputs'
    /// plus the fee.
    pub(crate) fn validate(&self) -> Result<(), SendError> {
        if self.inputs.is_empty() || self.outputs.is_empty() {
            return Err(SendError::Invalid(
                "transfer must have inputs and outputs".into(),
            ));
        }
        let mut outpoints = std::collections::HashSet::new();
        for input in &self.inputs {
            if input.utxo.3 != input.source.script_pubkey
                || !outpoints.insert((&input.utxo.0, input.utxo.1))
            {
                return Err(SendError::Invalid("Invalid prepared input".into()));
            }
        }
        let total_in = self
            .inputs
            .iter()
            .try_fold(0u64, |sum, input| sum.checked_add(input.utxo.2));
        let total_out = self
            .outputs
            .iter()
            .try_fold(self.fee, |sum, (_, value)| sum.checked_add(*value));
        if total_in.is_none() || total_in != total_out {
            return Err(SendError::Invalid(
                "Inputs do not pay the outputs and fee".into(),
            ));
        }
        Ok(())
    }

    /// Sign with `keys`, one per input in order.
    pub(crate) fn sign(
        &self,
        keys: &[Zeroizing<Vec<u8>>],
    ) -> Result<SignedAccountTransfer, SendError> {
        self.validate()?;
        if keys.len() != self.inputs.len() {
            return Err(SendError::Invalid("one key per input".into()));
        }
        let utxos = self.inputs.iter().map(|input| &input.utxo);
        let raw_hex = |raw: Vec<u8>| {
            let payload = hex::encode(raw);
            SignedAccountTransfer {
                transaction_hash: crate::send::payload::bitcoin_transaction_id(&payload),
                payload,
            }
        };
        Ok(match self.protocol {
            AccountProtocol::Bitcoin => raw_hex(super::bitcoin::sign_inputs(
                &utxos
                    .zip(keys)
                    .map(|(utxo, key)| super::bitcoin::SigningInput {
                        utxo,
                        private_key: key,
                    })
                    .collect::<Vec<_>>(),
                &self.outputs,
                bitcoin::transaction::Version::TWO,
                bitcoin::Sequence::ENABLE_RBF_NO_LOCKTIME,
            )?),
            AccountProtocol::LegacyP2pkh { fork_id } => raw_hex(super::legacy_p2pkh::sign(
                &utxos
                    .zip(keys)
                    .map(|(utxo, key)| super::legacy_p2pkh::LegacyInput {
                        utxo,
                        private_key: key,
                    })
                    .collect::<Vec<_>>(),
                &self.outputs,
                fork_id,
            )?),
            AccountProtocol::Zcash {
                expiry_height,
                upgrade,
            } => {
                let utxos: Vec<_> = utxos.cloned().collect();
                let keys: Vec<&[u8]> = keys.iter().map(|key| key.as_slice()).collect();
                let (raw, txid) = super::zcash::sign_transaction(
                    &utxos,
                    &keys,
                    &self.outputs,
                    expiry_height,
                    upgrade,
                )?;
                SignedAccountTransfer {
                    payload: hex::encode(raw),
                    transaction_hash: Some(txid),
                }
            }
            AccountProtocol::Decred => SignedAccountTransfer {
                payload: hex::encode(super::decred::sign(
                    &utxos
                        .zip(keys)
                        .map(|(utxo, key)| super::decred::DecredSigningInput {
                            utxo,
                            private_key: key,
                        })
                        .collect::<Vec<_>>(),
                    &self.outputs,
                )?),
                transaction_hash: None,
            },
            AccountProtocol::Kaspa => SignedAccountTransfer {
                payload: super::kaspa::sign(
                    &utxos
                        .zip(keys)
                        .map(|(utxo, key)| super::kaspa::KaspaSigningInput {
                            utxo,
                            private_key: key,
                        })
                        .collect::<Vec<_>>(),
                    &self.outputs,
                )?
                .to_string(),
                transaction_hash: None,
            },
        })
    }
}

/// The script an address of a wallet's own account pays, as the account's
/// signer spends it: a key's P2PKH everywhere, its nested and native P2WPKH
/// where the network has SegWit, its Taproot key path on Bitcoin and
/// Peercoin, and a Schnorr P2PK on Kaspa. Any other script is refused.
pub(crate) fn source_script(chain: Chain, address: &str) -> Result<Vec<u8>, SendError> {
    use crate::derivation::utxo_address::{ParsedUtxoAddress, parse_utxo_address};
    match chain.mainnet_counterpart() {
        Chain::Zcash => {
            let script = super::zcash_stages::address_script(address, chain)?;
            if script.len() != 25 {
                return Err(SendError::Invalid("Zcash sender must be P2PKH".into()));
            }
            Ok(script)
        }
        Chain::Decred => super::decred::sender_script(chain, address),
        Chain::Kaspa => super::kaspa::sender_script(chain, address),
        family => {
            let parsed = parse_utxo_address(chain, address)?;
            let segwit = matches!(family, Chain::Bitcoin | Chain::Litecoin | Chain::Peercoin);
            let supported = match &parsed {
                ParsedUtxoAddress::P2pkh(_) => true,
                ParsedUtxoAddress::P2sh(_) => segwit,
                ParsedUtxoAddress::Witness {
                    version: 0,
                    program,
                } => segwit && program.len() == 20,
                ParsedUtxoAddress::Witness {
                    version: 1,
                    program,
                } => matches!(family, Chain::Bitcoin | Chain::Peercoin) && program.len() == 32,
                _ => false,
            };
            if !supported {
                return Err(SendError::Invalid(
                    "UTXO source script is unsupported by its chain's account signer".into(),
                ));
            }
            Ok(parsed.script_pubkey())
        }
    }
}

/// The script paying `address` on `chain`, refused before any provider read
/// when it is not one of the network's addresses.
pub(crate) fn recipient_script(chain: Chain, address: &str) -> Result<Vec<u8>, SendError> {
    match chain.mainnet_counterpart() {
        Chain::Zcash => super::zcash_stages::address_script(address, chain),
        Chain::Decred => super::decred::recipient_script(chain, address),
        Chain::Kaspa => super::kaspa::recipient_script(chain, address),
        _ => Ok(
            crate::derivation::utxo_address::parse_utxo_address(chain, address)?.script_pubkey(),
        ),
    }
}

#[cfg(test)]
#[path = "tests/account_utxo.rs"]
mod tests;
