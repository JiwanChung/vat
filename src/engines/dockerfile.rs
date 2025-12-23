use std::path::Path;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

#[derive(Clone)]
enum DockerLine {
    From { image: String, alias: Option<String>, stage_num: usize },
    Instruction { cmd: String, args: String },
    Comment(String),
    Empty,
    Arg { name: String, default: Option<String> },
    Env { key: String, value: String },
    Label { key: String, value: String },
}

pub struct DockerfileEngine {
    lines: Vec<(usize, DockerLine)>,
    selection: usize,
    scroll: usize,
    file_name: String,
    last_query: Option<String>,
    pending_g: bool,
    last_view_height: usize,
    last_match: Option<String>,
}

impl DockerfileEngine {
    pub fn from_path(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        let lines = parse_dockerfile(&content);

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
                    DockerLine::From { image, alias, stage_num } => {
                        let cmd_style = if selected {
                            Style::default().fg(Color::Black).bg(Color::LightBlue).bold()
                        } else {
                            Style::default().fg(Color::LightMagenta).bold()
                        };
                        let img_style = if selected {
                            Style::default().fg(Color::Black).bg(Color::LightBlue)
                        } else {
                            Style::default().fg(Color::LightGreen)
                        };
                        spans.push(Span::styled(format!("[Stage {}] ", stage_num), Style::default().fg(Color::Cyan)));
                        spans.push(Span::styled("FROM ", cmd_style));
                        spans.push(Span::styled(image.clone(), img_style));
                        if let Some(a) = alias {
                            spans.push(Span::styled(" AS ", cmd_style));
                            spans.push(Span::styled(a.clone(), Style::default().fg(Color::LightCyan)));
                        }
                    }
                    DockerLine::Instruction { cmd, args } => {
                        let cmd_style = if selected {
                            Style::default().fg(Color::Black).bg(Color::LightBlue).bold()
                        } else {
                            Style::default().fg(Color::LightCyan).bold()
                        };
                        let args_style = if selected {
                            Style::default().fg(Color::Black).bg(Color::LightBlue)
                        } else {
                            Style::default().fg(Color::White)
                        };
                        spans.push(Span::styled(format!("{} ", cmd), cmd_style));
                        spans.push(Span::styled(truncate(args, 60), args_style));
                    }
                    DockerLine::Arg { name, default } => {
                        let cmd_style = if selected {
                            Style::default().fg(Color::Black).bg(Color::LightBlue).bold()
                        } else {
                            Style::default().fg(Color::LightYellow).bold()
                        };
                        spans.push(Span::styled("ARG ", cmd_style));
                        spans.push(Span::styled(name.clone(), Style::default().fg(Color::LightGreen)));
                        if let Some(def) = default {
                            spans.push(Span::styled("=", Style::default().fg(Color::White)));
                            spans.push(Span::styled(def.clone(), Style::default().fg(Color::LightCyan)));
                        }
                    }
                    DockerLine::Env { key, value } => {
                        let cmd_style = if selected {
                            Style::default().fg(Color::Black).bg(Color::LightBlue).bold()
                        } else {
                            Style::default().fg(Color::LightYellow).bold()
                        };
                        spans.push(Span::styled("ENV ", cmd_style));
                        spans.push(Span::styled(key.clone(), Style::default().fg(Color::LightGreen)));
                        spans.push(Span::styled("=", Style::default().fg(Color::White)));
                        spans.push(Span::styled(value.clone(), Style::default().fg(Color::LightCyan)));
                    }
                    DockerLine::Label { key, value } => {
                        let cmd_style = if selected {
                            Style::default().fg(Color::Black).bg(Color::LightBlue).bold()
                        } else {
                            Style::default().fg(Color::DarkGray).bold()
                        };
                        spans.push(Span::styled("LABEL ", cmd_style));
                        spans.push(Span::styled(format!("{}={}", key, value), Style::default().fg(Color::DarkGray)));
                    }
                    DockerLine::Comment(text) => {
                        let style = if selected {
                            Style::default().fg(Color::Black).bg(Color::LightBlue)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        };
                        spans.push(Span::styled(text.clone(), style));
                    }
                    DockerLine::Empty => {}
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
                // Jump to next FROM (stage)
                for i in (self.selection + 1)..total {
                    if matches!(self.lines[i].1, DockerLine::From { .. }) {
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
        // Find current stage
        let mut stage = 0;
        for i in (0..=self.selection).rev() {
            if let DockerLine::From { stage_num, .. } = &self.lines[i].1 {
                stage = *stage_num;
                break;
            }
        }
        format!("{} stage {} line {}", self.file_name, stage, self.selection + 1)
    }

    pub fn status_line(&self) -> String {
        let query = self
            .last_query
            .as_ref()
            .map(|q| format!(" | search: {}", q))
            .unwrap_or_default();
        format!(
            "j/k move | gg/G jump | Ctrl+u/d half-page | e next stage | n/N next/prev | / search{}",
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
                    DockerLine::From { image, alias, stage_num } => {
                        spans.push(Span::styled(format!("[Stage {}] ", stage_num), Style::default().fg(Color::Cyan)));
                        spans.push(Span::styled("FROM ", Style::default().fg(Color::LightMagenta).bold()));
                        spans.push(Span::styled(image.clone(), Style::default().fg(Color::LightGreen)));
                        if let Some(a) = alias {
                            spans.push(Span::styled(" AS ", Style::default().fg(Color::LightMagenta).bold()));
                            spans.push(Span::styled(a.clone(), Style::default().fg(Color::LightCyan)));
                        }
                    }
                    DockerLine::Instruction { cmd, args } => {
                        spans.push(Span::styled(format!("{} ", cmd), Style::default().fg(Color::LightCyan).bold()));
                        spans.push(Span::styled(args.clone(), Style::default().fg(Color::White)));
                    }
                    DockerLine::Comment(text) => {
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
                DockerLine::From { image, alias, .. } => {
                    format!("FROM {} {}", image, alias.as_deref().unwrap_or(""))
                }
                DockerLine::Instruction { cmd, args } => format!("{} {}", cmd, args),
                DockerLine::Comment(text) => text.clone(),
                DockerLine::Arg { name, default } => {
                    format!("ARG {} {}", name, default.as_deref().unwrap_or(""))
                }
                DockerLine::Env { key, value } => format!("ENV {}={}", key, value),
                DockerLine::Label { key, value } => format!("LABEL {}={}", key, value),
                DockerLine::Empty => String::new(),
            };
            if text.to_lowercase().contains(&lower) {
                self.selection = idx;
                break;
            }
        }
        self.last_match = Some(query.to_string());
    }
}

fn parse_dockerfile(content: &str) -> Vec<(usize, DockerLine)> {
    let mut lines = Vec::new();
    let mut stage_num = 0;
    let mut continued_line = String::new();
    let mut continued_start = 0;

    for (idx, line) in content.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = line.trim();

        // Handle line continuation
        if trimmed.ends_with('\\') {
            if continued_line.is_empty() {
                continued_start = line_no;
            }
            continued_line.push_str(&trimmed[..trimmed.len() - 1]);
            continued_line.push(' ');
            continue;
        }

        let full_line = if !continued_line.is_empty() {
            let mut full = std::mem::take(&mut continued_line);
            full.push_str(trimmed);
            full
        } else {
            trimmed.to_string()
        };

        let effective_line_no = if continued_start > 0 {
            let ln = continued_start;
            continued_start = 0;
            ln
        } else {
            line_no
        };

        if full_line.is_empty() {
            lines.push((effective_line_no, DockerLine::Empty));
            continue;
        }

        if full_line.starts_with('#') {
            lines.push((effective_line_no, DockerLine::Comment(full_line)));
            continue;
        }

        let parts: Vec<&str> = full_line.splitn(2, char::is_whitespace).collect();
        let cmd = parts[0].to_uppercase();
        let args = parts.get(1).map(|s| s.trim()).unwrap_or("").to_string();

        match cmd.as_str() {
            "FROM" => {
                stage_num += 1;
                let (image, alias) = if let Some(as_pos) = args.to_lowercase().find(" as ") {
                    let img = args[..as_pos].trim().to_string();
                    let al = args[as_pos + 4..].trim().to_string();
                    (img, Some(al))
                } else {
                    (args, None)
                };
                lines.push((effective_line_no, DockerLine::From { image, alias, stage_num }));
            }
            "ARG" => {
                if let Some(eq_pos) = args.find('=') {
                    let name = args[..eq_pos].trim().to_string();
                    let default = Some(args[eq_pos + 1..].trim().to_string());
                    lines.push((effective_line_no, DockerLine::Arg { name, default }));
                } else {
                    lines.push((effective_line_no, DockerLine::Arg { name: args, default: None }));
                }
            }
            "ENV" => {
                if let Some(eq_pos) = args.find('=') {
                    let key = args[..eq_pos].trim().to_string();
                    let value = args[eq_pos + 1..].trim().to_string();
                    lines.push((effective_line_no, DockerLine::Env { key, value }));
                } else if let Some(space_pos) = args.find(' ') {
                    let key = args[..space_pos].trim().to_string();
                    let value = args[space_pos + 1..].trim().to_string();
                    lines.push((effective_line_no, DockerLine::Env { key, value }));
                } else {
                    lines.push((effective_line_no, DockerLine::Instruction { cmd, args }));
                }
            }
            "LABEL" => {
                if let Some(eq_pos) = args.find('=') {
                    let key = args[..eq_pos].trim().to_string();
                    let value = args[eq_pos + 1..].trim().trim_matches('"').to_string();
                    lines.push((effective_line_no, DockerLine::Label { key, value }));
                } else {
                    lines.push((effective_line_no, DockerLine::Instruction { cmd, args }));
                }
            }
            _ => {
                lines.push((effective_line_no, DockerLine::Instruction { cmd, args }));
            }
        }
    }

    lines
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
