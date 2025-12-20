use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use arboard::Clipboard;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::style::{
    Attribute, Color as CtColor, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, terminal};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Terminal;

use crate::engines::EngineState;

struct InputState {
    active: bool,
    buffer: String,
}

pub struct App {
    engine: EngineState,
    should_quit: bool,
    input: InputState,
    status: Option<String>,
    file_path: String,
    paging: Paging,
}

impl App {
    pub fn new(engine: EngineState, file_path: String, paging: Paging) -> Self {
        Self {
            engine,
            should_quit: false,
            input: InputState {
                active: false,
                buffer: String::new(),
            },
            status: None,
            file_path,
            paging,
        }
    }

    pub fn run(&mut self) -> Result<()> {
        let (cols, rows) = terminal::size()?;
        match self.paging {
            Paging::Always => return self.run_tui(),
            Paging::Never => return self.run_plain(cols),
            Paging::Auto => {}
        }
        let content_height = self.engine.content_height();
        let inner_width = cols.saturating_sub(2) as usize;
        let header_lines = self.plain_header_lines(inner_width).len();
        let total_lines = content_height + header_lines + 2;
        if total_lines <= rows as usize {
            return self.run_plain(cols);
        }
        self.run_tui()
    }

    fn run_plain(&mut self, cols: u16) -> Result<()> {
        let inner_width = cols.saturating_sub(2) as usize;
        let mut lines = self.plain_header_lines(inner_width);
        lines.extend(self.engine.render_plain_lines(inner_width as u16));
        let boxed = box_lines(lines, inner_width);
        write_plain(boxed)?;
        Ok(())
    }

    fn run_tui(&mut self) -> Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        let res = self.run_loop(&mut terminal);
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;
        res
    }

    fn run_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
        loop {
            terminal.draw(|frame| self.draw(frame))?;
            if event::poll(Duration::from_millis(200))? {
                if let Event::Key(key) = event::read()? {
                    self.handle_key(key);
                }
            }
            if self.should_quit {
                break;
            }
        }
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if self.input.active {
            match key.code {
                KeyCode::Esc => {
                    self.input.active = false;
                    self.input.buffer.clear();
                }
                KeyCode::Enter => {
                    let query = self.input.buffer.trim().to_string();
                    if !query.is_empty() {
                        self.engine.apply_search(&query);
                    }
                    self.input.active = false;
                    self.input.buffer.clear();
                }
                KeyCode::Backspace => {
                    self.input.buffer.pop();
                }
                KeyCode::Char(c) => {
                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                        if c == 'c' {
                            self.input.active = false;
                            self.input.buffer.clear();
                        } else if c == 'u' {
                            self.input.buffer.clear();
                        }
                        return;
                    }
                    self.input.buffer.push(c);
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Char('q') => {
                self.should_quit = true;
            }
            KeyCode::Char('y') => {
                if let Some(path) = self.engine.selected_path() {
                    if let Ok(mut clipboard) = Clipboard::new() {
                        if clipboard.set_text(path.clone()).is_ok() {
                            self.status = Some(format!("Copied path: {}", path));
                        }
                    }
                }
            }
            KeyCode::Char('/') | KeyCode::Char('?') => {
                if self.engine.supports_search() {
                    self.input.active = true;
                    self.input.buffer.clear();
                }
            }
            _ => {
                self.engine.handle_key(key);
            }
        }
    }

    fn draw(&mut self, frame: &mut ratatui::Frame) {
        let outer = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ratatui::style::Color::LightBlue));
        let area = outer.inner(frame.size());
        frame.render_widget(outer, frame.size());

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(area);

        let header = Line::from(format!(
            "{}  {}",
            self.engine.name(),
            self.engine.breadcrumbs()
        ))
        .style(Style::default().bold());
        let header_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ratatui::style::Color::LightBlue));
        frame.render_widget(Paragraph::new(header).block(header_block), chunks[0]);

        self.engine.render(frame, chunks[1]);

        let status_text = if self.input.active {
            format!("Search: {}_", self.input.buffer)
        } else if let Some(status) = self.status.take() {
            status
        } else {
            self.engine.status_line()
        };

        let footer_style = if self.input.active {
            Style::default()
                .fg(ratatui::style::Color::Black)
                .bg(ratatui::style::Color::LightYellow)
                .bold()
        } else {
            Style::default().fg(ratatui::style::Color::DarkGray)
        };
        let footer = Paragraph::new(status_text)
            .block(Block::default().borders(Borders::TOP))
            .style(footer_style);
        frame.render_widget(footer, chunks[2]);
    }

    fn plain_header_lines(&self, inner_width: usize) -> Vec<Line<'static>> {
        let file_name = Path::new(&self.file_path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&self.file_path);
        let ext = Path::new(&self.file_path)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let language = language_label(ext);
        let header_text = format!(" {}  ({}) ", file_name, language);
        let padded = format!("{:width$}", header_text, width = inner_width);
        let header_line = Line::from(Span::styled(
            padded,
            Style::default().bg(ratatui::style::Color::LightBlue).fg(ratatui::style::Color::Black),
        ));
        let rule = "─".repeat(inner_width.max(1));
        let rule_line = Line::from(Span::styled(
            rule,
            Style::default().fg(ratatui::style::Color::LightBlue),
        ));
        vec![header_line, rule_line]
    }
}

