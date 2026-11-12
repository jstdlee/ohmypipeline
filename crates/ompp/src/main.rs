//! `ompp` — the Ohmypipeline binary.
//!
//! No arguments opens the window. `--pipeline` runs headless and writes CSV.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use app::{CliRequest, GuiOptions, run_gui, run_pipeline, run_usage, usage, version_line};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match dispatch(&args) {
        Ok(code) => ExitCode::from(code),
        Err(message) => {
            eprintln!("{message}");
            eprintln!("{}", usage());
            ExitCode::from(2)
        }
    }
}

fn dispatch(args: &[String]) -> Result<u8, String> {
    if args.is_empty() || args == ["gui"] {
        run_gui(GuiOptions::default());
        return Ok(0);
    }
    if args == ["--version"] || args == ["-V"] {
        println!("{}", version_line());
        return Ok(0);
    }
    if args.first().map(String::as_str) == Some("--help")
        || args.first().map(String::as_str) == Some("-h")
    {
        println!("{}", usage());
        return Ok(0);
    }
    if args.first().map(String::as_str) == Some("run") {
        if args.len() == 1 {
            println!("{}", run_usage());
            return Ok(0);
        }
        if matches!(args[1].as_str(), "--help" | "-h") {
            println!("{}", run_usage());
            return Ok(0);
        }
        return Ok(run_pipeline(parse_run(&args[1..])?) as u8);
    }
    if args.first().map(String::as_str) == Some("--demo") {
        let demo = args.get(1).cloned().unwrap_or_else(|| "empty".into());
        let ready = args.windows(2).find_map(|pair| {
            if pair[0] == "--ready" {
                Some(PathBuf::from(&pair[1]))
            } else {
                None
            }
        });
        run_gui(GuiOptions {
            demo: Some(demo),
            ready_file: ready,
            ..GuiOptions::default()
        });
        return Ok(0);
    }
    if let Some(path) = args
        .first()
        .filter(|arg| arg.ends_with(".pp") && !arg.starts_with('-'))
    {
        run_gui(GuiOptions {
            open: Some(PathBuf::from(path)),
            ..GuiOptions::default()
        });
        return Ok(0);
    }
    if args.iter().any(|arg| arg == "--pipeline") {
        return Ok(run_pipeline(parse_cli(args)?) as u8);
    }
    Err("unrecognized arguments".into())
}

fn parse_cli(args: &[String]) -> Result<CliRequest, String> {
    let mut pipeline = None;
    let mut inputs = Vec::new();
    let mut output = None;
    let mut node = None;
    let mut full = false;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "--pipeline" => {
                pipeline = Some(PathBuf::from(next(args, &mut index, "--pipeline")?));
            }
            "--input" => inputs.push(next(args, &mut index, "--input")?),
            "--output" => output = Some(PathBuf::from(next(args, &mut index, "--output")?)),
            "--node" => node = Some(next(args, &mut index, "--node")?),
            "--full" => full = true,
            other => return Err(format!("unknown argument {other}")),
        }
        index += 1;
    }
    let pipeline = pipeline.ok_or("--pipeline is required")?;
    Ok(CliRequest {
        pipeline,
        inputs,
        overrides: Default::default(),
        output,
        node,
        full,
    })
}

fn parse_run(args: &[String]) -> Result<CliRequest, String> {
    let mut pipeline = None;
    let mut sets = Vec::new();
    let mut inputs = Vec::new();
    let mut output = None;
    let mut node = None;
    let mut full = false;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "--set" => sets.push(next(args, &mut index, "--set")?),
            "--input" => inputs.push(next(args, &mut index, "--input")?),
            "--output" => output = Some(PathBuf::from(next(args, &mut index, "--output")?)),
            "--node" => node = Some(next(args, &mut index, "--node")?),
            "--full" => full = true,
            other if other.starts_with('-') => return Err(format!("unknown argument {other}")),
            other => {
                if pipeline.is_some() {
                    return Err(format!("one pipeline per run, already given {pipeline:?}"));
                }
                pipeline = Some(PathBuf::from(other));
            }
        }
        index += 1;
    }
    let pipeline = pipeline.ok_or("run needs a pipeline file")?;
    let mut overrides = std::collections::BTreeMap::new();
    for item in sets {
        let Some((name, text)) = item.split_once('=') else {
            return Err(format!("--set needs name=value, got {item}"));
        };
        if name.is_empty() {
            return Err(format!("--set needs a variable name, got {item}"));
        }
        overrides.insert(name.to_string(), text.to_string());
    }
    Ok(CliRequest {
        pipeline,
        inputs,
        overrides,
        output,
        node,
        full,
    })
}

fn next(args: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_args(items: &[&str]) -> Result<CliRequest, String> {
        parse_run(
            &items
                .iter()
                .map(|item| item.to_string())
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn a_run_request_parses_file_sets_and_flags() {
        let request = run_args(&[
            "examples/vars.pp",
            "--set",
            "rows=2",
            "--set",
            "data=other.csv",
            "--input",
            "n1=/tmp/x.csv",
            "--node",
            "n2",
            "--output",
            "/tmp/v.csv",
            "--full",
        ])
        .unwrap();
        assert_eq!(request.pipeline, PathBuf::from("examples/vars.pp"));
        assert_eq!(request.overrides.get("rows").map(String::as_str), Some("2"));
        assert_eq!(
            request.overrides.get("data").map(String::as_str),
            Some("other.csv")
        );
        assert_eq!(request.inputs, vec!["n1=/tmp/x.csv".to_string()]);
        assert_eq!(request.node.as_deref(), Some("n2"));
        assert_eq!(
            request.output.as_deref(),
            Some(std::path::Path::new("/tmp/v.csv"))
        );
        assert!(request.full);
    }

    #[test]
    fn bad_run_requests_are_usage_errors() {
        assert!(run_args(&[]).is_err());
        assert!(run_args(&["--set", "rows=2"]).is_err(), "no file");
        assert!(run_args(&["examples/vars.pp", "--set", "rows"]).is_err());
        assert!(run_args(&["examples/vars.pp", "--nope"]).is_err());
    }
}
