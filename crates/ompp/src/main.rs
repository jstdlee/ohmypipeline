//! `ompp` — the Ohmypipeline binary.
//!
//! No arguments opens the window. `--pipeline` runs headless and writes CSV.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use app::{CliRequest, GuiOptions, run_gui, run_pipeline, usage, version_line};

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
