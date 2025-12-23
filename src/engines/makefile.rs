use std::path::Path;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

#[derive(Clone)]
enum MakeLine {
    Target { name: String, deps: Vec<String>, is_phony: bool },
    Recipe(String),
    Variable { name: String, op: String, value: String },
    Include(String),
    Conditional(String),
    Comment(String),
    Empty,
}

pub struct MakefileEngine {
    lines: Vec<(usize, MakeLine)>,
    phony_targets: Vec<String>,
    selection: usize,
    scroll: usize,
    file_name: String,
    last_query: Option<String>,
    pending_g: bool,
    last_view_height: usize,
    last_match: Option<String>,
}

impl MakefileEngine {
    pub fn from_path(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        let (lines, phony_targets) = parse_makefile(&content);

        Ok(Self {
            lines,
            phony_targets,
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
                    MakeLine::Target { name, deps, is_phony } => {
                        if *is_phony {
                            spans.push(Span::styled("[P] ", Style::default().fg(Color::Magenta)));
                        }
                        let name_style = if selected {
                            Style::default().fg(Color::Black).bg(Color::LightBlue).bold()
                        } else {
                            Style::default().fg(Color::LightGreen).bold()
                        };
                        spans.push(Span::styled(name.clone(), name_style));
                        spans.push(Span::styled(":", Style::default().fg(Color::White)));
                        if !deps.is_empty() {
                            let dep_style = if selected {
                                Style::default().fg(Color::Black).bg(Color::LightBlue)
                            } else {
                                Style::default().fg(Color::LightCyan)
                            };
                            spans.push(Span::styled(format!(" {}", deps.join(" ")), dep_style));
                        }
                    }
                    MakeLine::Recipe(cmd) => {
                        let style = if selected {
                            Style::default().fg(Color::Black).bg(Color::LightBlue)
                        } else {
                            Style::default().fg(Color::White)
                        };
                        spans.push(Span::styled("    ", Style::default()));
                        spans.push(Span::styled(cmd.clone(), style));
                    }
                    MakeLine::Variable { name, op, value } => {
                        let name_style = if selected {
                            Style::default().fg(Color::Black).bg(Color::LightBlue)
                        } else {
                            Style::default().fg(Color::LightYellow)
                        };
                        let val_style = if selected {
                            Style::default().fg(Color::Black).bg(Color::LightBlue)
                        } else {
                            Style::default().fg(Color::LightCyan)
                        };
                        spans.push(Span::styled(name.clone(), name_style));
                        spans.push(Span::styled(format!(" {} ", op), Style::default().fg(Color::White)));
                        spans.push(Span::styled(truncate(value, 50), val_style));
                    }
                    MakeLine::Include(path) => {
                        let style = if selected {
                            Style::default().fg(Color::Black).bg(Color::LightBlue)
                        } else {
                            Style::default().fg(Color::LightMagenta)
                        };
                        spans.push(Span::styled("include ", style));
                        spans.push(Span::styled(path.clone(), Style::default().fg(Color::LightCyan)));
                    }
                    MakeLine::Conditional(text) => {
                        let style = if selected {
                            Style::default().fg(Color::Black).bg(Color::LightBlue)
                        } else {
                            Style::default().fg(Color::LightRed)
                        };
                        spans.push(Span::styled(text.clone(), style));
                    }
                    MakeLine::Comment(text) => {
                        let style = if selected {
                            Style::default().fg(Color::Black).bg(Color::LightBlue)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        };
                        spans.push(Span::styled(text.clone(), style));
                    }
                    MakeLine::Empty => {}
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
                // Jump to next target
                for i in (self.selection + 1)..total {
                    if matches!(self.lines[i].1, MakeLine::Target { .. }) {
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
        // Find current target
        let mut target = "".to_string();
        for i in (0..=self.selection).rev() {
            if let MakeLine::Target { name, .. } = &self.lines[i].1 {
                target = name.clone();
                break;
            }
        }
        if target.is_empty() {
            format!("{} line {}", self.file_name, self.selection + 1)
        } else {
            format!("{} target:{} line {}", self.file_name, target, self.selection + 1)
        }
    }

    pub fn status_line(&self) -> String {
        let query = self
            .last_query
            .as_ref()
            .map(|q| format!(" | search: {}", q))
            .unwrap_or_default();
        format!(
            "j/k move | gg/G jump | Ctrl+u/d half-page | e next target | n/N next/prev | / search{}",
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
                    MakeLine::Target { name, deps, is_phony } => {
                        if *is_phony {
                            spans.push(Span::styled("[P] ", Style::default().fg(Color::Magenta)));
                        }
                        spans.push(Span::styled(name.clone(), Style::default().fg(Color::LightGreen).bold()));
                        spans.push(Span::styled(":", Style::default().fg(Color::White)));
                        if !deps.is_empty() {
                            spans.push(Span::styled(format!(" {}", deps.join(" ")), Style::default().fg(Color::LightCyan)));
                        }
                    }
                    MakeLine::Recipe(cmd) => {
                        spans.push(Span::styled("    ", Style::default()));
                        spans.push(Span::styled(cmd.clone(), Style::default().fg(Color::White)));
                    }
                    MakeLine::Variable { name, op, value } => {
                        spans.push(Span::styled(name.clone(), Style::default().fg(Color::LightYellow)));
                        spans.push(Span::styled(format!(" {} ", op), Style::default().fg(Color::White)));
                        spans.push(Span::styled(value.clone(), Style::default().fg(Color::LightCyan)));
                    }
                    MakeLine::Comment(text) => {
                        spans.push(Span::styled(text.clone(), Style::default().fg(Color::DarkGray)));
                    }
                    _ => {}
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
                MakeLine::Target { name, deps, .. } => format!("{}: {}", name, deps.join(" ")),
                MakeLine::Recipe(cmd) => cmd.clone(),
                MakeLine::Variable { name, op, value } => format!("{} {} {}", name, op, value),
                MakeLine::Comment(text) => text.clone(),
                MakeLine::Include(path) => format!("include {}", path),
                MakeLine::Conditional(text) => text.clone(),
                MakeLine::Empty => String::new(),
            };
            if text.to_lowercase().contains(&lower) {
                self.selection = idx;
                break;
            }
        }
        self.last_match = Some(query.to_string());
    }
}

fn parse_makefile(content: &str) -> (Vec<(usize, MakeLine)>, Vec<String>) {
    let mut lines = Vec::new();
    let mut phony_targets = Vec::new();

    for (idx, line) in content.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = line.trim();

        if trimmed.is_empty() {
            lines.push((line_no, MakeLine::Empty));
            continue;
        }

        if trimmed.starts_with('#') {
            lines.push((line_no, MakeLine::Comment(trimmed.to_string())));
            continue;
        }

        // Recipe line (starts with tab)
        if line.starts_with('\t') {
            lines.push((line_no, MakeLine::Recipe(line[1..].to_string())));
            continue;
        }

        // .PHONY
        if trimmed.starts_with(".PHONY:") {
            let targets: Vec<String> = trimmed[7..]
                .split_whitespace()
                .map(|s| s.to_string())
                .collect();
            phony_targets.extend(targets);
            lines.push((line_no, MakeLine::Comment(trimmed.to_string())));
            continue;
        }

        // Include
        if trimmed.starts_with("include ") || trimmed.starts_with("-include ") {
            let path = trimmed.split_whitespace().nth(1).unwrap_or("").to_string();
            lines.push((line_no, MakeLine::Include(path)));
            continue;
        }

        // Conditionals
        if trimmed.starts_with("ifeq")
            || trimmed.starts_with("ifneq")
            || trimmed.starts_with("ifdef")
            || trimmed.starts_with("ifndef")
            || trimmed.starts_with("else")
            || trimmed.starts_with("endif")
        {
            lines.push((line_no, MakeLine::Conditional(trimmed.to_string())));
            continue;
        }

        // Variable assignment
        for op in &[":=", "?=", "+=", "="] {
            if let Some(pos) = trimmed.find(op) {
                let name = trimmed[..pos].trim().to_string();
                let value = trimmed[pos + op.len()..].trim().to_string();
                lines.push((line_no, MakeLine::Variable {
                    name,
                    op: op.to_string(),
                    value,
                }));
                continue;
            }
        }

        // Target
        if let Some(colon_pos) = trimmed.find(':') {
            // Check it's not a variable with ::
            if !trimmed[colon_pos..].starts_with("::") || trimmed[colon_pos..].starts_with("::") {
                let name = trimmed[..colon_pos].trim().to_string();
                let deps: Vec<String> = trimmed[colon_pos + 1..]
                    .split_whitespace()
                    .map(|s| s.to_string())
                    .collect();
                let is_phony = phony_targets.contains(&name);
                lines.push((line_no, MakeLine::Target { name, deps, is_phony }));
                continue;
            }
        }

        // Fallback
        lines.push((line_no, MakeLine::Comment(trimmed.to_string())));
    }

    // Update phony status after full parse
    for (_, line) in lines.iter_mut() {
        if let MakeLine::Target { name, is_phony, .. } = line {
            *is_phony = phony_targets.contains(name);
        }
    }

    (lines, phony_targets)
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
