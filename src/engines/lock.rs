use std::path::Path;

use anyhow::{anyhow, Result};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Row, Table, TableState};

#[derive(Clone)]
struct LockEntry {
    name: String,
    version: String,
    source: String,
    checksum: String,
    dependencies: Vec<String>,
}

pub struct LockEngine {
    entries: Vec<LockEntry>,
    selection: usize,
    scroll: usize,
    file_name: String,
    last_query: Option<String>,
    pending_g: bool,
    last_match: Option<String>,
}

impl LockEngine {
    pub fn from_path(path: &Path) -> Result<Self> {
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let entries = if file_name == "Cargo.lock" {
            parse_cargo_lock(path)?
        } else if file_name == "package-lock.json" {
            parse_package_lock(path)?
        } else if file_name == "pnpm-lock.yaml" || file_name == "pnpm-lock.yml" {
            parse_pnpm_lock(path)?
        } else {
            return Err(anyhow!("Unsupported lockfile: {}", file_name));
        };
        Ok(Self {
            entries,
            selection: 0,
            scroll: 0,
            file_name,
            last_query: None,
            pending_g: false,
            last_match: None,
        })
    }

    pub fn render(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let height = area.height.saturating_sub(1) as usize;
        if self.selection < self.scroll {
            self.scroll = self.selection;
        } else if self.selection >= self.scroll + height {
            self.scroll = self.selection.saturating_sub(height - 1);
        }

        let slice = if self.entries.is_empty() {
            &[][..]
        } else {
            let end = (self.scroll + height).min(self.entries.len());
            &self.entries[self.scroll..end]
        };

        let mut headers = Vec::new();
        let header_style = Style::default()
            .fg(Color::Black)
            .bg(Color::LightBlue)
            .bold();
        headers.push(Cell::from("#").style(header_style));
        headers.push(Cell::from("│").style(Style::default().fg(Color::LightBlue)));
        headers.push(Cell::from("Name").style(header_style));
        headers.push(Cell::from("Version").style(header_style));
        headers.push(Cell::from("Source").style(header_style));
        headers.push(Cell::from("Checksum").style(header_style));
        headers.push(Cell::from("Dependencies").style(header_style));
        let header = Row::new(headers);

        let mut rows = Vec::new();
        for (idx, entry) in slice.iter().enumerate() {
            let mut cells = Vec::new();
            cells.push(
                Cell::from((self.scroll + idx + 1).to_string())
                    .style(Style::default().fg(Color::LightYellow)),
            );
            cells.push(Cell::from("│").style(Style::default().fg(Color::LightBlue)));
            cells.push(Cell::from(truncate(&entry.name, 22)).style(Style::default().fg(Color::LightGreen)));
            cells.push(Cell::from(truncate(&entry.version, 12)).style(Style::default().fg(Color::LightCyan)));
            cells.push(Cell::from(truncate(&entry.source, 28)).style(Style::default().fg(Color::LightCyan)));
            cells.push(Cell::from(truncate(&entry.checksum, 16)).style(Style::default().fg(Color::LightMagenta)));
            cells.push(Cell::from(truncate(&entry.dependencies.join(", "), 40)).style(Style::default().fg(Color::White)));
            rows.push(Row::new(cells));
        }

        let widths = vec![
            Constraint::Length(6),
            Constraint::Length(2),
            Constraint::Length(24),
            Constraint::Length(12),
            Constraint::Length(28),
            Constraint::Length(16),
            Constraint::Min(12),
        ];
        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::default().borders(Borders::NONE))
            .highlight_style(Style::default().bg(Color::LightBlue).fg(Color::Black));

