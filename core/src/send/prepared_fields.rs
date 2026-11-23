//! A prepared transaction as rows: each field of `prepared_details` by its
//! name, with its exact value. The details panel lists these instead of the
//! JSON they come from, which led with the payload enum's variant tag.

use serde::de::{Deserialize, Deserializer, MapAccess, Visitor};
use serde_json::value::RawValue;

/// One field of a prepared transaction.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PreparedField {
    /// The field's path in the prepared transaction: `gas_limit`,
    /// `inputs.0.txid`. Empty when the details are not JSON and the value
    /// is the whole text.
    pub name: String,
    /// The value exactly as prepared: integers in the chain's base unit, a
    /// byte string as `0x` hex, `—` for nothing.
    pub value: String,
}

/// The fields of an artifact's `prepared_details`, in the order they were
/// prepared. Details that are not JSON come back as one unnamed field.
#[uniffi::export]
pub fn prepared_fields(prepared_details: String) -> Vec<PreparedField> {
    let Ok(mut node) =
        serde_json::from_str::<&RawValue>(&prepared_details).and_then(Node::from_raw)
    else {
        return vec![PreparedField {
            name: String::new(),
            value: prepared_details,
        }];
    };
    // The payload enum's tag (`{"Evm": {…}}`) names the family, which the
    // page already shows; its fields are what was prepared.
    while let Node::Map(entries) = &mut node
        && entries.len() == 1
        && matches!(entries[0].1, Node::Map(_))
    {
        node = entries.remove(0).1;
    }
    let mut fields = Vec::new();
    flatten(node, String::new(), &mut fields);
    fields
}

/// JSON as it was written: a record's keys in the order the transaction was
/// prepared in, and each number as its text. `serde_json::Value` sorts the
/// keys, and reads an integer past `u64` — a NEAR amount in yoctoNEAR, a wei
/// value — as a float.
enum Node {
    Map(Vec<(String, Node)>),
    List(Vec<Node>),
    Leaf(Leaf),
}

enum Leaf {
    /// The number exactly as written.
    Number(String),
    Text(String),
    Bool(bool),
    Null,
}

impl Node {
    fn from_raw(raw: &RawValue) -> Result<Self, serde_json::Error> {
        let text = raw.get().trim();
        Ok(match text.as_bytes().first() {
            Some(b'{') => Node::Map(
                serde_json::from_str::<Entries>(text)?
                    .0
                    .into_iter()
                    .map(|(key, value)| Ok((key, Node::from_raw(value)?)))
                    .collect::<Result<_, serde_json::Error>>()?,
            ),
            Some(b'[') => Node::List(
                serde_json::from_str::<Vec<&RawValue>>(text)?
                    .into_iter()
                    .map(Node::from_raw)
                    .collect::<Result<_, _>>()?,
            ),
            Some(b'"') => Node::Leaf(Leaf::Text(serde_json::from_str(text)?)),
            Some(b't' | b'f') => Node::Leaf(Leaf::Bool(serde_json::from_str(text)?)),
            Some(b'n') => Node::Leaf(Leaf::Null),
            // The whole document parsed as JSON, so the rest is a number.
            _ => Node::Leaf(Leaf::Number(text.to_string())),
        })
    }
}

/// A record's entries in the order they were written, each value unread.
struct Entries<'a>(Vec<(String, &'a RawValue)>);

impl<'de: 'a, 'a> Deserialize<'de> for Entries<'a> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct EntriesVisitor;
        impl<'de> Visitor<'de> for EntriesVisitor {
            type Value = Entries<'de>;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a JSON record")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Entries<'de>, A::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry()? {
                    entries.push(entry);
                }
                Ok(Entries(entries))
            }
        }
        deserializer.deserialize_map(EntriesVisitor)
    }
}

