//! `pallet-multisig` as the pinned runtime declares it: the calls an
//! approval makes, the deposit it reserves, and the `Multisigs` record of an
//! operation in flight, each checked against the runtime's own types before
//! a byte is encoded or read.
use super::*;
use crate::api::substrate_json_rpc::multisig::{MultisigPallet, PendingMultisig};

impl Metadata {
    fn balance_type(chain: Chain) -> Result<TypeDefPrimitive, ApiError> {
        match chain.substrate_balance_bytes() {
            Some(8) => Ok(TypeDefPrimitive::U64),
            Some(16) => Ok(TypeDefPrimitive::U128),
            _ => Err(unsupported()),
        }
    }

    fn sequence_of_accounts(&self, id: u32) -> bool {
        self.peel(id).and_then(|id| self.ty(id)).is_ok_and(
            |t| matches!(&t.type_def, TypeDef::Sequence(s) if self.bytes(s.type_param.id, 32)),
        )
    }

    fn named_fields(&self, id: u32, names: &[&str]) -> Option<Vec<u32>> {
        let TypeDef::Composite(composite) = &self.ty(id).ok()?.type_def else {
            return None;
        };
        (composite.fields.len() == names.len()
            && composite
                .fields
                .iter()
                .zip(names)
                .all(|(field, name)| field.name.as_deref() == Some(*name)))
        .then(|| composite.fields.iter().map(|field| field.ty.id).collect())
    }

    /// `Timepoint { height: u32, index: u32 }`.
    fn timepoint(&self, id: u32) -> bool {
        self.named_fields(id, &["height", "index"])
            .is_some_and(|fields| {
                fields
                    .iter()
                    .all(|field| self.primitive(*field, TypeDefPrimitive::U32))
            })
    }

    /// `Weight { ref_time: Compact<u64>, proof_size: Compact<u64> }`.
    fn weight(&self, id: u32) -> bool {
        self.named_fields(id, &["ref_time", "proof_size"])
            .is_some_and(|fields| {
                fields
                    .iter()
                    .all(|field| self.compact(*field, TypeDefPrimitive::U64))
            })
    }

    fn balance_constant(
        &self,
        pallet: &PalletMetadata<PortableForm>,
        name: &str,
        balance_type: &TypeDefPrimitive,
        balance_bytes: usize,
    ) -> Result<u128, ApiError> {
        let constant = pallet
            .constants
            .iter()
            .find(|c| c.name == name)
            .ok_or_else(unsupported)?;
        if !self.primitive(constant.ty.id, balance_type.clone())
            || constant.value.len() != balance_bytes
        {
            return Err(unsupported());
        }
        let mut word = [0u8; 16];
        word[..balance_bytes].copy_from_slice(&constant.value);
        Ok(u128::from_le_bytes(word))
    }

