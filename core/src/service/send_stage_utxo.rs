//! Account UTXO sends on every network without a protocol stage of its own
//! (all but Litecoin's and Peercoin's): the outputs at every address the
//! wallet's account is known to hold, the largest spent first until they pay
//! the amount and the fee, change back to the wallet's own address, and each
//! input signed with its own address's key.
use super::*;
use crate::api::utxo::{Txid, UtxoInput};
use crate::send::account_utxo::{AccountProtocol, PreparedAccountTransfer};
use crate::send::payload::PreparedSubmission;
use crate::send::stages::{PreparedPayload, StoredSend, UtxoPreparedInput, UtxoSendSource};
use std::collections::{BTreeMap, BTreeSet};
use zeroize::Zeroizing;

/// One confirmed output at a wallet address, as its indexer reports it.
struct AddressOutput {
    txid: Txid,
    vout: u32,
    value: u64,
}

impl AddressOutput {
    /// An indexer's output, its txid refused unless it is one.
    fn read(txid: &str, vout: u32, value: u64) -> Result<Self, SpectraBridgeError> {
        Ok(Self {
            txid: txid
                .parse()
                .map_err(|_| SpectraBridgeError::invalid("The indexer reported an invalid txid"))?,
            vout,
            value,
        })
    }
}

/// The indexer an account UTXO network's outputs are read from.
enum OutputReader {
    Utxo(crate::api::utxo::UtxoClient),
    Insight(InsightClient),
    Kaspa(KaspaClient),
}

impl OutputReader {
    async fn new(service: &WalletService, chain: Chain) -> Self {
        let endpoints = || service.endpoints_for(chain, &[EndpointCapability::Utxo]);
        match chain.mainnet_counterpart() {
            Chain::Decred => Self::Insight(InsightClient::new(endpoints().await)),
            Chain::Kaspa => Self::Kaspa(KaspaClient::new(endpoints().await)),
            _ => Self::Utxo(
                service
                    .utxo_client(chain, &[EndpointCapability::Utxo])
                    .await,
            ),
        }
    }

    /// The confirmed outputs paying `source`. An unconfirmed output is not
    /// spent; nor is a Kaspa coinbase output, whose maturity the indexer
    /// does not say. An output the indexer reports with another script than
    /// the address pays is refused rather than signed for.
    async fn confirmed(
        &self,
        source: &UtxoSendSource,
    ) -> Result<Vec<AddressOutput>, SpectraBridgeError> {
        Ok(match self {
            Self::Utxo(client) => client
                .fetch_utxos(&source.address)
                .await?
                .into_iter()
                .filter(|utxo| utxo.status.confirmed)
                .map(|utxo| AddressOutput::read(&utxo.txid, utxo.vout, utxo.value))
                .collect::<Result<_, _>>()?,
            Self::Insight(client) => client
                .fetch_utxos(&source.address)
                .await?
                .into_iter()
                .filter(|utxo| utxo.confirmations > 0)
                .map(|utxo| AddressOutput::read(&utxo.txid, utxo.vout, utxo.value_atoms))
                .collect::<Result<_, _>>()?,
            Self::Kaspa(client) => {
                let mut outputs = Vec::new();
                for utxo in client.fetch_utxos(&source.address).await? {
                    if utxo.script_version != 0
                        || hex::decode(&utxo.script_pubkey_hex).ok().as_deref()
                            != Some(source.script_pubkey.as_slice())
                    {
                        return Err(SpectraBridgeError::invalid(
                            "Kaspa output does not pay its address's script",
                        ));
                    }
                    if !utxo.is_coinbase {
                        outputs.push(AddressOutput::read(
                            &utxo.txid,
                            utxo.vout,
                            utxo.value_sompi,
                        )?);
                    }
                }
                outputs
            }
        })
    }
}

