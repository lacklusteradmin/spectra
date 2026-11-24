//! The `Assets` pallet's transfer call and storage, checked against the
//! pinned runtime's types.
use super::*;

/// Where `Assets.transfer` sits in the runtime's call enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetTransferCall {
    pub pallet: u8,
    pub call: u8,
}

impl Metadata {
    /// `Assets.transfer(id: Compact<u32>, target: Address, amount:
    /// Compact<u128>)`, reachable from the runtime's call enum and taking the
    /// address type the extrinsic is signed with; and the `Asset`, `Account`
    /// and `Metadata` maps keyed as the reads key them. Anything else is
    /// refused rather than encoded by guess.
    pub(crate) fn asset_transfer_call(&self) -> Result<AssetTransferCall, ApiError> {
        let assets = self.pallet("Assets")?;
        let call_ty = assets.calls.as_ref().ok_or_else(unsupported)?.ty.id;
        let params = &self.ty(self.0.extrinsic.ty.id)?.type_params;
        let param = |name| {
            params
                .iter()
                .find(|p| p.name == name)
                .and_then(|p| p.ty)
                .ok_or_else(unsupported)
        };
        if !self.variants(param("Call")?.id)?.iter().any(|v| {
            v.name == "Assets"
                && v.index == assets.index
                && v.fields.len() == 1
                && v.fields[0].ty.id == call_ty
        }) {
            return Err(unsupported());
        }
        let call = self
            .variants(call_ty)?
            .iter()
            .find(|v| v.name == "transfer")
            .ok_or_else(unsupported)?;
        if call.fields.len() != 3
            || !call
                .fields
                .iter()
                .zip(["id", "target", "amount"])
                .all(|(f, n)| f.name.as_deref() == Some(n))
            || !self.compact(call.fields[0].ty.id, TypeDefPrimitive::U32)
            || call.fields[1].ty.id != param("Address")?.id
            || !self.compact(call.fields[2].ty.id, TypeDefPrimitive::U128)
        {
            return Err(unsupported());
        }
        for (item, keys) in [("Asset", 1), ("Account", 2), ("Metadata", 1)] {
            let StorageEntryType::Map { hashers, .. } = &self.named_storage("Assets", item)?.ty
            else {
                return Err(unsupported());
            };
            if hashers.len() != keys {
                return Err(unsupported());
            }
        }
        Ok(AssetTransferCall {
            pallet: assets.index,
            call: call.index,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parity_scale_codec::Encode;

    fn fixture() -> Value {
        serde_json::from_str(include_str!(
            "../../../../tests/fixtures/asset-hub-assets.json"
        ))
        .unwrap()
    }

    fn metadata(chain: Chain) -> Metadata {
        Metadata::decode(crate::api::substrate_json_rpc::tests::fixture(chain)).unwrap()
    }

    #[test]
    fn the_transfer_call_is_where_each_runtime_puts_it() {
        let fixture = fixture();
        for (chain, network) in [
            (Chain::Polkadot, "polkadot"),
            (Chain::PolkadotWestend, "westend"),
        ] {
            let call = metadata(chain).asset_transfer_call().unwrap();
            assert_eq!(
                [call.pallet, call.call],
                [
                    fixture[network]["transfer"]["pallet"].as_u64().unwrap() as u8,
                    fixture[network]["transfer"]["call"].as_u64().unwrap() as u8
                ],
                "{network}"
            );
        }
    }

    #[test]
    fn asset_storage_uses_runtime_types_and_independent_storage_keys() {
        let fixture = fixture();
        let metadata = metadata(Chain::Polkadot);
        let owner = [7u8; 32];
        for (name, item, key) in [
            ("asset", "Asset", 1984u32.encode()),
            ("account", "Account", (1984u32, owner).encode()),
            ("metadata", "Metadata", 1984u32.encode()),
        ] {
            let vector = &fixture["polkadot"]["state"][name];
            assert_eq!(
                metadata.storage_key("Assets", item, &key).unwrap(),
                vector["key"],
                "{name}"
            );
            let bytes =
                hex::decode(vector["hex"].as_str().unwrap().trim_start_matches("0x")).unwrap();
            let value = metadata.decode_storage("Assets", item, &bytes).unwrap();
            match name {
                "asset" => {
                    assert_eq!(value["min_balance"], "700000");
                    assert_eq!(value["is_sufficient"], true);
                    assert_eq!(value["status"], "Live");
                }
                "account" => {
                    assert_eq!(value["balance"], "123456789");
                    assert_eq!(value["status"], "Liquid");
                }
                _ => assert_eq!(value["decimals"], "6"),
            }
            assert!(
                metadata
                    .decode_storage("Assets", item, &bytes[..bytes.len() - 1])
                    .is_err()
            );
        }
        // A key of the wrong shape: a short account, a trailing byte.
        assert!(
            metadata
                .storage_key("Assets", "Account", &(1984u32, [7u8; 31]).encode())
                .is_err()
        );
        let mut long = (1984u32, owner).encode();
        long.push(0);
        assert!(metadata.storage_key("Assets", "Account", &long).is_err());
    }
}
