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
    /// If true, input is a `:N` go-to-line prompt
    is_goto: bool,
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
    /// All files passed on the command line: (display, source). Switch with ]/[.
    files: Vec<(String, PathBuf)>,
    /// Index into `files` of the currently viewed file.
    current_file: usize,
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
    /// Accumulated numeric count prefix (e.g. `10j`, `25G`).
    pending_count: Option<usize>,
    /// Match count for the current search query, recomputed per keystroke (not
    /// per frame). `None` when the engine can't count or no query is active.
    search_match_count: Option<usize>,
    /// Whether to emit ANSI color in non-interactive (plain) output.
    use_color: bool,
    /// Optional 1-based line range for plain output (start, end inclusive).
    line_range: Option<(usize, usize)>,
    /// One clipboard handle for the process lifetime. Kept alive so copied text
    /// remains available to paste while vat is running (on X11 the selection is
    /// served by the owning process). `None` if the platform clipboard is
    /// unavailable.
    clipboard: Option<Clipboard>,
    /// Terminal graphics picker, detected once before entering the TUI. Shared
    /// with image engines (including files switched to) for inline rendering.
    graphics_picker: Option<ratatui_image::picker::Picker>,
    /// Set by `e`; the run loop then suspends the TUI and opens `$EDITOR`.
    pending_edit: bool,
}

impl App {
    pub fn new(
        engine: EngineState,
        files: Vec<(String, PathBuf)>,
        current_file: usize,
        paging: Paging,
        force_raw: bool,
        use_color: bool,
        line_range: Option<(usize, usize)>,
    ) -> Self {
        let (file_path, source_path) = files
            .get(current_file)
            .cloned()
            .unwrap_or_else(|| (String::new(), PathBuf::new()));
        Self {
            engine,
            should_quit: false,
            input: InputState {
                active: false,
                buffer: String::new(),
                is_filter: false,
                is_goto: false,
            },
            status: None,
            file_path,
            source_path,
            files,
            current_file,
            paging,
            force_raw,
            filter: None,
            show_help: false,
            visual_start: None,
            pending_y: false,
            wrap: false,
            clipboard: Clipboard::new().ok(),
            pending_count: None,
            search_match_count: None,
            use_color,
            line_range,
            graphics_picker: None,
            pending_edit: false,
        }
    }

    /// Switch to the next/previous file (`]` / `[`), re-analyzing it. Keeps the
    /// current file on failure and reports the error.
    fn switch_file(&mut self, forward: bool) {
        let n = self.files.len();
        if n <= 1 {
            return;
        }
        let next = if forward {
            (self.current_file + 1) % n
        } else {
            (self.current_file + n - 1) % n
        };
        let (display, source) = self.files[next].clone();
        match crate::analyzer::analyze(&source) {
            Ok(mut engine) => {
                if let Some(picker) = self.graphics_picker.as_ref() {
                    engine.set_graphics(picker);
                }
                self.engine = engine;
                self.current_file = next;
                self.file_path = display;
                self.source_path = source;
                self.input.active = false;
                self.visual_start = None;
                self.filter = None;
                self.pending_count = None;
                self.status = Some(format!("[{}/{}] {}", next + 1, n, self.file_path));
            }
            Err(e) => self.status = Some(format!("Cannot open {}: {}", display, e)),
        }
    }

    /// Apply the search query incrementally as the user types (search mode only,
    /// not filter/goto). Empty query is a no-op.
    fn live_search(&mut self) {
        if self.input.is_filter || self.input.is_goto {
            return;
        }
        if self.input.buffer.is_empty() {
            self.search_match_count = None;
            return;
        }
        self.engine.apply_search(&self.input.buffer);
        self.search_match_count = self.engine.match_count(&self.input.buffer);
    }

