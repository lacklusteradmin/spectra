//! A Safe's state as the network holds it: the proxy's code and the
//! singleton it delegates to (storage slot 0), the guard (its own storage
//! slot), and what the Safe answers for its owners, threshold, nonce,
//! version and enabled modules. One JSON-RPC batch, each answer decoded
//! strictly; `send::safe` judges what it means.

use serde_json::{Value, json};

use crate::api::error::ApiError;
use crate::api::evm_json_rpc::{EvmClient, decode_hex, parse_hex_u128};
use crate::send::safe::{EvmAddress, SafeState};

/// `keccak256("guard_manager.guard.address")`.
const GUARD_SLOT: &str = "0x4a204f620c8c5ccdca3fd54d003badd85ba500436a431f0cbda4f558c93c34c8";
const SEL_GET_OWNERS: &str = "a0e67e2b";
const SEL_GET_THRESHOLD: &str = "e75235b8";
const SEL_NONCE: &str = "affed0e0";
const SEL_VERSION: &str = "ffa1ad74";
/// `getModulesPaginated(address start, uint256 pageSize)`.
const SEL_GET_MODULES: &str = "cc2f8452";
/// The modules read in one page; a Safe with more is reported as having
/// more.
const MODULE_PAGE: u64 = 50;
/// The modules list's sentinel, where it starts and ends.
const SENTINEL: EvmAddress = {
    let mut sentinel = [0; 20];
    sentinel[19] = 1;
    sentinel
};

fn word(bytes: &[u8], index: usize) -> Result<&[u8], ApiError> {
    bytes
        .get(index * 32..index * 32 + 32)
        .ok_or_else(|| ApiError::decode("Safe answer is too short"))
}

fn small(word: &[u8]) -> Result<u64, ApiError> {
    if word[..24] != [0; 24] {
        return Err(ApiError::decode("Safe answer is out of range"));
    }
    Ok(u64::from_be_bytes(word[24..].try_into().expect("8 bytes")))
}

fn address(word: &[u8]) -> Result<EvmAddress, ApiError> {
    if word[..12] != [0; 12] {
        return Err(ApiError::decode("Safe answer is not an address"));
    }
    Ok(word[12..].try_into().expect("20 bytes"))
}

/// An ABI `address[]` whose offset word sits at `head`.
fn address_array(bytes: &[u8], head: usize) -> Result<Vec<EvmAddress>, ApiError> {
    let offset = small(word(bytes, head)?)? as usize;
    if !offset.is_multiple_of(32) {
        return Err(ApiError::decode("Safe answer is misaligned"));
    }
    let start = offset / 32;
    let length = small(word(bytes, start)?)? as usize;
    if length > 1024 {
        return Err(ApiError::decode("Safe answer lists too many addresses"));
    }
    (0..length)
        .map(|index| address(word(bytes, start + 1 + index)?))
        .collect()
}

/// An ABI `string` returned alone.
fn string(bytes: &[u8]) -> Result<String, ApiError> {
    if small(word(bytes, 0)?)? != 32 {
        return Err(ApiError::decode("Safe answer is not a string"));
    }
    let length = small(word(bytes, 1)?)? as usize;
    let text = bytes
        .get(64..64 + length)
        .ok_or_else(|| ApiError::decode("Safe answer is too short"))?;
    String::from_utf8(text.to_vec()).map_err(|_| ApiError::decode("Safe answer is not UTF-8"))
}

fn text(answer: &Result<Value, ApiError>) -> Result<&str, ApiError> {
    match answer {
        Ok(value) => value
            .as_str()
            .ok_or_else(|| ApiError::decode("Safe read: expected a string")),
        Err(error) => Err(error.clone()),
    }
}