/// How an account transfer's fee follows from its inputs and outputs.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FeePolicy {
    /// Bitcoin: the signed virtual size at a sat/vB rate (an exact decimal),
    /// rounded up once.
    Rate(String),
    /// The network's static fee per started kilobyte of the signed size:
    /// the legacy-format networks.
    PerKilobyte(u64),
    /// ZIP-317's conventional fee: 5,000 zatoshis for each logical action,
    /// at least two.
    Zip317,
    /// Decred: dcrd's default relay fee, 10 atoms per byte, and at least the
    /// network's static fee.
    Decred(u64),
    /// Kaspa: at least the transaction's mass in sompi, the minimum relay
    /// fee, and at least the network's static fee.
    Kaspa(u64),
}

/// A spend's shape for sizing: its inputs' scripts and its outputs' script
/// lengths.
struct Shape<'a> {
    inputs: Vec<&'a [u8]>,
    outputs: Vec<usize>,
}

impl FeePolicy {
    /// An upper bound of the signed transaction's size in bytes, or virtual
    /// bytes where witnesses are discounted.
    fn size(&self, shape: &Shape<'_>) -> Result<u64, SpectraBridgeError> {
        let count = |n: usize| u64::try_from(n).map_err(SpectraBridgeError::invalid);
        let (overhead, per_input, per_output) = match self {
            Self::Rate(_) => {
                return Ok(crate::send::bitcoin::estimate_vsize(
                    shape.inputs.iter().copied(),
                    shape.outputs.iter().copied(),
                )?);
            }
            // Version, counts and locktime; outpoint, a 107-byte P2PKH
            // signature script and sequence; value and a 25-byte script.
            Self::PerKilobyte(_) => (10, 148, 34),
            Self::Zip317 => (40, 150, 34),
            // Prefix and witness halves of each input; a script version on
            // each output.
            Self::Decred(_) => (16, 166, 36),
            // Outpoint, signature script, sequence and sig-op count; value,
            // script version and script; version, counts, locktime,
            // subnetwork, gas and payload.
            Self::Kaspa(_) => (62, 119, 52),
        };
        count(shape.inputs.len())?
            .checked_mul(per_input)
            .and_then(|size| {
                size.checked_add(count(shape.outputs.len()).ok()?.checked_mul(per_output)?)
            })
            .and_then(|size| size.checked_add(overhead))
            .ok_or_else(|| SpectraBridgeError::invalid("transaction size overflow"))
    }

    fn fee(&self, shape: &Shape<'_>) -> Result<u64, SpectraBridgeError> {
        let size = self.size(shape)?;
        let overflow = || SpectraBridgeError::invalid("fee overflow");
        Ok(match self {
            Self::Rate(rate) => super::send_stage_litecoin::fee_for_vsize(rate, size)?,
            Self::PerKilobyte(per_kilobyte) => size
                .div_ceil(1_000)
                .checked_mul(*per_kilobyte)
                .ok_or_else(overflow)?,
            Self::Zip317 => {
                let actions = shape.inputs.len().max(shape.outputs.len()).max(2);
                u64::try_from(actions)
                    .ok()
                    .and_then(|actions| actions.checked_mul(5_000))
                    .ok_or_else(overflow)?
            }
            Self::Decred(floor) => size.checked_mul(10).ok_or_else(overflow)?.max(*floor),
            Self::Kaspa(floor) => {
                // rusty-kaspa's compute mass: a gram per byte, ten per byte of
                // each output's script and its version, and a thousand per
                // signature operation.
                let scripts = shape
                    .outputs
                    .iter()
                    .try_fold(0u64, |sum, len| {
                        sum.checked_add(u64::try_from(*len).ok()? + 2)
                    })
                    .ok_or_else(overflow)?;
                let sig_ops = u64::try_from(shape.inputs.len()).map_err(|_| overflow())?;
                size.checked_add(scripts.checked_mul(10).ok_or_else(overflow)?)
                    .and_then(|mass| mass.checked_add(sig_ops.checked_mul(1_000)?))
                    .ok_or_else(overflow)?
                    .max(*floor)
            }
        })
    }

