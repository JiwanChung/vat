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
}

impl HtmlEngine {
    pub fn from_path(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
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
        })
    }

    pub fn render(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        self.last_view_height = area.height as usize;
        let visible = self.visible_rows();
        let height = area.height.saturating_sub(1) as usize;
        if self.selection < self.scroll {
            self.scroll = self.selection;
        } else if self.selection >= self.scroll + height {
            self.scroll = self.selection.saturating_sub(height - 1);
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
            let row = &self.rows[*row_idx];
            let mut cells = Vec::new();
            cells.push(
                Cell::from((self.scroll + idx + 1).to_string())
                    .style(Style::default().fg(Color::LightYellow)),
            );
            cells.push(Cell::from("│").style(Style::default().fg(Color::LightBlue)));
            cells.push(Cell::from(indent_tag(row.depth, &row.tag)).style(Style::default().fg(Color::LightGreen)));
            cells.push(Cell::from(row.id.clone()).style(Style::default().fg(Color::LightCyan)));
            cells.push(Cell::from(row.class.clone()).style(Style::default().fg(Color::LightCyan)));
            cells.push(Cell::from(row.text.clone()).style(Style::default().fg(Color::White)));
            rows.push(Row::new(cells));
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
                if self.selection + 1 < self.rows.len() {
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
            "j/k move | gg/G jump | Ctrl+u/d half-page | n/N next/prev | Enter fold | / search{}",
            query
        )
    }

    #[allow(dead_code)]
    pub fn selected_path(&self) -> Option<String> {
        None
    }

    pub fn content_height(&self) -> usize {
        self.visible_rows().len() + 1
    }

    pub fn render_plain_lines(&self, width: u16) -> Vec<Line<'static>> {
        let inner_width = width as usize;
        let (w_num, w_sep, w_tag, w_id, w_class, w_text) = html_column_widths(inner_width);
        let mut lines = Vec::new();

        let header_style = Style::default().fg(Color::Black).bg(Color::LightBlue);
        let headers = vec![
            Span::styled(pad_cell("#", w_num), header_style),
            Span::styled(pad_cell("│", w_sep), Style::default().fg(Color::LightBlue)),
            Span::styled(pad_cell("Tag", w_tag), header_style),
            Span::styled(pad_cell("Id", w_id), header_style),
            Span::styled(pad_cell("Class", w_class), header_style),
            Span::styled(pad_cell("Text", w_text), header_style),
        ];
        lines.push(Line::from(headers));

        for (idx, row_idx) in self.visible_rows().iter().enumerate() {
            let row = &self.rows[*row_idx];
            let spans = vec![
                Span::styled(pad_cell(&(idx + 1).to_string(), w_num), Style::default().fg(Color::LightYellow)),
                Span::styled(pad_cell("│", w_sep), Style::default().fg(Color::LightBlue)),
                Span::styled(pad_cell(&indent_tag(row.depth, &row.tag), w_tag), Style::default().fg(Color::LightGreen)),
                Span::styled(pad_cell(&row.id, w_id), Style::default().fg(Color::LightCyan)),
                Span::styled(pad_cell(&row.class, w_class), Style::default().fg(Color::LightCyan)),
                Span::styled(pad_cell(&row.text, w_text), Style::default().fg(Color::White)),
            ];
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
    let tag = node.value().name().to_string();
    let id = node.value().attr("id").unwrap_or("").to_string();
    let class = node.value().attr("class").unwrap_or("").to_string();
    let text = node.text().collect::<Vec<_>>().join(" ");
    let text = text.trim().to_string();
    let text = truncate_text(&text, 60);
    rows.push(HtmlRow {
        depth,
        tag,
        id,
        class,
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

fn truncate_text(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut out = text.chars().take(max.saturating_sub(3)).collect::<String>();
    out.push_str("...");
    out
}

fn html_column_widths(inner_width: usize) -> (usize, usize, usize, usize, usize, usize) {
    let w_num = 5;
    let w_sep = 2;
    let w_tag = 16;
    let w_id = 18;
    let w_class = 20;
    let used = w_num + w_sep + w_tag + w_id + w_class;
    let w_text = inner_width.saturating_sub(used).max(12);
    (w_num, w_sep, w_tag, w_id, w_class, w_text)
}

fn pad_cell(value: &str, width: usize) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if out.len() >= width {
            break;
        }
        out.push(ch);
    }
    if out.len() < width {
        out.push_str(&" ".repeat(width - out.len()));
    }
    out
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
            {
                self.selection = idx;
                break;
            }
        }
        self.last_match = Some(trimmed.to_string());
    }
}
