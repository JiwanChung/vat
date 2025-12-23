use std::path::Path;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

#[derive(Clone)]
enum IniLine {
    Section(String),
    KeyValue { key: String, value: String },
    Comment(String),
    Empty,
}

pub struct IniEngine {
    lines: Vec<(usize, IniLine)>, // (line_no, parsed)
    selection: usize,
    scroll: usize,
    file_name: String,
    last_query: Option<String>,
    pending_g: bool,
    last_view_height: usize,
    last_match: Option<String>,
}

impl IniEngine {
    pub fn from_path(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        let lines = parse_ini(&content);

        Ok(Self {
            lines,
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
        let height = area.height as usize;
        self.last_view_height = height;

        if self.selection < self.scroll {
            self.scroll = self.selection;
        } else if self.selection >= self.scroll + height {
            self.scroll = self.selection.saturating_sub(height - 1);
        }

        let line_no_width = self.lines.len().max(1).to_string().len().max(2);

        let visible: Vec<Line> = self.lines
            .iter()
            .skip(self.scroll)
            .take(height)
            .enumerate()
            .map(|(idx, (line_no, parsed))| {
                let row = self.scroll + idx;
                let selected = row == self.selection;

                let mut spans = Vec::new();
                let line_no_str = format!("{:>width$} ", line_no, width = line_no_width);
                let line_no_style = if selected {
                    Style::default().fg(Color::Black).bg(Color::LightBlue).bold()
                } else {
                    Style::default().fg(Color::LightYellow)
                };
                spans.push(Span::styled(line_no_str, line_no_style));
                spans.push(Span::styled("│ ", Style::default().fg(Color::LightBlue)));

                match parsed {
                    IniLine::Section(name) => {
                        let style = if selected {
                            Style::default().fg(Color::Black).bg(Color::LightBlue).bold()
                        } else {
                            Style::default().fg(Color::LightCyan).bold()
                        };
                        spans.push(Span::styled(format!("[{}]", name), style));
                    }
                    IniLine::KeyValue { key, value } => {
                        let key_style = if selected {
                            Style::default().fg(Color::Black).bg(Color::LightBlue)
                        } else {
                            Style::default().fg(Color::LightGreen)
                        };
                        let eq_style = if selected {
                            Style::default().fg(Color::Black).bg(Color::LightBlue)
                        } else {
                            Style::default().fg(Color::White)
                        };
                        let val_style = if selected {
                            Style::default().fg(Color::Black).bg(Color::LightBlue)
                        } else {
                            Style::default().fg(Color::LightYellow)
                        };
                        spans.push(Span::styled(key.clone(), key_style));
                        spans.push(Span::styled(" = ", eq_style));
                        spans.push(Span::styled(value.clone(), val_style));
                    }
                    IniLine::Comment(text) => {
                        let style = if selected {
                            Style::default().fg(Color::Black).bg(Color::LightBlue)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        };
                        spans.push(Span::styled(text.clone(), style));
                    }
                    IniLine::Empty => {}
                }

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
            KeyCode::Char('e') => {
                // Jump to next section
                for i in (self.selection + 1)..total {
                    if matches!(self.lines[i].1, IniLine::Section(_)) {
                        self.selection = i;
                        break;
                    }
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
        // Find current section
        let mut section = "root".to_string();
        for i in (0..=self.selection).rev() {
            if let IniLine::Section(name) = &self.lines[i].1 {
                section = name.clone();
                break;
            }
        }
        format!("{} [{}] line {}", self.file_name, section, self.selection + 1)
    }

    pub fn status_line(&self) -> String {
        let query = self
            .last_query
            .as_ref()
            .map(|q| format!(" | search: {}", q))
            .unwrap_or_default();
        format!(
            "j/k move | gg/G jump | Ctrl+u/d half-page | e next section | n/N next/prev | / search{}",
            query
        )
    }

    #[allow(dead_code)]
    pub fn selected_path(&self) -> Option<String> {
        None
    }

    pub fn content_height(&self) -> usize {
        self.lines.len()
    }

    pub fn render_plain_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let line_no_width = self.lines.len().max(1).to_string().len().max(2);
        self.lines
            .iter()
            .map(|(line_no, parsed)| {
                let mut spans = Vec::new();
                spans.push(Span::styled(
                    format!("{:>width$} ", line_no, width = line_no_width),
                    Style::default().fg(Color::LightYellow),
                ));
                spans.push(Span::styled("│ ", Style::default().fg(Color::LightBlue)));

                match parsed {
                    IniLine::Section(name) => {
                        spans.push(Span::styled(
                            format!("[{}]", name),
                            Style::default().fg(Color::LightCyan).bold(),
                        ));
                    }
                    IniLine::KeyValue { key, value } => {
                        spans.push(Span::styled(key.clone(), Style::default().fg(Color::LightGreen)));
                        spans.push(Span::styled(" = ", Style::default().fg(Color::White)));
                        spans.push(Span::styled(value.clone(), Style::default().fg(Color::LightYellow)));
                    }
                    IniLine::Comment(text) => {
                        spans.push(Span::styled(text.clone(), Style::default().fg(Color::DarkGray)));
                    }
                    IniLine::Empty => {}
                }

                Line::from(spans)
            })
            .collect()
    }

    fn search_next(&mut self, query: &str, forward: bool) {
        let lower = query.to_lowercase();
        let total = self.lines.len().max(1);
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
            let text = match &self.lines[idx].1 {
                IniLine::Section(name) => name.clone(),
                IniLine::KeyValue { key, value } => format!("{} = {}", key, value),
                IniLine::Comment(text) => text.clone(),
                IniLine::Empty => String::new(),
            };
            if text.to_lowercase().contains(&lower) {
                self.selection = idx;
                break;
            }
        }
        self.last_match = Some(query.to_string());
    }
}

fn parse_ini(content: &str) -> Vec<(usize, IniLine)> {
    let mut lines = Vec::new();

    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        let line_no = idx + 1;

        if trimmed.is_empty() {
            lines.push((line_no, IniLine::Empty));
        } else if trimmed.starts_with('#') || trimmed.starts_with(';') {
            lines.push((line_no, IniLine::Comment(trimmed.to_string())));
        } else if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let name = trimmed[1..trimmed.len() - 1].trim().to_string();
            lines.push((line_no, IniLine::Section(name)));
        } else if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos].trim().to_string();
            let value = trimmed[eq_pos + 1..].trim().to_string();
            lines.push((line_no, IniLine::KeyValue { key, value }));
        } else if let Some(colon_pos) = trimmed.find(':') {
            // Properties-style with colon
            let key = trimmed[..colon_pos].trim().to_string();
            let value = trimmed[colon_pos + 1..].trim().to_string();
            lines.push((line_no, IniLine::KeyValue { key, value }));
        } else {
            // Treat as comment/unknown
            lines.push((line_no, IniLine::Comment(trimmed.to_string())));
        }
    }

    lines
}

fn page_jump(view_height: usize) -> usize {
    let half = view_height / 2;
    if half == 0 { 1 } else { half }
}