    /// The runtime's Multisig pallet: its `as_multi` and `approve_as_multi`
    /// calls laid out as the encoder writes them, reachable from the
    /// runtime's call enum, and its deposit and signatory constants.
    pub(crate) fn multisig_pallet(&self, chain: Chain) -> Result<MultisigPallet, ApiError> {
        let balance_type = Self::balance_type(chain)?;
        let balance_bytes = chain.substrate_balance_bytes().ok_or_else(unsupported)?;
        let pallet = self.pallet("Multisig")?;
        let calls = pallet.calls.as_ref().ok_or_else(unsupported)?.ty.id;
        let params = &self.ty(self.0.extrinsic.ty.id)?.type_params;
        let runtime_call = params
            .iter()
            .find(|p| p.name == "Call")
            .and_then(|p| p.ty)
            .ok_or_else(unsupported)?
            .id;
        if !self.variants(runtime_call)?.iter().any(|v| {
            v.name == "Multisig"
                && v.index == pallet.index
                && v.fields.len() == 1
                && v.fields[0].ty.id == calls
        }) {
            return Err(unsupported());
        }
        let call = |name: &str, last: [&str; 2]| -> Result<u8, ApiError> {
            let call = self
                .variants(calls)?
                .iter()
                .find(|v| v.name == name)
                .ok_or_else(unsupported)?;
            let names = [
                "threshold",
                "other_signatories",
                "maybe_timepoint",
                last[0],
                last[1],
            ];
            let fields: Vec<u32> = call.fields.iter().map(|f| f.ty.id).collect();
            let timepoint = self.variants(fields.get(2).copied().ok_or_else(unsupported)?);
            let laid_out = call.fields.len() == names.len()
                && call
                    .fields
                    .iter()
                    .zip(names)
                    .all(|(field, name)| field.name.as_deref() == Some(name))
                && self.primitive(fields[0], TypeDefPrimitive::U16)
                && self.sequence_of_accounts(fields[1])
                && timepoint.is_ok_and(|variants| {
                    variants.len() == 2
                        && variants
                            .iter()
                            .any(|v| v.name == "None" && v.index == 0 && v.fields.is_empty())
                        && variants.iter().any(|v| {
                            v.name == "Some"
                                && v.index == 1
                                && v.fields.len() == 1
                                && self.timepoint(v.fields[0].ty.id)
                        })
                })
                && if last[0] == "call" {
                    self.peel(fields[3]).is_ok_and(|id| id == runtime_call)
                } else {
                    self.bytes(fields[3], 32)
                }
                && self.weight(fields[4]);
            if !laid_out {
                return Err(unsupported());
            }
            Ok(call.index)
        };
        let as_multi = call("as_multi", ["call", "max_weight"])?;
        let approve_as_multi = call("approve_as_multi", ["call_hash", "max_weight"])?;
        let max_signatories = pallet
            .constants
            .iter()
            .find(|c| c.name == "MaxSignatories")
            .filter(|c| self.primitive(c.ty.id, TypeDefPrimitive::U32) && c.value.len() == 4)
            .ok_or_else(unsupported)?;
        self.multisigs_layout(&balance_type)?;
        Ok(MultisigPallet {
            index: pallet.index,
            as_multi,
            approve_as_multi,
            deposit_base: self.balance_constant(
                pallet,
                "DepositBase",
                &balance_type,
                balance_bytes,
            )?,
            deposit_factor: self.balance_constant(
                pallet,
                "DepositFactor",
                &balance_type,
                balance_bytes,
            )?,
            max_signatories: u32::from_le_bytes(
                max_signatories
                    .value
                    .as_slice()
                    .try_into()
                    .map_err(|_| unsupported())?,
            ),
        })
    }

    /// `Multisigs`: a double map from the account (`Twox64Concat`) and the
    /// call hash (`Blake2_128Concat`) to `{when, deposit, depositor,
    /// approvals}`.
    fn multisigs_layout(&self, balance_type: &TypeDefPrimitive) -> Result<(), ApiError> {
        let StorageEntryType::Map {
            hashers,
            key,
            value,
        } = &self.named_storage("Multisig", "Multisigs")?.ty
        else {
            return Err(unsupported());
        };
        let TypeDef::Tuple(keys) = &self.ty(key.id)?.type_def else {
            return Err(unsupported());
        };
        let fields = self
            .named_fields(value.id, &["when", "deposit", "depositor", "approvals"])
            .ok_or_else(unsupported)?;
        if hashers.as_slice() != [StorageHasher::Twox64Concat, StorageHasher::Blake2_128Concat]
            || keys.fields.len() != 2
            || !keys.fields.iter().all(|field| self.bytes(field.id, 32))
            || !self.timepoint(fields[0])
            || !self.primitive(fields[1], balance_type.clone())
            || !self.bytes(fields[2], 32)
            || !self.sequence_of_accounts(fields[3])
        {
            return Err(unsupported());
        }
        Ok(())
    }

    /// The `Multisigs` key of `account`'s operation on `call_hash`.
    pub(crate) fn multisigs_key(
        &self,
        account: &[u8; 32],
        call_hash: &[u8; 32],
    ) -> Result<String, ApiError> {
        use blake2::digest::consts::U16;
        use blake2::{Blake2b, Digest};
        let mut key = self.storage_prefix("Multisig", "Multisigs")?;
        key.extend(xxhash_rust::xxh64::xxh64(account, 0).to_le_bytes());
        key.extend(account);
        key.extend(Blake2b::<U16>::digest(call_hash));
        key.extend(call_hash);
        Ok(format!("0x{}", hex::encode(key)))
    }
}