    /// Jump to a 1-based line number by going to the top and stepping down.
    /// This works for every engine without a per-engine goto method.
    fn goto_line(&mut self, line: usize) {
        let g = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE);
        self.engine.handle_key(g);
        self.engine.handle_key(g);
        let down = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
        for _ in 1..line.max(1) {
            self.engine.handle_key(down);
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
        // Images use the TUI so they can render inline graphics, even though
        // their metadata would otherwise fit on one screen.
        if self.engine.prefers_tui() {
            return self.run_tui();
        }
        // Cheap lower-bound pre-check: content_height counts unwrapped lines, so
        // if it already exceeds the screen the file cannot fit — page without
        // materializing (and syntax-highlighting / JSON-parsing) the whole file.
        // Reserve a few rows for the header and box borders.
        const CHROME_ROWS: usize = 4;
        if self.engine.content_height().saturating_add(CHROME_ROWS) > rows as usize {
            return self.run_tui();
        }
        let inner_width = cols.saturating_sub(2) as usize;
        let all_lines = self.build_plain_lines(inner_width);
        let boxed = box_lines(all_lines, inner_width);
        if boxed.len() <= rows as usize {
            write_plain(boxed, self.use_color)?;
            return Ok(());
        }
        self.run_tui()
    }

    /// Output raw file content without any formatting (for piping)
    /// Uses streaming to handle arbitrarily large files efficiently
    fn run_raw(&self) -> Result<()> {
        let mut stdout = io::stdout().lock();

        // With a line range, stream only the requested 1-based lines.
        if let Some((start, end)) = self.line_range {
            use std::io::BufRead;
            let file = fs::File::open(&self.source_path)?;
            let reader = io::BufReader::new(file);
            for (i, line) in reader.lines().enumerate() {
                let n = i + 1;
                if n < start {
                    continue;
                }
                if n > end {
                    break;
                }
                let line = line?;
                if let Err(e) = writeln!(stdout, "{}", line) {
                    if e.kind() == io::ErrorKind::BrokenPipe {
                        break;
                    }
                    return Err(e.into());
                }
            }
            let _ = stdout.flush();
            return Ok(());
        }

        let mut file = fs::File::open(&self.source_path)?;
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
        write_plain(boxed, self.use_color)?;
        Ok(())
    }

