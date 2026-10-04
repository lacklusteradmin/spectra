//! Pool storage and calls are interpreted through the pinned runtime's types.
use super::*;

impl Metadata {
    /// Asset Hub uses staking-async; its obsolete slashing-span argument is
    /// ignored. Refuse another backend instead of guessing a span count.
    pub(crate) fn require_async_staking(&self) -> Result<(), ApiError> {
        let StorageEntryType::Map { value, .. } = self.named_storage("Staking", "Ledger")?.ty
        else {
            return Err(unsupported());
        };
        if self.ty(value.id)?.path.segments.first().map(String::as_str)
            != Some("pallet_staking_async")
        {
            return Err(unsupported());
        }
        Ok(())
    }
    pub(crate) fn named_storage(
        &self,
        pallet: &str,
        item: &str,
    ) -> Result<&StorageEntryMetadata<PortableForm>, ApiError> {
        self.pallet(pallet)?
            .storage
            .as_ref()
            .ok_or_else(unsupported)?
            .entries
            .iter()
            .find(|s| s.name == item)
            .ok_or_else(unsupported)
    }

    pub(crate) fn storage_prefix(&self, pallet: &str, item: &str) -> Result<Vec<u8>, ApiError> {
        let storage = self
            .pallet(pallet)?
            .storage
            .as_ref()
            .ok_or_else(unsupported)?;
        self.named_storage(pallet, item)?;
        let mut result = twox128(storage.prefix.as_bytes());
        result.extend(twox128(item.as_bytes()));
        Ok(result)
    }

    pub(crate) fn storage_key(
        &self,
        pallet: &str,
        item: &str,
        key: &[u8],
    ) -> Result<String, ApiError> {
        let mut prefix = self.storage_prefix(pallet, item)?;
        let entry = self.named_storage(pallet, item)?;
        match &entry.ty {
            StorageEntryType::Plain(_) if key.is_empty() => {}
            StorageEntryType::Map {
                hashers, key: ty, ..
            } if hashers.len() == 1 => {
                // Validate the supplied key against its actual runtime type.
                self.decode_value(ty.id, key)?;
                match hashers[0] {
                    StorageHasher::Blake2_128Concat => {
                        use blake2::digest::consts::U16;
                        use blake2::{Blake2b, Digest};
                        prefix.extend(Blake2b::<U16>::digest(key));
                        prefix.extend(key);
                    }
                    StorageHasher::Twox64Concat => {
                        prefix.extend(xxhash_rust::xxh64::xxh64(key, 0).to_le_bytes());
                        prefix.extend(key);
                    }
                    StorageHasher::Identity => prefix.extend(key),
                    _ => return Err(unsupported()),
                }
            }
            _ => return Err(unsupported()),
        }
        Ok(format!("0x{}", hex::encode(prefix)))
    }

    pub(crate) fn decode_storage(
        &self,
        pallet: &str,
        item: &str,
        bytes: &[u8],
    ) -> Result<Value, ApiError> {
        let ty = match self.named_storage(pallet, item)?.ty {
            StorageEntryType::Plain(ty) => ty.id,
            StorageEntryType::Map { value, .. } => value.id,
        };
        self.decode_value(ty, bytes)
    }

    fn decode_value(&self, id: u32, bytes: &[u8]) -> Result<Value, ApiError> {
        let mut input = bytes;
        let value = self.value(id, &mut input, 0)?;
        if !input.is_empty() {
            return Err(unsupported());
        }
        Ok(value)
    }

