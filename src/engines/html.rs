use std::path::Path;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Row, Table, TableState};
use scraper::{ElementRef, Html};

struct HtmlRow {
    depth: usize,
    tag: String,
    id: String,
    class: String,
    attrs: Vec<(String, String)>,
    text: String,
}

pub struct HtmlEngine {
    rows: Vec<HtmlRow>,
    collapsed: std::collections::HashSet<usize>,
    selection: usize,
    scroll: usize,
    file_name: String,
    last_query: Option<String>,
    pending_g: bool,
    last_view_height: usize,
    last_match: Option<String>,
    /// Visual selection range (start, end) for highlighting
    pub visual_range: Option<(usize, usize)>,
}

impl HtmlEngine {
    pub fn from_path(path: &Path) -> Result<Self> {
        let content = super::read_text_file(path)?;
        let doc = Html::parse_document(&content);
        let mut rows = Vec::new();
        let root = doc.root_element();
        collect_elements(root, 0, &mut rows);
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        Ok(Self {
            rows,
            collapsed: std::collections::HashSet::new(),
            selection: 0,
            scroll: 0,
            file_name,
            last_query: None,
            pending_g: false,
            last_view_height: 0,
            last_match: None,
            visual_range: None,
        })
    }

    pub fn render(&mut self, frame: &mut ratatui::Frame, area: Rect, _wrap: bool) {
        self.last_view_height = area.height as usize;
        let visible = self.visible_rows();
        let height = area.height.saturating_sub(1) as usize;
        if self.selection < self.scroll {
            self.scroll = self.selection;
        } else if self.selection >= self.scroll + height {
            self.scroll = self.selection.saturating_sub(height.saturating_sub(1));
        }

        let slice = if visible.is_empty() {
            &[][..]
        } else {
            let end = (self.scroll + height).min(visible.len());
            &visible[self.scroll..end]
        };

        let mut headers = Vec::new();
        let header_style = Style::default()
            .fg(Color::Black)
            .bg(Color::LightBlue)
            .bold();
        headers.push(Cell::from("#").style(header_style));
        headers.push(Cell::from("│").style(Style::default().fg(Color::LightBlue)));
        headers.push(Cell::from("Tag").style(header_style));
        headers.push(Cell::from("Id").style(header_style));
        headers.push(Cell::from("Class").style(header_style));
        headers.push(Cell::from("Text").style(header_style));
        let header = Row::new(headers);

        let mut rows = Vec::new();
        for (idx, row_idx) in slice.iter().enumerate() {
            let row_data = &self.rows[*row_idx];
            let abs_row = self.scroll + idx;
            let in_visual = self.visual_range.map_or(false, |(start, end)| {
                let (lo, hi) = if start <= end { (start, end) } else { (end, start) };
                abs_row >= lo && abs_row <= hi
            });
            let mut cells = Vec::new();
            cells.push(
                Cell::from((abs_row + 1).to_string())
                    .style(Style::default().fg(Color::DarkGray)),
            );
            cells.push(Cell::from("│").style(Style::default().fg(Color::DarkGray)));
            cells.push(Cell::from(indent_tag(row_data.depth, &row_data.tag)).style(Style::default().fg(Color::Cyan).bold()));
            cells.push(Cell::from(row_data.id.clone()).style(Style::default().fg(Color::Magenta)));
            cells.push(Cell::from(row_data.class.clone()).style(Style::default().fg(Color::Green)));
            cells.push(Cell::from(row_data.text.clone()).style(Style::default().fg(Color::Yellow)));
            let mut table_row = Row::new(cells);
            if in_visual {
                table_row = table_row.style(Style::default().bg(Color::LightYellow).fg(Color::Black));
            }
            rows.push(table_row);
        }

        let widths = vec![
            Constraint::Length(6),
            Constraint::Length(2),
            Constraint::Length(12),
            Constraint::Length(18),
            Constraint::Length(20),
            Constraint::Min(10),
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
                if self.selection + 1 < self.visible_rows().len() {
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
                let visible_len = self.visible_rows().len();
                let jump = page_jump(self.last_view_height).min(visible_len.saturating_sub(1));
                self.selection = (self.selection + jump).min(visible_len.saturating_sub(1));
            }
            KeyCode::Char('G') => {
                let visible_len = self.visible_rows().len();
                if visible_len > 0 {
                    self.selection = visible_len - 1;
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
                if let Some(row_index) = self.visible_rows().get(self.selection).copied() {
                    if self.collapsed.contains(&row_index) {
                        self.collapsed.remove(&row_index);
                    } else {
                        self.collapsed.insert(row_index);
                    }
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
        format!(
            "j/k move | gg/G jump | Ctrl+u/d half-page | n/N next/prev | Enter fold | / search | f filter{}",
            query
        )
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

    /// Get the content of the currently selected line
    pub fn get_selected_line(&self) -> Option<String> {
        let visible = self.visible_rows();
        visible.get(self.selection).map(|&idx| {
            let row = &self.rows[idx];
            format!("<{}> {} {}", row.tag, row.id, row.text)
        })
    }

    /// Get lines in a range (inclusive), skipping children of selected parents
    pub fn get_lines_range(&self, start: usize, end: usize) -> Option<String> {
        let (start, end) = if start <= end { (start, end) } else { (end, start) };
        let visible = self.visible_rows();
        let total = visible.len();
        if start >= total { return None; }
        let end = end.min(total.saturating_sub(1));

        let mut results = Vec::new();
        let mut skip_depth: Option<usize> = None;

        for idx in start..=end {
            if let Some(&row_idx) = visible.get(idx) {
                let row = &self.rows[row_idx];

                // Skip children of already-selected parent
                if let Some(parent_depth) = skip_depth {
                    if row.depth > parent_depth {
                        continue;
                    } else {
                        skip_depth = None;
                    }
                }

                results.push(format!("<{}> {} {}", row.tag, row.id, row.text));

                // Check if next row is a child (has greater depth)
                if let Some(&next_row_idx) = visible.get(idx + 1) {
                    if self.rows[next_row_idx].depth > row.depth {
                        skip_depth = Some(row.depth);
                    }
                }
            }
        }

        if results.is_empty() { None } else { Some(results.join("\n")) }
    }

    /// Get current selection index (for visual mode)
    pub fn selection(&self) -> usize {
        self.selection
    }

    pub fn content_height(&self) -> usize {
        self.visible_rows().len() + 1
    }

    pub fn render_plain_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let visible = self.visible_rows();
        let num_width = visible.len().max(1).to_string().len().max(2);
        let sep_style = Style::default().fg(Color::LightBlue);

        let mut lines = Vec::new();

        for (idx, row_idx) in visible.iter().enumerate() {
            let row = &self.rows[*row_idx];
            let indent = "  ".repeat(row.depth);

            let mut spans = vec![
                Span::styled(
                    format!("{:>width$} ", idx + 1, width = num_width),
                    Style::default().fg(Color::LightYellow),
                ),
                Span::styled("│ ", sep_style),
                Span::raw(indent),
                Span::styled(row.tag.clone(), Style::default().fg(Color::LightGreen)),
            ];

            if !row.id.is_empty() {
                spans.push(Span::styled(
                    format!("#{}", row.id),
                    Style::default().fg(Color::LightMagenta),
                ));
            }

            if !row.class.is_empty() {
                let class_str: String = row
                    .class
                    .split_whitespace()
                    .map(|c| format!(".{}", c))
                    .collect();
                spans.push(Span::styled(
                    class_str,
                    Style::default().fg(Color::LightYellow),
                ));
            }

            if !row.attrs.is_empty() {
                let attr_str: String = row
                    .attrs
                    .iter()
                    .map(|(k, v)| format!("{}=\"{}\"", k, v))
                    .collect::<Vec<_>>()
                    .join(" ");
                spans.push(Span::styled(
                    format!("({})", attr_str),
                    Style::default().fg(Color::DarkGray),
                ));
            }

            if !row.text.is_empty() {
                spans.push(Span::styled(
                    format!("  {}", row.text),
                    Style::default().fg(Color::White),
                ));
            }

            lines.push(Line::from(spans));
        }

        lines
    }
}

impl HtmlEngine {
    fn visible_rows(&self) -> Vec<usize> {
        let mut visible = Vec::new();
        let mut skip_depth: Option<usize> = None;
        for (idx, row) in self.rows.iter().enumerate() {
            if let Some(depth) = skip_depth {
                if row.depth > depth {
                    continue;
                }
                skip_depth = None;
            }
            visible.push(idx);
            if self.collapsed.contains(&idx) {
                skip_depth = Some(row.depth);
            }
        }
        visible
    }
}

fn collect_elements(node: ElementRef<'_>, depth: usize, rows: &mut Vec<HtmlRow>) {
    let el = node.value();
    let tag = el.name().to_string();
    let mut id = String::new();
    let mut class = String::new();
    let mut attrs = Vec::new();
    for (name, value) in el.attrs() {
        match name {
            "id" => id = value.to_string(),
            "class" => class = value.to_string(),
            _ => attrs.push((name.to_string(), value.to_string())),
        }
    }
    // Only collect direct text children (not text from nested elements)
    // Normalize whitespace: collapse newlines and multiple spaces into single spaces
    let text: String = node
        .children()
        .filter_map(|child| {
            child
                .value()
                .as_text()
                .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    rows.push(HtmlRow {
        depth,
        tag,
        id,
        class,
        attrs,
        text,
    });
    for child in node.children() {
        if let Some(element) = ElementRef::wrap(child) {
            collect_elements(element, depth + 1, rows);
        }
    }
}

fn indent_tag(depth: usize, tag: &str) -> String {
    let indent = "  ".repeat(depth);
    format!("{}<{}>", indent, tag)
}

fn page_jump(view_height: usize) -> usize {
    let half = view_height / 2;
    if half == 0 { 1 } else { half }
}

impl HtmlEngine {
    fn search_next(&mut self, query: &str, forward: bool) {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return;
        }
        let lower = trimmed.to_lowercase();
        let visible = self.visible_rows();
        let total = visible.len().max(1);
        let start = if forward {
            (self.selection + 1) % total
        } else {
            self.selection.saturating_sub(1)
        };
        for offset in 0..visible.len() {
            let idx = if forward {
                (start + offset) % total
            } else {
                (start + total - offset % total) % total
            };
            let row = &self.rows[visible[idx]];
            if row.tag.to_lowercase().contains(&lower)
                || row.id.to_lowercase().contains(&lower)
                || row.class.to_lowercase().contains(&lower)
                || row.text.to_lowercase().contains(&lower)
                || row.attrs.iter().any(|(k, v)| {
                    k.to_lowercase().contains(&lower) || v.to_lowercase().contains(&lower)
                })
            {
                self.selection = idx;
                break;
            }
        }
        self.last_match = Some(trimmed.to_string());
    }
}
