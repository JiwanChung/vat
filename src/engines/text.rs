use std::fs::File;
use std::path::Path;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use memmap2::Mmap;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

/// TextEngine uses memory-mapped files for efficient handling of large files.
/// Only the visible portion is read into memory during rendering.
pub struct TextEngine {
    /// Memory-mapped file content
    mmap: Mmap,
    /// Byte offsets for the start of each line (built once on load)
    line_offsets: Vec<usize>,
    selection: usize,
    scroll: usize,
    file_name: String,
    last_query: Option<String>,
    pending_g: bool,
    last_view_height: usize,
    last_match: Option<String>,
    /// Filtered line indices (None = show all)
    filtered_indices: Option<Vec<usize>>,
}

impl TextEngine {
    pub fn from_path(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };

        // Build line offset index - O(n) one-time cost, but only stores offsets (~8 bytes per line)
        let line_offsets = build_line_offsets(&mmap);

        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        Ok(Self {
            mmap,
            line_offsets,
            selection: 0,
            scroll: 0,
            file_name,
            last_query: None,
            pending_g: false,
            last_view_height: 0,
            last_match: None,
            filtered_indices: None,
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

        // Find actual line end (strip \n or \r\n)
        let mut line_end = end;
        if line_end > start && self.mmap.get(line_end - 1) == Some(&b'\n') {
            line_end -= 1;
        }
        if line_end > start && self.mmap.get(line_end - 1) == Some(&b'\r') {
            line_end -= 1;
        }

        std::str::from_utf8(&self.mmap[start..line_end]).ok()
    }

    /// Total number of lines in the file
    fn line_count(&self) -> usize {
        self.line_offsets.len()
    }

    /// Number of lines to display (filtered or all)
    fn display_count(&self) -> usize {
        self.filtered_indices.as_ref().map_or(self.line_count(), |f| f.len())
    }

    /// Get the actual line index for a display position
    fn display_to_actual(&self, display_idx: usize) -> Option<usize> {
        match &self.filtered_indices {
            Some(indices) => indices.get(display_idx).copied(),
            None => Some(display_idx),
        }
    }

    pub fn render(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let height = area.height as usize;
        self.last_view_height = height;

        let display_total = self.display_count();

        // Clamp selection to display range
        if self.selection >= display_total && display_total > 0 {
            self.selection = display_total - 1;
        }

        // Adjust scroll to keep selection visible
        if self.selection < self.scroll {
            self.scroll = self.selection;
        } else if self.selection >= self.scroll + height {
            self.scroll = self.selection.saturating_sub(height - 1);
        }

        let total_lines = self.line_count();
        let line_no_width = total_lines.max(1).to_string().len().max(2);

        // Only read lines in the visible window
        let visible: Vec<Line> = (0..height)
            .filter_map(|i| {
                let display_row = self.scroll + i;
                let actual_row = self.display_to_actual(display_row)?;
                let line_content = self.get_line(actual_row)?;
                let selected = display_row == self.selection;

                let mut spans = Vec::new();
                let line_no = format!("{:>width$} ", actual_row + 1, width = line_no_width);
                let line_no_style = if selected {
                    Style::default().fg(Color::Black).bg(Color::LightBlue).bold()
                } else {
                    Style::default().fg(Color::LightYellow)
                };
                spans.push(Span::styled(line_no, line_no_style));
                spans.push(Span::styled("│ ", Style::default().fg(Color::LightBlue)));

                let mut content_style = Style::default().fg(Color::White);
                if line_content.contains("TODO") {
                    content_style = content_style.fg(Color::LightRed).bold();
                }
                if selected {
                    content_style = content_style.fg(Color::Black).bg(Color::LightBlue);
                }
                spans.push(Span::styled(line_content.to_string(), content_style));
                Some(Line::from(spans))
            })
            .collect();

        let block = Block::default().borders(Borders::NONE);
        frame.render_widget(Paragraph::new(visible).block(block), area);
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

        let total = self.display_count();
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

    pub fn breadcrumbs(&self) -> String {
        let filter_info = if self.filtered_indices.is_some() {
            format!(" [filtered: {}/{}]", self.display_count(), self.line_count())
        } else {
            String::new()
        };
        format!("{} line {}/{}{}",
            self.file_name,
            self.selection + 1,
            self.display_count(),
            filter_info
        )
    }

    pub fn status_line(&self) -> String {
        let query = self
            .last_query
            .as_ref()
            .map(|q| format!(" | search: {}", q))
            .unwrap_or_default();
        let filter = if self.filtered_indices.is_some() {
            " | f filter | F clear"
        } else {
            " | f filter"
        };
        format!(
            "j/k move | gg/G jump | Ctrl+u/d half-page | n/N next/prev | / search{}{}",
            filter, query
        )
    }

    #[allow(dead_code)]
    pub fn selected_path(&self) -> Option<String> {
        None
    }

    pub fn content_height(&self) -> usize {
        self.line_count()
    }

    pub fn render_plain_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let total = self.line_count();
        let line_no_width = total.max(1).to_string().len().max(2);

        (0..total)
            .filter_map(|idx| {
                let line_content = self.get_line(idx)?;
                let mut spans = Vec::new();
                let line_no = format!("{:>width$} ", idx + 1, width = line_no_width);
                spans.push(Span::styled(line_no, Style::default().fg(Color::LightYellow)));
                spans.push(Span::styled("│ ", Style::default().fg(Color::LightBlue)));
                spans.push(Span::styled(line_content.to_string(), Style::default().fg(Color::White)));
                Some(Line::from(spans))
            })
            .collect()
    }

    fn search_next(&mut self, query: &str, forward: bool) {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return;
        }
        let lower = trimmed.to_lowercase();
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
        self.last_match = Some(trimmed.to_string());
    }
}

/// Build an index of byte offsets for each line start.
/// This is O(n) but only stores ~8 bytes per line (just the offset).
fn build_line_offsets(data: &[u8]) -> Vec<usize> {
    let mut offsets = Vec::new();
    offsets.push(0); // First line starts at 0

    for (i, &byte) in data.iter().enumerate() {
        if byte == b'\n' && i + 1 < data.len() {
            offsets.push(i + 1);
        }
    }

    offsets
}

fn page_jump(view_height: usize) -> usize {
    let half = view_height / 2;
    if half == 0 { 1 } else { half }
}
