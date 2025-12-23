use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use nom::bytes::complete::{take_while1, take_while_m_n};
use nom::character::complete::space1;
use nom::sequence::tuple;

pub struct LogicEngine {
    lines: Vec<String>,
    scroll: usize,
    selection: usize,
    file_name: String,
    last_query: Option<String>,
    pending_g: bool,
    last_view_height: usize,
    last_match: Option<String>,
    /// Visual selection range (start, end) for highlighting
    pub visual_range: Option<(usize, usize)>,
}

impl LogicEngine {
    pub fn from_path(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let lines = if file_name == ".tmux.conf" {
            parse_tmux(&raw)
        } else if file_name == ".bashrc" {
            parse_bashrc(&raw)
        } else if file_name == "crontab" {
            parse_crontab(&raw)
        } else {
            parse_ssh_config(path, &raw)
        };
        Ok(Self {
            lines,
            scroll: 0,
            selection: 0,
            file_name,
            last_query: None,
            pending_g: false,
            last_view_height: 0,
            last_match: None,
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
        let line_no_width = self.lines.len().max(1).to_string().len().max(2);
        let visible: Vec<Line> = self
            .lines
            .iter()
            .skip(self.scroll)
            .take(height)
            .enumerate()
            .map(|(idx, line)| {
                let row = self.scroll + idx;
                let selected = row == self.selection;
                let mut spans = Vec::new();
                let line_no = format!("{:>width$} ", row + 1, width = line_no_width);
                let line_no_style = if selected {
                    Style::default().fg(Color::Black).bg(Color::LightBlue).bold()
                } else {
                    Style::default().fg(Color::LightYellow)
                };
                spans.push(Span::styled(line_no, line_no_style));
                spans.push(Span::styled("│ ", Style::default().fg(Color::LightBlue)));
                let content_style = if line.trim_end().ends_with(':') {
                    Style::default().fg(Color::LightCyan).bold()
                } else {
                    Style::default().fg(Color::White)
                };
                let content_style = if selected {
                    content_style.fg(Color::Black).bg(Color::LightBlue)
                } else {
                    content_style
                };
                spans.push(Span::styled(line.clone(), content_style));
                Line::from(spans)
            })
            .collect();
        let block = Block::default().borders(Borders::NONE);
        frame.render_widget(Paragraph::new(visible).block(block), area);
    }

    pub fn content_height(&self) -> usize {
        self.lines.len()
    }

    pub fn render_plain_lines(&self) -> Vec<Line<'static>> {
        let line_no_width = self.lines.len().max(1).to_string().len().max(2);
        self.lines
            .iter()
            .enumerate()
            .map(|(idx, line)| {
                let mut spans = Vec::new();
                let line_no = format!("{:>width$} ", idx + 1, width = line_no_width);
                spans.push(Span::styled(
                    line_no,
                    Style::default().fg(Color::LightYellow),
                ));
                spans.push(Span::styled("│ ", Style::default().fg(Color::LightBlue)));
                let content_style = if line.trim_end().ends_with(':') {
                    Style::default().fg(Color::LightCyan).bold()
                } else {
                    Style::default().fg(Color::White)
                };
                spans.push(Span::styled(line.clone(), content_style));
                Line::from(spans)
            })
            .collect()
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
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if self.selection + 1 < self.lines.len() {
                    self.selection += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selection = self.selection.saturating_sub(1);
            }
            KeyCode::Char('u')
                if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) =>
            {
                let jump = page_jump(self.last_view_height).min(self.selection);
                self.selection = self.selection.saturating_sub(jump);
            }
            KeyCode::Char('d')
                if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) =>
            {
                let jump = page_jump(self.last_view_height).min(self.lines.len().saturating_sub(1));
                self.selection = (self.selection + jump).min(self.lines.len().saturating_sub(1));
            }
            KeyCode::Char('G') => {
                if !self.lines.is_empty() {
                    self.selection = self.lines.len() - 1;
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

    pub fn breadcrumbs(&self) -> String {
        self.file_name.clone()
    }

    pub fn status_line(&self) -> String {
        let query = self
            .last_query
            .as_ref()
            .map(|q| format!(" | search: {}", q))
            .unwrap_or_default();
        format!(
            "j/k move | gg/G jump | Ctrl+u/d half-page | n/N next/prev | / search | f filter{}",
            query
        )
    }

    pub fn apply_filter(&mut self, query: &str) {
        self.apply_search(query);
    }

    pub fn clear_filter(&mut self) {
        self.last_query = None;
    }

    #[allow(dead_code)]
    pub fn selected_path(&self) -> Option<String> {
        None
    }

    /// Get the content of the currently selected line
    pub fn get_selected_line(&self) -> Option<String> {
        self.lines.get(self.selection).cloned()
    }

    /// Get lines in a range (inclusive), joined by newlines
    pub fn get_lines_range(&self, start: usize, end: usize) -> Option<String> {
        let (start, end) = if start <= end { (start, end) } else { (end, start) };
        let total = self.lines.len();
        if start >= total {
            return None;
        }
        let end = end.min(total.saturating_sub(1));
        let lines: Vec<String> = self.lines[start..=end].to_vec();
        if lines.is_empty() { None } else { Some(lines.join("\n")) }
    }

    /// Get current selection index (for visual mode)
    pub fn selection(&self) -> usize {
        self.selection
    }
}

fn parse_ssh_config(path: &Path, raw: &str) -> Vec<String> {
    #[derive(Clone)]
    struct HostEntry {
        host: String,
        hostname: String,
        user: String,
        port: String,
        identity: String,
        identity_status: String,
    }

    let mut entries: Vec<HostEntry> = Vec::new();
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    let mut current_hosts: Vec<String> = Vec::new();

    for line in raw.lines() {
        let Some((key, value)) = parse_ssh_line(line) else {
            continue;
        };
        if key.eq_ignore_ascii_case("Host") {
            current_hosts = value.split_whitespace().map(|s| s.to_string()).collect();
            for host in &current_hosts {
                if !index.contains_key(host) {
                    let entry = HostEntry {
                        host: host.clone(),
                        hostname: String::new(),
                        user: String::new(),
                        port: String::new(),
                        identity: String::new(),
                        identity_status: String::new(),
                    };
                    index.insert(host.clone(), entries.len());
                    entries.push(entry);
                }
            }
            continue;
        }

        if current_hosts.is_empty() {
            continue;
        }

        for host in &current_hosts {
            if let Some(entry_idx) = index.get(host).copied() {
                let entry = &mut entries[entry_idx];
                if key.eq_ignore_ascii_case("HostName") {
                    entry.hostname = value.clone();
                } else if key.eq_ignore_ascii_case("User") {
                    entry.user = value.clone();
                } else if key.eq_ignore_ascii_case("Port") {
                    entry.port = value.clone();
                } else if key.eq_ignore_ascii_case("IdentityFile") {
                    let expanded = expand_tilde(path, &value);
                    let exists = expanded.exists();
                    entry.identity = expanded.display().to_string();
                    entry.identity_status = if exists { "ok" } else { "missing" }.to_string();
                }
            }
        }
    }

    let mut lines = Vec::new();
    lines.push("SSH Config Summary".to_string());
    lines.push(format!(
        "{:<24}  {:<24}  {:<8}  {:<12}  {:<8}  {}",
        "Host", "HostName", "Port", "User", "Identity", "IdentityFile"
    ));
    lines.push("-".repeat(100));
    for entry in entries {
        lines.push(format!(
            "{:<24}  {:<24}  {:<8}  {:<12}  {:<8}  {}",
            entry.host,
            entry.hostname,
            entry.port,
            entry.user,
            entry.identity_status,
            entry.identity
        ));
    }
    lines
}

fn page_jump(view_height: usize) -> usize {
    let half = view_height / 2;
    if half == 0 { 1 } else { half }
}

impl LogicEngine {
    fn search_next(&mut self, query: &str, forward: bool) {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return;
        }
        let lower = trimmed.to_lowercase();
        let total = self.lines.len().max(1);
        let start = if forward {
            (self.selection + 1) % total
        } else {
            self.selection.saturating_sub(1)
        };
        for offset in 0..self.lines.len() {
            let idx = if forward {
                (start + offset) % total
            } else {
                (start + total - offset % total) % total
            };
            if self.lines[idx].to_lowercase().contains(&lower) {
                self.selection = idx;
                break;
            }
        }
        self.last_match = Some(trimmed.to_string());
    }
}

fn parse_tmux(raw: &str) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push("Tmux keybindings cheat sheet:".to_string());
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("bind") || trimmed.starts_with("bind-key") {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 3 {
                let key = parts[1];
                let cmd = parts[2..].join(" ");
                lines.push(format!("- {} -> {}", key, cmd));
            }
        }
    }
    lines
}

