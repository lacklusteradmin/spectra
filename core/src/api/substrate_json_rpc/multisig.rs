//! A multisig account's state on one verified node, pinned to one block:
//! the runtime's Multisig pallet, the operation in flight on a call hash, and
//! the account's balance; and the weight a call dispatches with, which the
//! approval that executes it caps.
use super::*;

/// The runtime's Multisig pallet: where its calls sit, and what an
/// operation's first approval reserves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultisigPallet {
    pub index: u8,
    pub as_multi: u8,
    pub approve_as_multi: u8,
    pub deposit_base: u128,
    pub deposit_factor: u128,
    pub max_signatories: u32,
}

impl MultisigPallet {
    /// What the first approval of an operation reserves from its signer:
    /// `DepositBase + DepositFactor × threshold`.
    pub fn deposit(&self, threshold: u16) -> Option<u128> {
        self.deposit_factor
            .checked_mul(u128::from(threshold))?
            .checked_add(self.deposit_base)
    }
}

/// An operation in flight: where its first approval landed, who reserved
/// its deposit, and the signatories who approved it, sorted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingMultisig {
    pub height: u32,
    pub index: u32,
    pub deposit: u128,
    pub depositor: [u8; 32],
    pub approvals: Vec<[u8; 32]>,
}

pub struct MultisigSnapshot {
    pub context: PolkadotContext,
    pub pallet: MultisigPallet,
    pub pending: Option<PendingMultisig>,
    pub balance: SubstrateBalance,
}

impl SubstrateClient {
    /// `account`'s operation on `call_hash` and its balance, at the block
    /// the runtime contract was read at.
    pub async fn multisig_snapshot(
        &self,
        chain: Chain,
        account: &[u8; 32],
        call_hash: &[u8; 32],
    ) -> Result<MultisigSnapshot, ApiError> {
        let context = self.polkadot_context(chain).await?;
        let raw = self
            .rpc_call("state_getMetadata", json!([context.block_hash]))
            .await?;
        let metadata = metadata::Metadata::decode(&decode_hex(
            raw.as_str().or_decode("Missing runtime metadata")?,
        )?)?;
        let pallet = metadata.multisig_pallet(chain)?;
        let stored = self
            .rpc_call(
                "state_getStorage",
                json!([
                    metadata.multisigs_key(account, call_hash)?,
                    context.block_hash
                ]),
            )
            .await?;
        let pending = match stored {
            Value::Null => None,
            Value::String(hex) => Some(metadata::decode_pending(
                &decode_hex(&hex)?,
                chain
                    .substrate_balance_bytes()
                    .or_decode("Unsupported Substrate balance layout")?,
            )?),
            _ => {
                return Err(ApiError::Decode(
                    "Invalid Multisig.Multisigs storage".into(),
                ));
            }
        };
        let balance = self
            .fetch_balance_at(chain, account, &context.block_hash)
            .await?;
        Ok(MultisigSnapshot {
            context,
            pallet,
            pending,
            balance,
        })
    }

    /// The weight `extrinsic`'s call dispatches with, as
    /// `payment_queryInfo` reports it: `ref_time` and `proof_size`.
    pub async fn query_weight(
        &self,
        extrinsic: &[u8],
        block_hash: &str,
    ) -> Result<(u64, u64), ApiError> {
        let value = self
            .rpc_call(
                "payment_queryInfo",
                json!([format!("0x{}", hex::encode(extrinsic)), block_hash]),
            )
            .await?;
        let part = |camel: &str, snake: &str| {
            let field = &value["weight"][camel];
            let field = if field.is_null() {
                &value["weight"][snake]
            } else {
                field
            };
            field
                .as_u64()
                .or_else(|| field.as_str().and_then(|text| text.parse().ok()))
                .or_decode("Missing Substrate call weight")
        };
        Ok((
            part("refTime", "ref_time")?,
            part("proofSize", "proof_size")?,
        ))
    }
}
