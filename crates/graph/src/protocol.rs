use std::io::{Read, Write};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::Graph;

/// Readers stop here in preview mode. The rest of the pipeline runs on that input.
pub const PREVIEW_ROW_CAP: usize = 10_000;

/// The chart refuses a frame larger than this and names the count.
pub const CHART_POINT_CAP: usize = 20_000;

/// Full mode still shows a window. This is how many rows come back to the table.
pub const TABLE_WINDOW: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    #[default]
    Preview,
    Full,
}

impl RunMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Preview => "preview",
            Self::Full => "full",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum JobOp {
    #[default]
    Preview,
    Export,
    /// Drop the cached outputs of the given node ids. The reply is empty.
    Reset,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub generation: u64,
    pub mode: RunMode,
    pub graph: Graph,
    pub node: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_dir: Option<String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub inputs: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub op: JobOp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// How many result rows to return. The worker still computes `n` for the whole result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<usize>,
    /// Ask for per-column stats of the whole result frame.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stats: bool,
    /// Node ids for `JobOp::Reset`. Empty everywhere else.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<String>,
    /// Variable value texts for this run, applied before substitution.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub overrides: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkerError {
    pub node: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub generation: u64,
    pub mode: RunMode,
    #[serde(default)]
    pub node: String,
    #[serde(default)]
    pub columns: Vec<String>,
    #[serde(default)]
    pub dtypes: Vec<String>,
    #[serde(default)]
    pub index: Vec<String>,
    #[serde(default)]
    pub rows: Vec<Vec<Value>>,
    pub n: u64,
    pub claim: String,
    pub error: Option<WorkerError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub csv_path: Option<String>,
    /// Fitted parameters from a scaler or OLS node, when this run fit them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fitted: Option<Value>,
    /// Every node computed or served from cache, topological order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub computed: Vec<String>,
    /// Per-column stats of the whole result frame, when asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<Vec<ColumnStats>>,
    /// Branch nodes the worker skipped. Reserved, empty for now.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inactive: Vec<String>,
}

/// Stats of one column of the whole result frame. Optional fields are absent
/// when the dtype does not have them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColumnStats {
    pub name: String,
    pub dtype: String,
    pub count: u64,
    pub nulls: u64,
    pub null_pct: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unique: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub std: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hist: Vec<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub top: Vec<(Value, u64)>,
}

/// A preview cell as text. Null is an em dash, never the letters n-u-l-l.
pub fn cell_text(value: &Value) -> String {
    match value {
        Value::Null => "—".to_string(),
        Value::String(text) => text.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(v) => v.to_string(),
        other => other.to_string(),
    }
}

pub struct CellText;

impl CellText {
    pub fn is_number(value: &Value) -> bool {
        matches!(value, Value::Number(_))
    }
}

pub fn status_line(mode: RunMode, n: u64, window: usize) -> String {
    match mode {
        RunMode::Preview => format!("preview · at most 10,000 source rows · {n} rows"),
        RunMode::Full => format!("full · showing first {window} · {n} rows"),
    }
}

pub fn encode_frame(value: &impl Serialize) -> Result<Vec<u8>, serde_json::Error> {
    let body = serde_json::to_vec(value)?;
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

pub fn write_frame<W: Write>(writer: &mut W, value: &impl Serialize) -> std::io::Result<()> {
    let frame = encode_frame(value).map_err(std::io::Error::other)?;
    writer.write_all(&frame)?;
    writer.flush()?;
    Ok(())
}

pub fn read_frame<R: Read, T: for<'de> Deserialize<'de>>(reader: &mut R) -> std::io::Result<T> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > 64 * 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "worker frame is larger than 64MB",
        ));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))
}