    /// Whether nodes relay a spend of `inputs` paying `outputs`: on Kaspa,
    /// one whose KIP-9 storage mass is within the standard limit.
    fn admits(&self, inputs: &[u64], outputs: &[u64]) -> bool {
        match self {
            Self::Kaspa(_) => crate::send::kaspa::storage_mass(inputs, outputs)
                .is_some_and(|mass| mass <= crate::send::kaspa::MAXIMUM_STANDARD_MASS),
            _ => true,
        }
    }
}

/// The smallest output the network relays: a recipient amount below it is
/// refused, and change below it joins the fee rather than standing as an
/// output.
pub(super) fn change_dust(chain: Chain) -> Result<u64, SpectraBridgeError> {
    Ok(match chain.mainnet_counterpart() {
        Chain::Bitcoin | Chain::Zcash => 546,
        Chain::Decred => 6_030,
        Chain::Kaspa => 1_000,
        _ => chain.legacy_change_dust()?,
    })
}

/// The selected inputs, the fee and the change of a spend of `amount`.
struct Selection {
    inputs: Vec<UtxoPreparedInput>,
    fee: u64,
    change: u64,
}

/// Spend the largest outputs first until they pay `amount` and the fee
/// their spend takes with a change output. Change too small to stand joins
/// the fee. `fixed_fee` is a reviewed fee the spend must pay instead, which
/// must not be below the policy's.
fn select(
    mut candidates: Vec<UtxoPreparedInput>,
    amount: u64,
    policy: &FeePolicy,
    recipient_script: &[u8],
    change_script: &[u8],
    dust: u64,
    fixed_fee: Option<u64>,
) -> Result<Selection, SpectraBridgeError> {
    candidates.sort_by(|a, b| {
        b.utxo
            .value
            .cmp(&a.utxo.value)
            .then_with(|| (a.utxo.txid, a.utxo.vout).cmp(&(b.utxo.txid, b.utxo.vout)))
    });
    let mut total = 0u64;
    let mut over_mass = false;
    for count in 1..=candidates.len() {
        total = total
            .checked_add(candidates[count - 1].utxo.value)
            .ok_or_else(|| SpectraBridgeError::invalid("input total overflow"))?;
        let shape = Shape {
            inputs: candidates[..count]
                .iter()
                .map(|input| input.utxo.script_pubkey.as_slice())
                .collect(),
            outputs: vec![recipient_script.len(), change_script.len()],
        };
        let minimum = policy.fee(&shape)?;
        let fee = match fixed_fee {
            Some(fee) if fee < minimum => {
                return Err(SpectraBridgeError::invalid(
                    "Network fee changed; build and review again",
                ));
            }
            Some(fee) => fee,
            None => minimum,
        };
        let Some(change) = amount
            .checked_add(fee)
            .and_then(|needed| total.checked_sub(needed))
        else {
            continue;
        };
        let (fee, change) = if change < dust {
            (fee + change, 0)
        } else {
            (fee, change)
        };
        let values: Vec<u64> = candidates[..count]
            .iter()
            .map(|input| input.utxo.value)
            .collect();
        let outputs: Vec<u64> = std::iter::once(amount)
            .chain((change > 0).then_some(change))
            .collect();
        // More inputs leave more change and weigh less against it.
        if !policy.admits(&values, &outputs) {
            over_mass = true;
            continue;
        }
        candidates.truncate(count);
        return Ok(Selection {
            inputs: candidates,
            fee,
            change,
        });
    }
    Err(if over_mass {
        SpectraBridgeError::invalid(
            "Kaspa's storage mass limit refuses an output this small beside the coins it spends. Send more, or the whole balance.",
        )
    } else {
        crate::send::error::SendError::insufficient_funds().into()
    })
}

impl WalletService {
    /// Every confirmed output of the wallet's account on `chain`, each with
    /// the source address whose key spends it.
    pub(super) async fn account_inputs(
        &self,
        chain: Chain,
        wallet_id: &str,
    ) -> Result<Vec<UtxoPreparedInput>, SpectraBridgeError> {
        let sources = self.account_utxo_send_sources(wallet_id, chain).await?;
        self.inputs_paying(chain, sources).await
    }

