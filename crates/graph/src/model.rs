use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::feature::{self, FeatureDef};
use crate::geom::{Camera, Point, Rect};
use crate::types::TypeName;

pub const HEADER_PAIRS: &str = "mapping";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PortRecord {
    pub name: String,
    pub accepts: Vec<TypeName>,
    #[serde(default)]
    pub multi: bool,
}

impl PortRecord {
    pub fn from_def(def: &crate::feature::PortDef) -> Self {
        Self {
            name: def.name.to_string(),
            accepts: def.accepts.to_vec(),
            multi: def.multi,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Anchor {
    #[default]
    Top,
    TopStart,
    TopEnd,
    Start,
    Center,
    End,
    Bottom,
    BottomStart,
    BottomEnd,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Placed {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub x: f32,
    pub y: f32,
    pub anchor: Anchor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Presentation {
    pub title: Placed,
    pub legend: Placed,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_series: Vec<String>,
}

impl Default for Presentation {
    fn default() -> Self {
        Self {
            title: Placed {
                text: Some(String::new()),
                x: 0.5,
                y: 0.04,
                anchor: Anchor::Top,
            },
            legend: Placed {
                text: None,
                x: 1.0,
                y: 0.04,
                anchor: Anchor::TopEnd,
            },
            hidden_series: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    #[serde(rename = "type")]
    pub feature: String,
    pub label: String,
    #[serde(serialize_with = "ser_point", deserialize_with = "de_point")]
    pub position: [f32; 2],
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
    /// Replaces the feature input list. Used by `python`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<Vec<PortRecord>>,
    /// Ports appended by Add input, on this node only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added_inputs: Vec<PortRecord>,
    /// Wireable parameters that currently show a port. The literal stays.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub wired: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<PortRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation: Option<Presentation>,
    /// A note the user wrote on this node. The worker ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Fitted scaler or regression parameters. Not part of the run fingerprint,
    /// so saving a preview fit does not turn Full off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fitted: Option<Value>,
    /// Set on load when `feature` is not in the catalog. Not written back;
    /// the unknown type string is what round-trips.
    #[serde(skip)]
    pub unknown: bool,
}

fn ser_point<S: serde::Serializer>(point: &[f32; 2], serializer: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(Some(2))?;
    seq.serialize_element(&JsonNum(point[0]))?;
    seq.serialize_element(&JsonNum(point[1]))?;
    seq.end()
}

fn de_point<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<[f32; 2], D::Error> {
    let pair = <[f32; 2]>::deserialize(deserializer)?;
    Ok(pair)
}

struct JsonNum(f32);

impl Serialize for JsonNum {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.0.is_finite() && self.0.fract() == 0.0 && self.0.abs() < 1.0e15 {
            serializer.serialize_i64(self.0 as i64)
        } else {
            serializer.serialize_f32(self.0)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    pub from: String,
    pub from_port: String,
    pub to: String,
    pub to_port: String,
}

impl Edge {
    pub fn between(from: &str, from_port: &str, to: &str, to_port: &str) -> Self {
        Self {
            id: format!("{from}:{from_port}->{to}:{to_port}"),
            from: from.to_string(),
            from_port: from_port.to_string(),
            to: to.to_string(),
            to_port: to_port.to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkKind {
    Label,
    Note,
    Button,
    Rectangle,
    Circle,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    pub id: String,
    pub kind: MarkKind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(serialize_with = "ser_point", deserialize_with = "de_point")]
    pub position: [f32; 2],
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "ser_size",
        deserialize_with = "de_size"
    )]
    pub size: Option<[f32; 2]>,
    #[serde(default = "neutral_swatch")]
    pub swatch: String,
    /// Button only. `None` means the button does nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
}

fn neutral_swatch() -> String {
    "neutral".to_string()
}

fn ser_size<S: serde::Serializer>(
    size: &Option<[f32; 2]>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match size {
        Some(point) => ser_point(point, serializer),
        None => serializer.serialize_none(),
    }
}

fn de_size<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<[f32; 2]>, D::Error> {
    Option::<[f32; 2]>::deserialize(deserializer)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Graph {
    #[serde(default = "default_format")]
    pub format: String,
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    #[serde(default)]
    pub marks: Vec<Mark>,
    #[serde(default)]
    pub camera: Camera,
}

fn default_format() -> String {
    crate::FORMAT.to_string()
}

fn default_version() -> u32 {
    crate::VERSION
}

impl Default for Graph {
    fn default() -> Self {
        Self {
            format: default_format(),
            version: default_version(),
            nodes: Vec::new(),
            edges: Vec::new(),
            marks: Vec::new(),
            camera: Camera::default(),
        }
    }
}

impl Graph {
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|node| node.id == id)
    }

    pub fn node_mut(&mut self, id: &str) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|node| node.id == id)
    }

    pub fn mark(&self, id: &str) -> Option<&Mark> {
        self.marks.iter().find(|mark| mark.id == id)
    }

    pub fn edges_into<'a>(
        &'a self,
        node: &'a str,
        port: &'a str,
    ) -> impl Iterator<Item = &'a Edge> {
        self.edges
            .iter()
            .filter(move |edge| edge.to == node && edge.to_port == port)
    }

    pub fn alloc_node_id(&self) -> String {
        alloc_id(self.nodes.iter().map(|node| node.id.as_str()), 'n')
    }

    pub fn alloc_mark_id(&self) -> String {
        alloc_id(self.marks.iter().map(|mark| mark.id.as_str()), 'm')
    }
}

fn alloc_id<'a>(ids: impl Iterator<Item = &'a str>, prefix: char) -> String {
    let mut max = 0usize;
    for id in ids {
        if let Some(Ok(n)) = id.strip_prefix(prefix).map(str::parse::<usize>) {
            max = max.max(n);
        }
    }
    format!("{prefix}{}", max + 1)
}

impl Node {
    pub fn new(id: impl Into<String>, feature_id: &str, position: Point) -> Self {
        let def = feature::feature(feature_id);
        let mut params = BTreeMap::new();
        if let Some(def) = def {
            for param in def.params {
                if let Ok(value) = serde_json::from_str::<Value>(param.default_json) {
                    params.insert(param.name.to_string(), value);
                }
            }
        }
        let label = def
            .map(|def| def.title.to_string())
            .unwrap_or_else(|| feature_id.to_string());
        let mut node = Self {
            id: id.into(),
            feature: feature_id.to_string(),
            label,
            position: [position.x, position.y],
            params,
            inputs: None,
            added_inputs: Vec::new(),
            wired: BTreeSet::new(),
            output: None,
            presentation: None,
            notes: None,
            fitted: None,
            unknown: def.is_none(),
        };
        if feature_id == "python" {
            node.inputs = Some(vec![PortRecord {
                name: "df".to_string(),
                accepts: vec![TypeName::Dataframe],
                multi: false,
            }]);
            node.output = Some(PortRecord {
                name: "out".to_string(),
                accepts: vec![TypeName::Dataframe],
                multi: false,
            });
        }
        if feature_id == "chart" {
            node.presentation = Some(Presentation::default());
        }
        node
    }

    pub fn def(&self) -> Option<&'static FeatureDef> {
        feature::feature(&self.feature)
    }

    pub fn position_point(&self) -> Point {
        Point::new(self.position[0], self.position[1])
    }

    pub fn param_str(&self, name: &str) -> String {
        match self.params.get(name) {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Number(n)) => n.to_string(),
            Some(Value::Bool(v)) => v.to_string(),
            Some(other) => other.to_string(),
            None => String::new(),
        }
    }

