use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use anyhow::{anyhow, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

#[derive(Clone)]
struct ArchiveEntry {
    path: String,
    size: u64,
    compressed_size: Option<u64>,
    is_dir: bool,
    modified: Option<String>,
}

pub struct ArchiveEngine {
    entries: Vec<ArchiveEntry>,
    total_size: u64,
    total_compressed: u64,
    selection: usize,
    scroll: usize,
    file_name: String,
    archive_type: String,
    last_query: Option<String>,
    pending_g: bool,
    last_view_height: usize,
    last_match: Option<String>,
}

impl ArchiveEngine {
    pub fn from_path(path: &Path) -> Result<Self> {
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();

        let (entries, archive_type) = match ext.as_str() {
            "zip" => (parse_zip(path)?, "ZIP".to_string()),
            "tar" => (parse_tar(path, None)?, "TAR".to_string()),
            "gz" | "tgz" => {
                if file_name.ends_with(".tar.gz") || ext == "tgz" {
                    (parse_tar(path, Some("gz"))?, "TAR.GZ".to_string())
                } else {
                    return Err(anyhow!("Single gzip files not supported, use tar.gz"));
                }
            }
            _ => return Err(anyhow!("Unsupported archive format")),
        };

        let total_size: u64 = entries.iter().map(|e| e.size).sum();
        let total_compressed: u64 = entries.iter().filter_map(|e| e.compressed_size).sum();

        Ok(Self {
            entries,
            total_size,
            total_compressed,
            selection: 0,
            scroll: 0,
            file_name,
            archive_type,
            last_query: None,
            pending_g: false,
            last_view_height: 0,
            last_match: None,
        })
    }

    pub fn render(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let height = area.height as usize;
        self.last_view_height = height;

        if self.selection < self.scroll {
            self.scroll = self.selection;
        } else if self.selection >= self.scroll + height {
            self.scroll = self.selection.saturating_sub(height - 1);
        }

        let visible: Vec<Line> = self.entries
            .iter()
            .skip(self.scroll)
            .take(height)
            .enumerate()
            .map(|(idx, entry)| {
                let row = self.scroll + idx;
                let selected = row == self.selection;

                let mut spans = Vec::new();

                // Icon
                let icon = if entry.is_dir { "D " } else { "  " };
                let icon_style = if selected {
                    Style::default().fg(Color::Black).bg(Color::LightBlue)
                } else {
                    Style::default().fg(Color::Cyan)
                };
                spans.push(Span::styled(icon, icon_style));

                // Size
                let size_str = format_size(entry.size);
                let size_style = if selected {
                    Style::default().fg(Color::Black).bg(Color::LightBlue)
                } else {
                    Style::default().fg(Color::LightYellow)
                };
                spans.push(Span::styled(format!("{:>8} ", size_str), size_style));

                // Compression ratio
                if let Some(compressed) = entry.compressed_size {
                    let ratio = if entry.size > 0 {
                        (compressed as f64 / entry.size as f64 * 100.0) as u64
                    } else {
                        100
                    };
                    let ratio_style = if selected {
                        Style::default().fg(Color::Black).bg(Color::LightBlue)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    };
                    spans.push(Span::styled(format!("{:>3}% ", ratio), ratio_style));
                } else {
                    spans.push(Span::raw("     "));
                }

                // Path
                let path_style = if selected {
                    Style::default().fg(Color::Black).bg(Color::LightBlue)
                } else if entry.is_dir {
                    Style::default().fg(Color::LightCyan).bold()
                } else {
                    Style::default().fg(Color::White)
                };
                spans.push(Span::styled(&entry.path, path_style));

                Line::from(spans)
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

        let total = self.entries.len();
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
        self.apply_search(query);
    }

    pub fn clear_filter(&mut self) {
        self.last_query = None;
    }

    pub fn breadcrumbs(&self) -> String {
        let ratio = if self.total_size > 0 {
            (self.total_compressed as f64 / self.total_size as f64 * 100.0) as u64
        } else {
            100
        };
        format!(
            "{} [{}] {} files, {} -> {} ({}%)",
            self.file_name,
            self.archive_type,
            self.entries.len(),
            format_size(self.total_size),
            format_size(self.total_compressed),
            ratio
        )
    }

    pub fn status_line(&self) -> String {
        let query = self
            .last_query
            .as_ref()
            .map(|q| format!(" | search: {}", q))
            .unwrap_or_default();
        format!(
            "j/k move | gg/G jump | Ctrl+u/d half-page | n/N next/prev | / search{}",
            query
        )
    }

    #[allow(dead_code)]
    pub fn selected_path(&self) -> Option<String> {
        self.entries.get(self.selection).map(|e| e.path.clone())
    }

    pub fn content_height(&self) -> usize {
        self.entries.len()
    }

    pub fn render_plain_lines(&self, _width: u16) -> Vec<Line<'static>> {
        self.entries
            .iter()
            .map(|entry| {
                let icon = if entry.is_dir { "D" } else { " " };
                let size = format_size(entry.size);
                Line::from(vec![
                    Span::styled(icon.to_string(), Style::default().fg(Color::Cyan)),
                    Span::styled(format!(" {:>8} ", size), Style::default().fg(Color::LightYellow)),
                    Span::styled(entry.path.clone(), Style::default().fg(Color::White)),
                ])
            })
            .collect()
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
            if self.entries[idx].path.to_lowercase().contains(&lower) {
                self.selection = idx;
                break;
            }
        }
        self.last_match = Some(query.to_string());
    }
}

fn parse_zip(path: &Path) -> Result<Vec<ArchiveEntry>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut archive = zip::ZipArchive::new(reader)?;

    let mut entries = Vec::new();
    for i in 0..archive.len() {
        let file = archive.by_index(i)?;
        entries.push(ArchiveEntry {
            path: file.name().to_string(),
            size: file.size(),
            compressed_size: Some(file.compressed_size()),
            is_dir: file.is_dir(),
            modified: None,
        });
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

fn parse_tar(path: &Path, compression: Option<&str>) -> Result<Vec<ArchiveEntry>> {
    let file = File::open(path)?;

    let reader: Box<dyn Read> = match compression {
        Some("gz") => {
            let decoder = flate2::read::GzDecoder::new(file);
            Box::new(decoder)
        }
        _ => Box::new(file),
    };

    let mut archive = tar::Archive::new(reader);
    let mut entries = Vec::new();

    for entry in archive.entries()? {
        let entry = entry?;
        let path = entry.path()?.to_string_lossy().to_string();
        let size = entry.size();
        let is_dir = entry.header().entry_type().is_dir();

        entries.push(ArchiveEntry {
            path,
            size,
            compressed_size: None,
            is_dir,
            modified: None,
        });
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1}G", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1}M", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1}K", bytes as f64 / KB as f64)
    } else {
        format!("{}B", bytes)
    }
}

fn page_jump(view_height: usize) -> usize {
    let half = view_height / 2;
    if half == 0 { 1 } else { half }
}
