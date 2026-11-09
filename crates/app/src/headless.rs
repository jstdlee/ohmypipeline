//! `ompp --pipeline` without a window. The worker writes the CSV.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use graph::{Document, Job, JobOp, RunMode, TABLE_WINDOW, ancestors, sinks};

use crate::worker;

pub struct CliRequest {
    pub pipeline: PathBuf,
    pub inputs: Vec<String>,
    pub output: Option<PathBuf>,
    pub node: Option<String>,
    pub full: bool,
}

pub fn run_pipeline(request: CliRequest) -> i32 {
    let doc = match Document::load(&request.pipeline) {
        Ok(doc) => doc,
        Err(err) => {
            eprintln!("{err}");
            return 2;
        }
    };
    let graph = doc.graph();
    let sources: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                node.feature.as_str(),
                "read_csv" | "read_excel" | "read_sql"
            )
        })
        .map(|node| node.id.clone())
        .collect();
    let mut overrides = std::collections::BTreeMap::new();
    if let Err(message) = apply_inputs(&sources, &request.inputs, &mut overrides) {
        eprintln!("{message}");
        return 2;
    }
    let node = match request.node.clone().or_else(|| {
        let sinks = sinks(graph);
        if sinks.len() == 1 {
            Some(sinks[0].clone())
        } else {
            None
        }
    }) {
        Some(node) => node,
        None => {
            let sinks = sinks(graph);
            eprintln!("choose a node with --node. sinks: {}", sinks.join(", "));
            return 3;
        }
    };
    if graph.node(&node).is_none() {
        eprintln!("unknown node {node}");
        return 3;
    }
    if ancestors(graph, &node).is_err() {
        eprintln!("{node}");
        eprintln!("cycle");
        return 3;
    }
    let mode = if request.full {
        RunMode::Full
    } else {
        RunMode::Preview
    };
    let output = request.output.clone().unwrap_or_else(|| {
        std::env::temp_dir().join(format!("ompp-{}-{}.csv", std::process::id(), node))
    });
    let job = Job {
        generation: 1,
        mode,
        graph: graph.clone(),
        node: node.clone(),
        base_dir: request
            .pipeline
            .parent()
            .map(|path| path.display().to_string()),
        inputs: overrides,
        op: JobOp::Export,
        output: Some(output.display().to_string()),
        window: Some(if request.full { TABLE_WINDOW } else { 10_000 }),
    };
    match worker::run_job(&job) {
        Ok(reply) => {
            if let Some(error) = reply.error {
                eprintln!("{}", error.node);
                eprintln!("{}", error.message);
                return 3;
            }
            if request.output.is_none() {
                match fs::read(&output) {
                    Ok(bytes) => {
                        let _ = io::stdout().write_all(&bytes);
                        let _ = fs::remove_file(&output);
                    }
                    Err(err) => {
                        eprintln!("{node}");
                        eprintln!("{err}");
                        return 3;
                    }
                }
            }
            0
        }
        Err(err) => {
            eprintln!("{node}");
            eprintln!("{err}");
            3
        }
    }
}

fn absolute_input(path: &str) -> String {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path.display().to_string()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
            .display()
            .to_string()
    }
}

fn apply_inputs(
    sources: &[String],
    inputs: &[String],
    overrides: &mut std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
    if inputs.is_empty() {
        return Ok(());
    }
    let bare: Vec<_> = inputs
        .iter()
        .filter(|item| !item.contains('='))
        .cloned()
        .collect();
    let named: Vec<_> = inputs
        .iter()
        .filter(|item| item.contains('='))
        .cloned()
        .collect();
    if sources.len() <= 1 {
        if bare.len() > 1 || !named.is_empty() {
            return Err("one source takes a single --input path".into());
        }
        if let Some(path) = bare.first() {
            if let Some(id) = sources.first() {
                overrides.insert(id.clone(), absolute_input(path));
            } else {
                return Err("the pipeline has no source to replace".into());
            }
        }
        return Ok(());
    }
    if !bare.is_empty() {
        return Err(format!(
            "this pipeline has more than one source. name them: {}",
            sources
                .iter()
                .map(|id| format!("--input {id}=path"))
                .collect::<Vec<_>>()
                .join(" ")
        ));
    }
    for item in named {
        let Some((id, path)) = item.split_once('=') else {
            continue;
        };
        if !sources.iter().any(|source| source == id) {
            return Err(format!("unknown source {id}"));
        }
        overrides.insert(id.to_string(), absolute_input(path));
    }
    Ok(())
}

pub fn usage() -> &'static str {
    "\
ompp [--pipeline file.pp] [--input path] [--input id=path] [--output path] [--node id] [--full]
ompp gui
ompp file.pp
ompp --version
ompp --demo empty|graph|preview|link-snap|link-reject|inspector|chart|dark|light"
}

pub fn version_line() -> String {
    format!("ompp {} {}", env!("CARGO_PKG_VERSION"), env!("OMPP_TARGET"))
}
