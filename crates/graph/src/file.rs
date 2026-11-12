use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use crate::feature::feature;
use crate::model::Graph;
use crate::{FORMAT, MIN_VERSION, VERSION};

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
    if !(MIN_VERSION..=VERSION).contains(&graph.version) {
        return Err(FileError::Version(graph.version));
    }
    // Files older than VERSION keep loading; missing fields arrive as
    // defaults, and the document moves to the current version in memory.
    graph.version = VERSION;
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
        if !is_swatch(&mark.swatch) {
            mark.swatch = "neutral".to_string();
        }
    }
    for frame in &mut graph.frames {
        if !is_swatch(&frame.swatch) {
            frame.swatch = "neutral".to_string();
        }
    }
    let mut variables: Vec<_> = graph.variables.clone();
    variables.retain(|variable| is_variable_name(&variable.name));
    let mut seen = std::collections::BTreeSet::new();
    variables.retain(|variable| seen.insert(variable.name.clone()));
    graph.variables = variables;
    Ok(graph)
}

fn is_swatch(swatch: &str) -> bool {
    matches!(swatch, "c1" | "c2" | "c3" | "c4" | "c5" | "c6" | "neutral")
}

/// Identifiers a `${name}` reference can use.
pub fn is_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some('a'..='z') | Some('A'..='Z') | Some('_') => {}
        _ => return false,
    }
    chars.all(|c| matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '_'))
}

pub fn load_path(path: &Path) -> Result<Graph, FileError> {
    let text = fs::read_to_string(path)?;
    load_str(&text)
}

