use std::path::Path;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

#[derive(Clone)]
enum GitIgnoreLine {
    Pattern { pattern: String, is_negated: bool, is_dir: bool },
    Comment(String),
    Empty,
}

pub struct GitIgnoreEngine {
    lines: Vec<(usize, GitIgnoreLine)>,
    selection: usize,
    scroll: usize,
    file_name: String,
    last_query: Option<String>,
    pending_g: bool,
    last_view_height: usize,
    last_match: Option<String>,
}

impl GitIgnoreEngine {
    pub fn from_path(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        let lines = parse_gitignore(&content);

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
                    GitIgnoreLine::Pattern { pattern, is_negated, is_dir } => {
                        if *is_negated {
                            let neg_style = if selected {
                                Style::default().fg(Color::Black).bg(Color::LightBlue)
                            } else {
                                Style::default().fg(Color::LightGreen)
                            };
                            spans.push(Span::styled("! ", neg_style));
                        }

                        let pattern_style = if selected {
                            Style::default().fg(Color::Black).bg(Color::LightBlue)
                        } else if *is_negated {
                            Style::default().fg(Color::LightGreen)
                        } else {
                            Style::default().fg(Color::LightRed)
                        };
                        spans.push(Span::styled(pattern.clone(), pattern_style));

                        if *is_dir {
                            let dir_style = if selected {
                                Style::default().fg(Color::Black).bg(Color::LightBlue)
                            } else {
                                Style::default().fg(Color::DarkGray)
                            };
                            spans.push(Span::styled(" (dir)", dir_style));
                        }

                        // Show pattern type hint
                        let hint = categorize_pattern(pattern);
                        if !hint.is_empty() {
                            let hint_style = if selected {
                                Style::default().fg(Color::Black).bg(Color::LightBlue)
                            } else {
                                Style::default().fg(Color::Cyan)
                            };
                            spans.push(Span::styled(format!("  # {}", hint), hint_style));
                        }
                    }
                    GitIgnoreLine::Comment(text) => {
                        let style = if selected {
                            Style::default().fg(Color::Black).bg(Color::LightBlue)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        };
                        spans.push(Span::styled(text.clone(), style));
                    }
                    GitIgnoreLine::Empty => {}
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
        format!("{} line {}", self.file_name, self.selection + 1)
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
                    GitIgnoreLine::Pattern { pattern, is_negated, .. } => {
                        if *is_negated {
                            spans.push(Span::styled("! ", Style::default().fg(Color::LightGreen)));
                        }
                        let color = if *is_negated { Color::LightGreen } else { Color::LightRed };
                        spans.push(Span::styled(pattern.clone(), Style::default().fg(color)));
                    }
                    GitIgnoreLine::Comment(text) => {
                        spans.push(Span::styled(text.clone(), Style::default().fg(Color::DarkGray)));
                    }
                    GitIgnoreLine::Empty => {}
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
                GitIgnoreLine::Pattern { pattern, .. } => pattern.clone(),
                GitIgnoreLine::Comment(text) => text.clone(),
                GitIgnoreLine::Empty => String::new(),
            };
            if text.to_lowercase().contains(&lower) {
                self.selection = idx;
                break;
            }
        }
        self.last_match = Some(query.to_string());
    }
}

fn parse_gitignore(content: &str) -> Vec<(usize, GitIgnoreLine)> {
    let mut lines = Vec::new();

    for (idx, line) in content.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = line.trim();

        if trimmed.is_empty() {
            lines.push((line_no, GitIgnoreLine::Empty));
            continue;
        }

        if trimmed.starts_with('#') {
            lines.push((line_no, GitIgnoreLine::Comment(trimmed.to_string())));
            continue;
        }

        let is_negated = trimmed.starts_with('!');
        let pattern = if is_negated {
            trimmed[1..].to_string()
        } else {
            trimmed.to_string()
        };

        let is_dir = pattern.ends_with('/');
        let pattern = if is_dir {
            pattern[..pattern.len() - 1].to_string()
        } else {
            pattern
        };

        lines.push((line_no, GitIgnoreLine::Pattern { pattern, is_negated, is_dir }));
    }

    lines
}

fn categorize_pattern(pattern: &str) -> &'static str {
    if pattern.starts_with("*.") {
        return "extension";
    }
    if pattern.contains("node_modules") {
        return "npm";
    }
    if pattern.contains("__pycache__") || pattern.ends_with(".pyc") {
        return "python";
    }
    if pattern.contains("target/") || pattern == "Cargo.lock" {
        return "rust";
    }
    if pattern.contains(".git") {
        return "git";
    }
    if pattern.starts_with('.') {
        return "dotfile";
    }
    if pattern.contains("build/") || pattern.contains("dist/") {
        return "build";
    }
    if pattern.contains(".env") {
        return "env";
    }
    if pattern.contains("log") {
        return "logs";
    }
    ""
}

fn page_jump(view_height: usize) -> usize {
    let half = view_height / 2;
    if half == 0 { 1 } else { half }
}
