//! Local metadata-driven SCALE construction for owned nomination-pool actions.
use crate::api::error::ApiError;
use crate::api::substrate_json_rpc::pools::PoolCallMetadata;
use parity_scale_codec::{Compact, Encode};
use scale_info::{TypeDef, TypeDefPrimitive, form::PortableForm};
use serde_json::{Value, json};
#[derive(Debug, Clone)]
pub enum PoolCall {
    Join {
        amount: u128,
        pool_id: u32,
    },
    BondExtra {
        amount: u128,
    },
    Unbond {
        member: [u8; 32],
        points: u128,
    },
    Withdraw {
        member: [u8; 32],
        slashing_spans: u32,
    },
    Claim,
    MigratePool {
        pool_id: u32,
    },
    MigrateMember {
        member: [u8; 32],
    },
    ApplySlash {
        member: [u8; 32],
    },
    BatchAll(Vec<PoolCall>),
}

pub fn encode_pool_call(metadata: &PoolCallMetadata, call: &PoolCall) -> Result<Vec<u8>, ApiError> {
    Builder(metadata).encode(call)
}
struct Builder<'a>(&'a PoolCallMetadata);
fn unsupported() -> ApiError {
    ApiError::Decode("Unsupported nomination-pool call metadata".into())
}
fn number(value: &Value) -> Result<u128, ApiError> {
    value
        .as_str()
        .ok_or_else(unsupported)?
        .parse()
        .map_err(ApiError::decode)
}
impl Builder<'_> {
    fn ty(&self, id: u32) -> Result<&scale_info::Type<PortableForm>, ApiError> {
        self.0.types.resolve(id).ok_or_else(unsupported)
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
    fn variants(&self, id: u32) -> Result<&[scale_info::Variant<PortableForm>], ApiError> {
        match &self.ty(self.peel(id)?)?.type_def {
            TypeDef::Variant(v) => Ok(&v.variants),
            _ => Err(unsupported()),
        }
    }
    fn encode(&self, call: &PoolCall) -> Result<Vec<u8>, ApiError> {
        if let PoolCall::BatchAll(calls) = call {
            if calls.is_empty() || calls.len() > 4 {
                return Err(unsupported());
            }
            let mut bytes = vec![self.0.utility_pallet, self.0.batch_all_call];
            bytes.extend(Compact(calls.len() as u32).encode());
            for call in calls {
                if matches!(call, PoolCall::BatchAll(_)) {
                    return Err(unsupported());
                }
                bytes.extend(self.encode(call)?);
            }
            return Ok(bytes);
        }
        let calls = self.variants(self.0.pool_call_type)?;
        let (name, arguments) = match call {
            PoolCall::Join { amount, pool_id } => (
                "join",
                vec![json!(amount.to_string()), json!(pool_id.to_string())],
            ),
            PoolCall::BondExtra { amount } => (
                "bond_extra",
                vec![json!({"variant":"FreeBalance","fields":[amount.to_string()]})],
            ),
            PoolCall::Unbond { member, points } => (
                "unbond",
                vec![
                    json!({"variant":"Id","fields":[hex::encode(member)]}),
                    json!(points.to_string()),
                ],
            ),
            PoolCall::Withdraw {
                member,
                slashing_spans,
            } => (
                "withdraw_unbonded",
                vec![
                    json!({"variant":"Id","fields":[hex::encode(member)]}),
                    json!(slashing_spans.to_string()),
                ],
            ),
            PoolCall::Claim => ("claim_payout", vec![]),
            PoolCall::MigratePool { pool_id } => (
                "migrate_pool_to_delegate_stake",
                vec![json!(pool_id.to_string())],
            ),
            PoolCall::MigrateMember { member } => (
                "migrate_delegation",
                vec![json!({"variant":"Id","fields":[hex::encode(member)]})],
            ),
            PoolCall::ApplySlash { member } => (
                "apply_slash",
                vec![json!({"variant":"Id","fields":[hex::encode(member)]})],
            ),
            PoolCall::BatchAll(_) => unreachable!(),
        };
        let variant = calls
            .iter()
            .find(|v| v.name == name)
            .ok_or_else(unsupported)?;
        if variant.fields.len() != arguments.len() {
            return Err(unsupported());
        }
        let mut bytes = vec![self.0.pool_pallet, variant.index];
        for (field, arg) in variant.fields.iter().zip(arguments) {
            self.encode_value(field.ty.id, &arg, &mut bytes, 0)?;
        }
        Ok(bytes)
    }

    fn encode_value(
        &self,
        id: u32,
        value: &Value,
        out: &mut Vec<u8>,
        depth: usize,
    ) -> Result<(), ApiError> {
        if depth > 32 {
            return Err(unsupported());
        }
        match &self.ty(id)?.type_def {
            TypeDef::Composite(c) if c.fields.len() == 1 => {
                self.encode_value(c.fields[0].ty.id, value, out, depth + 1)?
            }
            TypeDef::Compact(c) if self.primitive(c.type_param.id, TypeDefPrimitive::U128) => {
                out.extend(Compact(number(value)?).encode())
            }
            TypeDef::Compact(c) if self.primitive(c.type_param.id, TypeDefPrimitive::U32) => out
                .extend(Compact(u32::try_from(number(value)?).map_err(ApiError::decode)?).encode()),
            TypeDef::Primitive(TypeDefPrimitive::U128) => out.extend(number(value)?.to_le_bytes()),
            TypeDef::Primitive(TypeDefPrimitive::U32) => out.extend(
                u32::try_from(number(value)?)
                    .map_err(ApiError::decode)?
                    .to_le_bytes(),
            ),
            TypeDef::Array(a)
                if a.len == 32 && self.primitive(a.type_param.id, TypeDefPrimitive::U8) =>
            {
                let bytes = hex::decode(value.as_str().ok_or_else(unsupported)?)
                    .map_err(ApiError::decode)?;
                if bytes.len() != 32 {
                    return Err(unsupported());
                }
                out.extend(bytes);
            }
            TypeDef::Variant(v) => {
                let name = value["variant"].as_str().ok_or_else(unsupported)?;
                let fields = value["fields"].as_array().ok_or_else(unsupported)?;
                let variant = v
                    .variants
                    .iter()
                    .find(|v| v.name == name && v.fields.len() == fields.len())
                    .ok_or_else(unsupported)?;
                out.push(variant.index);
                for (field, value) in variant.fields.iter().zip(fields) {
                    self.encode_value(field.ty.id, value, out, depth + 1)?;
                }
            }
            _ => return Err(unsupported()),
        }
        Ok(())
    }
}
