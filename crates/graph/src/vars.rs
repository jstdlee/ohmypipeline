//! Flow variables: `${name}` in params, typed values, and CLI overrides.
//! Substitution itself runs in the worker; this module answers what a node
//! reads and validates `--set` before a job is ever spawned.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::feature::ParamKind;
use crate::model::{Node, VarKind, Variable};

/// Names a node reads out of its string params, recursively through arrays
/// and objects. Code params never hold `${}` text the user means literally,
/// so they are skipped.
pub fn referenced_vars(node: &Node) -> BTreeSet<String> {
    let skip_code: bool = node
        .def()
        .map(|def| {
            def.params
                .iter()
                .any(|param| matches!(param.kind, ParamKind::Code))
        })
        .unwrap_or(node.feature == "python");
    let mut refs = BTreeSet::new();
    for (name, value) in &node.params {
        if skip_code && name == "code" {
            continue;
        }
        scan_value(value, &mut refs);
    }
    refs
}

fn scan_value(value: &Value, refs: &mut BTreeSet<String>) {
    match value {
        Value::String(text) => scan_refs(text, refs),
        Value::Array(items) => {
            for item in items {
                scan_value(item, refs);
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                scan_value(item, refs);
            }
        }
        _ => {}
    }
}

fn scan_refs(text: &str, refs: &mut BTreeSet<String>) {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'$' {
            index += 1;
            continue;
        }
        if text[index..].starts_with("$${") {
            index += 3;
            continue;
        }
        if !text[index..].starts_with("${") {
            index += 1;
            continue;
        }
        let Some(close) = text[index + 2..].find('}') else {
            break;
        };
        let name = &text[index + 2..index + 2 + close];
        if is_ident(name) {
            refs.insert(name.to_string());
        }
        index += 2 + close + 1;
    }
}

fn is_ident(name: &str) -> bool {
    crate::file::is_variable_name(name)
}

/// One slice of a parameter string. `variable` is set when the slice is a
/// `${name}` reference the user wrote, not a `$${` escape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextPiece {
    pub text: String,
    pub variable: Option<String>,
}

/// Splits `text` into literal runs and `${name}` references. `$${` becomes a
/// literal `${` and is not a reference.
pub fn split_references(text: &str) -> Vec<TextPiece> {
    let mut pieces = Vec::new();
    let mut literal = String::new();
    let mut rest = text;
    let push_literal = |literal: &mut String, pieces: &mut Vec<TextPiece>| {
        if !literal.is_empty() {
            pieces.push(TextPiece {
                text: std::mem::take(literal),
                variable: None,
            });
        }
    };
    loop {
        let Some(index) = rest.find('$') else {
            literal.push_str(rest);
            break;
        };
        literal.push_str(&rest[..index]);
        let tail = &rest[index..];
        if let Some(after) = tail.strip_prefix("$${") {
            literal.push_str("${");
            rest = after;
            continue;
        }
        if !tail.starts_with("${") {
            literal.push('$');
            rest = &tail[1..];
            continue;
        }
        let Some(close) = tail[2..].find('}') else {
            literal.push_str(tail);
            break;
        };
        let name = &tail[2..2 + close];
        if !is_ident(name) {
            literal.push_str("${");
            rest = &tail[2..];
            continue;
        }
        push_literal(&mut literal, &mut pieces);
        pieces.push(TextPiece {
            text: format!("${{{name}}}"),
            variable: Some(name.to_string()),
        });
        rest = &tail[2 + close + 1..];
    }
    push_literal(&mut literal, &mut pieces);
    pieces
}

/// Names a `${}` reference can resolve: declared variables, plus `loop_index`
/// and each loop start's own name.
pub fn bound_names(graph: &crate::model::Graph) -> BTreeSet<String> {
    let mut names: BTreeSet<String> = graph
        .variables
        .iter()
        .map(|variable| variable.name.clone())
        .collect();
    for node in &graph.nodes {
        if node.feature != "loop_start" {
            continue;
        }
        let name = node.param_str("name");
        names.insert(if name.is_empty() {
            "loop_value".to_string()
        } else {
            name
        });
        names.insert("loop_index".to_string());
    }
    names
}

/// Text form of a variable value, for an input that edits it.
pub fn value_text_of(value: &Value) -> String {
    value_text(value)
}

