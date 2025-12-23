use std::path::Path;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Row, Table, TableState};

#[derive(Clone)]
struct EnvEntry {
    key: String,
    value: String,
    category: String,
    is_secret: bool,
    line_no: usize,
}

pub struct EnvEngine {
    entries: Vec<EnvEntry>,
    selection: usize,
    scroll: usize,
    file_name: String,
    last_query: Option<String>,
    pending_g: bool,
    last_view_height: usize,
    last_match: Option<String>,
    show_secrets: bool,
    /// Visual selection range (start, end) for highlighting
    pub visual_range: Option<(usize, usize)>,
}

impl EnvEngine {
    pub fn from_path(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        let entries = parse_env(&content);

        Ok(Self {
            entries,
            selection: 0,
            scroll: 0,
            file_name,
            last_query: None,
            pending_g: false,
            last_view_height: 0,
            last_match: None,
            show_secrets: false,
            visual_range: None,
        })
    }

    pub fn render(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        self.last_view_height = area.height as usize;
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

        let header_style = Style::default()
            .fg(Color::Black)
            .bg(Color::LightBlue)
            .bold();
        let headers = vec![
            Cell::from("#").style(header_style),
            Cell::from("│").style(Style::default().fg(Color::LightBlue)),
            Cell::from("Category").style(header_style),
            Cell::from("Key").style(header_style),
            Cell::from("Value").style(header_style),
        ];
        let header = Row::new(headers);

        let mut rows = Vec::new();
        for entry in slice.iter() {
            let display_value = if entry.is_secret && !self.show_secrets {
                "••••••••".to_string()
            } else {
                truncate(&entry.value, 50)
            };

            // Smart value coloring
            let value_style = if entry.is_secret {
                Style::default().fg(Color::Red)
            } else {
                let v = entry.value.to_lowercase();
                if v == "true" || v == "false" || v == "yes" || v == "no" {
                    Style::default().fg(Color::Cyan)
                } else if entry.value.parse::<f64>().is_ok() {
                    Style::default().fg(Color::Magenta)
                } else if entry.value.starts_with('/') || entry.value.contains("://") {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::Yellow)
                }
            };

            let cells = vec![
                Cell::from(entry.line_no.to_string())
                    .style(Style::default().fg(Color::DarkGray)),
                Cell::from("│").style(Style::default().fg(Color::DarkGray)),
                Cell::from(entry.category.clone())
                    .style(Style::default().fg(Color::Magenta)),
                Cell::from(entry.key.clone())
                    .style(Style::default().fg(Color::White).bold()),
                Cell::from(display_value).style(value_style),
            ];
            rows.push(Row::new(cells));
        }

        let widths = vec![
            Constraint::Length(5),
            Constraint::Length(2),
            Constraint::Length(12),
            Constraint::Length(28),
            Constraint::Min(20),
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
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let jump = page_jump(self.last_view_height).min(self.selection);
                self.selection = self.selection.saturating_sub(jump);
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let jump = page_jump(self.last_view_height).min(self.entries.len().saturating_sub(1));
                self.selection = (self.selection + jump).min(self.entries.len().saturating_sub(1));
            }
            KeyCode::Char('G') => {
                if !self.entries.is_empty() {
                    self.selection = self.entries.len() - 1;
                }
            }
            KeyCode::Char('s') => {
                self.show_secrets = !self.show_secrets;
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

    pub fn apply_filter(&mut self, query: &str) {
        self.apply_search(query);
    }

    pub fn clear_filter(&mut self) {
        self.last_query = None;
    }

    pub fn breadcrumbs(&self) -> String {
        format!("{} row {}/{}", self.file_name, self.selection + 1, self.entries.len())
    }

    pub fn status_line(&self) -> String {
        let secrets = if self.show_secrets { "shown" } else { "hidden" };
        let query = self
            .last_query
            .as_ref()
            .map(|q| format!(" | search: {}", q))
            .unwrap_or_default();
        format!(
            "j/k move | gg/G jump | Ctrl+u/d half-page | n/N next/prev | s toggle secrets ({}) | / search | f filter{}",
            secrets, query
        )
    }

    #[allow(dead_code)]
    pub fn selected_path(&self) -> Option<String> {
        None
    }

    /// Get the content of the currently selected line
    pub fn get_selected_line(&self) -> Option<String> {
        if self.selection == 0 {
            Some("Category\tKey\tValue".to_string())
        } else {
            self.entries.get(self.selection.saturating_sub(1)).map(|e| {
                let value = if self.show_secrets || !e.is_secret { &e.value } else { "********" };
                format!("{}\t{}\t{}", e.category, e.key, value)
            })
        }
    }

    /// Get lines in a range (inclusive), joined by newlines
    pub fn get_lines_range(&self, start: usize, end: usize) -> Option<String> {
        let (start, end) = if start <= end { (start, end) } else { (end, start) };
        let total = self.entries.len() + 1;
        if start >= total { return None; }
        let end = end.min(total.saturating_sub(1));
        let lines: Vec<String> = (start..=end)
            .filter_map(|idx| {
                if idx == 0 {
                    Some("Category\tKey\tValue".to_string())
                } else {
                    self.entries.get(idx.saturating_sub(1)).map(|e| {
                        let value = if self.show_secrets || !e.is_secret { &e.value } else { "********" };
                        format!("{}\t{}\t{}", e.category, e.key, value)
                    })
                }
            })
            .collect();
        if lines.is_empty() { None } else { Some(lines.join("\n")) }
    }

    /// Get current selection index (for visual mode)
    pub fn selection(&self) -> usize {
        self.selection
    }

    pub fn content_height(&self) -> usize {
        self.entries.len() + 1
    }

    pub fn render_plain_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let header_style = Style::default().fg(Color::Black).bg(Color::LightBlue);
        let headers = vec![
            Span::styled("#", header_style),
            Span::styled("│", Style::default().fg(Color::LightBlue)),
            Span::styled("Category", header_style),
            Span::styled("Key", header_style),
            Span::styled("Value", header_style),
        ];
        lines.push(Line::from(join_with_sep(headers, "  ")));

        for entry in &self.entries {
            let display_value = if entry.is_secret {
                "••••••••".to_string()
            } else {
                entry.value.clone()
            };

            let spans = vec![
                Span::styled(entry.line_no.to_string(), Style::default().fg(Color::LightYellow)),
                Span::styled("│", Style::default().fg(Color::LightBlue)),
                Span::styled(entry.category.clone(), Style::default().fg(Color::LightMagenta)),
                Span::styled(entry.key.clone(), Style::default().fg(Color::LightCyan)),
                Span::styled(display_value, Style::default().fg(Color::LightGreen)),
            ];
            lines.push(Line::from(join_with_sep(spans, "  ")));
        }
        lines
    }

