//! A prepared transaction as rows: each field of `prepared_details` by its
//! name, with its exact value. The details panel lists these instead of the
//! JSON they come from, which led with the payload enum's variant tag.

use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::Value;

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
    let Ok(mut node) = serde_json::from_str::<Node>(&prepared_details) else {
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

/// JSON as it was written. `serde_json::Value` sorts a record's keys; the
/// rows keep the order the transaction was prepared in.
enum Node {
    Map(Vec<(String, Node)>),
    List(Vec<Node>),
    Leaf(Value),
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(NodeVisitor)
    }
}

struct NodeVisitor;

impl<'de> Visitor<'de> for NodeVisitor {
    type Value = Node;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("JSON")
    }
    fn visit_bool<E>(self, value: bool) -> Result<Node, E> {
        Ok(Node::Leaf(Value::from(value)))
    }
    fn visit_i64<E>(self, value: i64) -> Result<Node, E> {
        Ok(Node::Leaf(Value::from(value)))
    }
    fn visit_u64<E>(self, value: u64) -> Result<Node, E> {
        Ok(Node::Leaf(Value::from(value)))
    }
    fn visit_f64<E>(self, value: f64) -> Result<Node, E> {
        Ok(Node::Leaf(Value::from(value)))
    }
    fn visit_str<E>(self, value: &str) -> Result<Node, E> {
        Ok(Node::Leaf(Value::from(value)))
    }
    fn visit_unit<E>(self) -> Result<Node, E> {
        Ok(Node::Leaf(Value::Null))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Node, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(Node::List(items))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Node, A::Error> {
        let mut entries = Vec::new();
        while let Some(entry) = map.next_entry()? {
            entries.push(entry);
        }
        Ok(Node::Map(entries))
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
        Node::Leaf(value) => fields.push(PreparedField {
            name: path,
            value: leaf_text(&value),
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
            Node::Leaf(value) => value.as_u64().and_then(|n| u8::try_from(n).ok()),
            _ => None,
        })
        .collect()
}

fn leaf_text(value: &Value) -> String {
    match value {
        Value::String(text) if !text.is_empty() => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        // Null or an empty string: nothing prepared.
        _ => "—".to_string(),
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
}
