use std::path::Path;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use regex::Regex;

#[derive(Clone)]
struct LogEntry {
    timestamp: Option<String>,
    level: Option<LogLevel>,
    source: Option<String>,
    message: String,
    raw: String,
}

#[derive(Clone, Copy, PartialEq)]
enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

pub struct LogEngine {
    entries: Vec<(usize, LogEntry)>,
    selection: usize,
    scroll: usize,
    file_name: String,
    last_query: Option<String>,
    pending_g: bool,
    last_view_height: usize,
    last_match: Option<String>,
    filter_level: Option<LogLevel>,
    /// Visual selection range (start, end) for highlighting
    pub visual_range: Option<(usize, usize)>,
}

impl LogEngine {
    pub fn from_path(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        let entries = parse_log(&content);

        Ok(Self {
            entries,
            selection: 0,
            scroll: 0,
            file_name,
            last_query: None,
            pending_g: false,
            last_view_height: 0,
            last_match: None,
            filter_level: None,
            visual_range: None,
        })
    }

    fn visible_entries(&self) -> Vec<usize> {
        match self.filter_level {
            Some(level) => self.entries
                .iter()
                .enumerate()
                .filter(|(_, (_, e))| e.level.map_or(true, |l| level_priority(l) >= level_priority(level)))
                .map(|(i, _)| i)
                .collect(),
            None => (0..self.entries.len()).collect(),
        }
    }

