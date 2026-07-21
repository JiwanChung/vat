use std::io::{self, Read, Write};
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use clap::{Parser, ValueEnum};

mod analyzer;
mod app;
mod engines;

#[derive(Parser, Debug)]
#[command(name = "vat", version, about = "Semantic file viewer")]
struct Args {
    /// Path to the file to view (use "-" for stdin)
    path: String,
    /// Paging mode: auto, always, never (bat-compatible)
    #[arg(long, value_enum, default_value = "auto")]
    paging: Paging,
    /// Output raw file content without formatting (useful for piping)
    #[arg(short = 'p', long)]
    plain: bool,
    /// Language/format hint for stdin (e.g., json, yaml, csv, jsonl)
    #[arg(short = 'l', long)]
    language: Option<String>,
}

#[derive(ValueEnum, Clone, Debug)]
enum Paging {
    Auto,
    Always,
    Never,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Handle stdin
    let (path, _temp_file) = if args.path == "-" {
        read_stdin_to_temp(&args.language)?
    } else {
        (PathBuf::from(&args.path), None)
    };

    let display_path = if args.path == "-" {
        format!("<stdin>{}", args.language.as_ref().map(|l| format!(".{}", l)).unwrap_or_default())
    } else {
        args.path.clone()
    };

    let engine = analyzer::analyze(&path)?;
    let mut app = app::App::new(engine, display_path, path, args.paging.into(), args.plain);
    app.run()
}

/// Read stdin to a temporary file, return path and handle (to keep file alive)
fn read_stdin_to_temp(language: &Option<String>) -> Result<(PathBuf, Option<tempfile::NamedTempFile>)> {
    let mut buffer = Vec::new();
    io::stdin().read_to_end(&mut buffer)?;

    if buffer.is_empty() {
        return Err(anyhow!("No input received from stdin"));
    }

    // Determine extension from language hint or try to detect
    let ext = language.clone().unwrap_or_else(|| detect_format(&buffer));

    let mut temp = tempfile::Builder::new()
        .suffix(&format!(".{}", ext))
        .tempfile()?;

    temp.write_all(&buffer)?;
    temp.flush()?;

    let path = temp.path().to_path_buf();
    Ok((path, Some(temp)))
}

/// Try to detect format from content
fn detect_format(content: &[u8]) -> String {
    let text = String::from_utf8_lossy(content);
    let trimmed = text.trim_start();
    let first_line = trimmed.lines().next().unwrap_or("").trim();

    // TOML: a `[section]` header plus a `key = value`. Checked before JSON so a
    // file starting with `[package]` is not mistaken for a JSON array.
    if first_line.starts_with('[')
        && first_line.ends_with(']')
        && !first_line.contains(',')
        && text.lines().any(|l| {
            let t = l.trim();
            !t.starts_with('#') && !t.starts_with('[') && t.contains('=')
        })
    {
        return "toml".to_string();
    }

    // JSON detection
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        // Check if it's JSONL (multiple JSON objects, one per line)
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        if lines.len() > 1 {
            let first_valid = lines.iter().all(|l| {
                let t = l.trim();
                t.starts_with('{') || t.starts_with('[')
            });
            if first_valid {
                return "jsonl".to_string();
            }
        }
        return "json".to_string();
    }

    // YAML detection (before CSV): a document marker, top-level sequence, or a
    // genuine `key: value` first meaningful line. Because the first meaningful
    // line must be YAML-structural, a CSV whose cells contain colons (e.g.
    // `foo,12:30`) is not taken as YAML — while comment-led YAML, sequences, and
    // flow values with commas (`ports: 80, 443`) still are.
    if looks_like_yaml(&text) {
        return "yaml".to_string();
    }

    // CSV detection (comma-separated with consistent column count).
    let lines: Vec<&str> = text.lines().take(5).collect();
    if lines.len() >= 2 {
        let comma_counts: Vec<usize> = lines.iter().map(|l| l.matches(',').count()).collect();
        if comma_counts.iter().all(|&c| c == comma_counts[0] && c > 0) {
            return "csv".to_string();
        }
    }

    // .env detection
    if lines.iter().all(|l| {
        let t = l.trim();
        t.is_empty() || t.starts_with('#') || t.contains('=')
    }) && lines.iter().any(|l| l.contains('=')) {
        return "env".to_string();
    }

    // Default to text
    "txt".to_string()
}

