use std::fs::File;
use std::path::Path;

use anyhow::{anyhow, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use polars::prelude::*;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Row, Table, TableState};

/// TableEngine for CSV/TSV/Parquet files.
/// Uses Polars DataFrame for efficient columnar storage.
/// Note: For CSV files, the entire file is loaded into memory since CSV doesn't support
/// random access. For Parquet, Polars uses efficient columnar storage with lazy evaluation.
pub struct TableEngine {
    df: DataFrame,
    selection: usize,
    scroll: usize,
    schema_view: bool,
    file_name: String,
    last_query: Option<String>,
    pending_g: bool,
    last_view_height: usize,
    last_match: Option<String>,
}

impl TableEngine {
    pub fn from_path(path: &Path) -> Result<Self> {
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        let df = match ext {
            "csv" => {
                CsvReader::from_path(path)
                    .map_err(|e| anyhow!("CSV open failed: {}", e))?
                    .has_header(true)
                    .finish()
                    .map_err(|e| anyhow!("CSV read failed: {}", e))?
            }
            "tsv" => {
                CsvReader::from_path(path)
                    .map_err(|e| anyhow!("TSV open failed: {}", e))?
                    .has_header(true)
                    .with_separator(b'\t')
                    .finish()
                    .map_err(|e| anyhow!("TSV read failed: {}", e))?
            }
            "parquet" => {
                let file = File::open(path)?;
                ParquetReader::new(file)
                    .finish()
                    .map_err(|e| anyhow!("Parquet read failed: {}", e))?
            }
            _ => return Err(anyhow!("Unsupported tabular format: {}", ext)),
        };

        Ok(Self {
            df,
            selection: 0,
            scroll: 0,
            schema_view: false,
            file_name: path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string(),
            last_query: None,
            pending_g: false,
            last_view_height: 0,
            last_match: None,
        })
    }

    pub fn render(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        self.last_view_height = area.height as usize;
        if self.schema_view {
            self.render_schema(frame, area);
        } else {
            self.render_table(frame, area);
        }
    }

    pub fn content_height(&self) -> usize {
        if self.schema_view {
            self.df.schema().len()
        } else {
            self.df.height() + 1
        }
    }

    pub fn render_plain_lines(&self, _width: u16) -> Vec<Line<'static>> {
        if self.schema_view {
            return self
                .df
                .schema()
                .iter_fields()
                .map(|field| {
                    Line::from(vec![
                        Span::styled(
                            field.name().to_string(),
                            Style::default().fg(Color::LightCyan).bold(),
                        ),
                        Span::raw(": "),
                        Span::styled(
                            field.data_type().to_string(),
                            Style::default().fg(Color::LightYellow),
                        ),
                    ])
                })
                .collect();
        }

        let mut lines = Vec::new();
        let mut headers = Vec::new();
        headers.push(Span::styled("#", Style::default().fg(Color::Black).bg(Color::LightBlue)));
        headers.push(Span::styled("│", Style::default().fg(Color::LightBlue)));
        headers.extend(
            self.df
                .get_column_names()
                .iter()
                .map(|name| {
                    Span::styled(
                        name.to_string(),
                        Style::default().fg(Color::Black).bg(Color::LightBlue),
                    )
                }),
        );
        lines.push(Line::from(join_with_sep(headers, "  ")));

