//! Decode the node's SCALE contract before interpreting storage or signing.
//! V14 is what both Asset Hub public RPC deployments expose through
//! `state_getMetadata`. Unknown versions and extensions are refused.

use super::*;
use frame_metadata::{RuntimeMetadata, RuntimeMetadataPrefixed, v14::*};
use parity_scale_codec::{Compact, Decode};
use scale_info::{PortableRegistry, TypeDef, TypeDefPrimitive, form::PortableForm};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolkadotExtension {
    Unit,
    SpecVersion,
    TransactionVersion,
    Genesis,
    Mortality,
    Nonce,
    AssetPayment,
    MetadataHash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolkadotRuntime {
    pub spec_version: u32,
    pub transaction_version: u32,
    pub genesis_hash: String,
    pub metadata_hash: String,
    pub transfer_pallet: u8,
    pub transfer_call: u8,
    pub existential_deposit: u128,
    pub extensions: Vec<PolkadotExtension>,
}

pub(super) struct Metadata(RuntimeMetadataV14);

fn unsupported() -> ApiError {
    ApiError::Decode("Unsupported Asset Hub runtime metadata contract".into())
}

impl Metadata {
    pub(super) fn decode(bytes: &[u8]) -> Result<Self, ApiError> {
        let mut input = bytes;
        let prefixed = RuntimeMetadataPrefixed::decode(&mut input).map_err(ApiError::decode)?;
        if prefixed.0 != frame_metadata::META_RESERVED || !input.is_empty() {
            return Err(unsupported());
        }
        match prefixed.1 {
            RuntimeMetadata::V14(metadata) => Ok(Self(metadata)),
            _ => Err(unsupported()),
        }
    }

    fn types(&self) -> &PortableRegistry {
        &self.0.types
    }

    fn ty(&self, id: u32) -> Result<&scale_info::Type<PortableForm>, ApiError> {
        self.types().resolve(id).ok_or_else(unsupported)
    }

    fn peel(&self, mut id: u32) -> Result<u32, ApiError> {
        for _ in 0..32 {
            match &self.ty(id)?.type_def {
                TypeDef::Composite(c) if c.fields.len() == 1 => id = c.fields[0].ty.id,
                _ => return Ok(id),
            }
        }
        Err(unsupported())
    }

    fn primitive(&self, id: u32, expected: TypeDefPrimitive) -> bool {
        self.peel(id)
            .and_then(|id| self.ty(id))
            .is_ok_and(|t| matches!(&t.type_def, TypeDef::Primitive(p) if *p == expected))
    }

    fn compact(&self, id: u32, expected: TypeDefPrimitive) -> bool {
        self.peel(id).and_then(|id| self.ty(id)).is_ok_and(|t| {
            matches!(&t.type_def, TypeDef::Compact(c) if self.primitive(c.type_param.id, expected))
        })
    }

    fn unit(&self, id: u32) -> bool {
        self.peel(id)
            .and_then(|id| self.ty(id))
            .is_ok_and(|t| match &t.type_def {
                TypeDef::Composite(c) => c.fields.is_empty(),
                TypeDef::Tuple(t) => t.fields.is_empty(),
                _ => false,
            })
    }

    fn bytes(&self, id: u32, len: u32) -> bool {
        self.peel(id).and_then(|id| self.ty(id)).is_ok_and(|t| {
            matches!(&t.type_def, TypeDef::Array(a) if a.len == len && self.primitive(a.type_param.id, TypeDefPrimitive::U8))
        })
    }

    fn variants(&self, id: u32) -> Result<&[scale_info::Variant<PortableForm>], ApiError> {
        match &self.ty(self.peel(id)?)?.type_def {
            TypeDef::Variant(v) => Ok(&v.variants),
            _ => Err(unsupported()),
        }
    }

    fn option(&self, id: u32, hash: bool) -> bool {
        self.variants(id).is_ok_and(|v| {
            v.len() == 2
                && v.iter()
                    .any(|v| v.name == "None" && v.index == 0 && v.fields.is_empty())
                && v.iter().any(|v| {
                    v.name == "Some"
                        && v.index == 1
                        && v.fields.len() == 1
                        && (!hash || self.bytes(v.fields[0].ty.id, 32))
                })
        })
    }

    fn pallet(&self, name: &str) -> Result<&PalletMetadata<PortableForm>, ApiError> {
        self.0
            .pallets
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(unsupported)
    }

    fn storage(&self, name: &str) -> Result<&StorageEntryMetadata<PortableForm>, ApiError> {
        let storage = self
            .pallet("System")?
            .storage
            .as_ref()
            .ok_or_else(unsupported)?;
        if storage.prefix != "System" {
            return Err(unsupported());
        }
        storage
            .entries
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(unsupported)
    }

    fn validate_account(&self) -> Result<(), ApiError> {
        let StorageEntryType::Map {
            hashers,
            key,
            value,
        } = &self.storage("Account")?.ty
        else {
            return Err(unsupported());
        };
        if hashers.as_slice() != [StorageHasher::Blake2_128Concat] || !self.bytes(key.id, 32) {
            return Err(unsupported());
        }
        let TypeDef::Composite(info) = &self.ty(value.id)?.type_def else {
            return Err(unsupported());
        };
        let names = ["nonce", "consumers", "providers", "sufficients", "data"];
        if info.fields.len() != names.len()
            || !info
                .fields
                .iter()
                .zip(names)
                .all(|(f, n)| f.name.as_deref() == Some(n))
            || !info.fields[..4]
                .iter()
                .all(|f| self.primitive(f.ty.id, TypeDefPrimitive::U32))
        {
            return Err(unsupported());
        }
        let TypeDef::Composite(data) = &self.ty(info.fields[4].ty.id)?.type_def else {
            return Err(unsupported());
        };
        let names = ["free", "reserved", "frozen", "flags"];
        if data.fields.len() != 4
            || !data.fields.iter().zip(names).all(|(f, n)| {
                f.name.as_deref() == Some(n) && self.primitive(f.ty.id, TypeDefPrimitive::U128)
            })
        {
            return Err(unsupported());
        }
        Ok(())
    }

    pub(super) fn contract(&self) -> Result<(u8, u8, u128, Vec<PolkadotExtension>), ApiError> {
        self.validate_account()?;
        let extrinsic = &self.0.extrinsic;
        if extrinsic.version != 4 {
            return Err(unsupported());
        }
        let params = &self.ty(extrinsic.ty.id)?.type_params;
        let param = |name| {
            params
                .iter()
                .find(|p| p.name == name)
                .and_then(|p| p.ty)
                .ok_or_else(unsupported)
        };
        let address = param("Address")?.id;
        let signature = param("Signature")?.id;
        if !self.variants(address)?.iter().any(|v| {
            v.name == "Id"
                && v.index == 0
                && v.fields.len() == 1
                && self.bytes(v.fields[0].ty.id, 32)
        }) || !self.variants(signature)?.iter().any(|v| {
            v.name == "Sr25519"
                && v.index == 1
                && v.fields.len() == 1
                && self.bytes(v.fields[0].ty.id, 64)
        }) {
            return Err(unsupported());
        }
        let balances = self.pallet("Balances")?;
        let call_ty = balances.calls.as_ref().ok_or_else(unsupported)?.ty.id;
        if !self.variants(param("Call")?.id)?.iter().any(|v| {
            v.name == "Balances"
                && v.index == balances.index
                && v.fields.len() == 1
                && v.fields[0].ty.id == call_ty
        }) {
            return Err(unsupported());
        }
        let call = self
            .variants(call_ty)?
            .iter()
            .find(|v| v.name == "transfer_keep_alive")
            .ok_or_else(unsupported)?;
        if call.fields.len() != 2
            || call.fields[0].name.as_deref() != Some("dest")
            || call.fields[0].ty.id != address
            || call.fields[1].name.as_deref() != Some("value")
            || !self.compact(call.fields[1].ty.id, TypeDefPrimitive::U128)
        {
            return Err(unsupported());
        }
        let deposit = balances
            .constants
            .iter()
            .find(|c| c.name == "ExistentialDeposit")
            .ok_or_else(unsupported)?;
        if !self.primitive(deposit.ty.id, TypeDefPrimitive::U128) {
            return Err(unsupported());
        }
        let ed = u128::from_le_bytes(
            deposit
                .value
                .as_slice()
                .try_into()
                .map_err(|_| unsupported())?,
        );
        if ed == 0 {
            return Err(unsupported());
        }
        let mut extensions = Vec::new();
        let mut identifiers = std::collections::HashSet::new();
        for extension in &extrinsic.signed_extensions {
            if !identifiers.insert(extension.identifier.as_str()) {
                return Err(unsupported());
            }
            let ty = extension.ty.id;
            let additional = extension.additional_signed.id;
            let unit_additional = self.unit(additional);
            let kind = match extension.identifier.as_str() {
                "AuthorizeCall"
                | "CheckNonZeroSender"
                | "CheckWeight"
                | "PrevalidateAttests"
                | "EthSetOrigin"
                | "StorageWeightReclaim"
                    if self.unit(ty) && unit_additional =>
                {
                    PolkadotExtension::Unit
                }
                "CheckSpecVersion"
                    if self.unit(ty) && self.primitive(additional, TypeDefPrimitive::U32) =>
                {
                    PolkadotExtension::SpecVersion
                }
                "CheckTxVersion"
                    if self.unit(ty) && self.primitive(additional, TypeDefPrimitive::U32) =>
                {
                    PolkadotExtension::TransactionVersion
                }
                "CheckGenesis" if self.unit(ty) && self.bytes(additional, 32) => {
                    PolkadotExtension::Genesis
                }
                "CheckMortality"
                    if self.bytes(additional, 32)
                        && self.variants(ty)?.iter().any(|v| {
                            v.name == "Immortal" && v.index == 0 && v.fields.is_empty()
                        }) =>
                {
                    PolkadotExtension::Mortality
                }
                "CheckNonce" if self.compact(ty, TypeDefPrimitive::U32) && unit_additional => {
                    PolkadotExtension::Nonce
                }
                "ChargeAssetTxPayment" if unit_additional => {
                    let TypeDef::Composite(fields) = &self.ty(ty)?.type_def else {
                        return Err(unsupported());
                    };
                    if fields.fields.len() != 2
                        || fields.fields[0].name.as_deref() != Some("tip")
                        || !self.compact(fields.fields[0].ty.id, TypeDefPrimitive::U128)
                        || fields.fields[1].name.as_deref() != Some("asset_id")
                        || !self.option(fields.fields[1].ty.id, false)
                    {
                        return Err(unsupported());
                    }
                    PolkadotExtension::AssetPayment
                }
                "CheckMetadataHash"
                    if self.option(additional, true)
                        && self.variants(ty)?.iter().any(|v| {
                            v.name == "Disabled" && v.index == 0 && v.fields.is_empty()
                        }) =>
                {
                    PolkadotExtension::MetadataHash
                }
                _ => {
                    return Err(ApiError::Decode(format!(
                        "Unsupported Asset Hub signed extension: {}",
                        extension.identifier
                    )));
                }
            };
            extensions.push(kind);
        }
        for required in [
            "CheckSpecVersion",
            "CheckTxVersion",
            "CheckGenesis",
            "CheckMortality",
            "CheckNonce",
            "ChargeAssetTxPayment",
            "CheckMetadataHash",
        ] {
            if !identifiers.contains(required) {
                return Err(unsupported());
            }
        }
        Ok((balances.index, call.index, ed, extensions))
    }

    /// Resolve only the target extrinsic's dispatch outcome; skip every other
    /// event using the block runtime's own type registry, never byte offsets.
    pub(super) fn dispatch_outcome(
        &self,
        bytes: &[u8],
        extrinsic_index: u32,
    ) -> Result<bool, ApiError> {
        let StorageEntryType::Plain(events) = self.storage("Events")?.ty else {
            return Err(unsupported());
        };
        let TypeDef::Sequence(sequence) = &self.ty(events.id)?.type_def else {
            return Err(unsupported());
        };
        let TypeDef::Composite(record) = &self.ty(sequence.type_param.id)?.type_def else {
            return Err(unsupported());
        };
        if record.fields.len() != 3
            || record
                .fields
                .iter()
                .zip(["phase", "event", "topics"])
                .any(|(f, n)| f.name.as_deref() != Some(n))
        {
            return Err(unsupported());
        }
        let phase_ty = record.fields[0].ty.id;
        let event_ty = record.fields[1].ty.id;
        let system = self.pallet("System")?;
        let runtime_system = self
            .variants(event_ty)?
            .iter()
            .find(|v| v.name == "System" && v.index == system.index)
            .ok_or_else(unsupported)?;
        if runtime_system.fields.len() != 1 {
            return Err(unsupported());
        }
        let system_events = runtime_system.fields[0].ty.id;
        if Some(system_events) != system.event.as_ref().map(|e| e.ty.id) {
            return Err(unsupported());
        }
        let mut input = bytes;
        let count = Compact::<u32>::decode(&mut input)
            .map_err(ApiError::decode)?
            .0;
        if count > 100_000 {
            return Err(unsupported());
        }
        let mut outcome = None;
        for _ in 0..count {
            let phase_index = take_byte(&mut input)?;
            let phase = self
                .variants(phase_ty)?
                .iter()
                .find(|v| v.index == phase_index)
                .ok_or_else(unsupported)?;
            let applies = if phase.name == "ApplyExtrinsic"
                && phase.fields.len() == 1
                && self.primitive(phase.fields[0].ty.id, TypeDefPrimitive::U32)
            {
                u32::decode(&mut input).map_err(ApiError::decode)? == extrinsic_index
            } else if (phase.name == "Finalization" || phase.name == "Initialization")
                && phase.fields.is_empty()
            {
                false
            } else {
                return Err(unsupported());
            };
            let pallet_index = take_byte(&mut input)?;
            let pallet_event = self
                .variants(event_ty)?
                .iter()
                .find(|v| v.index == pallet_index)
                .ok_or_else(unsupported)?;
            if pallet_index == system.index {
                let event_index = take_byte(&mut input)?;
                let event = self
                    .variants(system_events)?
                    .iter()
                    .find(|v| v.index == event_index)
                    .ok_or_else(unsupported)?;
                if applies && (event.name == "ExtrinsicSuccess" || event.name == "ExtrinsicFailed")
                {
                    if outcome.is_some() {
                        return Err(unsupported());
                    }
                    outcome = Some(event.name == "ExtrinsicSuccess");
                }
                for field in &event.fields {
                    self.skip(field.ty.id, &mut input, 0)?;
                }
            } else {
                for field in &pallet_event.fields {
                    self.skip(field.ty.id, &mut input, 0)?;
                }
            }
            self.skip(record.fields[2].ty.id, &mut input, 0)?;
        }
        if !input.is_empty() {
            return Err(unsupported());
        }
        outcome
            .ok_or_else(|| ApiError::Decode("Finalized extrinsic has no dispatch outcome".into()))
    }

    fn skip(&self, id: u32, input: &mut &[u8], depth: usize) -> Result<(), ApiError> {
        if depth > 64 {
            return Err(unsupported());
        }
        match &self.ty(id)?.type_def {
            TypeDef::Composite(c) => {
                for f in &c.fields {
                    self.skip(f.ty.id, input, depth + 1)?;
                }
            }
            TypeDef::Tuple(t) => {
                for f in &t.fields {
                    self.skip(f.id, input, depth + 1)?;
                }
            }
            TypeDef::Variant(v) => {
                let index = take_byte(input)?;
                let variant = v
                    .variants
                    .iter()
                    .find(|v| v.index == index)
                    .ok_or_else(unsupported)?;
                for f in &variant.fields {
                    self.skip(f.ty.id, input, depth + 1)?;
                }
            }
            TypeDef::Sequence(s) => {
                let count = Compact::<u32>::decode(input).map_err(ApiError::decode)?.0;
                if count > 100_000 {
                    return Err(unsupported());
                }
                for _ in 0..count {
                    self.skip(s.type_param.id, input, depth + 1)?;
                }
            }
            TypeDef::Array(a) => {
                if a.len > 100_000 {
                    return Err(unsupported());
                }
                for _ in 0..a.len {
                    self.skip(a.type_param.id, input, depth + 1)?;
                }
            }
            TypeDef::Compact(_) => {
                Compact::<u128>::decode(input).map_err(ApiError::decode)?;
            }
            TypeDef::Primitive(p) => {
                let size = match p {
                    TypeDefPrimitive::Bool => {
                        if take_byte(input)? > 1 {
                            return Err(unsupported());
                        }
                        return Ok(());
                    }
                    TypeDefPrimitive::U8 | TypeDefPrimitive::I8 => 1,
                    TypeDefPrimitive::U16 | TypeDefPrimitive::I16 => 2,
                    TypeDefPrimitive::U32 | TypeDefPrimitive::I32 | TypeDefPrimitive::Char => 4,
                    TypeDefPrimitive::U64 | TypeDefPrimitive::I64 => 8,
                    TypeDefPrimitive::U128 | TypeDefPrimitive::I128 => 16,
                    TypeDefPrimitive::U256 | TypeDefPrimitive::I256 => 32,
                    TypeDefPrimitive::Str => {
                        Compact::<u32>::decode(input).map_err(ApiError::decode)?.0 as usize
                    }
                };
                take(input, size)?;
            }
            TypeDef::BitSequence(b) => {
                let bits = Compact::<u32>::decode(input).map_err(ApiError::decode)?.0 as usize;
                let width = match self.ty(b.bit_store_type.id)?.type_def {
                    TypeDef::Primitive(TypeDefPrimitive::U8) => 1,
                    TypeDef::Primitive(TypeDefPrimitive::U16) => 2,
                    TypeDef::Primitive(TypeDefPrimitive::U32) => 4,
                    TypeDef::Primitive(TypeDefPrimitive::U64) => 8,
                    _ => return Err(unsupported()),
                };
                take(input, bits.div_ceil(width * 8) * width)?;
            }
        }
        Ok(())
    }
}

fn take(input: &mut &[u8], size: usize) -> Result<(), ApiError> {
    if input.len() < size {
        return Err(ApiError::Decode("Truncated SCALE event".into()));
    }
    *input = &input[size..];
    Ok(())
}

fn take_byte(input: &mut &[u8]) -> Result<u8, ApiError> {
    let byte = *input.first().ok_or_else(unsupported)?;
    take(input, 1)?;
    Ok(byte)
}

#[cfg(test)]
mod tests {
    use super::*;
    use parity_scale_codec::Encode;

    fn decoded() -> Metadata {
        Metadata::decode(crate::api::substrate_json_rpc::tests::fixture(
            Chain::Polkadot,
        ))
        .unwrap()
    }

    #[test]
    fn unsupported_extensions_and_account_layouts_are_refused() {
        let mut metadata = decoded();
        metadata.0.extrinsic.signed_extensions[0].identifier = "UnknownAuthorization".into();
        assert!(
            metadata
                .contract()
                .unwrap_err()
                .to_string()
                .contains("UnknownAuthorization")
        );
        let mut metadata = decoded();
        metadata.0.extrinsic.version = 5;
        assert!(metadata.contract().is_err());
        let mut metadata = decoded();
        let system = metadata
            .0
            .pallets
            .iter_mut()
            .find(|p| p.name == "System")
            .unwrap();
        let account = system
            .storage
            .as_mut()
            .unwrap()
            .entries
            .iter_mut()
            .find(|s| s.name == "Account")
            .unwrap();
        let StorageEntryType::Map { hashers, .. } = &mut account.ty else {
            panic!();
        };
        hashers[0] = StorageHasher::Identity;
        assert!(metadata.contract().is_err());
        let mut bytes = crate::api::substrate_json_rpc::tests::fixture(Chain::Polkadot).to_vec();
        bytes.push(0);
        assert!(Metadata::decode(&bytes).is_err());
    }

    fn event(index: u32, succeeded: bool) -> Vec<u8> {
        let mut bytes = vec![0]; // ApplyExtrinsic
        bytes.extend(index.encode());
        bytes.push(0); // System
        bytes.push(u8::from(!succeeded));
        if !succeeded {
            bytes.push(2);
        } // DispatchError::BadOrigin
        bytes.extend([0, 0, 0, 0]); // weight two compact u64, Normal, Pays::Yes
        bytes.push(0); // empty topics
        bytes
    }

    #[test]
    fn finalized_dispatch_events_distinguish_success_failure_and_wrong_extrinsic() {
        let metadata = decoded();
        let mut bytes = Compact(2u32).encode();
        bytes.extend(event(0, true));
        bytes.extend(event(1, false));
        assert!(metadata.dispatch_outcome(&bytes, 0).unwrap());
        assert!(!metadata.dispatch_outcome(&bytes, 1).unwrap());
        assert!(metadata.dispatch_outcome(&bytes, 2).is_err());
        bytes.pop();
        assert!(metadata.dispatch_outcome(&bytes, 1).is_err());
        for chain in [Chain::Polkadot, Chain::PolkadotWestend] {
            let metadata =
                Metadata::decode(crate::api::substrate_json_rpc::tests::fixture(chain)).unwrap();
            let mut success = Compact(1u32).encode();
            success.extend(event(3, true));
            assert!(metadata.dispatch_outcome(&success, 3).unwrap());
        }
    }
}