/// A `Multisigs` record, laid out as `multisigs_layout` checked: a
/// `balance_bytes` deposit, the rest fixed.
pub(crate) fn decode_pending(
    bytes: &[u8],
    balance_bytes: usize,
) -> Result<PendingMultisig, ApiError> {
    let mut input = bytes;
    let height = u32::decode(&mut input).map_err(ApiError::decode)?;
    let index = u32::decode(&mut input).map_err(ApiError::decode)?;
    if input.len() < balance_bytes + 32 {
        return Err(unsupported());
    }
    let mut word = [0u8; 16];
    word[..balance_bytes].copy_from_slice(&input[..balance_bytes]);
    input = &input[balance_bytes..];
    let depositor = <[u8; 32]>::decode(&mut input).map_err(ApiError::decode)?;
    let approvals = Vec::<[u8; 32]>::decode(&mut input).map_err(ApiError::decode)?;
    if !input.is_empty() || approvals.is_empty() {
        return Err(unsupported());
    }
    Ok(PendingMultisig {
        height,
        index,
        deposit: u128::from_le_bytes(word),
        depositor,
        approvals,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Value {
        serde_json::from_str(include_str!(
            "../../../../tests/fixtures/substrate-multisig.json"
        ))
        .unwrap()
    }

    fn unhex(value: &Value) -> Vec<u8> {
        hex::decode(value.as_str().unwrap().trim_start_matches("0x")).unwrap()
    }

    /// Each runtime's pallet, deposit, `Multisigs` key and record are
    /// polkadot.js's reading of the same metadata.
    #[test]
    fn pallets_keys_and_records_match_polkadot_js() {
        let fixture = fixture();
        for (chain, runtime) in [
            (Chain::Polkadot, "polkadot-asset-hub"),
            (Chain::Bittensor, "bittensor"),
        ] {
            let vector = &fixture["runtimes"][runtime];
            let metadata =
                Metadata::decode(crate::api::substrate_json_rpc::tests::fixture(chain)).unwrap();
            let pallet = metadata.multisig_pallet(chain).unwrap();
            let number = |value: &Value| {
                value
                    .as_u64()
                    .map(u128::from)
                    .unwrap_or_else(|| value.as_str().unwrap().parse().unwrap())
            };
            assert_eq!(
                pallet,
                MultisigPallet {
                    index: vector["multisig"]["pallet_index"].as_u64().unwrap() as u8,
                    as_multi: vector["multisig"]["calls"]["as_multi"].as_u64().unwrap() as u8,
                    approve_as_multi: vector["multisig"]["calls"]["approve_as_multi"]
                        .as_u64()
                        .unwrap() as u8,
                    deposit_base: number(&vector["multisig"]["deposit_base"]),
                    deposit_factor: number(&vector["multisig"]["deposit_factor"]),
                    max_signatories: vector["multisig"]["max_signatories"].as_u64().unwrap() as u32,
                }
            );
            let storage = &vector["storage"];
            let account: [u8; 32] = unhex(&storage["multisig_account"]).try_into().unwrap();
            let hash: [u8; 32] = unhex(&storage["call_hash"]).try_into().unwrap();
            assert_eq!(
                metadata.multisigs_key(&account, &hash).unwrap(),
                storage["key"].as_str().unwrap()
            );
            let input = &storage["value"]["input"];
            let record = unhex(&storage["value"]["hex"]);
            let pending =
                decode_pending(&record, chain.substrate_balance_bytes().unwrap()).unwrap();
            assert_eq!(
                (pending.height, pending.index),
                (
                    input["when"]["height"].as_u64().unwrap() as u32,
                    input["when"]["index"].as_u64().unwrap() as u32
                )
            );
            assert_eq!(pending.deposit, number(&input["deposit"]));
            assert_eq!(pending.depositor.to_vec(), unhex(&input["depositor"]));
            assert_eq!(pending.approvals.len(), 1);
            assert_eq!(pending.approvals[0].to_vec(), unhex(&input["approvals"][0]));
            let mut longer = record.clone();
            longer.push(0);
            assert!(decode_pending(&longer, chain.substrate_balance_bytes().unwrap()).is_err());
        }
        let westend = Metadata::decode(crate::api::substrate_json_rpc::tests::fixture(
            Chain::PolkadotWestend,
        ))
        .unwrap();
        westend.multisig_pallet(Chain::PolkadotWestend).unwrap();
        let polkadot = Metadata::decode(crate::api::substrate_json_rpc::tests::fixture(
            Chain::Polkadot,
        ))
        .unwrap();
        assert!(polkadot.multisig_pallet(Chain::Bittensor).is_err());
    }
}