    pub fn param_i64(&self, name: &str) -> Option<i64> {
        self.params.get(name).and_then(|value| match value {
            Value::Number(n) => n.as_i64(),
            Value::String(text) => text.parse().ok(),
            _ => None,
        })
    }

    pub fn param_bool(&self, name: &str) -> bool {
        match self.params.get(name) {
            Some(Value::Bool(v)) => *v,
            Some(Value::String(text)) => text == "true",
            _ => false,
        }
    }

    /// Input ports in card order.
    pub fn input_ports(&self) -> Vec<PortRecord> {
        let mut ports = if let Some(custom) = &self.inputs {
            custom.clone()
        } else if let Some(def) = self.def() {
            def.inputs.iter().map(PortRecord::from_def).collect()
        } else {
            Vec::new()
        };
        ports.extend(self.added_inputs.clone());
        if let Some(def) = self.def() {
            for param in def.params.iter().filter(|param| param.wireable) {
                if self.wired.contains(param.name) {
                    ports.push(PortRecord {
                        name: param.name.to_string(),
                        accepts: param.wire_types.to_vec(),
                        multi: false,
                    });
                }
            }
        }
        ports
    }

    pub fn output_port(&self) -> Option<PortRecord> {
        if self.feature == "chart" || self.unknown {
            return None;
        }
        if let Some(output) = &self.output {
            return Some(output.clone());
        }
        let def = self.def()?;
        if def.possible_outputs.is_empty() && def.output.is_none() {
            return None;
        }
        let accepts = if def.possible_outputs.is_empty() {
            def.output.into_iter().collect()
        } else {
            def.possible_outputs.to_vec()
        };
        Some(PortRecord {
            name: def.output_name.to_string(),
            accepts,
            multi: false,
        })
    }

