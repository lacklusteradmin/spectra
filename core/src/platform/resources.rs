use serde::Deserialize;
use std::collections::HashMap;
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

static STATIC_JSON_RESOURCES: &[EmbeddedJsonResource] = &[];

static STATIC_TEXT_RESOURCES: &[EmbeddedTextResource] = &[];

static JSON_RESOURCE_MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
static TEXT_RESOURCE_MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();

pub fn static_json_resource(name: &str) -> Option<&'static str> {
    json_resource_map().get(name).copied()
}

pub fn static_text_resource(name: &str) -> Option<&'static str> {
    text_resource_map().get(name).copied()
}

fn json_resource_map() -> &'static HashMap<&'static str, &'static str> {
    JSON_RESOURCE_MAP.get_or_init(|| {
        STATIC_JSON_RESOURCES
            .iter()
            .map(|resource| (resource.name, resource.json))
            .collect()
    })
}

fn text_resource_map() -> &'static HashMap<&'static str, &'static str> {
    TEXT_RESOURCE_MAP.get_or_init(|| {
        STATIC_TEXT_RESOURCES
            .iter()
            .map(|resource| (resource.name, resource.text))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::{static_json_resource, static_text_resource};

    #[test]
    fn missing_json_resource_returns_none() {
        assert!(static_json_resource("DoesNotExist").is_none());
    }

    #[test]
    fn missing_text_resource_returns_none() {
        assert!(static_text_resource("DoesNotExist").is_none());
    }
}