fn write_plain(lines: Vec<Line<'static>>) -> Result<()> {
    let mut stdout = io::stdout();
    for line in lines {
        for span in line.spans {
            apply_style(&mut stdout, span.style)?;
            write!(stdout, "{}", span.content)?;
            reset_style(&mut stdout)?;
        }
        writeln!(stdout)?;
    }
    stdout.flush()?;
    Ok(())
}

fn box_lines(lines: Vec<Line<'static>>, inner_width: usize) -> Vec<Line<'static>> {
    let border_style = Style::default().fg(ratatui::style::Color::LightBlue);
    let top = Line::from(Span::styled(
        format!("┌{}┐", "─".repeat(inner_width.max(1))),
        border_style,
    ));
    let bottom = Line::from(Span::styled(
        format!("└{}┘", "─".repeat(inner_width.max(1))),
        border_style,
    ));
    let mut boxed = Vec::new();
    boxed.push(top);
    for line in lines {
        let mut spans = Vec::new();
        spans.push(Span::styled("│", border_style));
        let mut content = fit_line_to_width(line, inner_width);
        spans.append(&mut content);
        spans.push(Span::styled("│", border_style));
        boxed.push(Line::from(spans));
    }
    boxed.push(bottom);
    boxed
}

fn fit_line_to_width(line: Line<'static>, width: usize) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut used = 0usize;
    for span in line.spans {
        if used >= width {
            break;
        }
        let mut text = String::new();
        for ch in span.content.chars() {
            if used + text.len() >= width {
                break;
            }
            text.push(ch);
        }
        let len = text.len();
        if len > 0 {
            spans.push(Span::styled(text, span.style));
            used += len;
        }
    }
    if used < width {
        spans.push(Span::raw(" ".repeat(width - used)));
    }
    spans
}

fn apply_style<W: Write>(out: &mut W, style: Style) -> Result<()> {
    if let Some(fg) = style.fg {
        execute!(out, SetForegroundColor(to_ct_color(fg)))?;
    }
    if let Some(bg) = style.bg {
        execute!(out, SetBackgroundColor(to_ct_color(bg)))?;
    }
    let modifiers = style.add_modifier;
    if modifiers.contains(ratatui::style::Modifier::BOLD) {
        execute!(out, SetAttribute(Attribute::Bold))?;
    }
    if modifiers.contains(ratatui::style::Modifier::ITALIC) {
        execute!(out, SetAttribute(Attribute::Italic))?;
    }
    if modifiers.contains(ratatui::style::Modifier::UNDERLINED) {
        execute!(out, SetAttribute(Attribute::Underlined))?;
    }
    Ok(())
}

fn reset_style<W: Write>(out: &mut W) -> Result<()> {
    execute!(out, SetAttribute(Attribute::Reset), ResetColor)?;
    Ok(())
}

fn to_ct_color(color: ratatui::style::Color) -> CtColor {
    match color {
        ratatui::style::Color::Reset => CtColor::Reset,
        ratatui::style::Color::Black => CtColor::Black,
        ratatui::style::Color::Red => CtColor::DarkRed,
        ratatui::style::Color::Green => CtColor::DarkGreen,
        ratatui::style::Color::Yellow => CtColor::DarkYellow,
        ratatui::style::Color::Blue => CtColor::DarkBlue,
        ratatui::style::Color::Magenta => CtColor::DarkMagenta,
        ratatui::style::Color::Cyan => CtColor::DarkCyan,
        ratatui::style::Color::Gray => CtColor::Grey,
        ratatui::style::Color::DarkGray => CtColor::DarkGrey,
        ratatui::style::Color::LightRed => CtColor::Red,
        ratatui::style::Color::LightGreen => CtColor::Green,
        ratatui::style::Color::LightYellow => CtColor::Yellow,
        ratatui::style::Color::LightBlue => CtColor::Blue,
        ratatui::style::Color::LightMagenta => CtColor::Magenta,
        ratatui::style::Color::LightCyan => CtColor::Cyan,
        ratatui::style::Color::White => CtColor::White,
        ratatui::style::Color::Rgb(r, g, b) => CtColor::Rgb { r, g, b },
        ratatui::style::Color::Indexed(value) => CtColor::AnsiValue(value),
    }
}

fn language_label(ext: &str) -> &'static str {
    match ext.to_lowercase().as_str() {
        "rs" => "Rust",
        "js" => "JavaScript",
        "jsx" => "JavaScript (React)",
        "ts" => "TypeScript",
        "tsx" => "TypeScript (React)",
        "py" => "Python",
        "css" | "tcss" => "CSS",
        "md" => "Markdown",
        "json" => "JSON",
        "yaml" | "yml" => "YAML",
        "toml" => "TOML",
        "kdl" => "KDL",
        "csv" => "CSV",
        "tsv" => "TSV",
        "parquet" => "Parquet",
        "html" => "HTML",
        _ => "Text",
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Paging {
    Auto,
    Always,
    Never,
}