fn flatten(node: Node, path: String, fields: &mut Vec<PreparedField>) {
    let child = |key: &str| {
        if path.is_empty() {
            key.to_string()
        } else {
            format!("{path}.{key}")
        }
    };
    match node {
        Node::Map(entries) if !entries.is_empty() => {
            for (key, inner) in entries {
                flatten(inner, child(&key), fields);
            }
        }
        Node::List(items) if !items.is_empty() => match as_bytes(&items) {
            Some(bytes) => fields.push(PreparedField {
                name: path,
                value: format!("0x{}", hex::encode(bytes)),
            }),
            None => {
                for (index, inner) in items.into_iter().enumerate() {
                    flatten(inner, child(&index.to_string()), fields);
                }
            }
        },
        Node::Leaf(leaf) => fields.push(PreparedField {
            name: path,
            value: leaf_text(leaf),
        }),
        // An empty list or record: nothing prepared.
        Node::Map(_) | Node::List(_) => fields.push(PreparedField {
            name: path,
            value: "—".to_string(),
        }),
    }
}

/// A list of integers that all fit a byte is a byte string: calldata, a
/// script, a memo.
fn as_bytes(items: &[Node]) -> Option<Vec<u8>> {
    items
        .iter()
        .map(|item| match item {
            Node::Leaf(Leaf::Number(text)) => text.parse::<u8>().ok(),
            _ => None,
        })
        .collect()
}

fn leaf_text(leaf: Leaf) -> String {
    match leaf {
        Leaf::Text(text) if !text.is_empty() => text,
        Leaf::Number(text) => text,
        Leaf::Bool(flag) => flag.to_string(),
        // Null or an empty string: nothing prepared.
        Leaf::Text(_) | Leaf::Null => "—".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str, value: &str) -> PreparedField {
        PreparedField {
            name: name.into(),
            value: value.into(),
        }
    }

    #[test]
    fn an_evm_transfer_reads_as_its_fields_in_order() {
        let details = r#"{"Evm": {
            "chain_id": 11155111,
            "nonce": 3,
            "max_fee_per_gas": 1000000015,
            "to": "0x000000000000000000000000000000000000dead",
            "value_wei": 1000000000000000,
            "data": [],
            "access_list": []
        }}"#;
        assert_eq!(
            prepared_fields(details.into()),
            vec![
                field("chain_id", "11155111"),
                field("nonce", "3"),
                field("max_fee_per_gas", "1000000015"),
                field("to", "0x000000000000000000000000000000000000dead"),
                field("value_wei", "1000000000000000"),
                field("data", "—"),
                field("access_list", "—"),
            ]
        );
    }

    #[test]
    fn bytes_read_as_hex_and_lists_by_index() {
        let details = r#"{"Utxo": {
            "script": [169, 5, 0],
            "inputs": [{"txid": "ab", "vout": 1}, {"txid": "cd", "vout": 0}],
            "change": null,
            "rbf": true
        }}"#;
        assert_eq!(
            prepared_fields(details.into()),
            vec![
                field("script", "0xa90500"),
                field("inputs.0.txid", "ab"),
                field("inputs.0.vout", "1"),
                field("inputs.1.txid", "cd"),
                field("inputs.1.vout", "0"),
                field("change", "—"),
                field("rbf", "true"),
            ]
        );
    }

    #[test]
    fn details_that_are_not_json_come_back_whole() {
        assert_eq!(
            prepared_fields("Nonce: 7".into()),
            vec![field("", "Nonce: 7")]
        );
    }

    /// An integer past `u64` reads as written, not as a float: 1 NEAR is
    /// 10^24 yoctoNEAR, and a wei value can be as wide.
    #[test]
    fn integers_past_u64_read_exactly() {
        let near = 10u128.pow(24);
        let wei = u128::MAX;
        let details =
            format!(r#"{{"Near": {{"amount": {near}, "fee": 1.5}}, "value_wei": {wei}}}"#);
        assert_eq!(
            prepared_fields(details),
            vec![
                field("Near.amount", "1000000000000000000000000"),
                field("Near.fee", "1.5"),
                field("value_wei", &wei.to_string()),
            ]
        );
    }
}
