use std::fs::File;
use std::path::Path;

use anyhow::{anyhow, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use polars::prelude::*;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState};

/// TableEngine for CSV/TSV/Parquet files.
/// Uses Polars DataFrame for efficient columnar storage.
/// Note: For CSV files, the entire file is loaded into memory since CSV doesn't support
/// random access. For Parquet, Polars uses efficient columnar storage with lazy evaluation.
pub struct TableEngine {
    df: DataFrame,
    selection: usize,
    scroll: usize,
    schema_view: bool,
    detail_view: bool,
    detail_scroll: usize,
    col_offset: usize,
    col_widths: Vec<usize>,
    file_name: String,
    last_query: Option<String>,
    pending_g: bool,
    last_view_height: usize,
    last_view_width: usize,
    last_match: Option<String>,
    /// Visual selection range (start, end) for highlighting
    pub visual_range: Option<(usize, usize)>,
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

        let col_widths = compute_col_widths(&df, 30);

        Ok(Self {
            df,
            selection: 0,
            scroll: 0,
            schema_view: false,
            detail_view: false,
            detail_scroll: 0,
            col_offset: 0,
            col_widths,
            file_name: path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string(),
            last_query: None,
            pending_g: false,
            last_view_height: 0,
            last_view_width: 0,
            last_match: None,
            visual_range: None,
        })
    }

    pub fn render(&mut self, frame: &mut ratatui::Frame, area: Rect, _wrap: bool) {
        self.last_view_height = area.height as usize;
        self.last_view_width = area.width as usize;
        if self.schema_view {
            self.render_schema(frame, area);
        } else if self.detail_view {
            self.render_detail(frame, area);
        } else {
            self.render_table(frame, area);
        }
    }

    pub fn content_height(&self) -> usize {
        if self.schema_view {
            self.df.schema().len()
        } else {
            // Rows are selectable data rows only; the header is a sticky,
            // non-selectable line, matching `selection`'s 0..height range.
            self.df.height()
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
        // Handle detail view keys separately — j/k scroll the field list
        if self.detail_view {
            match key.code {
                KeyCode::Char('g') => {
                    if self.pending_g {
                        self.detail_scroll = 0;
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
                    let max = self.df.width(); // +1 for header line, but 0-based
                    self.detail_scroll = (self.detail_scroll + 1).min(max);
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.detail_scroll = self.detail_scroll.saturating_sub(1);
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let jump = page_jump(self.last_view_height);
                    self.detail_scroll = self.detail_scroll.saturating_sub(jump);
                }
                KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let jump = page_jump(self.last_view_height);
                    let max = self.df.width();
                    self.detail_scroll = (self.detail_scroll + jump).min(max);
                }
                KeyCode::Char('G') => {
                    self.detail_scroll = self.df.width();
                }
                KeyCode::Char('n') => {
                    self.cycle_detail_row(true);
                }
                KeyCode::Char('N') => {
                    self.cycle_detail_row(false);
                }
                KeyCode::Enter | KeyCode::Esc => {
                    self.detail_view = false;
                    self.detail_scroll = 0;
                }
                KeyCode::Char('s') => {
                    self.detail_view = false;
                    self.detail_scroll = 0;
                    self.schema_view = true;
                }
                _ => {}
            }
            return;
        }

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
            KeyCode::Char('h') | KeyCode::Left => {
                if !self.schema_view {
                    self.col_offset = self.col_offset.saturating_sub(1);
                }
            }
            KeyCode::Char('l') | KeyCode::Right => {
                if !self.schema_view && self.df.width() > 0 {
                    self.col_offset = (self.col_offset + 1).min(self.df.width() - 1);
                }
            }
            KeyCode::Char('H') => {
                if !self.schema_view {
                    self.col_offset = 0;
                }
            }
            KeyCode::Char('L') => {
                if !self.schema_view && self.df.width() > 0 {
                    // Set col_offset so the last column is the rightmost visible one
                    self.col_offset = self.compute_last_col_offset();
                }
            }
            KeyCode::Enter => {
                if !self.schema_view {
                    self.detail_view = true;
                    self.detail_scroll = 0;
                }
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
        if self.schema_view {
            format!("{} schema {}/{}", self.file_name, self.selection + 1, self.df.schema().len())
        } else if self.detail_view {
            format!("{} row {}/{} (detail)", self.file_name, self.selection + 1, self.df.height())
        } else {
            format!(
                "{} row {}/{} col {}/{}",
                self.file_name,
                self.selection + 1,
                self.df.height(),
                self.col_offset + 1,
                self.df.width()
            )
        }
    }

    pub fn status_line(&self) -> String {
        let query = self
            .last_query
            .as_ref()
            .map(|q| format!(" | search: {}", q))
            .unwrap_or_default();
        if self.detail_view {
            format!(
                "j/k scroll | n/N next/prev row | Enter/Esc back | gg/G jump | Ctrl+u/d half-page | s schema{}",
                query
            )
        } else if self.schema_view {
            format!(
                "j/k move | gg/G jump | s toggle schema | / search | f filter{}",
                query
            )
        } else {
            format!(
                "j/k move | h/l cols | H/L first/last col | Enter detail | gg/G jump | Ctrl+u/d half-page | n/N next/prev | s schema | / search | f filter{}",
                query
            )
        }
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

    /// Get the content of the currently selected row. `selection` is a data-row
    /// index (`0..df.height()`), matching navigation and rendering — the sticky
    /// header is not a selectable row.
    pub fn get_selected_line(&self) -> Option<String> {
        if self.schema_view {
            self.df.schema().iter_fields().nth(self.selection).map(|f| {
                format!("{}: {}", f.name(), f.data_type())
            })
        } else {
            self.row_values(self.selection)
        }
    }

    /// Join one data row's cell values with tabs, or `None` if out of range.
    fn row_values(&self, row_idx: usize) -> Option<String> {
        if row_idx >= self.df.height() {
            return None;
        }
        let values: Vec<String> = self
            .df
            .get_columns()
            .iter()
            .map(|col| col.get(row_idx).map(|v| format!("{}", v)).unwrap_or_default())
            .collect();
        Some(values.join("\t"))
    }

    /// Get rows in a range (inclusive), joined by newlines
    pub fn get_lines_range(&self, start: usize, end: usize) -> Option<String> {
        let (start, end) = if start <= end { (start, end) } else { (end, start) };
        let total = self.content_height();
        if start >= total {
            return None;
        }
        let end = end.min(total.saturating_sub(1));
        let lines: Vec<String> = (start..=end)
            .filter_map(|idx| {
                if self.schema_view {
                    self.df.schema().iter_fields().nth(idx).map(|f| format!("{}: {}", f.name(), f.data_type()))
                } else {
                    self.row_values(idx)
                }
            })
            .collect();
        if lines.is_empty() { None } else { Some(lines.join("\n")) }
    }

    /// Get current selection index (for visual mode)
    pub fn selection(&self) -> usize {
        self.selection
    }

    /// Compute col_offset so the last column is the rightmost visible column.
    /// Walks backwards from the last column, accumulating widths until they exceed
    /// the available space, then returns the offset of the first column that fits.
    fn compute_last_col_offset(&self) -> usize {
        let row_num_width: usize = 6;
        let separator_width: usize = 2;
        let frozen_width = row_num_width + separator_width;
        let available = self.last_view_width.saturating_sub(frozen_width);

        let total_cols = self.df.width();
        if total_cols == 0 {
            return 0;
        }
        let mut used: usize = 0;
        let mut first_visible = total_cols - 1;
        for i in (0..total_cols).rev() {
            let w = self.col_widths.get(i).copied().unwrap_or(8);
            let needed = if i == total_cols - 1 { w } else { w + 1 };
            if used + needed > available && i < total_cols - 1 {
                break;
            }
            used += needed;
            first_visible = i;
        }
        first_visible
    }

    /// Cycle to the next/previous row while in detail view.
    fn cycle_detail_row(&mut self, forward: bool) {
        if forward {
            if self.selection + 1 < self.df.height() {
                self.selection += 1;
                self.detail_scroll = 0;
            }
        } else {
            if self.selection > 0 {
                self.selection -= 1;
                self.detail_scroll = 0;
            }
        }
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
            self.scroll = self.selection.saturating_sub(height.saturating_sub(1));
        }

        // Only render the visible slice (data is already in memory, just slicing the view)
        let slice = self
            .df
            .slice(self.scroll as i64, height.min(self.df.height()));

        // Determine which columns are visible based on col_offset and available width
        let row_num_width: usize = 6;
        let separator_width: usize = 2;
        let frozen_width = row_num_width + separator_width;
        let available = (area.width as usize).saturating_sub(frozen_width);

        let total_cols = self.df.width();
        let mut visible_count = 0;
        let mut used_width = 0;
        for i in self.col_offset..total_cols {
            let w = self.col_widths.get(i).copied().unwrap_or(8);
            // +1 for cell padding between columns
            let needed = if visible_count == 0 { w } else { w + 1 };
            if visible_count > 0 && used_width + needed > available {
                break;
            }
            used_width += needed;
            visible_count += 1;
        }
        if visible_count == 0 && total_cols > 0 {
            visible_count = 1;
        }

        let col_end = (self.col_offset + visible_count).min(total_cols);
        let col_names = self.df.get_column_names();

        let header_style = Style::default()
            .fg(Color::Black)
            .bg(Color::LightBlue)
            .bold();
        let mut headers: Vec<Cell> = Vec::new();
        headers.push(Cell::from("#").style(header_style));
        headers.push(Cell::from("│").style(Style::default().fg(Color::LightBlue)));
        for i in self.col_offset..col_end {
            headers.push(Cell::from(col_names[i]).style(header_style));
        }
        let header = Row::new(headers).style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::LightBlue)
                .bold(),
        );

        let columns = self.df.get_columns();
        let mut rows = Vec::new();
        for row_idx in 0..slice.height() {
            let abs_row = self.scroll + row_idx;
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
            for col_i in self.col_offset..col_end {
                let series = &columns[col_i];
                let value = series.get(self.scroll + row_idx).map(|v| v.to_string()).unwrap_or_default();
                let style = dtype_style(series.dtype());
                cells.push(Cell::from(value).style(style));
            }
            let mut table_row = Row::new(cells);
            if in_visual {
                table_row = table_row.style(Style::default().bg(Color::LightYellow).fg(Color::Black));
            }
            rows.push(table_row);
        }

        let row_count = rows.len();
        let mut widths: Vec<Constraint> = vec![
            Constraint::Length(row_num_width as u16),
            Constraint::Length(separator_width as u16),
        ];
        for (idx, i) in (self.col_offset..col_end).enumerate() {
            let w = self.col_widths.get(i).copied().unwrap_or(8) as u16;
            if idx == visible_count - 1 {
                // Last visible column fills remaining space
                widths.push(Constraint::Min(w));
            } else {
                widths.push(Constraint::Length(w));
            }
        }
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

    fn render_detail(&self, frame: &mut ratatui::Frame, area: Rect) {
        let row_idx = self.selection;
        let col_names = self.df.get_column_names();
        let columns = self.df.get_columns();

        // Find the max column name display width for alignment
        let max_name_len = col_names.iter().map(|n| super::display_width(n)).max().unwrap_or(0);

        let mut lines = Vec::new();

        // Header line
        let header_text = format!("── Row {} ──", row_idx + 1);
        lines.push(Line::from(Span::styled(
            header_text,
            Style::default().fg(Color::LightBlue).bold(),
        )));

        if row_idx < self.df.height() {
            for (i, name) in col_names.iter().enumerate() {
                let value = columns[i]
                    .get(row_idx)
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                let padding = " ".repeat(max_name_len.saturating_sub(super::display_width(name)));
                let style = dtype_style(columns[i].dtype());
                lines.push(Line::from(vec![
                    Span::styled(
                        name.to_string(),
                        Style::default().fg(Color::Cyan).bold(),
                    ),
                    Span::raw(format!("{}  ", padding)),
                    Span::styled(value, style),
                ]));
            }
        }

        let block = Block::default().borders(Borders::NONE);
        frame.render_widget(
            Paragraph::new(lines)
                .block(block)
                .scroll((self.detail_scroll as u16, 0)),
            area,
        );
    }

    fn render_schema(&self, frame: &mut ratatui::Frame, area: Rect) {
        let mut lines = Vec::new();
        for field in self.df.schema().iter_fields() {
            lines.push(Line::from(format!("{}: {}", field.name(), field.data_type())));
        }
        let block = Block::default().borders(Borders::NONE);
        frame.render_widget(Paragraph::new(lines).block(block), area);
    }

    fn search_next(&mut self, query: &str, forward: bool) {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return;
        }
        let matcher = crate::search::Matcher::new(trimmed);
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
                    if matcher.is_match(&value.to_string()) {
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

/// Compute content-aware column widths by scanning the first 100 rows.
fn compute_col_widths(df: &DataFrame, max_width: usize) -> Vec<usize> {
    let sample_rows = df.height().min(100);
    df.get_columns()
        .iter()
        .map(|col| {
            let header_len = super::display_width(col.name());
            let mut max_val_len = 0;
            for i in 0..sample_rows {
                if let Ok(val) = col.get(i) {
                    let len = super::display_width(&val.to_string());
                    if len > max_val_len {
                        max_val_len = len;
                    }
                }
            }
            let w = header_len.max(max_val_len).max(4);
            w.min(max_width)
        })
        .collect()
}

/// Get a style for a Polars data type.
fn dtype_style(dtype: &polars::datatypes::DataType) -> Style {
    match dtype {
        polars::datatypes::DataType::Int8
        | polars::datatypes::DataType::Int16
        | polars::datatypes::DataType::Int32
        | polars::datatypes::DataType::Int64
        | polars::datatypes::DataType::UInt8
        | polars::datatypes::DataType::UInt16
        | polars::datatypes::DataType::UInt32
        | polars::datatypes::DataType::UInt64
        | polars::datatypes::DataType::Float32
        | polars::datatypes::DataType::Float64 => Style::default().fg(Color::Magenta),
        polars::datatypes::DataType::Boolean => Style::default().fg(Color::Cyan),
        polars::datatypes::DataType::String => Style::default().fg(Color::Yellow),
        polars::datatypes::DataType::Date
        | polars::datatypes::DataType::Datetime(_, _)
        | polars::datatypes::DataType::Time => Style::default().fg(Color::Green),
        polars::datatypes::DataType::Null => Style::default().fg(Color::DarkGray),
        _ => Style::default().fg(Color::White),
    }
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
    fn compute_col_widths_respects_max() {
        // Create a simple DataFrame with known content
        let s1 = Series::new("id".into(), &[1i64, 2, 3]);
        let s2 = Series::new("a_very_long_column_name_here".into(), &["short", "x", "y"]);
        let df = DataFrame::new(vec![s1, s2]).unwrap();
        let widths = compute_col_widths(&df, 30);
        // "id" column: max(2, 1) = 2, but min is 4
        assert_eq!(widths[0], 4);
        // Long column name: 28 chars, values are shorter, so width = 28
        assert_eq!(widths[1], 28);
    }

    #[test]
    fn compute_col_widths_caps_at_max() {
        let long_val = "a".repeat(50);
        let s1 = Series::new("x".into(), &[long_val.as_str()]);
        let df = DataFrame::new(vec![s1]).unwrap();
        let widths = compute_col_widths(&df, 30);
        assert_eq!(widths[0], 30);
    }

    #[test]
    fn selected_row_is_data_not_header() {
        use std::io::Write;
        let mut f = tempfile::Builder::new().suffix(".csv").tempfile().unwrap();
        writeln!(f, "id,name").unwrap();
        writeln!(f, "1,alice").unwrap();
        writeln!(f, "2,bob").unwrap();
        f.flush().unwrap();
        let mut engine = TableEngine::from_path(f.path()).unwrap();

        // selection 0 == first data row (not the header); previously returned "id\tname".
        engine.selection = 0;
        let first = engine.get_selected_line().unwrap();
        assert!(first.starts_with("1\t") && first.contains("alice"), "got: {first}");
        engine.selection = 1;
        let second = engine.get_selected_line().unwrap();
        assert!(second.starts_with("2\t") && second.contains("bob"), "got: {second}");
        // Visual range 0..=1 copies both data rows, no header line.
        let range = engine.get_lines_range(0, 1).unwrap();
        assert!(!range.contains("name"), "range must not include header: {range}");
        assert_eq!(range.lines().count(), 2, "got: {range}");
    }

    #[test]
    fn dtype_style_returns_correct_colors() {
        let s = dtype_style(&polars::datatypes::DataType::Int64);
        assert_eq!(s, Style::default().fg(Color::Magenta));

        let s = dtype_style(&polars::datatypes::DataType::String);
        assert_eq!(s, Style::default().fg(Color::Yellow));

        let s = dtype_style(&polars::datatypes::DataType::Boolean);
        assert_eq!(s, Style::default().fg(Color::Cyan));
    }
}