/// Replaces `${name}` with the value as text. `$${` escapes to a literal `${`.
/// When the whole string is exactly `${name}`, the typed value comes back.
pub fn substitute(text: &str, vars: &BTreeMap<String, Value>) -> Result<Value, String> {
    if let Some(name) = exact_ref(text) {
        return vars
            .get(name)
            .cloned()
            .ok_or_else(|| format!("unknown variable ${{{name}}}"));
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let Some(index) = rest.find('$') else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..index]);
        let tail = &rest[index..];
        if let Some(after) = tail.strip_prefix("$${") {
            out.push_str("${");
            rest = after;
            continue;
        }
        if !tail.starts_with("${") {
            out.push('$');
            rest = &tail[1..];
            continue;
        }
        let Some(close) = tail[2..].find('}') else {
            out.push_str(tail);
            break;
        };
        let name = &tail[2..2 + close];
        if !is_ident(name) {
            out.push_str("${");
            rest = &tail[2..];
            continue;
        }
        let value = vars
            .get(name)
            .ok_or_else(|| format!("unknown variable ${{{name}}}"))?;
        out.push_str(&value_text(value));
        rest = &tail[2 + close + 1..];
    }
    Ok(Value::String(out))
}

fn exact_ref(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("${")?;
    let name = rest.strip_suffix('}')?;
    if is_ident(name) { Some(name) } else { None }
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Parses CLI text for a declared kind. Kept symmetric with the worker's
/// parser so `--set` is validated exactly once in Rust and once there.
pub fn parse_value(kind: VarKind, text: &str) -> Result<Value, String> {
    match kind {
        VarKind::String | VarKind::Path => Ok(json!(text)),
        VarKind::Int => text
            .parse::<i64>()
            .map(Value::from)
            .map_err(|_| format!("\"{text}\" is not an int")),
        VarKind::Float => text
            .parse::<f64>()
            .ok()
            .filter(|number| number.is_finite())
            .map(|number| Value::Number(serde_json::Number::from_f64(number).unwrap()))
            .ok_or_else(|| format!("\"{text}\" is not a float")),
        VarKind::Bool => match text.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Ok(Value::Bool(true)),
            "false" | "0" | "no" => Ok(Value::Bool(false)),
            _ => Err(format!("\"{text}\" is not a bool")),
        },
    }
}

/// Replaces the values of the declared variables with the CLI texts. Unknown
/// names are refused with the list of names the pipeline knows.
pub fn apply_overrides(
    vars: &[Variable],
    overrides: &BTreeMap<String, String>,
) -> Result<Vec<Variable>, String> {
    let mut out = vars.to_vec();
    for (name, text) in overrides {
        let Some(variable) = out.iter_mut().find(|variable| variable.name == *name) else {
            let known: Vec<&str> = vars.iter().map(|variable| variable.name.as_str()).collect();
            let known = if known.is_empty() {
                "the pipeline has no variables".to_string()
            } else {
                format!("known: {}", known.join(", "))
            };
            return Err(format!("unknown variable {name}. {known}"));
        };
        variable.value = parse_value(variable.kind, text).map_err(|message| {
            format!(
                "variable {name} is not a valid {}: {message}",
                kind_name(variable.kind)
            )
        })?;
    }
    Ok(out)
}

