use std::collections::HashSet;
use std::fs::File;
use std::path::Path;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use memmap2::Mmap;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

/// JsonlEngine uses memory-mapped files for efficient streaming of JSON Lines files.
/// Each line is parsed on-demand, only when visible.
pub struct JsonlEngine {
    /// Memory-mapped file content
    mmap: Mmap,
    /// Byte offsets for the start of each line
    line_offsets: Vec<usize>,
    /// Which lines are expanded (show full JSON tree)
    expanded: HashSet<usize>,
    /// Cached parsed previews for visible lines
    selection: usize,
    scroll: usize,
    file_name: String,
    last_query: Option<String>,
    pending_g: bool,
    last_view_height: usize,
    last_match: Option<String>,
    /// Filtered line indices (None = show all)
    filtered_indices: Option<Vec<usize>>,
    /// Visual selection range (start, end) for highlighting
    pub visual_range: Option<(usize, usize)>,
}

impl JsonlEngine {
    pub fn from_path(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        let line_offsets = build_line_offsets(&mmap);

        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        Ok(Self {
            mmap,
            line_offsets,
            expanded: HashSet::new(),
            selection: 0,
            scroll: 0,
            file_name,
            last_query: None,
            pending_g: false,
            last_view_height: 0,
            last_match: None,
            filtered_indices: None,
            visual_range: None,
        })
    }

    /// Get line content at given index (zero-copy from mmap)
    fn get_line(&self, idx: usize) -> Option<&str> {
        if idx >= self.line_offsets.len() {
            return None;
        }
        let start = self.line_offsets[idx];
        let end = if idx + 1 < self.line_offsets.len() {
            self.line_offsets[idx + 1]
        } else {
            self.mmap.len()
        };

        let mut line_end = end;
        if line_end > start && self.mmap.get(line_end - 1) == Some(&b'\n') {
            line_end -= 1;
        }
        if line_end > start && self.mmap.get(line_end - 1) == Some(&b'\r') {
            line_end -= 1;
        }

        std::str::from_utf8(&self.mmap[start..line_end]).ok()
    }

    fn line_count(&self) -> usize {
        self.line_offsets.len()
    }

    fn display_count(&self) -> usize {
        self.filtered_indices.as_ref().map_or(self.line_count(), |f| f.len())
    }

    fn display_to_actual(&self, display_idx: usize) -> Option<usize> {
        match &self.filtered_indices {
            Some(indices) => indices.get(display_idx).copied(),
            None => Some(display_idx),
        }
    }