/// Whether the content looks like YAML: a `---` marker, a top-level sequence
/// (`- item`), or a scalar `key:` mapping — decided by the first meaningful
/// (non-blank, non-comment) line. Comment-only lead-ins are skipped so
/// comment-first YAML is recognized, while a CSV header (no `key:` shape) is not.
fn looks_like_yaml(text: &str) -> bool {
    for line in text.lines().take(20) {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue; // skip blanks and comments
        }
        if t == "---" || t.starts_with("--- ") {
            return true;
        }
        if t == "-" || t.starts_with("- ") {
            return true; // top-level sequence item
        }
        // First meaningful line decides: YAML only if it is a scalar key line.
        return is_yaml_key_line(t);
    }
    false
}

/// Whether a line looks like a YAML mapping entry `key: value` / `key:` where
/// the key is a simple scalar (identifier-ish, no spaces) and the colon is
/// followed by a space or end of line. Rejects `12:30`, `http://x`, `a,b:c`.
fn is_yaml_key_line(line: &str) -> bool {
    match line.find(':') {
        Some(colon) => {
            let key = &line[..colon];
            let after = &line[colon + 1..];
            !key.is_empty()
                && key
                    .chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
                && (after.is_empty() || after.starts_with(' '))
        }
        None => false,
    }
}

impl From<Paging> for app::Paging {
    fn from(value: Paging) -> Self {
        match value {
            Paging::Auto => app::Paging::Auto,
            Paging::Always => app::Paging::Always,
            Paging::Never => app::Paging::Never,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::detect_format;

    fn detect(s: &str) -> String {
        detect_format(s.as_bytes())
    }

    #[test]
    fn toml_section_is_not_json() {
        assert_eq!(detect("[package]\nname = \"vat\"\nversion = \"0.1.0\"\n"), "toml");
        assert_eq!(detect("[a.b]\nx = 1\n"), "toml");
    }

    #[test]
    fn json_array_still_detected() {
        assert_eq!(detect("[1, 2, 3]\n"), "json");
        assert_eq!(detect("{\"a\": 1}\n"), "json");
    }

    #[test]
    fn csv_with_colons_is_not_yaml() {
        assert_eq!(detect("name,time\nfoo,12:30\nbar,09:15\n"), "csv");
    }

    #[test]
    fn yaml_key_value_detected() {
        assert_eq!(detect("name: vat\nversion: 0.1\n"), "yaml");
        assert_eq!(detect("---\nfoo: bar\n"), "yaml");
    }

    #[test]
    fn bare_colon_is_not_yaml() {
        // A single time-like token or URL should not be classed YAML.
        assert_eq!(detect("12:30\n"), "txt");
        assert_eq!(detect("see http://example.com for info\n"), "txt");
    }

    #[test]
    fn yaml_with_comment_lead_and_sequences() {
        // Comment-first YAML, top-level sequences, and flow values with commas
        // must still be YAML (regression guards).
        assert_eq!(detect("# my config\nname: app\n"), "yaml");
        assert_eq!(detect("- name: foo\n  value: bar\n"), "yaml");
        assert_eq!(detect("ports: 80, 443\nhosts: web, db\n"), "yaml");
    }

    #[test]
    fn csv_header_without_colons_still_csv() {
        assert_eq!(detect("id,name,city\n1,foo,ny\n2,bar,la\n"), "csv");
    }
}