    pub fn summary(&self) -> String {
        if self.unknown {
            return format!("unknown feature {}", self.feature);
        }
        match self.feature.as_str() {
            "read_csv" | "read_excel" | "read_sql" | "to_csv" | "to_excel" | "to_json"
            | "to_parquet" | "to_sql" => {
                let path = self.param_str("path");
                if path.is_empty() {
                    "no file".to_string()
                } else {
                    std::path::Path::new(&path)
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .filter(|name| !name.is_empty())
                        .unwrap_or(path)
                }
            }
            "head" => format!("n={}", self.param_i64("n").unwrap_or(5)),
            "change_headers" => mapping_summary(self.params.get(HEADER_PAIRS)),
            "query" => {
                let expr = self.param_str("expr");
                if expr.is_empty() {
                    "no expression".to_string()
                } else {
                    expr
                }
            }
            "column" => {
                let name = self.param_str("name");
                if name.is_empty() {
                    "no column".to_string()
                } else {
                    name
                }
            }
            "literal" => {
                let kind = self.param_str("kind");
                let value = self.param_str("value");
                if value.is_empty() {
                    kind
                } else {
                    format!("{kind} {value}")
                }
            }
            "python" => "code block".to_string(),
            "chart" => {
                let mark = self.param_str("mark");
                let x = self.param_str("x");
                let y = self.param_str("y");
                if x.is_empty() && y.is_empty() {
                    mark
                } else if mark == "pie" {
                    format!("pie {x} · {y}")
                } else {
                    format!("{mark} {x} · {y}")
                }
            }
            "pivot" => {
                let index = self.param_str("index");
                let columns = self.param_str("columns");
                let values = self.param_str("values");
                format!("{index} × {columns} · {values}")
            }
            "preview_table" => "table".to_string(),
            "str" => format!("{} {}", self.param_str("method"), self.param_str("column")),
            "dt" => format!(
                "{} {}",
                self.param_str("accessor"),
                self.param_str("column")
            ),
            "to_datetime" | "to_numeric" => self.param_str("column"),
            "isin" => self.param_str("values"),
            "groupby" => {
                let aggs = self.param_str("aggs");
                if aggs.is_empty() {
                    format!("{} {}", self.param_str("by"), self.param_str("agg"))
                } else {
                    format!("{} {aggs}", self.param_str("by"))
                }
            }
            "rolling" => format!("window {}", self.param_i64("window").unwrap_or(3)),
            "standard_scale" | "minmax_scale" | "robust_scale" | "ols" => self
                .fitted
                .as_ref()
                .and_then(|fitted| fitted.get("source"))
                .and_then(|source| source.as_str())
                .map(|source| match source {
                    "full" => "fit on the full file".to_string(),
                    _ => "fit on the preview".to_string(),
                })
                .unwrap_or_else(|| "not fit yet".to_string()),
            "calculate" => {
                let expr = self.param_str("expr");
                if expr.is_empty() {
                    "no expression".to_string()
                } else {
                    expr.lines().next().unwrap_or("").to_string()
                }
            }
            "crosstab" => format!(
                "{} × {}",
                self.param_str("index"),
                self.param_str("columns")
            ),
            "ffill" => "forward".to_string(),
            "bfill" => "back".to_string(),
            "clip" => {
                let lower = self.param_str("lower");
                let upper = self.param_str("upper");
                if lower.is_empty() && upper.is_empty() {
                    "clip".to_string()
                } else {
                    format!("{lower}–{upper}")
                }
            }
            "loc" | "iloc" => {
                let rows = self.param_str("rows");
                if rows.is_empty() {
                    self.feature.clone()
                } else {
                    rows
                }
            }
            "sort_values" => {
                let by = self.param_str("by");
                if by.is_empty() {
                    "sort".to_string()
                } else {
                    format!("by {by}")
                }
            }
            "merge" => format!("how={}", self.param_str("how")),
            "concat" => "concat".to_string(),
            "to_frame" => "series to frame".to_string(),
            _ => self.feature.clone(),
        }
    }
}