    fn search_next(&mut self, query: &str, forward: bool) {
        let lower = query.to_lowercase();
        let total = self.entries.len().max(1);
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
            let entry = &self.entries[idx];
            if entry.key.to_lowercase().contains(&lower)
                || entry.value.to_lowercase().contains(&lower)
                || entry.category.to_lowercase().contains(&lower)
            {
                self.selection = idx;
                break;
            }
        }
        self.last_match = Some(query.to_string());
    }
}

fn parse_env(content: &str) -> Vec<EnvEntry> {
    let mut entries = Vec::new();

    for (line_no, line) in content.lines().enumerate() {
        let trimmed = line.trim();

        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Parse KEY=value
        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos].trim().to_string();
            let mut value = trimmed[eq_pos + 1..].trim().to_string();

            // Remove quotes if present
            if (value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\''))
            {
                value = value[1..value.len() - 1].to_string();
            }

            let category = categorize_key(&key);
            let is_secret = is_secret_key(&key);

            entries.push(EnvEntry {
                key,
                value,
                category,
                is_secret,
                line_no: line_no + 1,
            });
        }
    }

    entries
}

fn categorize_key(key: &str) -> String {
    let upper = key.to_uppercase();

    if upper.contains("DATABASE") || upper.contains("DB_") || upper.starts_with("DB")
        || upper.contains("POSTGRES") || upper.contains("MYSQL") || upper.contains("MONGO")
        || upper.contains("REDIS") {
        return "Database".to_string();
    }

    if upper.contains("API") || upper.contains("ENDPOINT") || upper.contains("URL")
        || upper.contains("HOST") || upper.contains("PORT") {
        return "API/Network".to_string();
    }

    if upper.contains("KEY") || upper.contains("SECRET") || upper.contains("TOKEN")
        || upper.contains("PASSWORD") || upper.contains("CREDENTIAL") || upper.contains("AUTH") {
        return "Auth/Secret".to_string();
    }

    if upper.contains("AWS") || upper.contains("AZURE") || upper.contains("GCP")
        || upper.contains("CLOUD") || upper.contains("S3") {
        return "Cloud".to_string();
    }

    if upper.contains("LOG") || upper.contains("DEBUG") || upper.contains("ENV")
        || upper.contains("MODE") || upper.contains("NODE_ENV") {
        return "Config".to_string();
    }

    if upper.contains("MAIL") || upper.contains("SMTP") || upper.contains("EMAIL") {
        return "Email".to_string();
    }

    "General".to_string()
}

fn is_secret_key(key: &str) -> bool {
    let upper = key.to_uppercase();
    upper.contains("SECRET")
        || upper.contains("PASSWORD")
        || upper.contains("TOKEN")
        || upper.contains("KEY") && !upper.contains("PUBLIC")
        || upper.contains("CREDENTIAL")
        || upper.contains("PRIVATE")
}

fn truncate(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_string();
    }
    let mut out = value.chars().take(max.saturating_sub(3)).collect::<String>();
    out.push_str("...");
    out
}

fn page_jump(view_height: usize) -> usize {
    let half = view_height / 2;
    if half == 0 { 1 } else { half }
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