        let mut state = TableState::default();
        if !slice.is_empty() {
            let relative = self.selection.saturating_sub(self.scroll);
            state.select(Some(relative));
        }
        frame.render_stateful_widget(table, area, &mut state);
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('g') => {
                if self.pending_g {
                    self.selection = 0;
                    self.pending_g = false;
                } else {
                    self.pending_g = true;
                }
                return;
            }
            _ => {
                self.pending_g = false;
            }
        }
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if self.selection + 1 < self.entries.len() {
                    self.selection += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selection = self.selection.saturating_sub(1);
            }
            KeyCode::Char('G') => {
                if !self.entries.is_empty() {
                    self.selection = self.entries.len() - 1;
                }
            }
            KeyCode::Char('n') => {
                if let Some(query) = self.last_match.clone() {
                    self.search_next(&query, true);
                }
            }
            KeyCode::Char('N') => {
                if let Some(query) = self.last_match.clone() {
                    self.search_next(&query, false);
                }
            }
            _ => {}
        }
    }

    pub fn apply_search(&mut self, query: &str) {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return;
        }
        self.last_query = Some(trimmed.to_string());
        self.search_next(trimmed, true);
        self.last_match = Some(trimmed.to_string());
    }

    pub fn breadcrumbs(&self) -> String {
        format!("{} row {}", self.file_name, self.selection + 1)
    }

    pub fn status_line(&self) -> String {
        let query = self
            .last_query
            .as_ref()
            .map(|q| format!(" | search: {}", q))
            .unwrap_or_default();
        format!("j/k move | gg/G jump | n/N next/prev | / search | f filter{}", query)
    }

    pub fn apply_filter(&mut self, query: &str) {
        self.apply_search(query);
    }

    pub fn clear_filter(&mut self) {
        self.last_query = None;
    }

    #[allow(dead_code)]
    pub fn selected_path(&self) -> Option<String> {
        None
    }

    pub fn content_height(&self) -> usize {
        self.entries.len() + 1
    }

    pub fn render_plain_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let headers = vec![
            Span::styled("#", Style::default().fg(Color::Black).bg(Color::LightBlue)),
            Span::styled("│", Style::default().fg(Color::LightBlue)),
            Span::styled("Name", Style::default().fg(Color::Black).bg(Color::LightBlue)),
            Span::styled("Version", Style::default().fg(Color::Black).bg(Color::LightBlue)),
            Span::styled("Source", Style::default().fg(Color::Black).bg(Color::LightBlue)),
            Span::styled("Checksum", Style::default().fg(Color::Black).bg(Color::LightBlue)),
            Span::styled("Dependencies", Style::default().fg(Color::Black).bg(Color::LightBlue)),
        ];
        lines.push(Line::from(join_with_sep(headers, "  ")));
        for (idx, entry) in self.entries.iter().enumerate() {
            let spans = vec![
                Span::styled((idx + 1).to_string(), Style::default().fg(Color::LightYellow)),
                Span::styled("│", Style::default().fg(Color::LightBlue)),
                Span::styled(entry.name.clone(), Style::default().fg(Color::White)),
                Span::styled(entry.version.clone(), Style::default().fg(Color::LightCyan)),
                Span::styled(entry.source.clone(), Style::default().fg(Color::LightCyan)),
                Span::styled(entry.checksum.clone(), Style::default().fg(Color::LightCyan)),
                Span::styled(entry.dependencies.join(", "), Style::default().fg(Color::White)),
            ];
            lines.push(Line::from(join_with_sep(spans, "  ")));
        }
        lines
    }
}

fn parse_cargo_lock(path: &Path) -> Result<Vec<LockEntry>> {
    let content = std::fs::read_to_string(path)?;
    let value: toml::Value = toml::from_str(&content)?;
    let packages = value
        .get("package")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("Cargo.lock missing package list"))?;
    let mut entries = Vec::new();
    for pkg in packages {
        let name = pkg
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let version = pkg
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let source = pkg
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let checksum = pkg
            .get("checksum")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let dependencies = pkg
            .get("dependencies")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(dep_name_only)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        entries.push(LockEntry {
            name,
            version,
            source,
            checksum,
            dependencies,
        });
    }
    Ok(entries)
}

fn parse_package_lock(path: &Path) -> Result<Vec<LockEntry>> {
    let content = std::fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&content)?;
    if let Some(packages) = value.get("packages").and_then(|v| v.as_object()) {
        let mut entries = Vec::new();
        for (key, info) in packages {
            if key.is_empty() {
                continue;
            }
            let name = info
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| package_name_from_path(key));
            let version = info
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let source = info
                .get("resolved")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let checksum = info
                .get("integrity")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let dependencies = info
                .get("dependencies")
                .and_then(|v| v.as_object())
                .map(|deps| deps.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            entries.push(LockEntry {
                name,
                version,
                source,
                checksum,
                dependencies,
            });
        }
        return Ok(entries);
    }

    let deps = value
        .get("dependencies")
        .and_then(|v| v.as_object())
        .ok_or_else(|| anyhow!("package-lock.json missing dependencies"))?;
    let mut entries = Vec::new();
    flatten_package_lock_deps(deps, &mut entries);
    Ok(entries)
}

