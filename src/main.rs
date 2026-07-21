use std::io::{self, IsTerminal, Read, Write};
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use clap::{Parser, ValueEnum};

mod analyzer;
mod app;
mod engines;
mod search;
#[cfg(test)]
mod render_snapshots;

#[derive(Parser, Debug)]
#[command(name = "vat", version, about = "Semantic file viewer")]
struct Args {
    /// Files to view (use "-" for stdin). Switch between them with ] / [.
    paths: Vec<String>,
    /// Paging mode: auto, always, never (bat-compatible)
    #[arg(long, value_enum, default_value = "auto")]
    paging: Paging,
    /// Output raw file content without formatting (useful for piping)
    #[arg(short = 'p', long)]
    plain: bool,
    /// Language/format hint for stdin (e.g., json, yaml, csv, jsonl)
    #[arg(short = 'l', long)]
    language: Option<String>,
    /// When to use color: auto (default), always, never. NO_COLOR is honored.
    #[arg(long, value_enum, default_value = "auto")]
    color: ColorChoice,
    /// Show only a line range in plain output, e.g. "20:40", ":40", "20:".
    #[arg(short = 'r', long)]
    line_range: Option<String>,
    /// Print a shell completion script to stdout and exit.
    #[arg(long, value_name = "SHELL")]
    completions: Option<clap_complete::Shell>,
    /// Print a man page (roff) to stdout and exit.
    #[arg(long)]
    man: bool,
}

#[derive(ValueEnum, Clone, Debug)]
enum Paging {
    Auto,
    Always,
    Never,
}

#[derive(ValueEnum, Clone, Debug)]
enum ColorChoice {
    Auto,
    Always,
    Never,
}

/// Parse a `START:END` line range (either side optional) into 1-based bounds.
fn parse_line_range(s: &str) -> Option<(usize, usize)> {
    let (a, b) = s.split_once(':')?;
    let start = if a.trim().is_empty() { 1 } else { a.trim().parse().ok()? };
    let end = if b.trim().is_empty() { usize::MAX } else { b.trim().parse().ok()? };
    Some((start.max(1), end))
}

fn main() -> Result<()> {
    use clap::CommandFactory;
    let args = Args::parse();

    // Generator flags: print and exit without needing a path.
    if let Some(shell) = args.completions {
        clap_complete::generate(shell, &mut Args::command(), "vat", &mut io::stdout());
        return Ok(());
    }
    if args.man {
        clap_mangen::Man::new(Args::command()).render(&mut io::stdout())?;
        return Ok(());
    }

    if args.paths.is_empty() {
        Args::command().print_help()?;
        return Ok(());
    }

    // Build the list of files to view. `-` reads stdin into a temp file that we
    // keep alive (`_temps`) for the duration of the run.
    let mut _temps = Vec::new();
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    for arg_path in &args.paths {
        if arg_path == "-" {
            let (path, temp) = read_stdin_to_temp(&args.language)?;
            if let Some(t) = temp {
                _temps.push(t);
            }
            let display = format!(
                "<stdin>{}",
                args.language.as_ref().map(|l| format!(".{}", l)).unwrap_or_default()
            );
            files.push((display, path));
        } else {
            files.push((arg_path.clone(), PathBuf::from(arg_path)));
        }
    }

    // Resolve color: NO_COLOR (any value) forces off; auto follows the TTY.
    let use_color = match args.color {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => std::env::var_os("NO_COLOR").is_none() && io::stdout().is_terminal(),
    };
    let line_range = args.line_range.as_deref().and_then(parse_line_range);

    let engine = analyzer::analyze(&files[0].1)?;
    let mut app = app::App::new(
        engine,
        files,
        0,
        args.paging.into(),
        args.plain,
        use_color,
        line_range,
    );
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
