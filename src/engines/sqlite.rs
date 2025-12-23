use std::path::Path;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Row, Table, TableState};
use rusqlite::Connection;

#[derive(Clone)]
struct TableInfo {
    name: String,
    columns: Vec<ColumnInfo>,
    row_count: usize,
}

#[derive(Clone)]
struct ColumnInfo {
    name: String,
    col_type: String,
    is_pk: bool,
    nullable: bool,
}

pub struct SqliteEngine {
    tables: Vec<TableInfo>,
    current_table: usize,
    preview_rows: Vec<Vec<String>>,
    selection: usize,
    scroll: usize,
    file_name: String,
    last_query: Option<String>,
    pending_g: bool,
    last_view_height: usize,
    last_match: Option<String>,
    view_mode: ViewMode,
    db_path: std::path::PathBuf,
    /// Visual selection range (start, end) for highlighting
    pub visual_range: Option<(usize, usize)>,
}

#[derive(Clone, Copy, PartialEq)]
enum ViewMode {
    Schema,
    Preview,
}

impl SqliteEngine {
    pub fn from_path(path: &Path) -> Result<Self> {
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        let conn = Connection::open(path)?;
        let tables = get_table_info(&conn)?;
        let preview_rows = if !tables.is_empty() {
            get_preview_rows(&conn, &tables[0].name, &tables[0].columns)?
        } else {
            Vec::new()
        };

        Ok(Self {
            tables,
            current_table: 0,
            preview_rows,
            selection: 0,
            scroll: 0,
            file_name,
            last_query: None,
            pending_g: false,
            last_view_height: 0,
            last_match: None,
            view_mode: ViewMode::Schema,
            db_path: path.to_path_buf(),
            visual_range: None,
        })
    }

    fn refresh_preview(&mut self) {
        if let Ok(conn) = Connection::open(&self.db_path) {
            if let Some(table) = self.tables.get(self.current_table) {
                if let Ok(rows) = get_preview_rows(&conn, &table.name, &table.columns) {
                    self.preview_rows = rows;
                }
            }
        }
    }

    pub fn render(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let height = area.height as usize;
        self.last_view_height = height;

        match self.view_mode {
            ViewMode::Schema => self.render_schema(frame, area),
            ViewMode::Preview => self.render_preview(frame, area),
        }
    }

    fn render_schema(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let height = area.height.saturating_sub(1) as usize;

        // Build display lines
        let mut display_lines: Vec<(bool, Line)> = Vec::new();
        let mut line_idx = 0;

        for (table_idx, table) in self.tables.iter().enumerate() {
            let is_current = table_idx == self.current_table;
            let selected = line_idx == self.selection;

            // Table header
            let table_style = if selected {
                Style::default().fg(Color::Black).bg(Color::LightBlue).bold()
            } else if is_current {
                Style::default().fg(Color::LightGreen).bold()
            } else {
                Style::default().fg(Color::LightCyan).bold()
            };

            display_lines.push((selected, Line::from(vec![
                Span::styled(
                    format!("TABLE {} ({} rows)", table.name, table.row_count),
                    table_style,
                ),
            ])));
            line_idx += 1;

            // Columns
            for col in &table.columns {
                let selected = line_idx == self.selection;
                let col_style = if selected {
                    Style::default().fg(Color::Black).bg(Color::LightBlue)
                } else {
                    Style::default().fg(Color::White)
                };

                let pk_marker = if col.is_pk { " PK" } else { "" };
                let null_marker = if col.nullable { "" } else { " NOT NULL" };

                display_lines.push((selected, Line::from(vec![
                    Span::raw("  "),
                    Span::styled(&col.name, col_style),
                    Span::styled(format!(" {}", col.col_type), Style::default().fg(Color::LightYellow)),
                    Span::styled(pk_marker, Style::default().fg(Color::Magenta)),
                    Span::styled(null_marker, Style::default().fg(Color::DarkGray)),
                ])));
                line_idx += 1;
            }

            display_lines.push((false, Line::from("")));
            line_idx += 1;
        }

        let total = display_lines.len();
        if self.selection >= total && total > 0 {
            self.selection = total - 1;
        }

        if self.selection < self.scroll {
            self.scroll = self.selection;
        } else if self.selection >= self.scroll + height {
            self.scroll = self.selection.saturating_sub(height - 1);
        }

        let visible: Vec<Line> = display_lines
            .into_iter()
            .skip(self.scroll)
            .take(height)
            .map(|(_, line)| line)
            .collect();

        let block = Block::default().borders(Borders::NONE);
        frame.render_widget(ratatui::widgets::Paragraph::new(visible).block(block), area);
    }