    fn build_plain_lines(&mut self, inner_width: usize) -> Vec<Line<'static>> {
        let mut header_lines = self.plain_header_lines(inner_width);
        let mut content_lines = self.engine.render_plain_lines(inner_width as u16);
        // Apply an optional 1-based line range to the rendered content.
        if let Some((start, end)) = self.line_range {
            let lo = start.saturating_sub(1).min(content_lines.len());
            let hi = end.min(content_lines.len());
            content_lines = if lo < hi {
                content_lines[lo..hi].to_vec()
            } else {
                Vec::new()
            };
        }
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
        // Detect the terminal graphics protocol BEFORE raw mode / the alternate
        // screen — the query protocol must run on the normal screen. Only for
        // image files: the query round-trips with the terminal (and blocks up to
        // ~2s if it doesn't answer), so we skip it for everything else. Then hand
        // it to the current engine (an image decodes itself for inline render).
        let has_image = self.engine.prefers_tui()
            || self.files.iter().any(|(_, p)| {
                matches!(
                    p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(),
                    Some("png" | "jpg" | "jpeg" | "gif" | "webp")
                )
            });
        if has_image {
            let mut picker = ratatui_image::picker::Picker::from_query_stdio().ok();
            if let (Some(p), Some(proto)) = (picker.as_mut(), forced_image_protocol()) {
                // Auto-detection mis-picks sixel on some kitty-protocol terminals
                // (e.g. Ghostty); honor an explicit override / known terminal.
                p.set_protocol_type(proto);
            }
            self.graphics_picker = picker;
            if let Some(picker) = self.graphics_picker.as_ref() {
                self.engine.set_graphics(picker);
            }
        }

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
            if self.pending_edit {
                self.pending_edit = false;
                self.edit_in_editor(terminal)?;
            }
            if self.should_quit {
                break;
            }
        }
        Ok(())
    }

    /// Suspend the TUI, open the current file in `$VISUAL`/`$EDITOR`, then resume
    /// and reload the file so edits are reflected. No-op for non-file sources.
    fn edit_in_editor(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<()> {
        let editor = std::env::var("VISUAL")
            .ok()
            .filter(|e| !e.trim().is_empty())
            .or_else(|| std::env::var("EDITOR").ok().filter(|e| !e.trim().is_empty()));
        let editor = match editor {
            Some(e) => e,
            None => {
                self.status = Some("Set $EDITOR (or $VISUAL) to edit".to_string());
                return Ok(());
            }
        };
        if !self.source_path.is_file() {
            self.status = Some("Not an editable file".to_string());
            return Ok(());
        }

        // Suspend the TUI, run the editor, then resume and reload.
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;

        self.run_editor(&editor);

        enable_raw_mode()?;
        execute!(terminal.backend_mut(), EnterAlternateScreen)?;
        terminal.clear()?;
        Ok(())
    }

    /// Launch `editor` on the current file and reload the engine. Terminal-free
    /// (the caller suspends/resumes the TUI around it) so it is unit-testable.
    fn run_editor(&mut self, editor: &str) {
        // `$EDITOR` may include args (e.g. "code --wait"); the file goes last.
        let mut parts = editor.split_whitespace();
        let program = parts.next().unwrap_or("vi");
        let result = std::process::Command::new(program)
            .args(parts)
            .arg(&self.source_path)
            .status();

        match result {
            Ok(_) => {
                // Reload so edits show; keep the graphics picker for images.
                match crate::analyzer::analyze(&self.source_path) {
                    Ok(mut engine) => {
                        if let Some(picker) = self.graphics_picker.as_ref() {
                            engine.set_graphics(picker);
                        }
                        self.engine = engine;
                        self.status = Some("Reloaded after edit".to_string());
                    }
                    Err(e) => self.status = Some(format!("Reload failed: {}", e)),
                }
            }
            Err(e) => self.status = Some(format!("Could not launch editor: {}", e)),
        }
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
                    if self.input.is_goto {
                        if let Ok(n) = query.parse::<usize>() {
                            self.goto_line(n);
                        }
                    } else if !query.is_empty() {
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
                    self.live_search();
                }
                KeyCode::Char(c) => {
                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                        if c == 'c' {
                            self.input.active = false;
                            self.input.buffer.clear();
                        } else if c == 'u' {
                            self.input.buffer.clear();
                            self.live_search();
                        }
                        return;
                    }
                    self.input.buffer.push(c);
                    self.live_search();
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

        // Accumulate a numeric count prefix (e.g. `10j`, `25G`). A leading `0`
        // is passed through (no count semantics) rather than starting a count.
        if let KeyCode::Char(c) = key.code {
            if c.is_ascii_digit() && !(self.pending_count.is_none() && c == '0') {
                let d = c.to_digit(10).unwrap() as usize;
                self.pending_count =
                    Some(self.pending_count.unwrap_or(0).saturating_mul(10).saturating_add(d));
                return;
            }
        }
        // This keypress consumes (or discards) any accumulated count.
        let count = self.pending_count.take();

        match key.code {
            KeyCode::Char('q') => {
                self.should_quit = true;
            }
            KeyCode::Char('?') => {
                self.show_help = true;
            }
            KeyCode::Char('j') | KeyCode::Down | KeyCode::Char('k') | KeyCode::Up => {
                for _ in 0..count.unwrap_or(1) {
                    self.engine.handle_key(key);
                }
            }
            KeyCode::Char('G') => match count {
                Some(n) => self.goto_line(n), // NG: jump to line N
                None => self.engine.handle_key(key),
            },
            KeyCode::Char(':') => {
                // Open a `:N` go-to-line prompt.
                self.input.active = true;
                self.input.is_filter = false;
                self.input.is_goto = true;
                self.input.buffer.clear();
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
                    self.input.is_goto = false;
                    self.input.buffer.clear();
                }
            }
            KeyCode::Char('f') => {
                if self.engine.supports_search() {
                    self.input.active = true;
                    self.input.is_filter = true;
                    self.input.is_goto = false;
                    self.input.buffer.clear();
                }
            }
            KeyCode::Char('w') => {
                self.wrap = !self.wrap;
                self.status = Some(if self.wrap { "Wrap ON".to_string() } else { "Wrap OFF".to_string() });
            }
            KeyCode::Char(']') => self.switch_file(true),
            KeyCode::Char('[') => self.switch_file(false),
            KeyCode::Char('e') => self.pending_edit = true,
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
        let area = outer.inner(frame.area());
        frame.render_widget(outer, frame.area());

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
            let (icon, label) = if self.input.is_goto {
                (":", "Goto line")
            } else if self.input.is_filter {
                ("◉", "Filter")
            } else {
                ("⌕", "Search")
            };
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
                Span::styled(
                    if !self.input.is_goto && !self.input.is_filter {
                        match self.search_match_count {
                            Some(n) => format!("  {} match{}", n, if n == 1 { "" } else { "es" }),
                            None => String::new(),
                        }
                    } else {
                        String::new()
                    },
                    Style::default().fg(ratatui::style::Color::DarkGray),
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
            Line::from("  :N           Go to line N"),
            Line::from("  Nj/Nk/NG     Count prefix (e.g. 10j, 25G)"),
            Line::from("  Ctrl+u/d     Half-page up/down"),
            Line::from("  ] / [        Next / previous file"),
            Line::from(""),
            Line::from(vec![
                Span::styled("Search & Filter", Style::default().bold()),
            ]),
            Line::from("  /            Search"),
            Line::from("  f            Filter to matches (text/JSONL); jump to match elsewhere"),
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
            Line::from("  e            Edit current file in $EDITOR"),
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

        let area = frame.area();
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

/// An explicit image-protocol override, if the environment calls for one.
/// `VAT_IMAGE_PROTOCOL=kitty|iterm2|sixel|halfblocks` forces a protocol; failing
/// that, Ghostty is forced to kitty (auto-detection tends to mis-pick sixel,
/// which Ghostty does not render).
fn forced_image_protocol() -> Option<ratatui_image::picker::ProtocolType> {
    use ratatui_image::picker::ProtocolType;
    if let Ok(val) = std::env::var("VAT_IMAGE_PROTOCOL") {
        return match val.to_lowercase().as_str() {
            "kitty" => Some(ProtocolType::Kitty),
            "iterm2" => Some(ProtocolType::Iterm2),
            "sixel" => Some(ProtocolType::Sixel),
            "halfblocks" => Some(ProtocolType::Halfblocks),
            _ => None,
        };
    }
    // Terminal multiplexers usually strip graphics protocols (kitty/sixel/iTerm2)
    // but pass plain colored text, so half-blocks is the only thing that renders.
    // TODO(zellij): graphics passthrough was merged into zellij (issue #2814,
    // ~2026-08) but isn't released yet. Once it ships, stop forcing half-blocks
    // on $ZELLIJ and trust the capability query instead (fall back to half-blocks
    // only when no graphics protocol is detected). Workaround: VAT_IMAGE_PROTOCOL.
    let in_multiplexer = std::env::var_os("ZELLIJ").is_some()
        || std::env::var_os("TMUX").is_some()
        || std::env::var_os("STY").is_some();
    if in_multiplexer {
        return Some(ProtocolType::Halfblocks);
    }
    let is_ghostty = std::env::var("TERM_PROGRAM").as_deref() == Ok("ghostty")
        || std::env::var("TERM").as_deref() == Ok("xterm-ghostty")
        || std::env::var_os("GHOSTTY_RESOURCES_DIR").is_some();
    if is_ghostty {
        Some(ProtocolType::Kitty)
    } else {
        None
    }
}

fn write_plain(lines: Vec<Line<'static>>, color: bool) -> Result<()> {
    let mut stdout = io::stdout();
    for line in lines {
        for span in line.spans {
            if color {
                apply_style(&mut stdout, span.style)?;
                write!(stdout, "{}", span.content)?;
                reset_style(&mut stdout)?;
            } else {
                write!(stdout, "{}", span.content)?;
            }
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
        "pem" | "crt" | "cer" | "der" => "Certificate",
        "ipynb" => "Notebook",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn app_with_lines(n: usize) -> App {
        use std::io::Write;
        let mut f = tempfile::Builder::new().suffix(".txt").tempfile().unwrap();
        for i in 1..=n {
            writeln!(f, "line {}", i).unwrap();
        }
        f.flush().unwrap();
        let path = f.path().to_path_buf();
        let engine = crate::analyzer::analyze(&path).unwrap();
        std::mem::forget(f); // keep the mmap-backed file on disk for the test
        App::new(
            engine,
            vec![("test".into(), path)],
            0,
            Paging::Never,
            false,
            false,
            None,
        )
    }

    #[test]
    fn count_prefix_and_goto() {
        let mut app = app_with_lines(100);
        // 50G -> line 50 (0-based selection 49)
        for c in ['5', '0', 'G'] {
            app.handle_key(key(c));
        }
        assert_eq!(app.engine.selection(), 49, "50G should select line 50");

        // gg then 10j -> line 11 (selection 10)
        app.handle_key(key('g'));
        app.handle_key(key('g'));
        for c in ['1', '0', 'j'] {
            app.handle_key(key(c));
        }
        assert_eq!(app.engine.selection(), 10, "gg then 10j should select line 11");
    }

    #[test]
    fn incremental_search_and_match_count() {
        let mut app = app_with_lines(100); // lines "line 1".."line 100"
        app.handle_key(key('/'));
        assert!(app.input.active && !app.input.is_filter && !app.input.is_goto);
        for c in "line 100".chars() {
            app.handle_key(key(c));
        }
        // Only "line 100" contains "line 100".
        assert_eq!(app.search_match_count, Some(1));
        // Incremental: selection jumped to the match (line 100 -> index 99).
        assert_eq!(app.engine.selection(), 99);
    }

    #[test]
    fn run_editor_launches_and_reloads() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        // A temp editor script that appends a line to the file it is given.
        let mut ed = tempfile::Builder::new().suffix(".sh").tempfile().unwrap();
        writeln!(ed, "#!/bin/sh").unwrap();
        writeln!(ed, "printf 'line 3\\n' >> \"$1\"").unwrap();
        ed.flush().unwrap();
        std::fs::set_permissions(ed.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let ed_path = ed.path().to_str().unwrap().to_string();

        let mut app = app_with_lines(2); // file with "line 1", "line 2"
        assert_eq!(app.engine.content_height(), 2);
        app.run_editor(&ed_path);
        // The editor appended a line and the engine was reloaded to reflect it.
        assert_eq!(app.engine.content_height(), 3);
        assert_eq!(app.status.as_deref(), Some("Reloaded after edit"));
    }

    #[test]
    fn e_key_requests_edit() {
        let mut app = app_with_lines(3);
        assert!(!app.pending_edit);
        app.handle_key(key('e'));
        assert!(app.pending_edit, "`e` should request an edit");
    }

    #[test]
    fn goto_line_prompt() {
        let mut app = app_with_lines(100);
        // ':' opens goto prompt; type 30 then Enter -> line 30 (selection 29)
        app.handle_key(key(':'));
        assert!(app.input.active && app.input.is_goto);
        app.handle_key(key('3'));
        app.handle_key(key('0'));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.engine.selection(), 29, ":30 should select line 30");
    }
}
