use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

const BYTES_PER_LINE: usize = 16;
const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024; // 100MB limit

pub struct HexEngine {
    file_path: std::path::PathBuf,
    file_size: u64,
    selection: usize,
    scroll: usize,
    file_name: String,
    last_query: Option<String>,
    pending_g: bool,
    last_view_height: usize,
    cached_lines: Vec<(usize, Vec<u8>)>,
    cache_start: usize,
}

impl HexEngine {
    pub fn from_path(path: &Path) -> Result<Self> {
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        let metadata = std::fs::metadata(path)?;
        let file_size = metadata.len().min(MAX_FILE_SIZE);

        Ok(Self {
            file_path: path.to_path_buf(),
            file_size,
            selection: 0,
            scroll: 0,
            file_name,
            last_query: None,
            pending_g: false,
            last_view_height: 0,
            cached_lines: Vec::new(),
            cache_start: 0,
        })
    }

    fn total_lines(&self) -> usize {
        ((self.file_size as usize) + BYTES_PER_LINE - 1) / BYTES_PER_LINE
    }

    fn load_lines(&mut self, start: usize, count: usize) {
        // Check if already cached
        let cache_end = self.cache_start + self.cached_lines.len();
        if start >= self.cache_start && start + count <= cache_end {
            return;
        }

        // Load new cache
        let offset = (start * BYTES_PER_LINE) as u64;
        let bytes_to_read = (count * BYTES_PER_LINE).min(self.file_size as usize - offset as usize);

        if let Ok(mut file) = File::open(&self.file_path) {
            if file.seek(SeekFrom::Start(offset)).is_ok() {
                let mut buffer = vec![0u8; bytes_to_read];
                if let Ok(read) = file.read(&mut buffer) {
                    buffer.truncate(read);

                    self.cached_lines.clear();
                    self.cache_start = start;

                    for (i, chunk) in buffer.chunks(BYTES_PER_LINE).enumerate() {
                        self.cached_lines.push((start + i, chunk.to_vec()));
                    }
                }
            }
        }
    }

    fn get_line(&self, line_idx: usize) -> Option<&Vec<u8>> {
        if line_idx >= self.cache_start && line_idx < self.cache_start + self.cached_lines.len() {
            let cache_idx = line_idx - self.cache_start;
            self.cached_lines.get(cache_idx).map(|(_, data)| data)
        } else {
            None
        }
    }

    pub fn render(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let height = area.height as usize;
        self.last_view_height = height;

        let total = self.total_lines();
        if self.selection >= total && total > 0 {
            self.selection = total - 1;
        }

        if self.selection < self.scroll {
            self.scroll = self.selection;
        } else if self.selection >= self.scroll + height {
            self.scroll = self.selection.saturating_sub(height - 1);
        }

        // Load visible lines into cache
        self.load_lines(self.scroll, height + 10);

        let addr_width = format!("{:08X}", self.file_size).len();

        let visible: Vec<Line> = (0..height)
            .filter_map(|idx| {
                let line_idx = self.scroll + idx;
                if line_idx >= total {
                    return None;
                }

                let offset = line_idx * BYTES_PER_LINE;
                let selected = line_idx == self.selection;

                let bytes = self.get_line(line_idx).cloned().unwrap_or_default();

                let mut spans = Vec::new();

                // Address
                let addr_style = if selected {
                    Style::default().fg(Color::Black).bg(Color::LightBlue).bold()
                } else {
                    Style::default().fg(Color::LightYellow)
                };
                spans.push(Span::styled(
                    format!("{:0width$X}  ", offset, width = addr_width),
                    addr_style,
                ));

                // Hex bytes
                for (i, &byte) in bytes.iter().enumerate() {
                    if i == 8 {
                        spans.push(Span::raw(" "));
                    }

                    let byte_style = if selected {
                        Style::default().fg(Color::Black).bg(Color::LightBlue)
                    } else if byte == 0 {
                        Style::default().fg(Color::DarkGray)
                    } else if byte.is_ascii_printable() {
                        Style::default().fg(Color::LightGreen)
                    } else {
                        Style::default().fg(Color::LightCyan)
                    };

                    spans.push(Span::styled(format!("{:02X} ", byte), byte_style));
                }

                // Padding for incomplete lines
                for i in bytes.len()..BYTES_PER_LINE {
                    if i == 8 {
                        spans.push(Span::raw(" "));
                    }
                    spans.push(Span::raw("   "));
                }

                spans.push(Span::raw(" "));

                // ASCII representation
                let ascii_style = if selected {
                    Style::default().fg(Color::Black).bg(Color::LightBlue)
                } else {
                    Style::default().fg(Color::White)
                };

                let ascii: String = bytes
                    .iter()
                    .map(|&b| if b.is_ascii_graphic() || b == b' ' { b as char } else { '.' })
                    .collect();

                spans.push(Span::styled(ascii, ascii_style));

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

        let total = self.total_lines();
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
            _ => {}
        }
    }

    pub fn apply_search(&mut self, _query: &str) {
        // TODO: Implement hex search
    }

    pub fn apply_filter(&mut self, _query: &str) {}

    pub fn clear_filter(&mut self) {
        self.last_query = None;
    }

    pub fn breadcrumbs(&self) -> String {
        let offset = self.selection * BYTES_PER_LINE;
        format!(
            "{} offset 0x{:X} ({}/{})",
            self.file_name,
            offset,
            format_size(offset as u64),
            format_size(self.file_size)
        )
    }

    pub fn status_line(&self) -> String {
        format!(
            "j/k move | gg/G jump | Ctrl+u/d half-page | {} bytes | {} lines",
            self.file_size,
            self.total_lines()
        )
    }

    #[allow(dead_code)]
    pub fn selected_path(&self) -> Option<String> {
        None
    }

    pub fn content_height(&self) -> usize {
        self.total_lines()
    }

    pub fn render_plain_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let addr_width = format!("{:08X}", self.file_size).len();

        // Only show first 100 lines in plain mode
        if let Ok(mut file) = File::open(&self.file_path) {
            let mut buffer = vec![0u8; 100 * BYTES_PER_LINE];
            if let Ok(read) = file.read(&mut buffer) {
                buffer.truncate(read);

                for (line_idx, chunk) in buffer.chunks(BYTES_PER_LINE).enumerate() {
                    let offset = line_idx * BYTES_PER_LINE;

                    let hex: String = chunk
                        .iter()
                        .enumerate()
                        .map(|(i, b)| {
                            if i == 8 {
                                format!(" {:02X}", b)
                            } else {
                                format!("{:02X} ", b)
                            }
                        })
                        .collect();

                    let ascii: String = chunk
                        .iter()
                        .map(|&b| if b.is_ascii_graphic() || b == b' ' { b as char } else { '.' })
                        .collect();

                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("{:0width$X}  ", offset, width = addr_width),
                            Style::default().fg(Color::LightYellow),
                        ),
                        Span::styled(hex, Style::default().fg(Color::LightCyan)),
                        Span::raw(" "),
                        Span::styled(ascii, Style::default().fg(Color::White)),
                    ]));
                }
            }
        }

        lines
    }
}

trait AsciiPrintable {
    fn is_ascii_printable(&self) -> bool;
}

impl AsciiPrintable for u8 {
    fn is_ascii_printable(&self) -> bool {
        *self >= 0x20 && *self < 0x7F
    }
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1}GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1}MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1}KB", bytes as f64 / KB as f64)
    } else {
        format!("{}B", bytes)
    }
}

fn page_jump(view_height: usize) -> usize {
    let half = view_height / 2;
    if half == 0 { 1 } else { half }
}