fn flatten_package_lock_deps(deps: &serde_json::Map<String, serde_json::Value>, entries: &mut Vec<LockEntry>) {
    for (name, info) in deps {
        let version = info
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let source = info
            .get("resolved")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let checksum = info
            .get("integrity")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let dependencies = info
            .get("dependencies")
            .and_then(|v| v.as_object())
            .map(|deps| deps.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        entries.push(LockEntry {
            name: name.clone(),
            version,
            source,
            checksum,
            dependencies,
        });
        if let Some(nested) = info.get("dependencies").and_then(|v| v.as_object()) {
            flatten_package_lock_deps(nested, entries);
        }
    }
}

fn parse_pnpm_lock(path: &Path) -> Result<Vec<LockEntry>> {
    let content = std::fs::read_to_string(path)?;
    let value: serde_yaml::Value = serde_yaml::from_str(&content)?;
    let json = serde_json::to_value(value)?;
    let packages = json
        .get("packages")
        .and_then(|v| v.as_object())
        .ok_or_else(|| anyhow!("pnpm lock missing packages"))?;
    let mut entries = Vec::new();
    for (key, info) in packages {
        let (name, version) = parse_pnpm_key(key);
        let source = info
            .get("resolution")
            .and_then(|v| v.get("tarball"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let checksum = info
            .get("resolution")
            .and_then(|v| v.get("integrity"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let mut dependencies = Vec::new();
        for field in ["dependencies", "optionalDependencies", "devDependencies"] {
            if let Some(dep_map) = info.get(field).and_then(|v| v.as_object()) {
                dependencies.extend(dep_map.keys().cloned());
            }
        }
        entries.push(LockEntry {
            name,
            version,
            source,
            checksum,
            dependencies,
        });
    }
    Ok(entries)
}

fn parse_pnpm_key(key: &str) -> (String, String) {
    let trimmed = key.trim_start_matches('/');
    let parts: Vec<&str> = trimmed.split('/').collect();
    if parts.len() >= 3 && parts[0].starts_with('@') {
        let name = format!("{}/{}", parts[0], parts[1]);
        let version = parts[2..].join("/");
        return (name, version);
    }
    if parts.len() >= 2 {
        let name = parts[..parts.len() - 1].join("/");
        let version = parts[parts.len() - 1].to_string();
        return (name, version);
    }
    (trimmed.to_string(), String::new())
}

fn package_name_from_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() >= 2 && parts[parts.len() - 2].starts_with('@') {
        return format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1]);
    }
    parts.last().unwrap_or(&path).to_string()
}

fn dep_name_only(dep: &str) -> String {
    dep.split_whitespace().next().unwrap_or(dep).to_string()
}

fn truncate(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_string();
    }
    let mut out = value.chars().take(max.saturating_sub(3)).collect::<String>();
    out.push_str("...");
    out
}

fn join_with_sep(mut spans: Vec<Span<'static>>, sep: &str) -> Vec<Span<'static>> {
    if spans.is_empty() {
        return spans;
    }
    let mut joined = Vec::new();
    for (idx, span) in spans.drain(..).enumerate() {
        if idx > 0 {
            joined.push(Span::raw(sep.to_string()));
        }
        joined.push(span);
    }
    joined
}

impl LockEngine {
    fn search_next(&mut self, query: &str, forward: bool) {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return;
        }
        let lower = trimmed.to_lowercase();
        let total = self.entries.len().max(1);
        let start = if forward {
            (self.selection + 1) % total
        } else {
            self.selection.saturating_sub(1)
        };
        for offset in 0..self.entries.len() {
            let idx = if forward {
                (start + offset) % total
            } else {
                (start + total - offset % total) % total
            };
            let entry = &self.entries[idx];
            if entry.name.to_lowercase().contains(&lower)
                || entry.version.to_lowercase().contains(&lower)
                || entry.source.to_lowercase().contains(&lower)
                || entry.checksum.to_lowercase().contains(&lower)
                || entry
                    .dependencies
                    .iter()
                    .any(|dep| dep.to_lowercase().contains(&lower))
            {
                self.selection = idx;
                break;
            }
        }
        self.last_match = Some(trimmed.to_string());
    }
}