/// `answer` to an `eth_call` against a Safe, the call reverting refused as
/// "not a Safe".
fn called(answer: &Result<Value, ApiError>) -> Result<Vec<u8>, ApiError> {
    match answer {
        Err(ApiError::Rejected(_)) => Err(ApiError::invalid(
            "This contract does not answer as a Safe does.",
        )),
        _ => decode_hex(text(answer)?),
    }
}

impl EvmClient {
    /// What `safe`'s proxy and singleton say, at the latest block.
    pub(crate) async fn fetch_safe(&self, safe: &str) -> Result<SafeState, ApiError> {
        let call = |selector: &str| {
            (
                "eth_call",
                json!([{"to": safe, "data": format!("0x{selector}")}, "latest"]),
            )
        };
        let modules = {
            let mut data = String::from(SEL_GET_MODULES);
            data.push_str(&format!("{:0>64}", hex::encode(SENTINEL)));
            data.push_str(&format!("{MODULE_PAGE:064x}"));
            data
        };
        let answers = self
            .call_batch_each(vec![
                ("eth_getCode", json!([safe, "latest"])),
                ("eth_getStorageAt", json!([safe, "0x0", "latest"])),
                ("eth_getStorageAt", json!([safe, GUARD_SLOT, "latest"])),
                call(SEL_GET_OWNERS),
                call(SEL_GET_THRESHOLD),
                call(SEL_NONCE),
                call(SEL_VERSION),
                call(&modules),
                ("eth_getBalance", json!([safe, "latest"])),
            ])
            .await?;
        let [
            code,
            singleton,
            guard,
            owners,
            threshold,
            nonce,
            version,
            modules,
            balance,
        ] = <[Result<Value, ApiError>; 9]>::try_from(answers)
            .map_err(|_| ApiError::decode("Safe read: answers missing"))?;
        let code = decode_hex(text(&code)?)?;
        if !crate::send::safe::is_official_proxy(&code) {
            // Only an official proxy's answers mean what a Safe's do: report
            // the code alone and let the policy refuse it.
            return Ok(SafeState {
                code,
                singleton: [0; 20],
                version: String::new(),
                owners: Vec::new(),
                threshold: 0,
                nonce: 0,
                modules: Vec::new(),
                more_modules: false,
                guard: None,
                balance_wei: parse_hex_u128(text(&balance)?)?,
            });
        }
        let singleton = decode_hex(text(&singleton)?)?;
        let guard = decode_hex(text(&guard)?)?;
        let modules = called(&modules)?;
        let next = address(word(&modules, 1)?)?;
        let guard = address(word(&guard, 0)?)?;
        Ok(SafeState {
            singleton: address(word(&singleton, 0)?)?,
            version: string(&called(&version)?)?,
            owners: address_array(&called(&owners)?, 0)?,
            threshold: small(word(&called(&threshold)?, 0)?)?,
            nonce: small(word(&called(&nonce)?, 0)?)?,
            modules: address_array(&modules, 0)?,
            more_modules: next != SENTINEL && next != [0; 20],
            guard: (guard != [0; 20]).then_some(guard),
            balance_wei: parse_hex_u128(text(&balance)?)?,
            code,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_answers_decode_strictly() {
        let mut owners = hex::decode(format!("{:064x}{:064x}", 32, 2)).unwrap();
        owners.extend([0u8; 12]);
        owners.extend([0x11; 20]);
        owners.extend([0u8; 12]);
        owners.extend([0x22; 20]);
        assert_eq!(
            address_array(&owners, 0).unwrap(),
            vec![[0x11; 20], [0x22; 20]]
        );
        assert!(address_array(&owners[..100], 0).is_err());
        let mut version = hex::decode(format!("{:064x}{:064x}", 32, 5)).unwrap();
        version.extend(b"1.4.1");
        version.extend([0u8; 27]);
        assert_eq!(string(&version).unwrap(), "1.4.1");
        let mut dirty = [0u8; 32];
        dirty[0] = 1;
        assert!(address(&dirty).is_err() && small(&dirty).is_err());
    }
}