fn kind_name(kind: VarKind) -> &'static str {
    match kind {
        VarKind::String => "string",
        VarKind::Int => "int",
        VarKind::Float => "float",
        VarKind::Bool => "bool",
        VarKind::Path => "path",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dependency_key;
    use crate::geom::Point;
    use crate::model::{Edge, Graph};
    use serde_json::json;

    fn vars(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.clone()))
            .collect()
    }

    #[test]
    fn substitution_replaces_escapes_and_keeps_literals() {
        let vars = vars(&[("name", json!("Ada")), ("rows", json!(3))]);
        assert_eq!(
            substitute("hello ${name}", &vars).unwrap(),
            Value::String("hello Ada".into())
        );
        assert_eq!(substitute("${rows}", &vars).unwrap(), json!(3), "typed");
        assert_eq!(
            substitute("a $${name} b", &vars).unwrap(),
            Value::String("a ${name} b".into())
        );
        assert_eq!(
            substitute("${name} $${rows} plain", &vars).unwrap(),
            Value::String("Ada ${rows} plain".into())
        );
        // A non-identifier brace stays literal.
        assert_eq!(
            substitute("${not a var}", &vars).unwrap(),
            Value::String("${not a var}".into())
        );
        assert!(substitute("${missing}", &vars).is_err());
        assert!(
            substitute("${missing}", &vars)
                .unwrap_err()
                .contains("unknown variable ${missing}")
        );
    }

    #[test]
    fn parse_value_matches_the_declared_kind() {
        assert_eq!(parse_value(VarKind::Int, "12").unwrap(), json!(12));
        assert!(parse_value(VarKind::Int, "two").is_err());
        assert_eq!(parse_value(VarKind::Float, "0.5").unwrap(), json!(0.5));
        assert_eq!(
            parse_value(VarKind::Bool, "true").unwrap(),
            Value::Bool(true)
        );
        assert!(parse_value(VarKind::Bool, "maybe").is_err());
        assert_eq!(
            parse_value(VarKind::Path, "orders.csv").unwrap(),
            json!("orders.csv")
        );
    }

    #[test]
    fn apply_overrides_names_unknown_variables() {
        let declared = vec![
            Variable {
                name: "rows".into(),
                kind: VarKind::Int,
                value: json!(3),
            },
            Variable {
                name: "data".into(),
                kind: VarKind::Path,
                value: json!("orders.csv"),
            },
        ];
        let mut set = BTreeMap::new();
        set.insert("rows".to_string(), "5".to_string());
        let applied = apply_overrides(&declared, &set).unwrap();
        assert_eq!(applied[0].value, json!(5));
        assert_eq!(applied[0].kind, VarKind::Int);
        assert_eq!(applied[1].value, json!("orders.csv"), "declared untouched");
        let mut unknown = BTreeMap::new();
        unknown.insert("nope".to_string(), "1".to_string());
        let message = apply_overrides(&declared, &unknown).unwrap_err();
        assert!(message.contains("unknown variable nope"), "{message}");
        assert!(message.contains("rows"), "{message}");
        let mut bad = BTreeMap::new();
        bad.insert("rows".to_string(), "two".to_string());
        assert!(
            apply_overrides(&declared, &bad)
                .unwrap_err()
                .contains("rows")
        );
    }

    #[test]
    fn referenced_vars_scans_params_but_not_code() {
        let mut python = Node::new("p", "python", Point::new(0.0, 0.0));
        python.params.insert("code".into(), json!("x = ${rows}\n"));
        python.params.insert("notes".into(), json!("use ${rows}"));
        assert_eq!(
            referenced_vars(&python),
            BTreeSet::from(["rows".to_string()])
        );
        let mut reader = Node::new("r", "read_csv", Point::new(0.0, 0.0));
        reader.params.insert("path".into(), json!("${data}"));
        reader
            .params
            .insert("mapping".into(), json!([{"from": "${a}", "to": "b"}]));
        assert_eq!(
            referenced_vars(&reader),
            BTreeSet::from(["a".to_string(), "data".to_string()])
        );
    }

    #[test]
    fn the_dependency_key_changes_only_for_users_of_the_variable() {
        let mut graph = Graph::default();
        let mut user = Node::new("n1", "read_csv", Point::new(0.0, 0.0));
        user.params.insert("path".into(), json!("${data}"));
        let mut plain = Node::new("m1", "read_csv", Point::new(0.0, 0.0));
        plain.params.insert("path".into(), json!("orders.csv"));
        let head_user = Node::new("n2", "head", Point::new(0.0, 0.0));
        let head_plain = Node::new("m2", "head", Point::new(0.0, 0.0));
        graph.nodes = vec![user, plain, head_user, head_plain];
        graph.edges = vec![
            Edge::between("n1", "out", "n2", "in"),
            Edge::between("m1", "out", "m2", "in"),
        ];
        let variable = Variable {
            name: "data".into(),
            kind: VarKind::Path,
            value: json!("orders.csv"),
        };
        graph.variables.push(variable.clone());
        let plain_before = dependency_key(&graph, "m2");
        let user_before = dependency_key(&graph, "n2");
        graph.variables[0].value = json!("other.csv");
        assert_ne!(dependency_key(&graph, "n2"), user_before);
        assert_eq!(dependency_key(&graph, "m2"), plain_before);
    }

    #[test]
    fn split_references_keeps_literals_and_escapes() {
        let pieces = split_references("hello ${name} and $${rows}");
        assert_eq!(
            pieces,
            vec![
                TextPiece {
                    text: "hello ".into(),
                    variable: None,
                },
                TextPiece {
                    text: "${name}".into(),
                    variable: Some("name".into()),
                },
                TextPiece {
                    text: " and ${rows}".into(),
                    variable: None,
                },
            ]
        );
        assert!(split_references("${not a var}")[0].variable.is_none());
    }
}