pub fn mapping_summary(value: Option<&Value>) -> String {
    let Some(Value::Array(pairs)) = value else {
        return "rename".to_string();
    };
    let shown: Vec<String> = pairs
        .iter()
        .filter_map(|pair| {
            let from = pair.get("from")?.as_str()?;
            let to = pair.get("to").and_then(|v| v.as_str()).unwrap_or("");
            if from.is_empty() {
                return None;
            }
            if to.is_empty() {
                Some(from.to_string())
            } else {
                Some(format!("{from} → {to}"))
            }
        })
        .take(3)
        .collect();
    if shown.is_empty() {
        "rename".to_string()
    } else {
        shown.join(", ")
    }
}

pub fn header_pairs(value: &Value) -> Vec<(String, String)> {
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                let from = item.get("from")?.as_str()?.to_string();
                let to = item
                    .get("to")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Some((from, to))
            })
            .collect(),
        Value::Object(map) => map
            .iter()
            .map(|(from, to)| {
                let to = match to {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                };
                (from.clone(), to)
            })
            .collect(),
        _ => Vec::new(),
    }
}

pub fn duplicate_headers(pairs: &[(String, String)]) -> Option<String> {
    let mut seen = BTreeSet::new();
    for (from, to) in pairs {
        let new_name = if to.is_empty() { from } else { to };
        if new_name.is_empty() {
            continue;
        }
        if !seen.insert(new_name.clone()) {
            return Some(new_name.clone());
        }
    }
    None
}

/// Nodes with no outgoing edge. Chart nodes count: they are sinks.
pub fn sinks(graph: &Graph) -> Vec<String> {
    graph
        .nodes
        .iter()
        .filter(|node| !graph.edges.iter().any(|edge| edge.from == node.id))
        .map(|node| node.id.clone())
        .collect()
}

/// Ancestors of `id`, including `id`, in an order parents come before children.
pub fn ancestors(graph: &Graph, id: &str) -> Result<Vec<String>, String> {
    let mut order = Vec::new();
    let mut seen = HashSet::new();
    fn walk(
        graph: &Graph,
        id: &str,
        seen: &mut HashSet<String>,
        stack: &mut HashSet<String>,
        order: &mut Vec<String>,
    ) -> Result<(), String> {
        if !stack.insert(id.to_string()) {
            return Err(format!("cycle through {id}"));
        }
        if seen.insert(id.to_string()) {
            let parents: Vec<String> = graph
                .edges
                .iter()
                .filter(|edge| edge.to == id)
                .map(|edge| edge.from.clone())
                .collect();
            for parent in parents {
                walk(graph, &parent, seen, stack, order)?;
            }
            order.push(id.to_string());
        }
        stack.remove(id);
        Ok(())
    }
    let mut stack = HashSet::new();
    walk(graph, id, &mut seen, &mut stack, &mut order)?;
    Ok(order)
}