pub fn to_json(graph: &Graph) -> Result<String, FileError> {
    if graph.version == VERSION {
        return Ok(serde_json::to_string_pretty(graph)?);
    }
    // Saving always writes the current version, whatever the document said.
    let mut current = graph.clone();
    current.version = VERSION;
    Ok(serde_json::to_string_pretty(&current)?)
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
    fn a_version_three_document_is_refused() {
        let text = SAMPLE.replace("\"version\": 1", "\"version\": 3");
        let err = load_str(&text).unwrap_err();
        assert!(matches!(err, FileError::Version(3)));
        let text = SAMPLE.replace("\"version\": 1", "\"version\": 0");
        let err = load_str(&text).unwrap_err();
        assert!(matches!(err, FileError::Version(0)));
    }

    #[test]
    fn saving_writes_version_two_even_for_a_migrated_one() {
        let graph = load_str(SAMPLE).unwrap();
        let text = to_json(&graph).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["version"], 2);
        let again: Graph = serde_json::from_str(&text).unwrap();
        assert_eq!(again.version, VERSION);
    }

    #[test]
    fn every_example_pipeline_loads_and_resaves_as_version_two() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let mut checked = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("pp") {
                continue;
            }
            let graph = load_path(&path).unwrap_or_else(|err| {
                panic!("{} did not load: {err:?}", path.display());
            });
            assert_eq!(graph.version, VERSION);
            assert_eq!(graph.format, FORMAT);
            let text = to_json(&graph).unwrap();
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(value["version"], 2, "{}", path.display());
            let reloaded = load_str(&text).unwrap();
            assert_eq!(reloaded, graph, "{}", path.display());
            checked += 1;
        }
        assert!(checked > 0, "no example pipelines found under {dir:?}");
    }

    #[test]
    fn a_saved_document_is_stable_across_two_round_trips() {
        let text = to_json(&round_trip_fixture()).unwrap();
        assert_eq!(text, to_json(&load_str(&text).unwrap()).unwrap());
    }

    #[test]
    fn presentation_round_trips_through_save_and_load() {
        let mut graph = round_trip_fixture();
        let presentation = crate::model::Presentation {
            title: crate::model::Placed {
                text: Some("Revenue by month".into()),
                x: 0.25,
                y: 0.75,
                anchor: crate::model::Anchor::BottomStart,
            },
            legend: crate::model::Placed {
                text: None,
                x: 0.1,
                y: 0.9,
                anchor: crate::model::Anchor::End,
            },
            hidden_series: vec!["qty".into()],
        };
        graph.nodes[0].presentation = Some(presentation.clone());
        let text = to_json(&graph).unwrap();
        let loaded = load_str(&text).unwrap();
        assert_eq!(loaded.nodes[0].presentation, Some(presentation));
        assert_eq!(loaded.nodes[0].presentation, graph.nodes[0].presentation);
    }

    #[test]
    fn frames_variables_view_and_reroutes_round_trip() {
        let loaded = load_str(&to_json(&round_trip_fixture()).unwrap()).unwrap();
        assert_eq!(loaded.frames, round_trip_fixture().frames);
        assert_eq!(loaded.variables, round_trip_fixture().variables);
        assert_eq!(loaded.view, round_trip_fixture().view);
        assert_eq!(loaded.edges[0].reroutes, [[64.0, 192.0], [160.0, 96.0]]);
        let text = to_json(&loaded).unwrap();
        assert!(text.contains("\"frames\"") && text.contains("\"variables\""));
        assert!(text.contains("\"view\"") && text.contains("\"reroutes\""));
    }

    #[test]
    fn a_default_document_writes_no_view_and_no_empty_collections() {
        let text = to_json(&Graph::default()).unwrap();
        assert!(text.contains("\"version\": 2"));
        assert!(!text.contains("\"frames\""));
        assert!(!text.contains("\"variables\""));
        assert!(!text.contains("\"view\""));
    }

    #[test]
    fn frames_fall_back_to_the_neutral_swatch() {
        let value = serde_json::json!({
            "format": FORMAT,
            "version": 2,
            "nodes": [],
            "edges": [],
            "marks": [],
            "frames": [
                { "id": "f1", "title": "kept", "swatch": "c2", "position": [0, 0], "size": [200, 100] },
                { "id": "f2", "title": "wildcard", "swatch": "rainbow", "position": [0, 0], "size": [200, 100] },
                { "id": "f3", "title": "default", "position": [0, 0], "size": [200, 100] }
            ]
        });
        let graph = load_str(&value.to_string()).unwrap();
        assert_eq!(graph.frames.len(), 3);
        assert_eq!(graph.frames[0].swatch, "c2");
        assert_eq!(graph.frames[1].swatch, "neutral");
        assert_eq!(graph.frames[2].swatch, "neutral");
    }

    #[test]
    fn variable_names_must_be_identifiers_and_unique() {
        let value = serde_json::json!({
            "format": FORMAT,
            "version": 2,
            "variables": [
                { "name": "city", "kind": "string", "value": "New York" },
                { "name": "city", "kind": "string", "value": "Paris" },
                { "name": "rows", "kind": "int", "value": 12 },
                { "name": "not allowed", "kind": "string", "value": 0 },
                { "name": "2nd", "kind": "int", "value": 0 },
                { "name": "ok_name2", "kind": "path", "value": "a/b.csv" }
            ]
        });
        let graph = load_str(&value.to_string()).unwrap();
        let names: Vec<&str> = graph.variables.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["city", "rows", "ok_name2"]);
        assert_eq!(graph.variables[0].value, serde_json::json!("New York"));
        assert_eq!(graph.variables[1].value, serde_json::json!(12));
        assert!(names.iter().all(|name| is_variable_name(name)));
        assert!(!is_variable_name("not allowed"));
        assert!(!is_variable_name("2nd"));
    }

    /// A loaded document that exercises every version-2 field.
    fn round_trip_fixture() -> Graph {
        let text = SAMPLE.replace("\"version\": 1", "\"version\": 2").replace(
            "\"marks\": [",
            concat!(
                "\"frames\": [{ \"id\": \"f1\", \"title\": \"Orders\", \"swatch\": \"c1\", ",
                "\"position\": [32, 48], \"size\": [480, 220] }], ",
                "\"variables\": [",
                "{ \"name\": \"city\", \"kind\": \"string\", \"value\": \"New York\" },",
                "{ \"name\": \"top\", \"kind\": \"int\", \"value\": 5 }], ",
                "\"view\": { \"dock\": { \"left\": 240, \"bottom\": 180 }, \"minimap\": true }, ",
                "\"marks\": [",
            ),
        );
        let mut graph = load_str(&text).unwrap();
        graph.edges[0].reroutes = vec![[64.0, 192.0], [160.0, 96.0]];
        graph
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
