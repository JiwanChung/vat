use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
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
    /// If true, input is for filter mode instead of search
    is_filter: bool,
}

pub struct App {
    engine: EngineState,
    should_quit: bool,
    input: InputState,
    status: Option<String>,
    /// Display path (shown in header)
    file_path: String,
    /// Actual file path for raw mode (may differ from display path for stdin)
    source_path: PathBuf,
    paging: Paging,
    force_raw: bool,
    /// Active filter query (shows only matching lines)
    filter: Option<String>,
    /// Show help overlay
    show_help: bool,
    /// Visual line mode: stores the starting selection index
    visual_start: Option<usize>,
    /// Track if 'y' was pressed (for 'yy' detection)
    pending_y: bool,
    /// Global line-wrap toggle
    wrap: bool,
    /// One clipboard handle for the process lifetime. Kept alive so copied text
    /// remains available to paste while vat is running (on X11 the selection is
    /// served by the owning process). `None` if the platform clipboard is
    /// unavailable.
    clipboard: Option<Clipboard>,
}

impl App {
    pub fn new(
        engine: EngineState,
        file_path: String,
        source_path: PathBuf,
        paging: Paging,
        force_raw: bool,
    ) -> Self {
        Self {
            engine,
            should_quit: false,
            input: InputState {
                active: false,
                buffer: String::new(),
                is_filter: false,
            },
            status: None,
            file_path,
            source_path,
            paging,
            force_raw,
            filter: None,
            show_help: false,
            visual_start: None,
            pending_y: false,
            wrap: false,
            clipboard: Clipboard::new().ok(),
        }
    }

    /// Copy `text` to the clipboard, reporting the outcome in the status line.
    /// A failed copy is surfaced rather than silently looking like success.
    fn copy_to_clipboard(&mut self, text: String, success: String) {
        match self.clipboard.as_mut() {
            Some(clipboard) => match clipboard.set_text(text) {
                Ok(()) => self.status = Some(success),
                Err(e) => self.status = Some(format!("Clipboard error: {}", e)),
            },
            None => self.status = Some("Clipboard unavailable".to_string()),
        }
    }

    pub fn run(&mut self) -> Result<()> {
        // When stdout is piped (not a TTY) or --plain flag is set, output raw content
        if self.force_raw || !io::stdout().is_terminal() {
            return self.run_raw();
        }

        let (cols, rows) = terminal::size()?;
        match self.paging {
            Paging::Always => return self.run_tui(),
            Paging::Never => return self.run_plain(cols),
            Paging::Auto => {}
        }
        let inner_width = cols.saturating_sub(2) as usize;
        let all_lines = self.build_plain_lines(inner_width);
        let boxed = box_lines(all_lines, inner_width);
        if boxed.len() <= rows as usize {
            write_plain(boxed)?;
            return Ok(());
        }
        self.run_tui()
    }

    /// Output raw file content without any formatting (for piping)
    /// Uses streaming to handle arbitrarily large files efficiently
    fn run_raw(&self) -> Result<()> {
        let mut file = fs::File::open(&self.source_path)?;
        let mut stdout = io::stdout().lock();
        // Ignore broken pipe errors (e.g., when piping to head/tail)
        if let Err(e) = io::copy(&mut file, &mut stdout) {
            if e.kind() != io::ErrorKind::BrokenPipe {
                return Err(e.into());
            }
        }
        let _ = stdout.flush();
        Ok(())
    }

    fn run_plain(&mut self, cols: u16) -> Result<()> {
        let inner_width = cols.saturating_sub(2) as usize;
        let lines = self.build_plain_lines(inner_width);
        let boxed = box_lines(lines, inner_width);
        write_plain(boxed)?;
        Ok(())
    }

