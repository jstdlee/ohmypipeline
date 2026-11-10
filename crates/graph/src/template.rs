//! A versioned template for one node. Built-in features are not edited in place.
//!
//! Export writes a template. Customize stores a copy in the user library.
//! Import refuses a format or version it does not know.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::Node;

pub const TEMPLATE_FORMAT: &str = "ohmypipeline-template";
pub const TEMPLATE_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemplatePort {
    pub name: String,
    pub accepts: Vec<String>,
    #[serde(default)]
    pub multi: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemplateParam {
    pub name: String,
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default)]
    pub default: Value,
}

fn default_kind() -> String {
    "string".into()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeTemplate {
    pub format: String,
    pub version: u32,
    pub id: String,
    pub title: String,
    #[serde(default = "default_group")]
    pub group: String,
    /// Built-in feature this template started from, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default)]
    pub ports: Vec<TemplatePort>,
    #[serde(default)]
    pub params: Vec<TemplateParam>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

fn default_group() -> String {
    "Library".into()
}

impl NodeTemplate {
    pub fn from_node(node: &Node, id: impl Into<String>) -> Self {
        let ports = node
            .input_ports()
            .into_iter()
            .map(|port| TemplatePort {
                name: port.name,
                accepts: port
                    .accepts
                    .iter()
                    .map(|ty| ty.as_str().to_string())
                    .collect(),
                multi: port.multi,
            })
            .collect();
        let params = node
            .params
            .iter()
            .map(|(name, value)| TemplateParam {
                name: name.clone(),
                kind: "string".into(),
                default: value.clone(),
            })
            .collect();
        let code = node.param_str("code");
        Self {
            format: TEMPLATE_FORMAT.into(),
            version: TEMPLATE_VERSION,
            id: id.into(),
            title: node.label.clone(),
            group: "Library".into(),
            base: Some(node.feature.clone()),
            notes: node.notes.clone(),
            ports,
            params,
            code: if code.is_empty() { None } else { Some(code) },
            help: None,
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into())
    }
}

pub fn load_template(text: &str) -> Result<NodeTemplate, String> {
    let value: Value = serde_json::from_str(text).map_err(|err| err.to_string())?;
    let format = value
        .get("format")
        .and_then(|item| item.as_str())
        .unwrap_or("");
    if format != TEMPLATE_FORMAT {
        return Err(format!("expected format {TEMPLATE_FORMAT}, found {format}"));
    }
    let version = value
        .get("version")
        .and_then(|item| item.as_u64())
        .unwrap_or(0) as u32;
    if version != TEMPLATE_VERSION {
        return Err(format!("unknown version {version}"));
    }
    if value
        .get("id")
        .and_then(|item| item.as_str())
        .unwrap_or("")
        .is_empty()
    {
        return Err("template needs an id".into());
    }
    if value
        .get("title")
        .and_then(|item| item.as_str())
        .unwrap_or("")
        .is_empty()
    {
        return Err("template needs a title".into());
    }
    serde_json::from_value(value).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Point;
    use crate::model::Node;

    #[test]
    fn a_template_round_trips_and_an_unknown_version_is_refused() {
        let mut node = Node::new("n1", "python", Point::new(0.0, 0.0));
        node.label = "Keep the north rows".into();
        node.notes = Some("Drops the other regions.".into());
        let template = NodeTemplate::from_node(&node, "keep_north");
        let loaded = load_template(&template.to_json()).unwrap();
        assert_eq!(loaded.id, "keep_north");
        assert_eq!(loaded.base.as_deref(), Some("python"));
        assert_eq!(loaded.notes.as_deref(), Some("Drops the other regions."));
        let err =
            load_template(r#"{"format":"ohmypipeline-template","version":9,"id":"x","title":"X"}"#)
                .unwrap_err();
        assert_eq!(err, "unknown version 9");
    }
}