    fn render_preview(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        if self.tables.is_empty() {
            return;
        }

        let table = &self.tables[self.current_table];
        let height = area.height.saturating_sub(2) as usize;

        if self.selection < self.scroll {
            self.scroll = self.selection;
        } else if self.selection >= self.scroll + height {
            self.scroll = self.selection.saturating_sub(height - 1);
        }

        let header_style = Style::default().fg(Color::Black).bg(Color::LightBlue).bold();
        let headers: Vec<Cell> = table.columns
            .iter()
            .map(|c| Cell::from(c.name.clone()).style(header_style))
            .collect();
        let header = Row::new(headers);

        let rows: Vec<Row> = self.preview_rows
            .iter()
            .skip(self.scroll)
            .take(height)
            .map(|row| {
                let cells: Vec<Cell> = row
                    .iter()
                    .map(|v| Cell::from(truncate(v, 30)).style(Style::default().fg(Color::White)))
                    .collect();
                Row::new(cells)
            })
            .collect();

        let widths: Vec<Constraint> = table.columns
            .iter()
            .map(|_| Constraint::Percentage(100 / table.columns.len().max(1) as u16))
            .collect();

        let table_widget = Table::new(rows, widths)
            .header(header)
            .block(Block::default().borders(Borders::NONE).title(format!(" {} ", table.name)))
            .highlight_style(Style::default().bg(Color::LightBlue).fg(Color::Black));

        let mut state = TableState::default();
        if !self.preview_rows.is_empty() {
            let relative = self.selection.saturating_sub(self.scroll);
            state.select(Some(relative));
        }
        frame.render_stateful_widget(table_widget, area, &mut state);
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

        let total = match self.view_mode {
            ViewMode::Schema => {
                self.tables.iter().map(|t| t.columns.len() + 2).sum::<usize>()
            }
            ViewMode::Preview => self.preview_rows.len(),
        };

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
            KeyCode::Char('s') => {
                self.view_mode = match self.view_mode {
                    ViewMode::Schema => ViewMode::Preview,
                    ViewMode::Preview => ViewMode::Schema,
                };
                self.selection = 0;
                self.scroll = 0;
            }
            KeyCode::Tab => {
                if !self.tables.is_empty() {
                    self.current_table = (self.current_table + 1) % self.tables.len();
                    self.refresh_preview();
                    self.selection = 0;
                    self.scroll = 0;
                }
            }
            KeyCode::BackTab => {
                if !self.tables.is_empty() {
                    self.current_table = if self.current_table == 0 {
                        self.tables.len() - 1
                    } else {
                        self.current_table - 1
                    };
                    self.refresh_preview();
                    self.selection = 0;
                    self.scroll = 0;
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

    pub fn apply_filter(&mut self, query: &str) {
        self.apply_search(query);
    }

    pub fn clear_filter(&mut self) {
        self.last_query = None;
    }

    pub fn breadcrumbs(&self) -> String {
        let table_name = self.tables.get(self.current_table).map(|t| t.name.as_str()).unwrap_or("");
        let mode = match self.view_mode {
            ViewMode::Schema => "schema",
            ViewMode::Preview => "data",
        };
        format!("{} [{}] {} line {}", self.file_name, table_name, mode, self.selection + 1)
    }

    pub fn status_line(&self) -> String {
        let query = self
            .last_query
            .as_ref()
            .map(|q| format!(" | search: {}", q))
            .unwrap_or_default();
        let mode = match self.view_mode {
            ViewMode::Schema => "schema",
            ViewMode::Preview => "preview",
        };
        format!(
            "j/k move | gg/G jump | Tab/Shift+Tab tables | s toggle view ({}) | / search{}",
            mode, query
        )
    }

    #[allow(dead_code)]
    pub fn selected_path(&self) -> Option<String> {
        None
    }

    /// Get the content of the currently selected line
    pub fn get_selected_line(&self) -> Option<String> {
        match self.view_mode {
            ViewMode::Schema => {
                let mut idx = 0;
                for table in &self.tables {
                    if idx == self.selection {
                        return Some(format!("Table: {}", table.name));
                    }
                    idx += 1;
                    for col in &table.columns {
                        if idx == self.selection {
                            return Some(format!("{}\t{}\t{}", col.name, col.col_type, if col.is_pk { "PK" } else { "" }));
                        }
                        idx += 1;
                    }
                    idx += 1; // empty line
                }
                None
            }
            ViewMode::Preview => {
                if self.selection == 0 {
                    self.tables.get(self.current_table).map(|t| {
                        t.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join("\t")
                    })
                } else {
                    self.preview_rows.get(self.selection.saturating_sub(1)).map(|row| row.join("\t"))
                }
            }
        }
    }

    /// Get lines in a range (inclusive), joined by newlines
    pub fn get_lines_range(&self, start: usize, end: usize) -> Option<String> {
        let (start, end) = if start <= end { (start, end) } else { (end, start) };
        let total = self.content_height();
        if start >= total { return None; }
        let end = end.min(total.saturating_sub(1));
        let lines: Vec<String> = (start..=end)
            .filter_map(|idx| {
                // Compute the line at each index inline
                match self.view_mode {
                    ViewMode::Schema => {
                        let mut cur = 0;
                        for table in &self.tables {
                            if cur == idx { return Some(format!("Table: {}", table.name)); }
                            cur += 1;
                            for col in &table.columns {
                                if cur == idx { return Some(format!("{}\t{}\t{}", col.name, col.col_type, if col.is_pk { "PK" } else { "" })); }
                                cur += 1;
                            }
                            cur += 1;
                        }
                        None
                    }
                    ViewMode::Preview => {
                        if idx == 0 {
                            self.tables.get(self.current_table).map(|t| t.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join("\t"))
                        } else {
                            self.preview_rows.get(idx.saturating_sub(1)).map(|row| row.join("\t"))
                        }
                    }
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
        match self.view_mode {
            ViewMode::Schema => self.tables.iter().map(|t| t.columns.len() + 2).sum(),
            ViewMode::Preview => self.preview_rows.len() + 1,
        }
    }

    pub fn render_plain_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        for table in &self.tables {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("TABLE {} ({} rows)", table.name, table.row_count),
                    Style::default().fg(Color::LightCyan).bold(),
                ),
            ]));

            for col in &table.columns {
                let pk = if col.is_pk { " PK" } else { "" };
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(col.name.clone(), Style::default().fg(Color::White)),
                    Span::styled(format!(" {}{}", col.col_type, pk), Style::default().fg(Color::LightYellow)),
                ]));
            }