    fn build_plain_lines(&mut self, inner_width: usize) -> Vec<Line<'static>> {
        let mut header_lines = self.plain_header_lines(inner_width);
        let content_lines = self.engine.render_plain_lines(inner_width as u16);
        // Connect rule line to content gutter if present
        if let Some(first) = content_lines.first() {
            let (gutter_width, _) = detect_gutter(&first.spans);
            if gutter_width > 0 {
                // The │ is at column (gutter_width - 2) within the content
                // (gutter = num_str + "│ ", so │ is at gutter_width - 2)
                let pipe_col = gutter_width.saturating_sub(2);
                if let Some(rule_line) = header_lines.last_mut() {
                    let rule_style = Style::default().fg(ratatui::style::Color::LightBlue);
                    let before = "─".repeat(pipe_col);
                    let after = "─".repeat(inner_width.saturating_sub(pipe_col + 1));
                    *rule_line = Line::from(vec![
                        Span::styled(before, rule_style),
                        Span::styled("┼", rule_style),
                        Span::styled(after, rule_style),
                    ]);
                }
            }
        }
        header_lines.extend(content_lines);
        header_lines
    }

    fn run_tui(&mut self) -> Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;

        // Restore the terminal on panic before the default hook prints, so a
        // crash inside an engine never leaves the shell in raw mode on the
        // alternate screen (which would require `reset` to recover).
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
            default_hook(info);
        }));

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
        // Handle help overlay first
        if self.show_help {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')) {
                self.show_help = false;
            }
            return;
        }

        if self.input.active {
            match key.code {
                KeyCode::Esc => {
                    self.input.active = false;
                    self.input.buffer.clear();
                }
                KeyCode::Enter => {
                    let query = self.input.buffer.trim().to_string();
                    if !query.is_empty() {
                        if self.input.is_filter {
                            self.filter = Some(query.clone());
                            self.engine.apply_filter(&query);
                        } else {
                            self.engine.apply_search(&query);
                        }
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

        // Handle visual mode
        if self.visual_start.is_some() {
            match key.code {
                KeyCode::Esc => {
                    self.visual_start = None;
                    self.status = Some("Visual mode cancelled".to_string());
                }
                KeyCode::Char('y') => {
                    // Copy selection in visual mode
                    if let Some(start) = self.visual_start {
                        let end = self.engine.selection();
                        if let Some(content) = self.engine.get_lines_range(start, end) {
                            let line_count = if start <= end { end - start + 1 } else { start - end + 1 };
                            self.copy_to_clipboard(content, format!("Yanked {} line(s)", line_count));
                        }
                        self.visual_start = None;
                    }
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    self.engine.handle_key(key);
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.engine.handle_key(key);
                }
                KeyCode::Char('G') => {
                    self.engine.handle_key(key);
                }
                KeyCode::Char('g') => {
                    self.engine.handle_key(key);
                }
                KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.engine.handle_key(key);
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.engine.handle_key(key);
                }
                _ => {}
            }
            return;
        }

        // Reset pending_y for non-y keys
        if key.code != KeyCode::Char('y') {
            self.pending_y = false;
        }

        match key.code {
            KeyCode::Char('q') => {
                self.should_quit = true;
            }
            KeyCode::Char('?') => {
                self.show_help = true;
            }
            KeyCode::Char('y') => {
                if self.pending_y {
                    // yy: copy current line
                    if let Some(line) = self.engine.get_selected_line() {
                        self.copy_to_clipboard(line, "Yanked 1 line".to_string());
                    }
                    self.pending_y = false;
                } else {
                    // First 'y' press - wait for second 'y' or copy path for tree
                    self.pending_y = true;
                }
            }
            KeyCode::Char('v') => {
                // Enter visual line mode
                self.visual_start = Some(self.engine.selection());
                self.status = Some("-- VISUAL LINE --".to_string());
            }
            KeyCode::Char('/') => {
                if self.engine.supports_search() {
                    self.input.active = true;
                    self.input.is_filter = false;
                    self.input.buffer.clear();
                }
            }
            KeyCode::Char('f') => {
                if self.engine.supports_search() {
                    self.input.active = true;
                    self.input.is_filter = true;
                    self.input.buffer.clear();
                }
            }
            KeyCode::Char('w') => {
                self.wrap = !self.wrap;
                self.status = Some(if self.wrap { "Wrap ON".to_string() } else { "Wrap OFF".to_string() });
            }
            KeyCode::Char('F') => {
                // Clear filter
                self.filter = None;
                self.engine.clear_filter();
                self.status = Some("Filter cleared".to_string());
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

        let footer_height = if self.input.active { 2 } else { 1 };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(1),
                Constraint::Length(footer_height),
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

        // Set visual range for highlighting
        if let Some(start) = self.visual_start {
            let end = self.engine.selection();
            self.engine.set_visual_range(Some((start, end)));
        } else {
            self.engine.set_visual_range(None);
        }

        self.engine.render(frame, chunks[1], self.wrap);

        if self.input.active {
            // Render search/filter input box
            let (icon, label) = if self.input.is_filter { ("◉", "Filter") } else { ("⌕", "Search") };
            let input_line = Line::from(vec![
                Span::styled(
                    format!(" {} {} ", icon, label),
                    Style::default()
                        .fg(ratatui::style::Color::Black)
                        .bg(ratatui::style::Color::LightCyan)
                        .bold(),
                ),
                Span::styled(" ", Style::default()),
                Span::styled(
                    format!("{}", self.input.buffer),
                    Style::default()
                        .fg(ratatui::style::Color::White)
                        .bold(),
                ),
                Span::styled(
                    "▌",
                    Style::default()
                        .fg(ratatui::style::Color::LightCyan),
                ),
            ]);
            let hint = Line::from(vec![
                Span::styled(
                    " Enter",
                    Style::default().fg(ratatui::style::Color::DarkGray),
                ),
                Span::styled(" confirm  ", Style::default().fg(ratatui::style::Color::Gray)),
                Span::styled(
                    "Esc",
                    Style::default().fg(ratatui::style::Color::DarkGray),
                ),
                Span::styled(" cancel", Style::default().fg(ratatui::style::Color::Gray)),
            ]);
            let footer = Paragraph::new(vec![input_line, hint])
                .block(Block::default().borders(Borders::TOP).border_style(Style::default().fg(ratatui::style::Color::LightCyan)));
            frame.render_widget(footer, chunks[2]);
        } else if self.visual_start.is_some() {
            // Render visual mode indicator with styled banner
            let start = self.visual_start.unwrap();
            let end = self.engine.selection();
            let line_count = if start <= end { end - start + 1 } else { start - end + 1 };
            let range_text = if line_count == 1 {
                "1 line".to_string()
            } else {
                format!("{} lines", line_count)
            };
            let visual_line = Line::from(vec![
                Span::styled(
                    " ▌ VISUAL ",
                    Style::default()
                        .fg(ratatui::style::Color::Black)
                        .bg(ratatui::style::Color::LightMagenta)
                        .bold(),
                ),
                Span::styled(" ", Style::default()),
                Span::styled(
                    range_text,
                    Style::default()
                        .fg(ratatui::style::Color::LightMagenta)
                        .bold(),
                ),
                Span::styled("  ", Style::default()),
                Span::styled(
                    "y",
                    Style::default().fg(ratatui::style::Color::White).bold(),
                ),
                Span::styled(" yank  ", Style::default().fg(ratatui::style::Color::Gray)),
                Span::styled(
                    "j/k",
                    Style::default().fg(ratatui::style::Color::White).bold(),
                ),
                Span::styled(" extend  ", Style::default().fg(ratatui::style::Color::Gray)),
                Span::styled(
                    "Esc",
                    Style::default().fg(ratatui::style::Color::White).bold(),
                ),
                Span::styled(" cancel", Style::default().fg(ratatui::style::Color::Gray)),
            ]);
            let footer = Paragraph::new(visual_line)
                .block(Block::default().borders(Borders::TOP).border_style(Style::default().fg(ratatui::style::Color::LightMagenta)));
            frame.render_widget(footer, chunks[2]);
        } else {
            let status_text = if let Some(status) = self.status.take() {
                status
            } else {
                self.engine.status_line()
            };
            let footer = Paragraph::new(status_text)
                .block(Block::default().borders(Borders::TOP))
                .style(Style::default().fg(ratatui::style::Color::DarkGray));
            frame.render_widget(footer, chunks[2]);
        }

        // Help overlay
        if self.show_help {
            self.render_help_overlay(frame);
        }
    }

    fn render_help_overlay(&self, frame: &mut ratatui::Frame) {
        use ratatui::widgets::Clear;

        let help_text = vec![
            Line::from(Span::styled("Keyboard Shortcuts", Style::default().bold().fg(ratatui::style::Color::LightCyan))),
            Line::from(""),
            Line::from(vec![
                Span::styled("Navigation", Style::default().bold()),
            ]),
            Line::from("  j/k, ↑/↓     Move up/down"),
            Line::from("  gg           Jump to top"),
            Line::from("  G            Jump to bottom"),
            Line::from("  Ctrl+u/d     Half-page up/down"),
            Line::from(""),
            Line::from(vec![
                Span::styled("Search & Filter", Style::default().bold()),
            ]),
            Line::from("  /            Search"),
            Line::from("  f            Filter (show only matches)"),
            Line::from("  F            Clear filter"),
            Line::from("  n/N          Next/previous match"),
            Line::from(""),
            Line::from(vec![
                Span::styled("Actions", Style::default().bold()),
            ]),
            Line::from("  Enter        Expand/collapse (tree/json)"),
            Line::from("  yy           Copy current line"),
            Line::from("  v            Enter visual line mode"),
            Line::from("  s            Toggle sidebar/schema"),
            Line::from("  w            Toggle line wrap"),
            Line::from("  e            Next section/heading"),
            Line::from(""),
            Line::from(vec![
                Span::styled("General", Style::default().bold()),
            ]),
            Line::from("  ?            Show/hide this help"),
            Line::from("  q            Quit"),
            Line::from(""),
            Line::from(Span::styled("Press ? or Esc to close", Style::default().fg(ratatui::style::Color::DarkGray))),
        ];

        let block = Block::default()
            .title(" Help ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ratatui::style::Color::LightCyan))
            .style(Style::default().bg(ratatui::style::Color::Black));

        let area = frame.size();
        let width = 50.min(area.width.saturating_sub(4));
        let height = (help_text.len() as u16 + 2).min(area.height.saturating_sub(4));
        let x = (area.width.saturating_sub(width)) / 2;
        let y = (area.height.saturating_sub(height)) / 2;

        let popup_area = ratatui::layout::Rect::new(x, y, width, height);

        frame.render_widget(Clear, popup_area);
        frame.render_widget(Paragraph::new(help_text).block(block), popup_area);
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
        let rows = wrap_line_to_width(line, inner_width);
        for row in rows {
            let mut spans = Vec::new();
            spans.push(Span::styled("│", border_style));
            spans.extend(row);
            spans.push(Span::styled("│", border_style));
            boxed.push(Line::from(spans));
        }
    }
    boxed.push(bottom);
    boxed
}

fn wrap_line_to_width(line: Line<'static>, width: usize) -> Vec<Vec<Span<'static>>> {
    if width == 0 {
        return vec![vec![Span::raw("")]];
    }

    // Detect gutter (line number + "│" separator) for continuation indent
    let (gutter_width, mut cont_spans) = detect_gutter(&line.spans);
    let mut cont_width = gutter_width;

    // Extend continuation with tree indent (whitespace span right after "│ ")
    if gutter_width > 0 {
        if let Some(gutter_idx) = line.spans.iter().position(|s| *s.content == *"│ ") {
            if let Some(indent_span) = line.spans.get(gutter_idx + 1) {
                if !indent_span.content.is_empty()
                    && indent_span.content.chars().all(|c| c == ' ')
                    && cont_width + crate::engines::display_width(&indent_span.content) < width
                {
                    let indent_width = crate::engines::display_width(&indent_span.content);
                    cont_width += indent_width;
                    cont_spans.push(Span::raw(" ".repeat(indent_width)));
                }
            }
        }
    }

    let mut rows: Vec<Vec<Span<'static>>> = Vec::new();
    let mut current_row: Vec<Span<'static>> = Vec::new();
    let mut col = 0usize;

    for span in line.spans {
        let style = span.style;
        let mut buf = String::new();

        for ch in span.content.chars() {
            // Treat embedded newlines as forced line breaks
            if ch == '\n' {
                if !buf.is_empty() {
                    current_row.push(Span::styled(buf.clone(), style));
                    buf.clear();
                }
                if col < width {
                    current_row.push(Span::raw(" ".repeat(width - col)));
                }
                rows.push(current_row);
                current_row = Vec::new();
                if cont_width > 0 && cont_width < width {
                    current_row.extend(cont_spans.clone());
                    col = cont_width;
                } else {
                    col = 0;
                }
                continue;
            }
            let cw = crate::engines::char_width(ch);
            if col + cw > width && col > 0 {
                // Flush current buffer into the row before starting a new one
                if !buf.is_empty() {
                    current_row.push(Span::styled(buf.clone(), style));
                    buf.clear();
                }
                rows.push(current_row);
                current_row = Vec::new();
                // Indent continuation rows to align past the gutter + indent
                if cont_width > 0 && cont_width < width {
                    current_row.extend(cont_spans.clone());
                    col = cont_width;
                } else {
                    col = 0;
                }
            }
            buf.push(ch);
            col += cw;
        }

        if !buf.is_empty() {
            current_row.push(Span::styled(buf, style));
        }
    }

    // Pad the final row to fill width
    if col < width {
        current_row.push(Span::raw(" ".repeat(width - col)));
    }
    rows.push(current_row);

    rows
}

/// Detect line-number gutter by finding a "│ " separator span in the first few spans.
/// Line-number engines use exactly "│ " (pipe + space); table engines use just "│",
/// so this only matches real line-number gutters.
/// Returns (total_gutter_width, continuation_spans) where continuation_spans
/// is blank padding for the line number + the original "│ " separator with its style.
fn detect_gutter(spans: &[Span<'static>]) -> (usize, Vec<Span<'static>>) {
    let mut pre_width = 0;
    for (i, span) in spans.iter().enumerate() {
        if i > 2 {
            break;
        }
        let span_width = crate::engines::display_width(&span.content);
        if *span.content == *"│ " {
            let total = pre_width + span_width;
            let mut continuation = Vec::new();
            if pre_width > 0 {
                continuation.push(Span::raw(" ".repeat(pre_width)));
            }
            continuation.push(Span::styled(span.content.to_string(), span.style));
            return (total, continuation);
        }
        pre_width += span_width;
    }
    (0, Vec::new())
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