    /// Every confirmed output paying one of `sources`.
    pub(super) async fn inputs_paying(
        &self,
        chain: Chain,
        sources: Vec<UtxoSendSource>,
    ) -> Result<Vec<UtxoPreparedInput>, SpectraBridgeError> {
        let reader = OutputReader::new(self, chain).await;
        let mut inputs = Vec::new();
        let mut outpoints = BTreeSet::new();
        for source in sources {
            for output in reader.confirmed(&source).await? {
                if output.value == 0 || !outpoints.insert((output.txid, output.vout)) {
                    return Err(SpectraBridgeError::invalid("Invalid or duplicate input"));
                }
                inputs.push(UtxoPreparedInput {
                    utxo: UtxoInput {
                        txid: output.txid,
                        vout: output.vout,
                        value: output.value,
                        script_pubkey: source.script_pubkey.clone(),
                    },
                    source: source.clone(),
                });
            }
        }
        Ok(inputs)
    }

    /// The fee rule a spend on `chain` follows: Bitcoin's reviewed rate or
    /// the network's current one, else the network's own.
    async fn account_fee_policy(
        &self,
        chain: Chain,
        rate: Option<&str>,
    ) -> Result<FeePolicy, SpectraBridgeError> {
        let static_fee = || helpers::fee_or_static(chain, None);
        Ok(match chain.mainnet_counterpart() {
            Chain::Bitcoin => FeePolicy::Rate(match rate {
                Some(rate) => crate::amount::decimal::canonical(rate)
                    .filter(|rate| rate != "0" && !rate.starts_with('-'))
                    .ok_or_else(|| SpectraBridgeError::invalid("Invalid fee rate"))?,
                None => {
                    let rate = self.bitcoin_fee_rate(chain).await?.sats_per_vbyte;
                    if !rate.is_finite() || rate <= 0.0 {
                        return Err(SpectraBridgeError::failure("Invalid Bitcoin fee rate"));
                    }
                    // Whole sat/vB, rounded up: the rate the review states.
                    (rate.ceil() as u64).to_string()
                }
            }),
            Chain::Zcash => FeePolicy::Zip317,
            Chain::Decred => FeePolicy::Decred(static_fee()?),
            Chain::Kaspa => FeePolicy::Kaspa(static_fee()?),
            _ => FeePolicy::PerKilobyte(static_fee()?),
        })
    }

    /// The protocol a transfer on `chain` signs under, read from the network
    /// where it changes: Zcash's expiry and network upgrade, so the reviewed
    /// transaction cannot span an upgrade.
    async fn account_protocol(&self, chain: Chain) -> Result<AccountProtocol, SpectraBridgeError> {
        if chain.mainnet_counterpart() != Chain::Zcash {
            return Ok(AccountProtocol::for_chain(chain)?);
        }
        let (height, branch) = self.zcash_blockbook(chain).await.zcash_context().await?;
        let mut expiry = crate::send::zcash::expiry_height(u64::from(height))?;
        while chain.zcash_consensus_branch(expiry)? != branch {
            expiry = expiry
                .checked_sub(1)
                .ok_or_else(|| SpectraBridgeError::failure("Zcash expiry underflow"))?;
        }
        Ok(AccountProtocol::Zcash {
            expiry_height: expiry,
            upgrade: crate::send::zcash::ZcashNetworkUpgrade {
                version_group_id: 0x26a7_270a,
                consensus_branch_id: branch,
            },
        })
    }

    async fn zcash_blockbook(&self, chain: Chain) -> BlockbookClient {
        BlockbookClient::new(
            self.endpoints_for(
                chain,
                &[EndpointCapability::Verification, EndpointCapability::Utxo],
            )
            .await,
            chain,
        )
    }

