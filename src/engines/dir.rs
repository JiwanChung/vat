//! Render a directory as an annotated Markdown listing (subdirectories, then
//! files with size and detected format), viewed through the Markdown engine.

use anyhow::Result;
use std::path::Path;

/// Build a Markdown listing for a directory.
pub fn to_markdown(path: &Path) -> Result<String> {
    let mut dirs: Vec<String> = Vec::new();
    let mut files: Vec<(String, u64, String)> = Vec::new();

    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_dir() {
            dirs.push(name);
        } else {
            let ext = Path::new(&name)
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            files.push((name, meta.len(), format_label(&ext)));
        }
    }
    dirs.sort();
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = format!("# {}\n\n", path.display());
    out.push_str(&format!(
        "{} directories, {} files\n\n",
        dirs.len(),
        files.len()
    ));

    if !dirs.is_empty() {
        out.push_str("## Directories\n\n");
        for d in &dirs {
            out.push_str(&format!("- `{}/`\n", d));
        }
        out.push('\n');
    }

    if !files.is_empty() {
        out.push_str("## Files\n\n");
        out.push_str("| Name | Size | Type |\n|------|------|------|\n");
        for (name, size, ty) in &files {
            out.push_str(&format!("| {} | {} | {} |\n", name, format_size(*size), ty));
        }
    }
    Ok(out)
}

fn format_label(ext: &str) -> String {
    match ext {
        "" => "—".to_string(),
        e => e.to_uppercase(),
    }
}

fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{:.1} {}", size, UNITS[unit])
    }
}