fn parse_bashrc(raw: &str) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push("Environment exports:".to_string());
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("export ") {
            lines.push(format!("- {}", rest));
        }
    }
    lines
}

fn parse_crontab(raw: &str) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push("Cron schedule (humanized):".to_string());
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('@') {
            lines.push(format!("- {}", trimmed));
            continue;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() < 6 {
            continue;
        }
        let schedule = format!(
            "min {} hour {} dom {} mon {} dow {}",
            humanize_field(parts[0]),
            humanize_field(parts[1]),
            humanize_field(parts[2]),
            humanize_field(parts[3]),
            humanize_field(parts[4])
        );
        let command = parts[5..].join(" ");
        lines.push(format!("- {} -> {}", schedule, command));
    }
    lines
}

fn humanize_field(field: &str) -> String {
    if field == "*" {
        "every".to_string()
    } else if let Some(step) = field.strip_prefix("*/") {
        format!("every {}", step)
    } else {
        format!("at {}", field)
    }
}

fn expand_tilde(path: &Path, value: &str) -> PathBuf {
    if let Some(stripped) = value.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    if value.starts_with("/") {
        return PathBuf::from(value);
    }
    path.parent().unwrap_or(path).join(value)
}

fn parse_ssh_line(line: &str) -> Option<(String, String)> {
    let clean = line.split('#').next().unwrap_or("").trim();
    if clean.is_empty() {
        return None;
    }
    let mut parser = tuple((key_token, space1, value_token));
    let (_, (key, _, value)) = parser(clean).ok()?;
    Some((key.to_string(), value.to_string()))
}

fn key_token(input: &str) -> nom::IResult<&str, &str> {
    take_while1(|c: char| !c.is_whitespace())(input)
}

fn value_token(input: &str) -> nom::IResult<&str, &str> {
    take_while_m_n(1, input.len(), |c: char| c != '\n' && c != '\r')(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ssh_line_basic() {
        let line = "Host github.com";
        let parsed = parse_ssh_line(line).unwrap();
        assert_eq!(parsed.0, "Host");
        assert_eq!(parsed.1, "github.com");
    }
}