    /// Build a transfer of `amount` to the request's recipient from the
    /// wallet's account, its change to `sender`, the wallet's own address.
    pub(super) async fn prepare_account_transfer(
        &self,
        chain: Chain,
        request: &crate::send::SendExecutionRequest,
        sender: &str,
        amount: u64,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        if self
            .stored_wallet(&request.wallet_id)
            .await?
            .multisig_policy
            .is_some()
        {
            return Err(SpectraBridgeError::invalid(
                "A multisig wallet spends from its Multisig page, where its signers sign.",
            ));
        }
        // Both scripts before any provider read.
        let recipient_script =
            crate::send::account_utxo::recipient_script(chain, &request.to_address)?;
        let change_script = crate::send::account_utxo::source_script(chain, sender)?;
        let dust = change_dust(chain)?;
        if amount < dust {
            return Err(SpectraBridgeError::invalid(
                "The amount is below the network's dust threshold",
            ));
        }
        let protocol = self.account_protocol(chain).await?;
        let policy = self
            .account_fee_policy(chain, request.fee_rate_svb.as_deref())
            .await?;
        let inputs = self.account_inputs(chain, &request.wallet_id).await?;
        if chain.mainnet_counterpart() == Chain::Zcash {
            let total = inputs
                .iter()
                .try_fold(0u64, |sum, input| sum.checked_add(input.utxo.value));
            if total.is_none_or(|total| total > 21_000_000 * 100_000_000) {
                return Err(SpectraBridgeError::invalid(
                    "Zcash inputs exceed the available money range",
                ));
            }
        }
        let selection = select(
            inputs,
            amount,
            &policy,
            &recipient_script,
            &change_script,
            dust,
            request.fee_sat,
        )?;
        let mut outputs = vec![(recipient_script, amount)];
        if selection.change > 0 {
            outputs.push((change_script, selection.change));
        }
        let prepared = PreparedAccountTransfer {
            inputs: selection.inputs,
            outputs,
            fee: selection.fee,
            protocol,
        };
        prepared.validate()?;
        Ok(PreparedPayload::AccountTransfer(prepared))
    }

    /// Sign a reviewed account transfer: each input still the wallet's,
    /// unspent and as reviewed, signed with its source's key, and on Zcash
    /// the network still before the reviewed expiry and upgrade.
    pub(super) async fn sign_account_transfer(
        &self,
        chain: Chain,
        stored: &StoredSend,
        signer: &super::send_identity::ResolvedSendIdentity,
    ) -> Result<PreparedSubmission, SpectraBridgeError> {
        let PreparedPayload::AccountTransfer(prepared) = &stored.prepared else {
            return Err(SpectraBridgeError::invalid("Expected an account transfer"));
        };
        prepared.validate()?;
        if let AccountProtocol::Zcash {
            expiry_height,
            upgrade,
        } = prepared.protocol
        {
            let (height, branch) = self.zcash_blockbook(chain).await.zcash_context().await?;
            if height >= expiry_height || branch != upgrade.consensus_branch_id {
                return Err(SpectraBridgeError::failure(
                    "Zcash transaction expired or consensus changed; build and review again",
                ));
            }
        }
        let by_address: BTreeMap<&str, _> = signer
            .account_utxo_sources
            .iter()
            .map(|key| (key.source.address.as_str(), key))
            .collect();
        let reader = OutputReader::new(self, chain).await;
        let mut current: BTreeMap<String, Vec<AddressOutput>> = BTreeMap::new();
        let mut keys = Vec::with_capacity(prepared.inputs.len());
        for input in &prepared.inputs {
            let key = by_address
                .get(input.source.address.as_str())
                .filter(|key| key.source == input.source)
                .ok_or_else(|| {
                    SpectraBridgeError::invalid("Input source changed; build and review again")
                })?;
            if !current.contains_key(&input.source.address) {
                let outputs = reader.confirmed(&input.source).await?;
                current.insert(input.source.address.clone(), outputs);
            }
            if !current[&input.source.address].iter().any(|output| {
                output.txid == input.utxo.txid
                    && output.vout == input.utxo.vout
                    && output.value == input.utxo.value
            }) {
                return Err(SpectraBridgeError::failure(
                    "Input changed or was spent; build and review again",
                ));
            }
            keys.push(Zeroizing::new(hex::decode(key.private_key_hex.as_str())?));
        }
        let signed = prepared.sign(&keys)?;
        Ok(PreparedSubmission {
            payload: signed.payload,
            transaction_hash: signed.transaction_hash,
            nonce: None,
        })
    }