        for row_idx in 0..self.df.height() {
            let mut spans = Vec::new();
            spans.push(Span::styled(
                (row_idx + 1).to_string(),
                Style::default().fg(Color::LightYellow),
            ));
            spans.push(Span::styled("│", Style::default().fg(Color::LightBlue)));
            for series in self.df.get_columns() {
                let value = series
                    .get(row_idx)
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                spans.push(Span::styled(value, Style::default().fg(Color::LightGreen)));
            }
            lines.push(Line::from(join_with_sep(spans, "  ")));
        }
        lines
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
                if self.selection + 1 < self.df.height() {
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
                let max_rows = if self.schema_view {
                    self.df.schema().len()
                } else {
                    self.df.height()
                };
                let jump = page_jump(self.last_view_height).min(max_rows.saturating_sub(1));
                self.selection = (self.selection + jump).min(max_rows.saturating_sub(1));
            }
            KeyCode::Char('s') => {
                self.schema_view = !self.schema_view;
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
            KeyCode::Char('G') => {
                if self.df.height() > 0 {
                    self.selection = self.df.height() - 1;
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
        format!("{} row {}/{}", self.file_name, self.selection + 1, self.df.height())
    }

    pub fn status_line(&self) -> String {
        let view = if self.schema_view { "schema" } else { "data" };
        let query = self
            .last_query
            .as_ref()
            .map(|q| format!(" | search: {}", q))
            .unwrap_or_default();
        format!(
            "j/k move | gg/G jump | Ctrl+u/d half-page | n/N next/prev | s toggle schema | / search | f filter{} | view: {}",
            query, view
        )
    }

    pub fn apply_filter(&mut self, query: &str) {
        // For table, filter acts like search - jump to matching rows
        self.apply_search(query);
    }

    pub fn clear_filter(&mut self) {
        self.last_query = None;
    }

    #[allow(dead_code)]
    pub fn selected_path(&self) -> Option<String> {
        None
    }

    fn render_table(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        if self.df.width() == 0 {
            frame.render_widget(Block::default().borders(Borders::ALL).title("Empty"), area);
            return;
        }

        let height = area.height.saturating_sub(1) as usize;
        if self.selection < self.scroll {
            self.scroll = self.selection;
        } else if self.selection >= self.scroll + height {
            self.scroll = self.selection.saturating_sub(height - 1);
        }

        // Only render the visible slice (data is already in memory, just slicing the view)
        let slice = self
            .df
            .slice(self.scroll as i64, height.min(self.df.height()));

        let header_style = Style::default()
            .fg(Color::Black)
            .bg(Color::LightBlue)
            .bold();
        let mut headers: Vec<Cell> = Vec::new();
        headers.push(Cell::from("#").style(header_style));
        headers.push(Cell::from("│").style(Style::default().fg(Color::LightBlue)));
        headers.extend(
            slice
                .get_column_names()
                .iter()
                .map(|name| Cell::from(*name).style(header_style)),
        );
        let header = Row::new(headers).style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::LightBlue)
                .bold(),
        );

        let mut rows = Vec::new();
        for row_idx in 0..slice.height() {
            let mut cells = Vec::new();
            cells.push(
                Cell::from((self.scroll + row_idx + 1).to_string())
                    .style(Style::default().fg(Color::LightYellow)),
            );
            cells.push(Cell::from("│").style(Style::default().fg(Color::LightBlue)));
            for series in slice.get_columns() {
                let value = series.get(row_idx).map(|v| v.to_string()).unwrap_or_default();
                cells.push(Cell::from(value).style(Style::default().fg(Color::LightGreen)));
            }
            rows.push(Row::new(cells));
        }

        let row_count = rows.len();
        let mut widths = vec![Constraint::Length(6), Constraint::Length(2)];
        widths.extend(make_widths(slice.width()));
        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::default().borders(Borders::NONE))
            .highlight_style(Style::default().bg(Color::LightBlue).fg(Color::Black));

        let mut state = TableState::default();
        if row_count != 0 {
            let relative = self.selection.saturating_sub(self.scroll);
            state.select(Some(relative));
        }
        frame.render_stateful_widget(table, area, &mut state);
    }

    fn render_schema(&self, frame: &mut ratatui::Frame, area: Rect) {
        let mut lines = Vec::new();
        for field in self.df.schema().iter_fields() {
            lines.push(Line::from(format!("{}: {}", field.name(), field.data_type())));
        }
        let block = Block::default().borders(Borders::NONE);
        frame.render_widget(ratatui::widgets::Paragraph::new(lines).block(block), area);
    }

    fn search_next(&mut self, query: &str, forward: bool) {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return;
        }
        let lower = trimmed.to_lowercase();
        let total = self.df.height().max(1);
        let start = if forward {
            (self.selection + 1) % total
        } else {
            self.selection.saturating_sub(1)
        };
        for offset in 0..self.df.height() {
            let idx = if forward {
                (start + offset) % total
            } else {
                (start + total - offset % total) % total
            };
            let mut hit = false;
            for series in self.df.get_columns() {
                if let Ok(value) = series.get(idx) {
                    if value.to_string().to_lowercase().contains(&lower) {
                        hit = true;
                        break;
                    }
                }
            }
            if hit {
                self.selection = idx;
                break;
            }
        }
        self.last_match = Some(trimmed.to_string());
    }
}

fn make_widths(cols: usize) -> Vec<Constraint> {
    if cols == 0 {
        return vec![Constraint::Percentage(100)];
    }
    let base = 100 / cols as u16;
    let mut widths = vec![Constraint::Percentage(base); cols];
    if let Some(last) = widths.last_mut() {
        *last = Constraint::Percentage(100 - base * (cols as u16 - 1));
    }
    widths
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_cover_full_percentage() {
        let widths = make_widths(3);
        let mut total = 0;
        for width in widths {
            if let Constraint::Percentage(value) = width {
                total += value;
            }
        }
        assert_eq!(total, 100);
    }
}