            lines.push(Line::from(""));
        }

        lines
    }

    fn search_next(&mut self, query: &str, _forward: bool) {
        let lower = query.to_lowercase();
        // Search in table names and column names
        for (idx, table) in self.tables.iter().enumerate() {
            if table.name.to_lowercase().contains(&lower) {
                self.current_table = idx;
                self.refresh_preview();
                return;
            }
            for col in &table.columns {
                if col.name.to_lowercase().contains(&lower) {
                    self.current_table = idx;
                    self.refresh_preview();
                    return;
                }
            }
        }
        self.last_match = Some(query.to_string());
    }
}

fn get_table_info(conn: &Connection) -> Result<Vec<TableInfo>> {
    let mut tables = Vec::new();

    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
    )?;

    let table_names: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    for name in table_names {
        let mut col_stmt = conn.prepare(&format!("PRAGMA table_info(\"{}\")", name))?;
        let columns: Vec<ColumnInfo> = col_stmt
            .query_map([], |row| {
                Ok(ColumnInfo {
                    name: row.get(1)?,
                    col_type: row.get::<_, String>(2).unwrap_or_default(),
                    is_pk: row.get::<_, i32>(5).unwrap_or(0) > 0,
                    nullable: row.get::<_, i32>(3).unwrap_or(1) == 0,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        let row_count: usize = conn
            .query_row(&format!("SELECT COUNT(*) FROM \"{}\"", name), [], |row| row.get(0))
            .unwrap_or(0);

        tables.push(TableInfo { name, columns, row_count });
    }

    Ok(tables)
}

fn get_preview_rows(conn: &Connection, table_name: &str, columns: &[ColumnInfo]) -> Result<Vec<Vec<String>>> {
    let col_names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    let query = format!(
        "SELECT {} FROM \"{}\" LIMIT 100",
        col_names.iter().map(|c| format!("\"{}\"", c)).collect::<Vec<_>>().join(", "),
        table_name
    );

    let mut stmt = conn.prepare(&query)?;
    let rows: Vec<Vec<String>> = stmt
        .query_map([], |row| {
            let mut values = Vec::new();
            for i in 0..columns.len() {
                let value: String = row.get::<_, rusqlite::types::Value>(i)
                    .map(|v| match v {
                        rusqlite::types::Value::Null => "NULL".to_string(),
                        rusqlite::types::Value::Integer(i) => i.to_string(),
                        rusqlite::types::Value::Real(f) => f.to_string(),
                        rusqlite::types::Value::Text(s) => s,
                        rusqlite::types::Value::Blob(_) => "[BLOB]".to_string(),
                    })
                    .unwrap_or_default();
                values.push(value);
            }
            Ok(values)
        })?
        .filter_map(|r| r.ok())
        .collect();

    Ok(rows)
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
