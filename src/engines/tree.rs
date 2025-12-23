use std::collections::HashSet;
use std::fs::File;
use std::path::Path;

use anyhow::{anyhow, Result};
use crossterm::event::{KeyCode, KeyEvent};
use memmap2::Mmap;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem};

/// Maximum file size for TreeEngine (50MB)
/// For larger files, recommend using JSONL format instead
const MAX_TREE_FILE_SIZE: u64 = 50 * 1024 * 1024;

#[derive(Clone)]
enum NodeKind {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Object,
    Array,
}

#[derive(Clone)]
struct Node {
    label: String,
    kind: NodeKind,
    children: Vec<usize>,
}

struct FlatNode {
    depth: usize,
    copy_path: String,
    breadcrumb: String,
    label: String,
    value_preview: String,
    is_container: bool,
}

pub struct TreeEngine {
    arena: Vec<Node>,
    root: usize,
    selection: usize,
    scroll: usize,
    collapsed: HashSet<String>,
    flat: Vec<FlatNode>,
    last_query: Option<String>,
    pending_g: bool,
    last_view_height: usize,
    last_match: Option<String>,
}

impl TreeEngine {
    /// Create TreeEngine from file path
    /// Uses mmap for efficient file reading
    pub fn from_path(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let metadata = file.metadata()?;

        // Warn for very large files
        if metadata.len() > MAX_TREE_FILE_SIZE {
            return Err(anyhow!(
                "File too large ({:.1}MB) for tree view. Maximum: {}MB.\n\
                 Tip: For large datasets, use JSONL format (.jsonl) which supports streaming.",
                metadata.len() as f64 / 1024.0 / 1024.0,
                MAX_TREE_FILE_SIZE / 1024 / 1024
            ));
        }

        // Use mmap for efficient reading (avoids memory copy)
        let mmap = unsafe { Mmap::map(&file)? };
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        Self::from_bytes_internal(ext, &mmap)
    }

    /// Create TreeEngine from bytes (used by tests)
    #[allow(dead_code)]
    pub fn from_bytes(path: &Path, bytes: &[u8]) -> Result<Self> {
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        Self::from_bytes_internal(ext, bytes)
    }

    fn from_bytes_internal(ext: &str, bytes: &[u8]) -> Result<Self> {
        let value = parse_value(ext, bytes)?;
        let mut arena = Vec::new();
        let root = build_json_node(&value, "root".to_string(), &mut arena);
        let mut engine = Self {
            arena,
            root,
            selection: 0,
            scroll: 0,
            collapsed: HashSet::new(),
            flat: Vec::new(),
            last_query: None,
            pending_g: false,
            last_view_height: 0,
            last_match: None,
        };
        engine.rebuild_flat();
        Ok(engine)
    }

    pub fn render(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        self.rebuild_flat();
        if self.selection >= self.flat.len() {
            self.selection = self.flat.len().saturating_sub(1);
        }

        let height = area.height as usize;
        self.last_view_height = height;
        if self.selection < self.scroll {
            self.scroll = self.selection;
        } else if self.selection >= self.scroll + height {
            self.scroll = self.selection.saturating_sub(height - 1);
        }

        let line_no_width = self.flat.len().max(1).to_string().len().max(2);
        let items: Vec<ListItem> = self
            .flat
            .iter()
            .enumerate()
            .skip(self.scroll)
            .take(height)
            .map(|(idx, flat)| {
                let mut spans = Vec::new();
                let line_no = format!("{:>width$} ", idx + 1, width = line_no_width);
                spans.push(Span::styled(
                    line_no,
                    Style::default().fg(Color::LightYellow),
                ));
                spans.push(Span::styled("│ ", Style::default().fg(Color::LightBlue)));
                let indent = "  ".repeat(flat.depth);
                spans.push(Span::raw(indent));
                if flat.is_container {
                    let marker = if self.collapsed.contains(&flat.copy_path) {
                        "[+] "
                    } else {
                        "[-] "
                    };
                    spans.push(Span::styled(marker, Style::default().fg(Color::Cyan)));
                } else {
                    spans.push(Span::raw("    "));
                }
                spans.push(Span::styled(
                    format!("{}", flat.label),
                    Style::default().bold().fg(Color::LightCyan),
                ));
                if !flat.value_preview.is_empty() {
                    spans.push(Span::raw(" = "));
                    spans.push(Span::styled(
                        flat.value_preview.clone(),
                        Style::default().fg(Color::LightGreen),
                    ));
                }
                ListItem::new(Line::from(spans))
            })
            .collect();

        let list = List::new(items)
            .block(Block::default().borders(Borders::NONE))
            .highlight_style(Style::default().bg(Color::LightBlue).fg(Color::Black));
        frame.render_stateful_widget(list, area, &mut self.list_state());
    }

