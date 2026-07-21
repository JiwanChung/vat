//! Convert a Jupyter `.ipynb` notebook into a Markdown document so it can be
//! rendered by the Markdown engine: markdown cells inline, code cells as fenced
//! code blocks, and cell outputs (text/stream) inline.

use anyhow::{anyhow, Result};
use serde_json::Value;

/// Read an `.ipynb` file and render it to a Markdown string.
pub fn to_markdown(path: &std::path::Path) -> Result<String> {
    let bytes = super::read_text_file_bytes(path)?;
    let nb: Value = serde_json::from_slice(&bytes).map_err(|e| anyhow!("invalid notebook: {}", e))?;

    let language = nb
        .pointer("/metadata/kernelspec/language")
        .or_else(|| nb.pointer("/metadata/language_info/name"))
        .and_then(Value::as_str)
        .unwrap_or("python")
        .to_string();

    let cells = nb
        .get("cells")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("notebook has no cells"))?;

    let mut out = String::new();
    for cell in cells {
        let kind = cell.get("cell_type").and_then(Value::as_str).unwrap_or("");
        let source = join_source(cell.get("source"));
        match kind {
            "markdown" => {
                out.push_str(source.trim_end());
                out.push_str("\n\n");
            }
            "code" => {
                if !source.trim().is_empty() {
                    out.push_str(&format!("```{}\n{}\n```\n\n", language, source.trim_end()));
                }
                append_outputs(cell.get("outputs"), &mut out);
            }
            "raw" => {
                out.push_str("```\n");
                out.push_str(source.trim_end());
                out.push_str("\n```\n\n");
            }
            _ => {}
        }
    }
    Ok(out)
}

/// `source` in ipynb is either a string or an array of line strings.
fn join_source(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(lines)) => lines.iter().filter_map(Value::as_str).collect(),
        _ => String::new(),
    }
}

fn append_outputs(outputs: Option<&Value>, out: &mut String) {
    let Some(outputs) = outputs.and_then(Value::as_array) else {
        return;
    };
    for output in outputs {
        let text = match output.get("output_type").and_then(Value::as_str) {
            Some("stream") => join_source(output.get("text")),
            Some("execute_result") | Some("display_data") => {
                join_source(output.pointer("/data/text~1plain"))
            }
            Some("error") => output
                .get("traceback")
                .and_then(Value::as_array)
                .map(|tb| tb.iter().filter_map(Value::as_str).collect::<String>())
                .unwrap_or_default(),
            _ => String::new(),
        };
        let text = strip_ansi(text.trim_end());
        if !text.is_empty() {
            // Render output as a fenced block labeled "output".
            out.push_str("```output\n");
            out.push_str(&text);
            out.push_str("\n```\n\n");
        }
    }
}

/// Remove ANSI escape sequences (common in stream/error output).
fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // Skip until the terminating letter of the escape sequence.
            while let Some(&n) = chars.peek() {
                chars.next();
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}