    /// What a send of `amount` from the wallet's account would spend and
    /// cost, and the most it can send: the outputs it would select, their
    /// fee, whether change stands, and the size it is sized at.
    pub(super) async fn preview_account_send(
        &self,
        chain: Chain,
        wallet_id: &str,
        amount: &str,
        destination: &str,
    ) -> Result<crate::send::preview_types::SendPreview, SpectraBridgeError> {
        let decimals = u32::from(chain.native_decimals());
        let amount = u64::try_from(crate::amount::input::parse_raw_amount(amount, decimals)?)
            .map_err(|_| SpectraBridgeError::invalid("Amount exceeds protocol range"))?;
        let sender = self
            .stored_wallet(wallet_id)
            .await?
            .address_on(chain)
            .map(str::to_string)
            .ok_or_else(|| SpectraBridgeError::invalid("wallet has no address on this network"))?;
        let change_script = crate::send::account_utxo::source_script(chain, &sender)?;
        let recipient_script = if destination.trim().is_empty() {
            change_script.clone()
        } else {
            crate::send::account_utxo::recipient_script(chain, destination)?
        };
        let policy = self.account_fee_policy(chain, None).await?;
        let dust = change_dust(chain)?;
        let inputs = self.account_inputs(chain, wallet_id).await?;
        let total = inputs
            .iter()
            .try_fold(0u64, |sum, input| sum.checked_add(input.utxo.value))
            .ok_or_else(|| SpectraBridgeError::invalid("input total overflow"))?;
        let units = |raw: u64| crate::amount::decimal::from_units(u128::from(raw), decimals);
        let all = Shape {
            inputs: inputs
                .iter()
                .map(|input| input.utxo.script_pubkey.as_slice())
                .collect(),
            outputs: vec![recipient_script.len()],
        };
        let max_sendable = if inputs.is_empty() {
            0
        } else {
            total.saturating_sub(policy.fee(&all)?)
        };
        let selection = (amount > 0)
            .then(|| {
                select(
                    inputs.clone(),
                    amount,
                    &policy,
                    &recipient_script,
                    &change_script,
                    dust,
                    None,
                )
            })
            .transpose()
            .ok()
            .flatten();
        let (count, fee, change, size) = match &selection {
            Some(selection) => {
                let mut outputs = vec![recipient_script.len()];
                if selection.change > 0 {
                    outputs.push(change_script.len());
                }
                let shape = Shape {
                    inputs: selection
                        .inputs
                        .iter()
                        .map(|input| input.utxo.script_pubkey.as_slice())
                        .collect(),
                    outputs,
                };
                (
                    selection.inputs.len(),
                    selection.fee,
                    selection.change > 0,
                    policy.size(&shape)?,
                )
            }
            // Nothing selected yet, or not enough to send the amount: the
            // fee of spending everything, which the affordability check
            // then refuses rather than this preview guessing.
            None if !inputs.is_empty() => {
                (inputs.len(), policy.fee(&all)?, false, policy.size(&all)?)
            }
            None => (0, 0, false, 0),
        };
        let (rate, rate_description) = match &policy {
            FeePolicy::Rate(rate) => (
                crate::amount::decimal::to_f64(rate).ceil() as u64,
                Some(format!("{rate} sat/vB")),
            ),
            _ => (0, None),
        };
        Ok(crate::send::preview_types::SendPreview {
            estimated_network_fee: units(fee),
            spendable_balance: units(total),
            max_sendable: units(max_sendable),
            fee_rate_description: rate_description,
            estimated_transaction_bytes: Some(i64::try_from(size).unwrap_or(i64::MAX)),
            terms: crate::send::preview_types::SendFeeTerms::Utxo {
                fee_rate_sat_vb: rate,
                selected_input_count: i64::try_from(count).unwrap_or(i64::MAX),
                uses_change_output: change,
            },
        })
    }
}

#[cfg(test)]
#[path = "tests/send_stage_utxo.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/send_account_utxo.rs"]
mod account_transfer_tests;