    pub fn render(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let height = area.height as usize;
        self.last_view_height = height;

        let visible = self.visible_entries();
        let total = visible.len();

        if self.selection >= total && total > 0 {
            self.selection = total - 1;
        }

        if self.selection < self.scroll {
            self.scroll = self.selection;
        } else if self.selection >= self.scroll + height {
            self.scroll = self.selection.saturating_sub(height - 1);
        }

        let line_no_width = self.entries.len().max(1).to_string().len().max(2);

        let display: Vec<Line> = visible
            .iter()
            .skip(self.scroll)
            .take(height)
            .enumerate()
            .map(|(display_idx, &entry_idx)| {
                let (line_no, entry) = &self.entries[entry_idx];
                let row = self.scroll + display_idx;
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

                // Timestamp
                if let Some(ts) = &entry.timestamp {
                    let ts_style = if selected {
                        Style::default().fg(Color::Black).bg(Color::LightBlue)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    };
                    spans.push(Span::styled(format!("{} ", ts), ts_style));
                }

                // Level
                if let Some(level) = entry.level {
                    let (text, color) = match level {
                        LogLevel::Debug => ("DBG", Color::Gray),
                        LogLevel::Info => ("INF", Color::Green),
                        LogLevel::Warn => ("WRN", Color::Yellow),
                        LogLevel::Error => ("ERR", Color::Red),
                        LogLevel::Fatal => ("FTL", Color::LightRed),
                    };
                    let level_style = if selected {
                        Style::default().fg(Color::Black).bg(Color::LightBlue).bold()
                    } else {
                        Style::default().fg(color).bold()
                    };
                    spans.push(Span::styled(format!("[{}] ", text), level_style));
                }

                // Source
                if let Some(src) = &entry.source {
                    let src_style = if selected {
                        Style::default().fg(Color::Black).bg(Color::LightBlue)
                    } else {
                        Style::default().fg(Color::Cyan)
                    };
                    spans.push(Span::styled(format!("{}: ", src), src_style));
                }

                // Message
                let msg_style = if selected {
                    Style::default().fg(Color::Black).bg(Color::LightBlue)
                } else {
                    Style::default().fg(Color::White)
                };
                spans.push(Span::styled(truncate(&entry.message, 80), msg_style));

                Line::from(spans)
            })
            .collect();

        let block = Block::default().borders(Borders::NONE);
        frame.render_widget(Paragraph::new(display).block(block), area);
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

        let visible = self.visible_entries();
        let total = visible.len();

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
            KeyCode::Char('1') => self.filter_level = Some(LogLevel::Debug),
            KeyCode::Char('2') => self.filter_level = Some(LogLevel::Info),
            KeyCode::Char('3') => self.filter_level = Some(LogLevel::Warn),
            KeyCode::Char('4') => self.filter_level = Some(LogLevel::Error),
            KeyCode::Char('0') => self.filter_level = None,
            KeyCode::Char('e') => {
                // Jump to next error
                for i in (self.selection + 1)..total {
                    if let Some(&entry_idx) = visible.get(i) {
                        if self.entries[entry_idx].1.level == Some(LogLevel::Error)
                            || self.entries[entry_idx].1.level == Some(LogLevel::Fatal)
                        {
                            self.selection = i;
                            break;
                        }
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
        self.filter_level = None;
    }

    pub fn breadcrumbs(&self) -> String {
        let filter = match self.filter_level {
            Some(LogLevel::Debug) => " [>=DEBUG]",
            Some(LogLevel::Info) => " [>=INFO]",
            Some(LogLevel::Warn) => " [>=WARN]",
            Some(LogLevel::Error) => " [>=ERROR]",
            Some(LogLevel::Fatal) => " [FATAL]",
            None => "",
        };
        format!("{} line {}{}", self.file_name, self.selection + 1, filter)
    }

    pub fn status_line(&self) -> String {
        let query = self
            .last_query
            .as_ref()
            .map(|q| format!(" | search: {}", q))
            .unwrap_or_default();
        format!(
            "j/k move | gg/G jump | e next error | 1-4 filter level | 0 clear | n/N next/prev | / search{}",
            query
        )
    }

    #[allow(dead_code)]
    pub fn selected_path(&self) -> Option<String> {
        None
    }

    /// Get the content of the currently selected line
    pub fn get_selected_line(&self) -> Option<String> {
        self.entries.get(self.selection).map(|(_, entry)| entry.raw.clone())
    }

    /// Get lines in a range (inclusive), joined by newlines
    pub fn get_lines_range(&self, start: usize, end: usize) -> Option<String> {
        let (start, end) = if start <= end { (start, end) } else { (end, start) };
        let total = self.entries.len();
        if start >= total { return None; }
        let end = end.min(total.saturating_sub(1));
        let lines: Vec<String> = self.entries[start..=end].iter().map(|(_, entry)| entry.raw.clone()).collect();
        if lines.is_empty() { None } else { Some(lines.join("\n")) }
    }

    /// Get current selection index (for visual mode)
    pub fn selection(&self) -> usize {
        self.selection
    }

    pub fn content_height(&self) -> usize {
        self.visible_entries().len()
    }

    pub fn render_plain_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let line_no_width = self.entries.len().max(1).to_string().len().max(2);
        self.entries
            .iter()
            .map(|(line_no, entry)| {
                let mut spans = Vec::new();
                spans.push(Span::styled(
                    format!("{:>width$} ", line_no, width = line_no_width),
                    Style::default().fg(Color::LightYellow),
                ));
                spans.push(Span::styled("│ ", Style::default().fg(Color::LightBlue)));

                if let Some(level) = entry.level {
                    let (text, color) = match level {
                        LogLevel::Debug => ("DBG", Color::Gray),
                        LogLevel::Info => ("INF", Color::Green),
                        LogLevel::Warn => ("WRN", Color::Yellow),
                        LogLevel::Error => ("ERR", Color::Red),
                        LogLevel::Fatal => ("FTL", Color::LightRed),
                    };
                    spans.push(Span::styled(format!("[{}] ", text), Style::default().fg(color).bold()));
                }

                spans.push(Span::styled(entry.message.clone(), Style::default().fg(Color::White)));

                Line::from(spans)
            })
            .collect()
    }

    fn search_next(&mut self, query: &str, forward: bool) {
        let lower = query.to_lowercase();
        let visible = self.visible_entries();
        let total = visible.len().max(1);
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
            if let Some(&entry_idx) = visible.get(idx) {
                if self.entries[entry_idx].1.raw.to_lowercase().contains(&lower) {
                    self.selection = idx;
                    break;
                }
            }
        }
        self.last_match = Some(query.to_string());
    }
}

fn parse_log(content: &str) -> Vec<(usize, LogEntry)> {
    let mut entries = Vec::new();

    // Common patterns
    let timestamp_re = Regex::new(r"^\[?(\d{4}[-/]\d{2}[-/]\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})?)\]?").ok();
    let level_re = Regex::new(r"(?i)\b(DEBUG|DBG|INFO|INF|WARN(?:ING)?|WRN|ERROR|ERR|FATAL|FTL|CRITICAL|CRIT)\b").ok();

    for (idx, line) in content.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = line.trim();

        if trimmed.is_empty() {
            continue;
        }

        let mut timestamp = None;
        let mut level = None;
        let mut remaining = trimmed.to_string();

        // Extract timestamp
        if let Some(ref re) = timestamp_re {
            if let Some(caps) = re.captures(&remaining) {
                timestamp = Some(caps[1].to_string());
                remaining = remaining[caps[0].len()..].trim().to_string();
            }
        }

        // Extract level
        if let Some(ref re) = level_re {
            if let Some(caps) = re.captures(&remaining) {
                level = Some(match caps[1].to_uppercase().as_str() {
                    "DEBUG" | "DBG" => LogLevel::Debug,
                    "INFO" | "INF" => LogLevel::Info,
                    "WARN" | "WARNING" | "WRN" => LogLevel::Warn,
                    "ERROR" | "ERR" => LogLevel::Error,
                    "FATAL" | "FTL" | "CRITICAL" | "CRIT" => LogLevel::Fatal,
                    _ => LogLevel::Info,
                });
            }
        }

        entries.push((line_no, LogEntry {
            timestamp,
            level,
            source: None,
            message: remaining,
            raw: trimmed.to_string(),
        }));
    }

    entries
}

fn level_priority(level: LogLevel) -> u8 {
    match level {
        LogLevel::Debug => 0,
        LogLevel::Info => 1,
        LogLevel::Warn => 2,
        LogLevel::Error => 3,
        LogLevel::Fatal => 4,
    }
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