pub fn decode_frame<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, String> {
    if bytes.len() < 4 {
        return Err("short frame".into());
    }
    let len = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
    if bytes.len() != 4 + len {
        return Err(format!(
            "frame length {len} does not match {}",
            bytes.len() - 4
        ));
    }
    serde_json::from_slice(&bytes[4..]).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Point;
    use crate::model::Node;
    use serde_json::json;

    #[test]
    fn a_job_and_reply_round_trip_through_a_length_prefix() {
        let mut graph = Graph::default();
        graph
            .nodes
            .push(Node::new("n3", "head", Point::new(0.0, 0.0)));
        let job = Job {
            generation: 1,
            mode: RunMode::Preview,
            graph,
            node: "n3".into(),
            base_dir: None,
            inputs: Default::default(),
            op: JobOp::Preview,
            output: None,
            window: None,
            stats: false,
            nodes: Vec::new(),
            overrides: Default::default(),
        };
        let frame = encode_frame(&job).unwrap();
        let back: Job = decode_frame(&frame).unwrap();
        assert_eq!(back.generation, 1);
        assert_eq!(back.node, "n3");
        assert_eq!(back.mode, RunMode::Preview);
        assert_eq!(back.op, JobOp::Preview);
        assert!(!back.stats);

        let reply = Reply {
            generation: 1,
            mode: RunMode::Preview,
            node: "n3".into(),
            columns: vec!["a".into()],
            dtypes: vec!["object".into()],
            index: vec!["0".into()],
            rows: vec![vec![json!("x")]],
            n: 10_000,
            claim: "preview".into(),
            error: None,
            csv_path: None,
            fitted: None,
            computed: vec!["n1".into(), "n3".into()],
            stats: None,
            inactive: Vec::new(),
        };
        let frame = encode_frame(&reply).unwrap();
        let back: Reply = decode_frame(&frame).unwrap();
        assert_eq!(back.rows[0][0], json!("x"));
        assert!(back.error.is_none());
        assert_eq!(back.computed, vec!["n1".to_string(), "n3".to_string()]);
        assert!(back.stats.is_none());
        assert!(back.inactive.is_empty());
        assert_eq!(
            status_line(RunMode::Preview, back.n, 0),
            "preview · at most 10,000 source rows · 10000 rows"
        );
    }

    #[test]
    fn a_reset_job_and_a_stats_reply_round_trip() {
        let mut graph = Graph::default();
        graph
            .nodes
            .push(Node::new("n1", "head", Point::new(0.0, 0.0)));
        let mut overrides = std::collections::BTreeMap::new();
        overrides.insert("rows".to_string(), "2".to_string());
        let job = Job {
            generation: 2,
            mode: RunMode::Preview,
            graph,
            node: String::new(),
            base_dir: None,
            inputs: Default::default(),
            op: JobOp::Reset,
            output: None,
            window: None,
            stats: false,
            nodes: vec!["n1".into()],
            overrides,
        };
        let back: Job = decode_frame(&encode_frame(&job).unwrap()).unwrap();
        assert_eq!(back.op, JobOp::Reset);
        assert_eq!(back.nodes, vec!["n1".to_string()]);
        assert_eq!(back.overrides.get("rows").map(String::as_str), Some("2"));

        let reply = Reply {
            generation: 1,
            mode: RunMode::Preview,
            node: "n1".into(),
            columns: vec!["a".into()],
            dtypes: vec!["int64".into()],
            index: Vec::new(),
            rows: Vec::new(),
            n: 4,
            claim: "preview".into(),
            error: None,
            csv_path: None,
            fitted: None,
            computed: Vec::new(),
            stats: Some(vec![ColumnStats {
                name: "a".into(),
                dtype: "int64".into(),
                count: 4,
                nulls: 1,
                null_pct: 25.0,
                unique: Some(3),
                min: Some(json!(1)),
                max: Some(json!(3)),
                mean: Some(2.0),
                std: None,
                hist: vec![0, 1, 2, 3, 0, 0, 0, 0, 0, 0, 0, 0],
                top: vec![(json!("Ada"), 2)],
            }]),
            inactive: Vec::new(),
        };
        let back: Reply = decode_frame(&encode_frame(&reply).unwrap()).unwrap();
        let stats = back.stats.expect("stats");
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].name, "a");
        assert_eq!(stats[0].null_pct, 25.0);
        assert_eq!(stats[0].hist.len(), 12);
        assert_eq!(stats[0].top, vec![(json!("Ada"), 2)]);
    }

    #[test]
    fn null_is_an_em_dash() {
        assert_eq!(cell_text(&Value::Null), "—");
        assert_eq!(cell_text(&json!(12.4)), "12.4");
    }
}
