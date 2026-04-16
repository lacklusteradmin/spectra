use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StaticResourceRequest {
    pub resource_name: String,
}

#[derive(Clone, Copy)]
struct EmbeddedJsonResource {
    name: &'static str,
    json: &'static str,
}

#[derive(Clone, Copy)]
struct EmbeddedTextResource {
    name: &'static str,
    text: &'static str,
}

static STATIC_JSON_RESOURCES: &[EmbeddedJsonResource] = &[
    EmbeddedJsonResource {
        name: "TokenVisualRegistry",
        json: include_str!("../embedded/TokenVisualRegistry.json"),
    },
    EmbeddedJsonResource {
        name: "ChainVisualRegistry",
        json: include_str!("../embedded/ChainVisualRegistry.json"),
    },
    EmbeddedJsonResource {
        name: "BuyCryptoProviders",
        json: include_str!("../embedded/BuyCryptoProviders.json"),
    },
];

static STATIC_TEXT_RESOURCES: &[EmbeddedTextResource] = &[EmbeddedTextResource {
    name: "BIP39EnglishWordList",
    text: include_str!("../embedded/BIP39EnglishWordList.txt"),
}];

static JSON_RESOURCE_MAP: OnceLock<BTreeMap<String, &'static str>> = OnceLock::new();
static TEXT_RESOURCE_MAP: OnceLock<BTreeMap<String, &'static str>> = OnceLock::new();

pub fn static_json_resource(name: &str) -> Option<&'static str> {
    json_resource_map().get(name).copied()
}

pub fn static_text_resource(name: &str) -> Option<&'static str> {
    text_resource_map().get(name).copied()
}

fn json_resource_map() -> &'static BTreeMap<String, &'static str> {
    JSON_RESOURCE_MAP.get_or_init(|| {
        STATIC_JSON_RESOURCES
            .iter()
            .map(|resource| (resource.name.to_string(), resource.json))
            .collect()
    })
}

fn text_resource_map() -> &'static BTreeMap<String, &'static str> {
    TEXT_RESOURCE_MAP.get_or_init(|| {
        STATIC_TEXT_RESOURCES
            .iter()
            .map(|resource| (resource.name.to_string(), resource.text))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::{static_json_resource, static_text_resource};

    #[test]
    fn exposes_embedded_json_resources() {
        let json = static_json_resource("TokenVisualRegistry").expect("token visual registry");
        assert!(json.contains("\"USDT\""));
    }

    #[test]
    fn exposes_embedded_text_resources() {
        let text = static_text_resource("BIP39EnglishWordList").expect("bip39 word list");
        assert!(text.lines().any(|line| line == "abandon"));
    }
}