    fn value(&self, id: u32, input: &mut &[u8], depth: usize) -> Result<Value, ApiError> {
        if depth > 64 {
            return Err(unsupported());
        }
        Ok(match &self.ty(id)?.type_def {
            TypeDef::Composite(c) => {
                if c.fields.len() == 1 && c.fields[0].name.is_none() {
                    self.value(c.fields[0].ty.id, input, depth + 1)?
                } else {
                    let mut result = serde_json::Map::new();
                    for (i, f) in c.fields.iter().enumerate() {
                        result.insert(
                            f.name.clone().unwrap_or_else(|| i.to_string()),
                            self.value(f.ty.id, input, depth + 1)?,
                        );
                    }
                    Value::Object(result)
                }
            }
            TypeDef::Tuple(t) => Value::Array(
                t.fields
                    .iter()
                    .map(|f| self.value(f.id, input, depth + 1))
                    .collect::<Result<_, _>>()?,
            ),
            TypeDef::Variant(v) => {
                let index = take_byte(input)?;
                let variant = v
                    .variants
                    .iter()
                    .find(|v| v.index == index)
                    .ok_or_else(unsupported)?;
                if variant.name == "None" && variant.fields.is_empty() {
                    Value::Null
                } else if variant.name == "Some" && variant.fields.len() == 1 {
                    self.value(variant.fields[0].ty.id, input, depth + 1)?
                } else if variant.fields.is_empty() {
                    json!(variant.name)
                } else {
                    json!({"variant":variant.name,"fields":variant.fields.iter().map(|f|self.value(f.ty.id,input,depth+1)).collect::<Result<Vec<_>,_>>()?})
                }
            }
            TypeDef::Sequence(s) => {
                let count = Compact::<u32>::decode(input).map_err(ApiError::decode)?.0;
                if count > 10_000 {
                    return Err(unsupported());
                }
                Value::Array(
                    (0..count)
                        .map(|_| self.value(s.type_param.id, input, depth + 1))
                        .collect::<Result<_, _>>()?,
                )
            }
            TypeDef::Array(a) => {
                if a.len > 10_000 {
                    return Err(unsupported());
                }
                Value::Array(
                    (0..a.len)
                        .map(|_| self.value(a.type_param.id, input, depth + 1))
                        .collect::<Result<_, _>>()?,
                )
            }
            TypeDef::Compact(_) => json!(
                Compact::<u128>::decode(input)
                    .map_err(ApiError::decode)?
                    .0
                    .to_string()
            ),
            TypeDef::Primitive(p) => {
                let bytes = match p {
                    TypeDefPrimitive::Bool => {
                        return Ok(json!(bool::decode(input).map_err(ApiError::decode)?));
                    }
                    TypeDefPrimitive::U8 => 1,
                    TypeDefPrimitive::U16 => 2,
                    TypeDefPrimitive::U32 => 4,
                    TypeDefPrimitive::U64 => 8,
                    TypeDefPrimitive::U128 => 16,
                    _ => return Err(unsupported()),
                };
                if input.len() < bytes {
                    return Err(unsupported());
                }
                let mut word = [0; 16];
                word[..bytes].copy_from_slice(&input[..bytes]);
                take(input, bytes)?;
                json!(u128::from_le_bytes(word).to_string())
            }
            _ => return Err(unsupported()),
        })
    }

    pub(crate) fn pool_call_metadata(
        &self,
    ) -> Result<super::super::pools::PoolCallMetadata, ApiError> {
        let pools = self.pallet("NominationPools")?;
        let pool_call_type = pools.calls.as_ref().ok_or_else(unsupported)?.ty.id;
        self.variants(pool_call_type)?;
        let utility = self.pallet("Utility")?;
        let batch = self
            .variants(utility.calls.as_ref().ok_or_else(unsupported)?.ty.id)?
            .iter()
            .find(|v| v.name == "batch_all")
            .ok_or_else(unsupported)?;
        if batch.fields.len() != 1 || batch.fields[0].name.as_deref() != Some("calls") {
            return Err(unsupported());
        }
        let TypeDef::Sequence(sequence) = &self.ty(self.peel(batch.fields[0].ty.id)?)?.type_def
        else {
            return Err(unsupported());
        };
        if !self.variants(sequence.type_param.id)?.iter().any(|v| {
            v.name == "NominationPools"
                && v.index == pools.index
                && v.fields.len() == 1
                && v.fields[0].ty.id == pool_call_type
        }) {
            return Err(unsupported());
        }
        Ok(super::super::pools::PoolCallMetadata {
            types: self.0.types.clone(),
            pool_pallet: pools.index,
            pool_call_type,
            utility_pallet: utility.index,
            batch_all_call: batch.index,
        })
    }
}

