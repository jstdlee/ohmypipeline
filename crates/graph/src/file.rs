use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use crate::feature::feature;
use crate::model::Graph;
use crate::{FORMAT, VERSION};

#[derive(Debug)]
pub enum FileError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Format(String),
    Version(u32),
}

impl std::fmt::Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "{err}"),
            Self::Json(err) => write!(f, "{err}"),
            Self::Format(text) => write!(f, "{text}"),
            Self::Version(version) => write!(f, "unknown version {version}"),
        }
    }
}

impl std::error::Error for FileError {}

impl From<std::io::Error> for FileError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<serde_json::Error> for FileError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}

pub fn load_str(text: &str) -> Result<Graph, FileError> {
    let mut graph: Graph = serde_json::from_str(text)?;
    if graph.format != FORMAT {
        return Err(FileError::Format(format!(
            "expected format {FORMAT}, found {}",
            graph.format
        )));
    }
    if graph.version != VERSION {
        return Err(FileError::Version(graph.version));
    }
    for node in &mut graph.nodes {
        node.unknown = feature(&node.feature).is_none();
    }
    for edge in &mut graph.edges {
        if edge.id.is_empty() {
            edge.id = format!(
                "{}:{}->{}:{}",
                edge.from, edge.from_port, edge.to, edge.to_port
            );
        }
    }
    for mark in &mut graph.marks {
        if !matches!(
            mark.swatch.as_str(),
            "c1" | "c2" | "c3" | "c4" | "c5" | "c6" | "neutral"
        ) {
            mark.swatch = "neutral".to_string();
        }
    }
    Ok(graph)
}

pub fn load_path(path: &Path) -> Result<Graph, FileError> {
    let text = fs::read_to_string(path)?;
    load_str(&text)
}

pub fn to_json(graph: &Graph) -> Result<String, FileError> {
    Ok(serde_json::to_string_pretty(graph)?)
}

/// Write via a temp file in the same directory, fsync, then rename.
pub fn save_atomic(path: &Path, graph: &Graph) -> Result<(), FileError> {
    let text = to_json(graph)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp_name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    tmp_name.push(".tmp");
    let tmp = parent.join(tmp_name);
    {
        let mut file = File::create(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn recovery_path(path: &Path) -> std::path::PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "untitled.pp".to_string());
    path.with_file_name(format!("{name}.recovery"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Node;

    const SAMPLE: &str = r#"{
  "format": "ohmypipeline",
  "version": 1,
  "nodes": [
    {
      "id": "n1",
      "type": "read_csv",
      "label": "orders.csv",
      "position": [48, 64],
      "params": { "path": "orders.csv" }
    },
    {
      "id": "n2",
      "type": "query",
      "label": "region",
      "position": [288, 64],
      "params": { "expr": "region == 'N'" }
    }
  ],
  "edges": [{ "from": "n1", "from_port": "out", "to": "n2", "to_port": "in" }],
  "marks": [
    { "id": "m1", "kind": "note", "text": "Preview reads at most 10,000 rows.", "position": [48, 200], "size": [220, 72], "swatch": "c3" }
  ],
  "camera": { "x": 0, "y": 0, "zoom": 1 }
}"#;

    #[test]
    fn the_design_document_loads() {
        let graph = load_str(SAMPLE).unwrap();
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.nodes[0].feature, "read_csv");
        assert_eq!(graph.nodes[0].position, [48.0, 64.0]);
        assert_eq!(graph.edges[0].from, "n1");
        assert_eq!(graph.edges[0].id, "n1:out->n2:in");
        assert_eq!(graph.marks[0].swatch, "c3");
        assert!(!graph.nodes[0].unknown);
    }

    #[test]
    fn an_unknown_version_names_the_number() {
        let text = SAMPLE.replace("\"version\": 1", "\"version\": 9");
        let err = load_str(&text).unwrap_err();
        assert_eq!(err.to_string(), "unknown version 9");
    }

    #[test]
    fn an_unknown_feature_loads_as_an_error_node() {
        let text = SAMPLE.replace("\"query\"", "\"not_a_feature\"");
        let graph = load_str(&text).unwrap();
        assert!(graph.nodes[1].unknown);
        assert_eq!(graph.nodes[1].feature, "not_a_feature");
    }

    #[test]
    fn round_trip_keeps_positions_links_and_the_camera() {
        let graph = load_str(SAMPLE).unwrap();
        let text = to_json(&graph).unwrap();
        let again = load_str(&text).unwrap();
        assert_eq!(again.nodes[0].position, graph.nodes[0].position);
        assert_eq!(again.nodes[1].position, [288.0, 64.0]);
        assert_eq!(again.edges.len(), 1);
        assert_eq!(again.edges[0].from_port, "out");
        assert_eq!(again.camera.zoom, 1.0);
        assert_eq!(again.marks[0].text, graph.marks[0].text);
        assert_eq!(again.marks[0].size, Some([220.0, 72.0]));
        let third = to_json(&again).unwrap();
        assert_eq!(text, third);
    }

    #[test]
    fn save_replaces_the_file_and_leaves_no_temp() {
        let dir = std::env::temp_dir().join(format!("ompp-save-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("revenue.pp");
        let mut graph = Graph::default();
        graph.nodes.push(Node::new(
            "n1",
            "read_csv",
            crate::geom::Point::new(16.0, 32.0),
        ));
        save_atomic(&path, &graph).unwrap();
        assert!(path.is_file());
        assert!(!dir.join("revenue.pp.tmp").exists());
        let loaded = load_path(&path).unwrap();
        assert_eq!(loaded.nodes[0].position, [16.0, 32.0]);
        let _ = fs::remove_dir_all(&dir);
    }
}
