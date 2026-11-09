//! Fill one inspector field from the pipeline context.
//!
//! The model is optional. The endpoint and key live in the local settings file,
//! never in a pipeline, and this module does not invent columns or files.

use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};

use serde_json::{Value, json};

#[derive(Clone, Debug, Default)]
pub struct ModelSettings {
    pub endpoint: String,
    pub model: String,
    pub key: String,
}

#[derive(Clone, Debug)]
pub struct AssistJob {
    pub endpoint: String,
    pub model: String,
    pub key: String,
    pub field: String,
    pub feature: String,
    pub purpose: String,
    pub current: String,
    pub columns: Vec<String>,
    pub sample: Vec<Vec<String>>,
    pub neighbors: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub enum AssistReply {
    Value(String),
    Ask(String),
    Failed(String),
}

/// A question we can answer without calling a model.
pub fn local_question(job: &AssistJob) -> Option<String> {
    if job.endpoint.trim().is_empty() || job.key.trim().is_empty() {
        return Some(
            "Add a model endpoint and key in Settings. They stay on this computer and are not saved in the pipeline."
                .into(),
        );
    }
    let wants_column = matches!(
        job.field.as_str(),
        "name" | "x" | "y" | "by" | "on" | "series" | "from"
    );
    if wants_column && job.columns.is_empty() {
        return Some(
            "This needs a column from the upstream table. Run Preview on the previous node, or type the column name."
                .into(),
        );
    }
    None
}

pub fn spawn(job: AssistJob) -> Receiver<AssistReply> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(call(&job));
    });
    rx
}

fn call(job: &AssistJob) -> AssistReply {
    let url = completions_url(&job.endpoint);
    let model = if job.model.trim().is_empty() {
        "gpt-4o-mini"
    } else {
        job.model.trim()
    };
    let body = json!({
        "model": model,
        "temperature": 0,
        "messages": [
            {
                "role": "system",
                "content": "You fill one parameter of a table pipeline. Use only the context JSON. Do not invent column names, file paths, or numbers that are not in the context. Reply with only the parameter value, no markdown and no explanation. If the context is not enough, reply with exactly: NEED: <one short question>"
            },
            {
                "role": "user",
                "content": json!({
                    "feature": job.feature,
                    "purpose": job.purpose,
                    "field": job.field,
                    "current": job.current,
                    "columns": job.columns,
                    "sample": job.sample,
                    "neighbors": job.neighbors,
                }).to_string()
            }
        ]
    });
    let config_path = std::env::temp_dir().join(format!("ompp-model-{}.curl", std::process::id()));
    let config = format!(
        "header = \"Authorization: Bearer {}\"\nheader = \"Content-Type: application/json\"\nurl = \"{}\"\nsilent\nshow-error\nmax-time = 20\n",
        job.key.trim().replace(['\n', '"'], ""),
        url.replace('"', "")
    );
    if std::fs::write(&config_path, config).is_err() {
        return AssistReply::Failed("Could not store the model request on this computer.".into());
    }
    let mut child = match Command::new("curl")
        .args([
            "--config",
            config_path.to_str().unwrap_or(""),
            "--data-binary",
            "@-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            let _ = std::fs::remove_file(&config_path);
            return AssistReply::Failed(format!("Could not start curl: {err}"));
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(body.to_string().as_bytes());
    }
    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(err) => {
            let _ = std::fs::remove_file(&config_path);
            return AssistReply::Failed(format!("The model request failed: {err}"));
        }
    };
    let _ = std::fs::remove_file(&config_path);
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        let detail = detail.trim();
        return AssistReply::Failed(if detail.is_empty() {
            "The model request failed.".into()
        } else {
            format!("The model request failed: {detail}")
        });
    }
    parse_completion(&output.stdout)
}

fn completions_url(endpoint: &str) -> String {
    let trimmed = endpoint.trim().trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/chat/completions")
    }
}

fn parse_completion(bytes: &[u8]) -> AssistReply {
    let text = String::from_utf8_lossy(bytes);
    let value: Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(_) => {
            return AssistReply::Failed(
                "The model did not return JSON. Check the endpoint in Settings.".into(),
            );
        }
    };
    if let Some(message) = value
        .get("error")
        .and_then(|err| err.get("message"))
        .and_then(|m| m.as_str())
    {
        return AssistReply::Failed(message.to_string());
    }
    let content = value
        .pointer("/choices/0/message/content")
        .and_then(|item| item.as_str())
        .unwrap_or("")
        .trim();
    if content.is_empty() {
        return AssistReply::Failed("The model returned an empty value.".into());
    }
    if let Some(question) = content.strip_prefix("NEED:") {
        let question = question.trim();
        return AssistReply::Ask(if question.is_empty() {
            "The model needs more context.".into()
        } else {
            question.to_string()
        });
    }
    AssistReply::Value(content.to_string())
}

/// Pretty-print JSON. Invalid text is returned unchanged as `None`.
pub fn pretty_json(text: &str) -> Option<String> {
    let value: Value = serde_json::from_str(text).ok()?;
    serde_json::to_string_pretty(&value).ok()
}

/// Trim trailing spaces. This does not rewrite Python tokens.
pub fn pretty_python(text: &str) -> String {
    let mut lines: Vec<String> = text
        .lines()
        .map(|line| line.trim_end().to_string())
        .collect();
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    if lines.is_empty() {
        return String::new();
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_json_leaves_invalid_text_alone() {
        assert!(pretty_json("{").is_none());
        assert_eq!(
            pretty_json("{\"a\":1}").as_deref(),
            Some("{\n  \"a\": 1\n}")
        );
    }

    #[test]
    fn a_missing_key_is_a_question_not_a_guess() {
        let job = AssistJob {
            endpoint: String::new(),
            model: String::new(),
            key: String::new(),
            field: "n".into(),
            feature: "head".into(),
            purpose: "keep rows".into(),
            current: String::new(),
            columns: Vec::new(),
            sample: Vec::new(),
            neighbors: BTreeMap::new(),
        };
        assert!(local_question(&job).unwrap().contains("Settings"));
    }

    #[test]
    fn need_prefix_is_a_question() {
        let raw = br#"{"choices":[{"message":{"content":"NEED: Which column?"}}]}"#;
        match parse_completion(raw) {
            AssistReply::Ask(text) => assert_eq!(text, "Which column?"),
            other => panic!("unexpected {other:?}"),
        }
    }
}
