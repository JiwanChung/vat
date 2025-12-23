use std::path::Path;

use anyhow::{anyhow, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use image::GenericImageView;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

#[derive(Clone)]
struct ImageInfo {
    width: u32,
    height: u32,
    format: String,
    color_type: String,
    file_size: u64,
    bits_per_pixel: u16,
}

#[derive(Clone)]
struct InfoLine {
    label: String,
    value: String,
}

pub struct ImageEngine {
    info: ImageInfo,
    lines: Vec<InfoLine>,
    selection: usize,
    scroll: usize,
    file_name: String,
    last_query: Option<String>,
    pending_g: bool,
    last_view_height: usize,
    /// Visual selection range (start, end) for highlighting
    pub visual_range: Option<(usize, usize)>,
}

impl ImageEngine {
    pub fn from_path(path: &Path) -> Result<Self> {
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        let metadata = std::fs::metadata(path)?;
        let file_size = metadata.len();

        let img = image::open(path).map_err(|e| anyhow!("Failed to open image: {}", e))?;

        let (width, height) = img.dimensions();
        let color = img.color();

        let format = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_uppercase();

        let color_type = format!("{:?}", color);
        let bits_per_pixel = color.bits_per_pixel();

        let info = ImageInfo {
            width,
            height,
            format,
            color_type,
            file_size,
            bits_per_pixel,
        };

        let lines = build_info_lines(&info, &file_name);

        Ok(Self {
            info,
            lines,
            selection: 0,
            scroll: 0,
            file_name,
            last_query: None,
            pending_g: false,
            last_view_height: 0,
            visual_range: None,
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

        let visible: Vec<Line> = self.lines
            .iter()
            .skip(self.scroll)
            .take(height)
            .enumerate()
            .map(|(idx, line)| {
                let row = self.scroll + idx;
                let selected = row == self.selection;

                let label_style = if selected {
                    Style::default().fg(Color::Black).bg(Color::LightBlue).bold()
                } else {
                    Style::default().fg(Color::LightCyan)
                };

                let value_style = if selected {
                    Style::default().fg(Color::Black).bg(Color::LightBlue)
                } else {
                    Style::default().fg(Color::White)
                };

                if line.label.is_empty() {
                    Line::from("")
                } else if line.label.starts_with("---") {
                    Line::from(Span::styled(&line.label, Style::default().fg(Color::DarkGray)))
                } else {
                    Line::from(vec![
                        Span::styled(format!("{:<20}", line.label), label_style),
                        Span::styled(&line.value, value_style),
                    ])
                }
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

        let total = self.lines.len();
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
        // No search for image metadata
    }

    pub fn apply_filter(&mut self, _query: &str) {}

    pub fn clear_filter(&mut self) {
        self.last_query = None;
    }

    pub fn breadcrumbs(&self) -> String {
        format!(
            "{} {}x{} {}",
            self.file_name,
            self.info.width,
            self.info.height,
            self.info.format
        )
    }

    pub fn status_line(&self) -> String {
        format!(
            "j/k move | gg/G jump | Ctrl+u/d half-page | {}x{} {} {}bpp",
            self.info.width,
            self.info.height,
            self.info.color_type,
            self.info.bits_per_pixel
        )
    }

    #[allow(dead_code)]
    pub fn selected_path(&self) -> Option<String> {
        None
    }

    /// Get the content of the currently selected line
    pub fn get_selected_line(&self) -> Option<String> {
        self.lines.get(self.selection).map(|line| {
            format!("{}: {}", line.label, line.value)
        })
    }

    /// Get lines in a range (inclusive), joined by newlines
    pub fn get_lines_range(&self, start: usize, end: usize) -> Option<String> {
        let (start, end) = if start <= end { (start, end) } else { (end, start) };
        let total = self.lines.len();
        if start >= total { return None; }
        let end = end.min(total.saturating_sub(1));
        let lines: Vec<String> = self.lines[start..=end].iter().map(|line| format!("{}: {}", line.label, line.value)).collect();
        if lines.is_empty() { None } else { Some(lines.join("\n")) }
    }

    /// Get current selection index (for visual mode)
    pub fn selection(&self) -> usize {
        self.selection
    }

    pub fn content_height(&self) -> usize {
        self.lines.len()
    }

    pub fn render_plain_lines(&self, _width: u16) -> Vec<Line<'static>> {
        self.lines
            .iter()
            .map(|line| {
                if line.label.is_empty() {
                    Line::from("")
                } else if line.label.starts_with("---") {
                    Line::from(Span::styled(line.label.clone(), Style::default().fg(Color::DarkGray)))
                } else {
                    Line::from(vec![
                        Span::styled(format!("{:<20}", line.label), Style::default().fg(Color::LightCyan)),
                        Span::styled(line.value.clone(), Style::default().fg(Color::White)),
                    ])
                }
            })
            .collect()
    }
}

fn build_info_lines(info: &ImageInfo, file_name: &str) -> Vec<InfoLine> {
    let mut lines = Vec::new();

    lines.push(InfoLine {
        label: "--- Basic Info ---".to_string(),
        value: String::new(),
    });
    lines.push(InfoLine {
        label: "File".to_string(),
        value: file_name.to_string(),
    });
    lines.push(InfoLine {
        label: "Format".to_string(),
        value: info.format.clone(),
    });
    lines.push(InfoLine {
        label: "Dimensions".to_string(),
        value: format!("{} x {} pixels", info.width, info.height),
    });
    lines.push(InfoLine {
        label: "Megapixels".to_string(),
        value: format!("{:.2} MP", (info.width as f64 * info.height as f64) / 1_000_000.0),
    });
    lines.push(InfoLine {
        label: "Aspect Ratio".to_string(),
        value: calculate_aspect_ratio(info.width, info.height),
    });

    lines.push(InfoLine { label: String::new(), value: String::new() });

    lines.push(InfoLine {
        label: "--- Color Info ---".to_string(),
        value: String::new(),
    });
    lines.push(InfoLine {
        label: "Color Type".to_string(),
        value: info.color_type.clone(),
    });
    lines.push(InfoLine {
        label: "Bits per Pixel".to_string(),
        value: format!("{} bpp", info.bits_per_pixel),
    });
    lines.push(InfoLine {
        label: "Color Depth".to_string(),
        value: format!("{}-bit", info.bits_per_pixel.saturating_div(3)),
    });

    lines.push(InfoLine { label: String::new(), value: String::new() });

    lines.push(InfoLine {
        label: "--- File Info ---".to_string(),
        value: String::new(),
    });
    lines.push(InfoLine {
        label: "File Size".to_string(),
        value: format_size(info.file_size),
    });
    lines.push(InfoLine {
        label: "Raw Size".to_string(),
        value: format_size(info.width as u64 * info.height as u64 * (info.bits_per_pixel as u64).saturating_div(8)),
    });

    let compression = if info.file_size > 0 {
        let raw = info.width as u64 * info.height as u64 * (info.bits_per_pixel as u64).saturating_div(8);
        if raw > 0 {
            format!("{:.1}x", raw as f64 / info.file_size as f64)
        } else {
            "N/A".to_string()
        }
    } else {
        "N/A".to_string()
    };
    lines.push(InfoLine {
        label: "Compression".to_string(),
        value: compression,
    });

    lines
}

fn calculate_aspect_ratio(width: u32, height: u32) -> String {
    let gcd = gcd(width, height);
    let w = width / gcd;
    let h = height / gcd;

    // Common ratios
    let ratio = width as f64 / height as f64;
    let common = if (ratio - 16.0 / 9.0).abs() < 0.01 {
        " (16:9)"
    } else if (ratio - 4.0 / 3.0).abs() < 0.01 {
        " (4:3)"
    } else if (ratio - 1.0).abs() < 0.01 {
        " (1:1)"
    } else if (ratio - 3.0 / 2.0).abs() < 0.01 {
        " (3:2)"
    } else if (ratio - 21.0 / 9.0).abs() < 0.01 {
        " (21:9)"
    } else {
        ""
    };

    format!("{}:{}{}", w, h, common)
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} bytes", bytes)
    }
}

fn page_jump(view_height: usize) -> usize {
    let half = view_height / 2;
    if half == 0 { 1 } else { half }
}