fn twox128(bytes: &[u8]) -> Vec<u8> {
    let mut result = xxhash_rust::xxh64::xxh64(bytes, 0).to_le_bytes().to_vec();
    result.extend(xxhash_rust::xxh64::xxh64(bytes, 1).to_le_bytes());
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::send::polkadot_pools::{PoolCall, encode_pool_call};
    use parity_scale_codec::Encode;

    fn fixture() -> Value {
        serde_json::from_str(include_str!(
            "../../../../tests/fixtures/polkadot-staking-vectors.json"
        ))
        .unwrap()
    }
    fn metadata() -> Metadata {
        Metadata::decode(crate::api::substrate_json_rpc::tests::fixture(
            Chain::Polkadot,
        ))
        .unwrap()
    }
    #[test]
    fn pool_calls_match_independent_polkadot_js_scale_vectors() {
        let fixture = fixture();
        let metadata = metadata();
        let member = [7; 32];
        let cases = [
            (
                "join",
                PoolCall::Join {
                    amount: 10_000_000_000,
                    pool_id: 7,
                },
            ),
            (
                "bondExtra",
                PoolCall::BondExtra {
                    amount: 10_000_000_000,
                },
            ),
            (
                "unbond",
                PoolCall::Unbond {
                    member,
                    points: 50_000_000_000,
                },
            ),
            (
                "withdraw",
                PoolCall::Withdraw {
                    member,
                    slashing_spans: 0,
                },
            ),
            ("claim", PoolCall::Claim),
            ("migratePool", PoolCall::MigratePool { pool_id: 7 }),
            ("migrateMember", PoolCall::MigrateMember { member }),
            ("applySlash", PoolCall::ApplySlash { member }),
            (
                "batchAll",
                PoolCall::BatchAll(vec![
                    PoolCall::MigratePool { pool_id: 7 },
                    PoolCall::MigrateMember { member },
                    PoolCall::ApplySlash { member },
                    PoolCall::Claim,
                ]),
            ),
        ];
        for (name, call) in cases {
            assert_eq!(
                format!(
                    "0x{}",
                    hex::encode(
                        encode_pool_call(&metadata.pool_call_metadata().unwrap(), &call).unwrap()
                    )
                ),
                fixture["calls"][name],
                "{name}"
            );
        }
    }
    #[test]
    fn pool_storage_uses_runtime_types_and_independent_storage_keys() {
        let fixture = fixture();
        let metadata = metadata();
        metadata.require_async_staking().unwrap();
        for (name, pallet, item, key) in [
            ("member", "NominationPools", "PoolMembers", vec![7; 32]),
            ("pool", "NominationPools", "BondedPools", 7u32.encode()),
            (
                "subpools",
                "NominationPools",
                "SubPoolsStorage",
                7u32.encode(),
            ),
            ("name", "NominationPools", "Metadata", 7u32.encode()),
            ("minimum", "NominationPools", "MinJoinBond", vec![]),
            ("era", "Staking", "ActiveEra", vec![]),
        ] {
            assert_eq!(
                metadata.storage_key(pallet, item, &key).unwrap(),
                fixture["state"][name]["key"]
            );
            let bytes = hex::decode(
                fixture["state"][name]["hex"]
                    .as_str()
                    .unwrap()
                    .trim_start_matches("0x"),
            )
            .unwrap();
            let value = metadata.decode_storage(pallet, item, &bytes).unwrap();
            match name {
                "member" => {
                    assert_eq!(value["pool_id"], "7");
                    assert_eq!(value["points"], "1000000000000");
                    assert_eq!(value["unbonding_eras"][1], json!(["20", "50000000000"]));
                }
                "pool" => {
                    assert_eq!(value["state"], "Open");
                    assert_eq!(value.pointer("/commission/current/0").unwrap(), "100000000");
                }
                "subpools" => {
                    assert_eq!(value["with_era"][0][0], "20");
                    assert_eq!(value["no_era"]["balance"], "900000000000");
                }
                "minimum" => assert_eq!(value, "10000000000"),
                "era" => assert_eq!(value["index"], "10"),
                _ => {}
            }
            assert!(
                metadata
                    .decode_storage(pallet, item, &bytes[..bytes.len() - 1])
                    .is_err()
            );
            let mut extra = bytes;
            extra.push(0);
            assert!(metadata.decode_storage(pallet, item, &extra).is_err());
        }
        assert!(
            metadata
                .storage_key("NominationPools", "PoolMembers", &[7; 31])
                .is_err()
        );
    }
}