    /// Parse a line as JSON and create a preview
    fn parse_line_preview(&self, line: &str) -> (String, bool) {
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(value) => {
                let preview = match &value {
                    serde_json::Value::Object(map) => {
                        let keys: Vec<_> = map.keys().take(3).collect();
                        let key_str = keys.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(", ");
                        if map.len() > 3 {
                            format!("{{{}... ({} keys)}}", key_str, map.len())
                        } else {
                            format!("{{{}}}", key_str)
                        }
                    }
                    serde_json::Value::Array(arr) => format!("[{} items]", arr.len()),
                    _ => format!("{}", value),
                };
                (preview, true)
            }
            Err(_) => (line.chars().take(60).collect::<String>(), false),
        }
    }

    /// Render expanded JSON tree for a line
    fn render_expanded(&self, line: &str) -> Vec<(usize, String, Style)> {
        let mut result = Vec::new();
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
            self.flatten_json(&value, 1, &mut result);
        }
        result
    }

    fn flatten_json(&self, value: &serde_json::Value, depth: usize, out: &mut Vec<(usize, String, Style)>) {
        let indent = "  ".repeat(depth);
        match value {
            serde_json::Value::Object(map) => {
                for (key, val) in map.iter() {
                    let preview = self.value_preview(val);
                    out.push((depth, format!("{}{}: {}", indent, key, preview), Style::default().fg(Color::LightCyan)));
                    if matches!(val, serde_json::Value::Object(_) | serde_json::Value::Array(_)) {
                        self.flatten_json(val, depth + 1, out);
                    }
                }
            }
            serde_json::Value::Array(arr) => {
                for (idx, val) in arr.iter().enumerate() {
                    let preview = self.value_preview(val);
                    out.push((depth, format!("{}[{}]: {}", indent, idx, preview), Style::default().fg(Color::LightYellow)));
                    if matches!(val, serde_json::Value::Object(_) | serde_json::Value::Array(_)) {
                        self.flatten_json(val, depth + 1, out);
                    }
                }
            }
            _ => {}
        }
    }

    fn value_preview(&self, value: &serde_json::Value) -> String {
        match value {
            serde_json::Value::Null => "null".to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::String(s) => {
                if s.len() > 40 {
                    format!("\"{}...\"", &s[..37])
                } else {
                    format!("\"{}\"", s)
                }
            }
            serde_json::Value::Array(arr) => format!("[{} items]", arr.len()),
            serde_json::Value::Object(map) => format!("{{{} keys}}", map.len()),
        }
    }

    pub fn render(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let height = area.height as usize;
        self.last_view_height = height;

        if self.selection < self.scroll {
            self.scroll = self.selection;
        } else if self.selection >= self.scroll + height {
            self.scroll = self.selection.saturating_sub(height - 1);
        }

        let total_lines = self.line_count();
        let line_no_width = total_lines.max(1).to_string().len().max(2);

        let mut visible_lines: Vec<Line> = Vec::new();
        let mut line_idx = 0;

        // Build visible content, accounting for expanded lines
        while visible_lines.len() < height && line_idx < total_lines {
            if line_idx < self.scroll {
                line_idx += 1;
                continue;
            }

            if let Some(content) = self.get_line(line_idx) {
                let (preview, is_valid) = self.parse_line_preview(content);
                let selected = line_idx == self.selection;
                let is_expanded = self.expanded.contains(&line_idx);

                // Main line
                let mut spans = Vec::new();
                let line_no = format!("{:>width$} ", line_idx + 1, width = line_no_width);
                let line_no_style = if selected {
                    Style::default().fg(Color::Black).bg(Color::LightBlue).bold()
                } else {
                    Style::default().fg(Color::LightYellow)
                };
                spans.push(Span::styled(line_no, line_no_style));
                spans.push(Span::styled("│ ", Style::default().fg(Color::LightBlue)));

                // Expand/collapse marker
                if is_valid {
                    let marker = if is_expanded { "[-] " } else { "[+] " };
                    spans.push(Span::styled(marker, Style::default().fg(Color::Cyan)));
                } else {
                    spans.push(Span::raw("    "));
                }

                let content_style = if selected {
                    Style::default().fg(Color::Black).bg(Color::LightBlue)
                } else if is_valid {
                    Style::default().fg(Color::LightGreen)
                } else {
                    Style::default().fg(Color::Red)
                };
                spans.push(Span::styled(preview, content_style));
                visible_lines.push(Line::from(spans));

                // Expanded content
                if is_expanded && visible_lines.len() < height {
                    let expanded = self.render_expanded(content);
                    for (_depth, text, style) in expanded {
                        if visible_lines.len() >= height {
                            break;
                        }
                        let mut spans = Vec::new();
                        spans.push(Span::styled(
                            " ".repeat(line_no_width + 1),
                            Style::default(),
                        ));
                        spans.push(Span::styled("│ ", Style::default().fg(Color::LightBlue)));
                        spans.push(Span::styled(text, style));
                        visible_lines.push(Line::from(spans));
                    }
                }
            }
            line_idx += 1;
        }

        let block = Block::default().borders(Borders::NONE);
        frame.render_widget(Paragraph::new(visible_lines).block(block), area);
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

        let total = self.line_count();
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if self.selection + 1 < total {
                    self.selection += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selection = self.selection.saturating_sub(1);
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let jump = page_jump(self.last_view_height).min(self.selection);
                self.selection = self.selection.saturating_sub(jump);
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let jump = page_jump(self.last_view_height).min(total.saturating_sub(1));
                self.selection = (self.selection + jump).min(total.saturating_sub(1));
            }
            KeyCode::Char('G') => {
                if total > 0 {
                    self.selection = total - 1;
                }
            }
            KeyCode::Enter => {
                // Toggle expand/collapse
                if self.expanded.contains(&self.selection) {
                    self.expanded.remove(&self.selection);
                } else {
                    self.expanded.insert(self.selection);
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

    fn search_next(&mut self, query: &str, forward: bool) {
        let lower = query.to_lowercase();
        let total = self.line_count().max(1);
        let start = if forward {
            (self.selection + 1) % total
        } else {
            self.selection.saturating_sub(1)
        };

        for offset in 0..total {
            let idx = if forward {
                (start + offset) % total
            } else {
                (start + total - offset % total) % total
            };
            if let Some(line) = self.get_line(idx) {
                if line.to_lowercase().contains(&lower) {
                    self.selection = idx;
                    break;
                }
            }
        }
        self.last_match = Some(query.to_string());
    }

    pub fn breadcrumbs(&self) -> String {
        format!("{} line {}/{}", self.file_name, self.selection + 1, self.line_count())
    }

    pub fn status_line(&self) -> String {
        let query = self
            .last_query
            .as_ref()
            .map(|q| format!(" | search: {}", q))
            .unwrap_or_default();
        let filter = if self.filtered_indices.is_some() {
            " | F clear filter"
        } else {
            ""
        };
        format!(
            "j/k move | gg/G jump | Ctrl+u/d half-page | Enter expand/collapse | n/N next/prev | / search | f filter{}{}",
            filter, query
        )
    }

    pub fn apply_filter(&mut self, query: &str) {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return;
        }
        let lower = trimmed.to_lowercase();
        let mut matches = Vec::new();
        for idx in 0..self.line_count() {
            if let Some(line) = self.get_line(idx) {
                if line.to_lowercase().contains(&lower) {
                    matches.push(idx);
                }
            }
        }
        self.filtered_indices = Some(matches);
        self.selection = 0;
        self.scroll = 0;
    }

    pub fn clear_filter(&mut self) {
        self.filtered_indices = None;
        self.selection = 0;
        self.scroll = 0;
    }

    #[allow(dead_code)]
    pub fn selected_path(&self) -> Option<String> {
        None
    }

    /// Get the content of the currently selected line
    pub fn get_selected_line(&self) -> Option<String> {
        let actual_idx = self.display_to_actual(self.selection)?;
        self.get_line(actual_idx).map(|s| s.to_string())
    }

    /// Get lines in a range (inclusive), joined by newlines
    pub fn get_lines_range(&self, start: usize, end: usize) -> Option<String> {
        let (start, end) = if start <= end { (start, end) } else { (end, start) };
        let total = self.display_count();
        if start >= total { return None; }
        let end = end.min(total.saturating_sub(1));
        let lines: Vec<String> = (start..=end)
            .filter_map(|display_idx| {
                let actual_idx = self.display_to_actual(display_idx)?;
                self.get_line(actual_idx).map(|s| s.to_string())
            })
            .collect();
        if lines.is_empty() { None } else { Some(lines.join("\n")) }
    }

    /// Get current selection index (for visual mode)
    pub fn selection(&self) -> usize {
        self.selection
    }

    pub fn content_height(&self) -> usize {
        // Base line count + expanded content
        let mut height = self.line_count();
        for &idx in &self.expanded {
            if let Some(line) = self.get_line(idx) {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
                    height += count_json_nodes(&value);
                }
            }
        }
        height
    }

    pub fn render_plain_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let total = self.line_count();
        let line_no_width = total.max(1).to_string().len().max(2);

        (0..total)
            .filter_map(|idx| {
                let content = self.get_line(idx)?;
                let (preview, _) = self.parse_line_preview(content);
                let mut spans = Vec::new();
                let line_no = format!("{:>width$} ", idx + 1, width = line_no_width);
                spans.push(Span::styled(line_no, Style::default().fg(Color::LightYellow)));
                spans.push(Span::styled("│ ", Style::default().fg(Color::LightBlue)));
                spans.push(Span::styled(preview, Style::default().fg(Color::LightGreen)));
                Some(Line::from(spans))
            })
            .collect()
    }
}

fn build_line_offsets(data: &[u8]) -> Vec<usize> {
    let mut offsets = Vec::new();
    offsets.push(0);
    for (i, &byte) in data.iter().enumerate() {
        if byte == b'\n' && i + 1 < data.len() {
            offsets.push(i + 1);
        }
    }
    // Filter out empty lines
    offsets.retain(|&offset| {
        let end = data.iter().skip(offset).position(|&b| b == b'\n').map(|p| offset + p).unwrap_or(data.len());
        end > offset && data[offset..end].iter().any(|&b| !b.is_ascii_whitespace())
    });
    offsets
}

fn count_json_nodes(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Object(map) => {
            map.values().map(|v| 1 + count_json_nodes(v)).sum()
        }
        serde_json::Value::Array(arr) => {
            arr.iter().map(|v| 1 + count_json_nodes(v)).sum()
        }
        _ => 0,
    }
}

fn page_jump(view_height: usize) -> usize {
    let half = view_height / 2;
    if half == 0 { 1 } else { half }
}