    pub fn content_height(&mut self) -> usize {
        self.rebuild_flat();
        self.flat.len()
    }

    pub fn render_plain_lines(&mut self) -> Vec<Line<'static>> {
        self.rebuild_flat();
        let line_no_width = self.flat.len().max(1).to_string().len().max(2);
        self.flat
            .iter()
            .enumerate()
            .map(|(idx, flat)| {
                let mut spans = Vec::new();
                let line_no = format!("{:>width$} ", idx + 1, width = line_no_width);
                spans.push(Span::styled(
                    line_no,
                    Style::default().fg(Color::LightYellow),
                ));
                spans.push(Span::styled("│ ", Style::default().fg(Color::LightBlue)));
                let indent = "  ".repeat(flat.depth);
                spans.push(Span::raw(indent));
                if flat.is_container {
                    let marker = if self.collapsed.contains(&flat.copy_path) {
                        "[+] "
                    } else {
                        "[-] "
                    };
                    spans.push(Span::styled(marker, Style::default().fg(Color::Cyan)));
                } else {
                    spans.push(Span::raw("    "));
                }
                spans.push(Span::styled(
                    format!("{}", flat.label),
                    Style::default().bold().fg(Color::LightCyan),
                ));
                if !flat.value_preview.is_empty() {
                    spans.push(Span::raw(" = "));
                    spans.push(Span::styled(
                        flat.value_preview.clone(),
                        Style::default().fg(Color::LightGreen),
                    ));
                }
                Line::from(spans)
            })
            .collect()
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
                if self.selection + 1 < self.flat.len() {
                    self.selection += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selection = self.selection.saturating_sub(1);
            }
            KeyCode::Char('u')
                if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) =>
            {
                let jump = page_jump(self.last_view_height).min(self.selection);
                self.selection = self.selection.saturating_sub(jump);
            }
            KeyCode::Char('d')
                if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) =>
            {
                let jump = page_jump(self.last_view_height).min(self.flat.len().saturating_sub(1));
                self.selection = (self.selection + jump).min(self.flat.len().saturating_sub(1));
            }
            KeyCode::Char('G') => {
                if !self.flat.is_empty() {
                    self.selection = self.flat.len() - 1;
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
            KeyCode::Enter => {
                if let Some(node) = self.flat.get(self.selection) {
                    if node.is_container {
                        if self.collapsed.contains(&node.copy_path) {
                            self.collapsed.remove(&node.copy_path);
                        } else {
                            self.collapsed.insert(node.copy_path.clone());
                        }
                    }
                }
            }
            KeyCode::Char('e') => {
                if let Some(next) = next_top_level_index(&self.flat, self.selection) {
                    self.selection = next;
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
        self.last_match = Some(trimmed.to_string());
        self.rebuild_flat();
        self.search_next(trimmed, true);
    }

    pub fn breadcrumbs(&self) -> String {
        self.flat
            .get(self.selection)
            .map(|f| f.breadcrumb.clone())
            .unwrap_or_else(|| "root".to_string())
    }

    pub fn status_line(&self) -> String {
        let query = self
            .last_query
            .as_ref()
            .map(|q| format!(" | search: {}", q))
            .unwrap_or_default();
        format!(
            "j/k move | gg/G jump | Ctrl+u/d half-page | e next top | n/N next/prev | Enter fold | y copy path | / search | f filter{}",
            query
        )
    }

    pub fn apply_filter(&mut self, query: &str) {
        // For tree, filter acts like search - jump to matching nodes
        self.apply_search(query);
    }

    pub fn clear_filter(&mut self) {
        self.last_query = None;
    }

    pub fn selected_path(&self) -> Option<String> {
        self.flat.get(self.selection).map(|f| f.copy_path.clone())
    }

    fn rebuild_flat(&mut self) {
        self.flat.clear();
        let mut segments = vec!["root".to_string()];
        self.flatten(self.root, 0, &mut segments);
    }

    fn flatten(&mut self, index: usize, depth: usize, segments: &mut Vec<String>) {
        let (label, kind, children) = {
            let node = &self.arena[index];
            (node.label.clone(), node.kind.clone(), node.children.clone())
        };
        let copy_path = path_from_segments(segments);
        let breadcrumb = segments.join(" > ");
        let (value_preview, is_container) = match &kind {
            NodeKind::Null => ("[null]".to_string(), false),
            NodeKind::Bool(value) => (format!("[bool] {}", value), false),
            NodeKind::Number(value) => (format!("[num] {}", value), false),
            NodeKind::String(value) => {
                let mut preview = value.clone();
                if preview.len() > 40 {
                    preview.truncate(37);
                    preview.push_str("...");
                }
                (format!("[str] \"{}\"", preview), false)
            }
            NodeKind::Object => ("[obj]".to_string(), true),
            NodeKind::Array => ("[arr]".to_string(), true),
        };

        self.flat.push(FlatNode {
            depth,
            copy_path,
            breadcrumb,
            label,
            value_preview,
            is_container,
        });

        if is_container && self.collapsed.contains(&path_from_segments(segments)) {
            return;
        }

        for child in children {
            match &self.arena[child].kind {
                NodeKind::Array | NodeKind::Object => {
                    let label = self.arena[child].label.clone();
                    segments.push(label);
                }
                _ => {
                    let label = self.arena[child].label.clone();
                    segments.push(label);
                }
            }
            self.flatten(child, depth + 1, segments);
            segments.pop();
        }
    }

    fn list_state(&self) -> ratatui::widgets::ListState {
        let mut state = ratatui::widgets::ListState::default();
        if !self.flat.is_empty() {
            let relative = self.selection.saturating_sub(self.scroll);
            state.select(Some(relative));
        }
        state
    }
}

fn parse_value(ext: &str, bytes: &[u8]) -> Result<serde_json::Value> {
    match ext {
        "json" => Ok(serde_json::from_slice(bytes)?),
        "yaml" | "yml" => {
            let value: serde_yaml::Value = serde_yaml::from_slice(bytes)?;
            Ok(serde_json::to_value(value)?)
        }
        "toml" => {
            let raw = std::str::from_utf8(bytes)?;
            let value: toml::Value = toml::from_str(raw)?;
            Ok(serde_json::to_value(value)?)
        }
        "kdl" => {
            let raw = std::str::from_utf8(bytes)?;
            let doc: kdl::KdlDocument = raw.parse()?;
            Ok(kdl_to_json(&doc))
        }
        _ => Err(anyhow!("Unsupported structured data extension: {}", ext)),
    }
}

fn build_json_node(value: &serde_json::Value, label: String, arena: &mut Vec<Node>) -> usize {
    let kind = match value {
        serde_json::Value::Null => NodeKind::Null,
        serde_json::Value::Bool(value) => NodeKind::Bool(*value),
        serde_json::Value::Number(value) => NodeKind::Number(value.to_string()),
        serde_json::Value::String(value) => NodeKind::String(value.clone()),
        serde_json::Value::Array(_) => NodeKind::Array,
        serde_json::Value::Object(_) => NodeKind::Object,
    };
    let index = arena.len();
    arena.push(Node {
        label,
        kind: kind.clone(),
        children: Vec::new(),
    });

    match value {
        serde_json::Value::Array(items) => {
            for (idx, child) in items.iter().enumerate() {
                let label = format!("[{}]", idx);
                let child_index = build_json_node(child, label, arena);
                arena[index].children.push(child_index);
            }
        }
        serde_json::Value::Object(map) => {
            for (key, child) in map.iter() {
                let child_index = build_json_node(child, key.clone(), arena);
                arena[index].children.push(child_index);
            }
        }
        _ => {}
    }

    index
}

fn kdl_to_json(doc: &kdl::KdlDocument) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for node in doc.nodes() {
        let name = node.name().to_string();
        let mut node_map = serde_json::Map::new();
        let mut args = Vec::new();
        let mut props = serde_json::Map::new();
        for entry in node.entries() {
            if let Some(name) = entry.name() {
                props.insert(name.to_string(), kdl_value_to_json(entry.value()));
            } else {
                args.push(kdl_value_to_json(entry.value()));
            }
        }
        if !args.is_empty() {
            node_map.insert("args".to_string(), serde_json::Value::Array(args));
        }
        if !props.is_empty() {
            node_map.insert("props".to_string(), serde_json::Value::Object(props));
        }
        if let Some(children) = node.children() {
            node_map.insert("children".to_string(), kdl_to_json(children));
        }
        map.insert(name, serde_json::Value::Object(node_map));
    }
    serde_json::Value::Object(map)
}

fn kdl_value_to_json(value: &kdl::KdlValue) -> serde_json::Value {
    match value {
        kdl::KdlValue::String(value) => serde_json::Value::String(value.clone()),
        kdl::KdlValue::RawString(value) => serde_json::Value::String(value.clone()),
        kdl::KdlValue::Base2(value)
        | kdl::KdlValue::Base8(value)
        | kdl::KdlValue::Base10(value)
        | kdl::KdlValue::Base16(value) => serde_json::Value::Number((*value).into()),
        kdl::KdlValue::Base10Float(value) => serde_json::Number::from_f64(*value)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        kdl::KdlValue::Bool(value) => serde_json::Value::Bool(*value),
        kdl::KdlValue::Null => serde_json::Value::Null,
    }
}

fn path_from_segments(segments: &[String]) -> String {
    let mut path = String::new();
    for (idx, seg) in segments.iter().enumerate() {
        if idx == 0 {
            path.push_str(seg);
            continue;
        }
        if seg.starts_with('[') {
            path.push_str(seg);
        } else {
            path.push('.');
            path.push_str(seg);
        }
    }
    path
}

fn next_top_level_index(flat: &[FlatNode], current: usize) -> Option<usize> {
    for (idx, node) in flat.iter().enumerate().skip(current + 1) {
        if node.depth == 1 {
            return Some(idx);
        }
    }
    None
}

fn page_jump(view_height: usize) -> usize {
    let half = view_height / 2;
    if half == 0 { 1 } else { half }
}

impl TreeEngine {
    pub fn search_next(&mut self, query: &str, forward: bool) {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return;
        }
        self.rebuild_flat();
        let lower = trimmed.to_lowercase();
        let total = self.flat.len().max(1);
        let start = if forward {
            (self.selection + 1) % total
        } else {
            self.selection.saturating_sub(1)
        };
        for offset in 0..self.flat.len() {
            let idx = if forward {
                (start + offset) % total
            } else {
                (start + total - offset % total) % total
            };
            let flat = &self.flat[idx];
            if flat.label.to_lowercase().contains(&lower)
                || flat.value_preview.to_lowercase().contains(&lower)
            {
                self.selection = idx;
                break;
            }
        }
        self.last_match = Some(trimmed.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_segments_handle_arrays() {
        let segments = vec![
            "root".to_string(),
            "metadata".to_string(),
            "[2]".to_string(),
        ];
        assert_eq!(path_from_segments(&segments), "root.metadata[2]");
    }

    #[test]
    fn build_json_node_collects_children() {
        let value = serde_json::json!({
            "name": "vat",
            "tags": [true, false],
        });
        let mut arena = Vec::new();
        let root = build_json_node(&value, "root".to_string(), &mut arena);
        assert_eq!(arena[root].children.len(), 2);
    }
}
