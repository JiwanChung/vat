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

    // YAML detection (starts with ---, or key: value pattern)
    if trimmed.starts_with("---") || (trimmed.contains(':') && !trimmed.contains('{')) {
        return "yaml".to_string();
    }

    // CSV detection (comma-separated with consistent column count)
    let lines: Vec<&str> = text.lines().take(5).collect();
    if lines.len() >= 2 {
        let comma_counts: Vec<usize> = lines.iter().map(|l| l.matches(',').count()).collect();
        if comma_counts.iter().all(|&c| c == comma_counts[0] && c > 0) {
            return "csv".to_string();
        }
    }

    // TOML detection
    if trimmed.starts_with('[') && trimmed.contains(']') && trimmed.contains('=') {
        return "toml".to_string();
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

impl From<Paging> for app::Paging {
    fn from(value: Paging) -> Self {
        match value {
            Paging::Auto => app::Paging::Auto,
            Paging::Always => app::Paging::Always,
            Paging::Never => app::Paging::Never,
        }
    }
}