fn hash_json(value: &impl Serialize) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let text = serde_json::to_string(value).unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Identity of the computation at `id`. Positions, labels, marks, and the camera are not part of it.
pub fn dependency_key(graph: &Graph, id: &str) -> u64 {
    fn rec(graph: &Graph, id: &str, parts: &mut Vec<String>, seen: &mut HashSet<String>) {
        if !seen.insert(id.to_string()) {
            return;
        }
        let mut parents: Vec<&Edge> = graph.edges.iter().filter(|edge| edge.to == id).collect();
        parents.sort_by(|a, b| (&a.from, &a.to_port).cmp(&(&b.from, &b.to_port)));
        for edge in parents {
            rec(graph, &edge.from, parts, seen);
            parts.push(format!(
                "{}:{}->{}:{}",
                edge.from, edge.from_port, edge.to, edge.to_port
            ));
        }
        if let Some(node) = graph.node(id) {
            parts.push(format!(
                "{}|{}|{}|{}|{}|{}|{}",
                node.feature,
                serde_json::to_string(&node.params).unwrap_or_default(),
                serde_json::to_string(&node.inputs).unwrap_or_default(),
                serde_json::to_string(&node.added_inputs).unwrap_or_default(),
                serde_json::to_string(&node.wired).unwrap_or_default(),
                serde_json::to_string(&node.output).unwrap_or_default(),
                node.unknown
            ));
        }
    }
    let mut parts = Vec::new();
    rec(graph, id, &mut parts, &mut HashSet::new());
    hash_json(&parts)
}

/// Fingerprint of everything that changes a run. A successful preview is valid only while this matches.
pub fn run_fingerprint(graph: &Graph) -> u64 {
    let nodes: Vec<_> = graph
        .nodes
        .iter()
        .map(|node| {
            json!({
                "id": node.id,
                "type": node.feature,
                "params": node.params,
                "inputs": node.inputs,
                "added_inputs": node.added_inputs,
                "wired": node.wired,
                "output": node.output,
            })
        })
        .collect();
    let edges: Vec<_> = graph
        .edges
        .iter()
        .map(|edge| json!([edge.from, edge.from_port, edge.to, edge.to_port]))
        .collect();
    hash_json(&json!({ "nodes": nodes, "edges": edges }))
}

pub fn bounds_of(
    graph: &Graph,
    ids: &[String],
    card_height: impl Fn(&Node) -> f32,
) -> Option<Rect> {
    let mut rect: Option<Rect> = None;
    for node in &graph.nodes {
        if !ids.is_empty() && !ids.iter().any(|id| id == &node.id) {
            continue;
        }
        let r = Rect {
            x: node.position[0],
            y: node.position[1],
            w: crate::card_width(&node.feature),
            h: card_height(node),
        };
        rect = Some(match rect {
            Some(prev) => prev.union(r),
            None => r,
        });
    }
    for mark in &graph.marks {
        if !ids.is_empty() && !ids.iter().any(|id| id == &mark.id) {
            continue;
        }
        let size = mark.size.unwrap_or([120.0, 32.0]);
        let r = Rect {
            x: mark.position[0],
            y: mark.position[1],
            w: size[0],
            h: size[1],
        };
        rect = Some(match rect {
            Some(prev) => prev.union(r),
            None => r,
        });
    }
    rect
}

pub fn literal_type(node: &Node) -> TypeName {
    match node.param_str("kind").as_str() {
        "list" => TypeName::List,
        "dict" => TypeName::Dict,
        "code" => TypeName::Code,
        _ => TypeName::Scalar,
    }
}

pub fn empty_mapping() -> Value {
    json!([])
}

/// Argument names of `def run(...)`. Empty when the source has no such function.
pub fn python_arg_names(code: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in code.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("def run(") else {
            continue;
        };
        let Some(args) = rest.split(')').next() else {
            continue;
        };
        names.clear();
        for arg in args.split(',') {
            let arg = arg.trim();
            if arg.is_empty() {
                continue;
            }
            let name = arg.split(':').next().unwrap_or(arg).trim();
            let name = name.split('=').next().unwrap_or(name).trim();
            if !name.is_empty() && name != "self" {
                names.push(name.to_string());
            }
        }
    }
    names
}
