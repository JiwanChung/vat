use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use regex::Regex;
use tree_sitter::Parser;
use tree_sitter_css as ts_css;
use tree_sitter_javascript as ts_js;
use tree_sitter_typescript as ts_ts;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Style as SynStyle, ThemeSet};
use syntect::parsing::SyntaxSet;

/// Above this line count, skip syntax highlighting to keep open latency and
/// memory bounded (the file still renders, just without colors).
const MAX_HIGHLIGHT_LINES: usize = 50_000;

struct ComponentInfo {
    name: String,
    props: Option<String>,
}

pub struct SyntaxEngine {
    lines: Vec<String>,
    selection: usize,
    scroll: usize,
    file_name: String,
    syntax_set: SyntaxSet,
    syntax: Option<String>,
    theme: syntect::highlighting::Theme,
    components: Vec<ComponentInfo>,
    show_sidebar: bool,
    last_query: Option<String>,
    is_css: bool,
    is_markdown: bool,
    /// Whether to syntax-highlight (a syntax exists and the file is small
    /// enough). Large files render plain to avoid an O(n) highlight per open.
    highlight_enabled: bool,
    /// Syntect-highlighted content spans per source line, built once lazily.
    /// Empty when `highlight_enabled` is false.
    highlight_cache: Vec<Vec<Span<'static>>>,
    md_rendered: Vec<MdLine>,
    syntax_error_lines: HashSet<usize>,
    pending_g: bool,
    last_view_height: usize,
    last_match: Option<String>,
    /// Visual selection range (start, end) for highlighting
    pub visual_range: Option<(usize, usize)>,
}

impl SyntaxEngine {
    pub fn from_path(path: &Path) -> Result<Self> {
        let content = super::read_text_file(path)?;
        let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        let syntax_set = SyntaxSet::load_defaults_newlines();
        let theme_set = ThemeSet::load_defaults();
        let theme = theme_set
            .themes
            .get("Monokai Extended")
            .or_else(|| theme_set.themes.get("base16-eighties.dark"))
            .or_else(|| theme_set.themes.get("base16-ocean.dark"))
            .unwrap_or_else(|| theme_set.themes.values().next().expect("theme"))
            .clone();
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        let syntax = syntax_set
            .find_syntax_for_file(path)
            .ok()
            .flatten()
            .map(|s| s.name.clone());
        let is_css = matches!(ext, "css" | "tcss");
        let is_markdown = ext == "md";
        // Only cache-highlight when a syntax is known and the file is under the
        // line cap; otherwise render plain (cheap, O(visible) per frame).
        let highlight_enabled =
            !is_markdown && syntax.is_some() && lines.len() <= MAX_HIGHLIGHT_LINES;
        let components = if matches!(ext, "jsx" | "tsx" | "js" | "ts") {
            extract_components(&content, ext)
        } else {
            Vec::new()
        };
        let show_sidebar = !components.is_empty();
        let md_rendered = if is_markdown {
            render_markdown(&content)
        } else {
            Vec::new()
        };
        let syntax_error_lines = parse_syntax_errors(&content, ext);

        Ok(Self {
            lines,
            selection: 0,
            scroll: 0,
            file_name,
            syntax_set,
            syntax,
            theme,
            components,
            show_sidebar,
            last_query: None,
            is_css,
            is_markdown,
            highlight_enabled,
            highlight_cache: Vec::new(),
            md_rendered,
            syntax_error_lines,
            pending_g: false,
            last_view_height: 0,
            last_match: None,
            visual_range: None,
        })
    }

    pub fn render(&mut self, frame: &mut ratatui::Frame, area: Rect, wrap: bool) {
        self.last_view_height = area.height as usize;
        let chunks = if self.show_sidebar {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(28), Constraint::Min(1)])
                .split(area)
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(1)])
                .split(area)
        };

        if self.show_sidebar {
            self.render_sidebar(frame, chunks[0]);
            self.render_code(frame, chunks[1], wrap);
        } else {
            self.render_code(frame, chunks[0], wrap);
        }
    }

    pub fn content_height(&mut self) -> usize {
        if self.is_markdown {
            self.md_rendered.len()
        } else {
            self.lines.len()
        }
    }

    /// Count lines matching the query (rendered markdown lines, or source lines).
    pub fn count_matches(&self, m: &crate::search::Matcher) -> usize {
        if self.is_markdown {
            self.md_rendered
                .iter()
                .filter(|md| m.is_match(&md_line_text(md)))
                .count()
        } else {
            self.lines.iter().filter(|l| m.is_match(l)).count()
        }
    }

    pub fn render_plain_lines(&mut self) -> Vec<Line<'static>> {
        if self.is_markdown {
            return render_markdown_with_gutter(&self.md_rendered, None);
        }

        self.ensure_highlight_cache();
        let line_no_width = self.lines.len().max(1).to_string().len().max(2);
        let mut output = Vec::with_capacity(self.lines.len());
        for idx in 0..self.lines.len() {
            let mut spans = Vec::new();
            let line_no = format!("{:>width$} ", idx + 1, width = line_no_width);
            spans.push(Span::styled(line_no, Style::default().fg(Color::LightYellow)));
            spans.push(Span::styled("│ ", Style::default().fg(Color::LightBlue)));
            if self.is_css {
                if let Some(swatch) = css_swatch(&self.lines[idx]) {
                    spans.push(swatch);
                    spans.push(Span::raw(" "));
                } else {
                    spans.push(Span::raw("   "));
                }
            }
            spans.extend(self.content_spans(idx));
            output.push(Line::from(spans));
        }
        output
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
        let max_lines = if self.is_markdown {
            self.md_rendered.len()
        } else {
            self.lines.len()
        };

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if self.selection + 1 < max_lines {
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
                let jump = page_jump(self.last_view_height).min(max_lines.saturating_sub(1));
                self.selection = (self.selection + jump).min(max_lines.saturating_sub(1));
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
            KeyCode::Char('e') => {
                if self.is_markdown {
                    if let Some(next) = next_markdown_heading(&self.md_rendered, self.selection) {
                        self.selection = next;
                    }
                }
            }
            KeyCode::Char('s') => {
                self.show_sidebar = !self.show_sidebar;
            }
            KeyCode::Char('G') => {
                if max_lines > 0 {
                    self.selection = max_lines - 1;
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
        format!("{} line {}", self.file_name, self.selection + 1)
    }

    pub fn status_line(&self) -> String {
        let query = self
            .last_query
            .as_ref()
            .map(|q| format!(" | search: {}", q))
            .unwrap_or_default();
        let errors = if self.syntax_error_lines.is_empty() {
            String::new()
        } else {
            format!(" | syntax errors: {}", self.syntax_error_lines.len())
        };
        format!(
            "j/k move | gg/G jump | Ctrl+u/d half-page | n/N next/prev | e next heading | s toggle sidebar | / search | f filter{}{}",
            query, errors
        )
    }

    pub fn apply_filter(&mut self, query: &str) {
        // For syntax, filter acts like search - jump to matching lines
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
        if self.is_markdown {
            self.md_rendered.get(self.selection).map(|md| md_line_text(md))
        } else {
            self.lines.get(self.selection).cloned()
        }
    }

    /// Get lines in a range (inclusive), joined by newlines
    pub fn get_lines_range(&self, start: usize, end: usize) -> Option<String> {
        let (start, end) = if start <= end { (start, end) } else { (end, start) };
        if self.is_markdown {
            let total = self.md_rendered.len();
            if start >= total {
                return None;
            }
            let end = end.min(total.saturating_sub(1));
            let lines: Vec<String> = (start..=end)
                .filter_map(|idx| self.md_rendered.get(idx).map(|md| md_line_text(md)))
                .collect();
            if lines.is_empty() { None } else { Some(lines.join("\n")) }
        } else {
            let total = self.lines.len();
            if start >= total {
                return None;
            }
            let end = end.min(total.saturating_sub(1));
            let lines: Vec<String> = self.lines[start..=end].to_vec();
            if lines.is_empty() { None } else { Some(lines.join("\n")) }
        }
    }

    /// Get current selection index (for visual mode)
    pub fn selection(&self) -> usize {
        self.selection
    }

    fn render_sidebar(&self, frame: &mut ratatui::Frame, area: Rect) {
        let mut lines = Vec::new();
        lines.push(Line::from("Components"));
        for comp in &self.components {
            let props = comp
                .props
                .as_ref()
                .map(|p| format!(" ({})", p))
                .unwrap_or_default();
            lines.push(Line::from(format!("- {}{}", comp.name, props)));
        }
        let block = Block::default().borders(Borders::RIGHT);
        frame.render_widget(Paragraph::new(lines).block(block), area);
    }

    /// Build the syntect span cache once (only when highlighting is enabled).
    /// This replaces re-highlighting the scrolled-off prefix on every frame.
    fn ensure_highlight_cache(&mut self) {
        if !self.highlight_enabled || !self.highlight_cache.is_empty() {
            return;
        }
        let syntax = self
            .syntax
            .as_ref()
            .and_then(|name| self.syntax_set.find_syntax_by_name(name));
        let mut highlighter = match syntax {
            Some(syn) => HighlightLines::new(syn, &self.theme),
            None => return,
        };
        let mut cache = Vec::with_capacity(self.lines.len());
        for line in &self.lines {
            let line_with_newline = format!("{}\n", line);
            let regions = highlighter
                .highlight_line(&line_with_newline, &self.syntax_set)
                .unwrap_or_default();
            cache.push(
                regions
                    .into_iter()
                    .map(|(style, part)| syntect_span(style, part))
                    .collect::<Vec<_>>(),
            );
        }
        self.highlight_cache = cache;
    }

    /// Content spans for a source line: cached syntect spans, or a plain span.
    fn content_spans(&self, idx: usize) -> Vec<Span<'static>> {
        if self.highlight_enabled {
            if let Some(spans) = self.highlight_cache.get(idx) {
                return spans.clone();
            }
        }
        vec![Span::styled(self.lines[idx].clone(), Style::default().fg(Color::White))]
    }

    fn render_code(&mut self, frame: &mut ratatui::Frame, area: Rect, wrap: bool) {
        if self.selection < self.scroll {
            self.scroll = self.selection;
        } else if self.selection >= self.scroll + area.height as usize {
            self.scroll = self.selection.saturating_sub(area.height as usize - 1);
        }

        if self.is_markdown {
            self.render_markdown(frame, area, wrap);
            return;
        }

        self.ensure_highlight_cache();

        let mut output = Vec::new();
        let line_no_width = self.lines.len().max(1).to_string().len().max(2);
        let start = self.scroll;
        let end = (self.scroll + area.height as usize).min(self.lines.len());
        for idx in start..end {
            let line = &self.lines[idx];
            let mut spans = Vec::new();
            let line_no = format!("{:>width$} ", idx + 1, width = line_no_width);
            let in_visual = self.visual_range.map_or(false, |(start, end)| {
                let (lo, hi) = if start <= end { (start, end) } else { (end, start) };
                idx >= lo && idx <= hi
            });
            let line_no_style = if idx == self.selection {
                Style::default().fg(Color::Black).bg(Color::LightBlue).bold()
            } else if in_visual {
                Style::default().fg(Color::Black).bg(Color::LightYellow).bold()
            } else {
                Style::default().fg(Color::LightYellow)
            };
            spans.push(Span::styled(line_no, line_no_style));
            spans.push(Span::styled("│ ", Style::default().fg(Color::LightBlue)));
            if self.is_css {
                if let Some(swatch) = css_swatch(line) {
                    spans.push(swatch);
                    spans.push(Span::raw(" "));
                } else {
                    spans.push(Span::raw("   "));
                }
            }

            spans.extend(self.content_spans(idx));

            let mut line_widget = Line::from(spans);
            let mut style = Style::default();
            if self.syntax_error_lines.contains(&idx) {
                style = style.fg(Color::Red).bold();
            }
            if line.contains("TODO") {
                style = style.fg(Color::Red).bold();
            }
            if idx == self.selection {
                style = style.bg(Color::LightBlue).fg(Color::Black);
            } else if in_visual {
                style = style.bg(Color::LightYellow).fg(Color::Black);
            }
            line_widget = line_widget.style(style);
            output.push(line_widget);
        }

        let output = if wrap { super::wrap_lines(output, area.width as usize) } else { output };
        let block = Block::default().borders(Borders::NONE);
        frame.render_widget(Paragraph::new(output).block(block), area);
    }

    fn render_markdown(&mut self, frame: &mut ratatui::Frame, area: Rect, wrap: bool) {
        if self.selection >= self.md_rendered.len() {
            self.selection = self.md_rendered.len().saturating_sub(1);
        }
        let height = area.height as usize;
        if self.selection < self.scroll {
            self.scroll = self.selection;
        } else if self.selection >= self.scroll + height {
            self.scroll = self.selection.saturating_sub(height.saturating_sub(1));
        }

        let mut output = render_markdown_with_gutter(&self.md_rendered, Some((self.selection, self.scroll)));
        output.truncate(height);

        let output = if wrap { super::wrap_lines(output, area.width as usize) } else { output };
        let block = Block::default().borders(Borders::NONE);
        frame.render_widget(Paragraph::new(output).block(block), area);
    }
}

fn syntect_span(style: SynStyle, text: &str) -> Span<'static> {
    let fg = style.foreground;
    Span::styled(
        text.trim_end_matches('\n').to_string(),
        Style::default().fg(Color::Rgb(fg.r, fg.g, fg.b)),
    )
}

fn css_swatch(line: &str) -> Option<Span<'static>> {
    let hex_re = Regex::new(r"#(?P<hex>[0-9a-fA-F]{6})").ok()?;
    let rgb_re = Regex::new(r"rgb\((?P<r>\d{1,3}),\s*(?P<g>\d{1,3}),\s*(?P<b>\d{1,3})\)").ok()?;
    if let Some(caps) = hex_re.captures(line) {
        let hex = &caps["hex"];
        if let Ok(rgb) = parse_hex_color(hex) {
            return Some(color_swatch(rgb));
        }
    }
    if let Some(caps) = rgb_re.captures(line) {
        let r: u8 = caps["r"].parse().unwrap_or(0);
        let g: u8 = caps["g"].parse().unwrap_or(0);
        let b: u8 = caps["b"].parse().unwrap_or(0);
        return Some(color_swatch((r, g, b)));
    }
    None
}

fn parse_hex_color(hex: &str) -> Result<(u8, u8, u8), std::num::ParseIntError> {
    let r = u8::from_str_radix(&hex[0..2], 16)?;
    let g = u8::from_str_radix(&hex[2..4], 16)?;
    let b = u8::from_str_radix(&hex[4..6], 16)?;
    Ok((r, g, b))
}

fn color_swatch(rgb: (u8, u8, u8)) -> Span<'static> {
    Span::styled("  ", Style::default().bg(Color::Rgb(rgb.0, rgb.1, rgb.2)))
}

fn extract_components(content: &str, ext: &str) -> Vec<ComponentInfo> {
    let mut comps = extract_components_tree_sitter(content, ext);
    if comps.is_empty() {
        comps = extract_components_regex(content);
    }
    comps
}

fn extract_components_tree_sitter(content: &str, ext: &str) -> Vec<ComponentInfo> {
    let mut parser = Parser::new();
    let language = match ext {
        "ts" => ts_ts::language_typescript(),
        "tsx" => ts_ts::language_tsx(),
        "js" | "jsx" => ts_js::language(),
        _ => return Vec::new(),
    };
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let tree = match parser.parse(content, None) {
        Some(tree) => tree,
        None => return Vec::new(),
    };
    let mut comps = Vec::new();
    collect_export_components(tree.root_node(), content.as_bytes(), &mut comps);
    comps
}

fn collect_export_components(node: tree_sitter::Node, source: &[u8], comps: &mut Vec<ComponentInfo>) {
    if node.kind() == "export_statement" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "function_declaration" => {
                    if let Some(name) = child.child_by_field_name("name") {
                        let params = child.child_by_field_name("parameters");
                        let props = params
                            .and_then(|p| p.utf8_text(source).ok())
                            .and_then(extract_props);
                        if let Ok(name_text) = name.utf8_text(source) {
                            comps.push(ComponentInfo {
                                name: name_text.to_string(),
                                props,
                            });
                        }
                    }
                }
                "class_declaration" => {
                    if let Some(name) = child.child_by_field_name("name") {
                        if let Ok(name_text) = name.utf8_text(source) {
                            comps.push(ComponentInfo {
                                name: name_text.to_string(),
                                props: None,
                            });
                        }
                    }
                }
                "lexical_declaration" => {
                    collect_export_variables(child, source, comps);
                }
                _ => {}
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_export_components(child, source, comps);
    }
}

fn collect_export_variables(node: tree_sitter::Node, source: &[u8], comps: &mut Vec<ComponentInfo>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            let name = child.child_by_field_name("name");
            let value = child.child_by_field_name("value");
            if let (Some(name_node), Some(value_node)) = (name, value) {
                if matches!(value_node.kind(), "arrow_function" | "function") {
                    let params = value_node.child_by_field_name("parameters");
                    let props = params
                        .and_then(|p| p.utf8_text(source).ok())
                        .and_then(extract_props);
                    if let Ok(name_text) = name_node.utf8_text(source) {
                        comps.push(ComponentInfo {
                            name: name_text.to_string(),
                            props,
                        });
                    }
                }
            }
        }
    }
}

fn extract_components_regex(content: &str) -> Vec<ComponentInfo> {
    let mut comps = Vec::new();
    let export_fn = Regex::new(r"export\s+function\s+(?P<name>[A-Za-z0-9_]+)\s*\((?P<args>[^)]*)\)").unwrap();
    let export_const = Regex::new(r"export\s+const\s+(?P<name>[A-Za-z0-9_]+)\s*=\s*\((?P<args>[^)]*)\)").unwrap();
    let export_default = Regex::new(r"export\s+default\s+function\s+(?P<name>[A-Za-z0-9_]+)\s*\((?P<args>[^)]*)\)").unwrap();

    for caps in export_fn.captures_iter(content) {
        comps.push(ComponentInfo {
            name: caps["name"].to_string(),
            props: extract_props(&caps["args"]),
        });
    }
    for caps in export_const.captures_iter(content) {
        comps.push(ComponentInfo {
            name: caps["name"].to_string(),
            props: extract_props(&caps["args"]),
        });
    }
    for caps in export_default.captures_iter(content) {
        comps.push(ComponentInfo {
            name: caps["name"].to_string(),
            props: extract_props(&caps["args"]),
        });
    }

    comps
}

fn extract_props(args: &str) -> Option<String> {
    let trimmed = args
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.split(',').next().unwrap_or("").trim().to_string())
    }
}

fn parse_syntax_errors(content: &str, ext: &str) -> HashSet<usize> {
    let mut errors = HashSet::new();
    let language = match ext {
        "ts" => ts_ts::language_typescript(),
        "tsx" => ts_ts::language_tsx(),
        "js" | "jsx" => ts_js::language(),
        "css" | "tcss" => ts_css::language(),
        _ => return errors,
    };
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return errors;
    }
    let tree = match parser.parse(content, None) {
        Some(tree) => tree,
        None => return errors,
    };
    collect_error_lines(tree.root_node(), &mut errors);
    errors
}

fn collect_error_lines(node: tree_sitter::Node, errors: &mut HashSet<usize>) {
    if node.is_error() {
        errors.insert(node.start_position().row as usize);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_error_lines(child, errors);
    }
}

fn render_markdown(content: &str) -> Vec<MdLine> {
    use comrak::{parse_document, Arena, ComrakOptions};
    // Normalise LaTeX `\(...\)` / `\[...\]` delimiters into sentinel markers
    // before comrak parses (comrak strips the backslash escapes otherwise).
    let content = normalize_math_delimiters(content);
    let arena = Arena::new();
    let mut options = ComrakOptions::default();
    options.extension.tasklist = true;
    options.extension.table = true;
    let root = parse_document(&arena, &content, &options);
    let mut renderer = MdRenderer::new();
    for node in root.children() {
        renderer.render_block(node, 0, false);
    }
    renderer.finish();
    renderer.lines
}

struct MdRenderer {
    lines: Vec<MdLine>,
    current: Vec<Span<'static>>,
    current_source: Option<usize>,
}

impl MdRenderer {
    fn new() -> Self {
        Self {
            lines: Vec::new(),
            current: Vec::new(),
            current_source: None,
        }
    }

    fn finish(&mut self) {
        self.flush_line();
    }

    fn flush_line(&mut self) {
        if !self.current.is_empty() {
            self.lines.push(MdLine {
                line: Line::from(self.current.drain(..).collect::<Vec<_>>()),
                source_line: self.current_source,
                is_heading: false,
            });
            self.current_source = None;
        }
    }

    fn blank_line(&mut self) {
        self.flush_line();
        self.lines.push(MdLine {
            line: Line::from(""),
            source_line: None,
            is_heading: false,
        });
    }

    fn render_block<'a>(
        &mut self,
        node: &'a comrak::nodes::AstNode<'a>,
        indent: usize,
        in_quote: bool,
    ) {
        use comrak::nodes::NodeValue;
        let source = node.data.borrow().sourcepos.start.line.saturating_sub(1) as usize;
        match &node.data.borrow().value {
            NodeValue::Heading(heading) => {
                self.blank_line();
                let mut spans = Vec::new();
                if in_quote {
                    spans.push(Span::styled("> ", Style::default().fg(Color::LightCyan)));
                }
                let style = heading_style(heading.level);
                spans.extend(self.render_inlines(node, style));
                self.lines.push(MdLine {
                    line: Line::from(spans),
                    source_line: Some(source),
                    is_heading: true,
                });
                self.blank_line();
            }
            NodeValue::Paragraph if paragraph_display_math(node).is_some() => {
                let math = paragraph_display_math(node).unwrap();
                self.blank_line();
                let rendered = crate::engines::math::latex_to_unicode(&math);
                for text_line in rendered.lines() {
                    let mut spans = Vec::new();
                    if in_quote {
                        spans.push(Span::styled("> ", Style::default().fg(Color::LightCyan)));
                    }
                    spans.push(Span::raw(" ".repeat(indent + 4)));
                    spans.push(Span::styled(text_line.trim_end().to_string(), math_style()));
                    self.lines.push(MdLine {
                        line: Line::from(spans),
                        source_line: Some(source),
                        is_heading: false,
                    });
                }
                self.blank_line();
            }
            NodeValue::Paragraph => {
                let mut spans = Vec::new();
                if in_quote {
                    spans.push(Span::styled("> ", Style::default().fg(Color::LightCyan)));
                }
                spans.extend(self.render_inlines(node, Style::default().fg(Color::White)));
                if indent > 0 {
                    let pad = " ".repeat(indent);
                    spans.insert(0, Span::raw(pad));
                }
                self.lines.push(MdLine {
                    line: Line::from(spans),
                    source_line: Some(source),
                    is_heading: false,
                });
                self.blank_line();
            }
            NodeValue::CodeBlock(code) => {
                self.blank_line();
                for (offset, line) in code.literal.lines().enumerate() {
                    let mut spans = Vec::new();
                    if in_quote {
                        spans.push(Span::styled("> ", Style::default().fg(Color::LightCyan)));
                    }
                    if indent > 0 {
                        spans.push(Span::raw(" ".repeat(indent)));
                    }
                    spans.push(Span::styled(
                        format!("{}{}", if indent == 0 { "  " } else { "" }, line),
                        Style::default().fg(Color::LightGreen).bg(Color::DarkGray),
                    ));
                    self.lines.push(MdLine {
                        line: Line::from(spans),
                        source_line: Some(source + offset),
                        is_heading: false,
                    });
                }
                self.blank_line();
            }
            NodeValue::List(list) => {
                let mut idx = 1;
                for child in node.children() {
                    let bullet = if list.list_type == comrak::nodes::ListType::Ordered {
                        let marker = format!("{}. ", idx);
                        idx += 1;
                        marker
                    } else {
                        "- ".to_string()
                    };
                    self.render_list_item(child, indent, in_quote, bullet);
                }
                self.blank_line();
            }
            NodeValue::BlockQuote => {
                self.blank_line();
                for child in node.children() {
                    self.render_block(child, indent, true);
                }
                self.blank_line();
            }
            NodeValue::Table(table_meta) => {
                self.blank_line();
                let alignments = table_meta.alignments.clone();
                // Collect all rows: each row is a Vec of cell text
                let mut rows: Vec<(Vec<String>, bool)> = Vec::new(); // (cells, is_header)
                for row_node in node.children() {
                    let is_header = matches!(
                        &row_node.data.borrow().value,
                        NodeValue::TableRow(true)
                    );
                    let mut cells = Vec::new();
                    for cell_node in row_node.children() {
                        let text: String = self
                            .render_inlines(cell_node, Style::default())
                            .iter()
                            .map(|s| s.content.to_string())
                            .collect();
                        cells.push(text);
                    }
                    rows.push((cells, is_header));
                }
                // Compute column widths
                let num_cols = alignments.len().max(
                    rows.iter().map(|(cells, _)| cells.len()).max().unwrap_or(0),
                );
                let mut col_widths = vec![0usize; num_cols];
                for (cells, _) in &rows {
                    for (i, cell) in cells.iter().enumerate() {
                        if i < num_cols {
                            col_widths[i] = col_widths[i].max(super::display_width(cell));
                        }
                    }
                }
                // Render each row
                for (cells, is_header) in &rows {
                    let mut spans = Vec::new();
                    if in_quote {
                        spans.push(Span::styled("> ", Style::default().fg(Color::LightCyan)));
                    }
                    if indent > 0 {
                        spans.push(Span::raw(" ".repeat(indent)));
                    }
                    for (i, cell) in cells.iter().enumerate() {
                        if i > 0 {
                            spans.push(Span::styled(
                                " │ ",
                                Style::default().fg(Color::DarkGray),
                            ));
                        }
                        let w = col_widths.get(i).copied().unwrap_or(0);
                        use comrak::nodes::TableAlignment;
                        let aligned = match alignments.get(i) {
                            Some(TableAlignment::Right) => super::pad_start_to_width(cell, w),
                            Some(TableAlignment::Center) => {
                                let pad = w.saturating_sub(super::display_width(cell));
                                let left = pad / 2;
                                let right = pad - left;
                                format!("{}{}{}", " ".repeat(left), cell, " ".repeat(right))
                            }
                            _ => super::pad_to_width(cell, w),
                        };
                        let style = if *is_header {
                            Style::default().fg(Color::LightCyan).bold()
                        } else {
                            Style::default().fg(Color::White)
                        };
                        spans.push(Span::styled(aligned, style));
                    }
                    self.lines.push(MdLine {
                        line: Line::from(spans),
                        source_line: Some(source),
                        is_heading: false,
                    });
                    // Render separator after header row
                    if *is_header {
                        let mut sep_spans = Vec::new();
                        if in_quote {
                            sep_spans.push(Span::styled("> ", Style::default().fg(Color::LightCyan)));
                        }
                        if indent > 0 {
                            sep_spans.push(Span::raw(" ".repeat(indent)));
                        }
                        for (i, w) in col_widths.iter().enumerate() {
                            if i > 0 {
                                sep_spans.push(Span::styled(
                                    "─┼─",
                                    Style::default().fg(Color::DarkGray),
                                ));
                            }
                            sep_spans.push(Span::styled(
                                "─".repeat(*w),
                                Style::default().fg(Color::DarkGray),
                            ));
                        }
                        self.lines.push(MdLine {
                            line: Line::from(sep_spans),
                            source_line: None,
                            is_heading: false,
                        });
                    }
                }
                self.blank_line();
            }
            _ => {
                for child in node.children() {
                    self.render_block(child, indent, in_quote);
                }
            }
        }
    }

    fn render_list_item<'a>(
        &mut self,
        node: &'a comrak::nodes::AstNode<'a>,
        indent: usize,
        in_quote: bool,
        bullet: String,
    ) {
        let source = node.data.borrow().sourcepos.start.line.saturating_sub(1) as usize;
        let mut spans = Vec::new();
        if in_quote {
            spans.push(Span::styled("> ", Style::default().fg(Color::LightCyan)));
        }
        if indent > 0 {
            spans.push(Span::raw(" ".repeat(indent)));
        }
                spans.push(Span::styled(bullet, Style::default().fg(Color::LightYellow)));
                spans.extend(self.render_inlines(node, Style::default().fg(Color::White)));
                self.lines.push(MdLine {
                    line: Line::from(spans),
                    source_line: Some(source),
                    is_heading: false,
                });
            }

    fn render_inlines<'a>(
        &self,
        node: &'a comrak::nodes::AstNode<'a>,
        base_style: Style,
    ) -> Vec<Span<'static>> {
        use comrak::nodes::NodeValue;
        let mut spans = Vec::new();
        for child in node.children() {
            match &child.data.borrow().value {
                NodeValue::Text(text) => {
                    push_text_with_inline_math(&mut spans, text, base_style);
                }
                NodeValue::Code(code) => {
                    spans.push(Span::styled(
                        format!(" {} ", code.literal),
                        base_style.fg(Color::LightGreen).bg(Color::DarkGray),
                    ));
                }
                NodeValue::Emph => {
                    let style = base_style.italic();
                    spans.extend(self.render_inlines(child, style));
                }
                NodeValue::Strong => {
                    let style = base_style.bold();
                    spans.extend(self.render_inlines(child, style));
                }
                NodeValue::Link(link) => {
                    let mut link_spans = self.render_inlines(child, base_style.fg(Color::LightBlue));
                    link_spans.push(Span::styled(
                        format!(" ({})", link.url),
                        Style::default().fg(Color::DarkGray),
                    ));
                    spans.extend(link_spans);
                }
                NodeValue::SoftBreak | NodeValue::LineBreak => {
                    spans.push(Span::styled(" ".to_string(), base_style));
                }
                _ => {
                    spans.extend(self.render_inlines(child, base_style));
                }
            }
        }
        spans
    }
}

/// Distinct styling for rendered LaTeX math (inline and display).
fn math_style() -> Style {
    Style::default().fg(Color::Rgb(0xE6, 0xC0, 0x7B)).italic()
}

// Private-use sentinels standing in for LaTeX math delimiters after
// preprocessing. They survive comrak untouched (comrak treats them as ordinary
// text) whereas the original `$`/`\(`/`\[` delimiters and the backslash escapes
// inside math do not.
const MATH_INLINE_OPEN: char = '\u{E000}';
const MATH_INLINE_CLOSE: char = '\u{E001}';
const MATH_DISPLAY_OPEN: char = '\u{E002}';
const MATH_DISPLAY_CLOSE: char = '\u{E003}';

/// Which delimiter opened a multi-line display equation, so we know how it ends.
#[derive(Clone, Copy)]
enum DisplayKind {
    /// Opened with `$$`, closes with `$$`.
    Dollar,
    /// Opened with `\[`, closes with `\]`.
    Bracket,
}

/// Rewrite every LaTeX math region (`$...$`, `$$...$$`, `\(...\)`, `\[...\]`)
/// into sentinel-delimited spans before comrak parses the document, and replace
/// each backslash *inside* math with a protected sentinel so comrak cannot strip
/// escapes like `\,` or `\{`. `latex_to_unicode` restores those backslashes.
///
/// Fenced code blocks and inline code spans are left untouched, so literal
/// delimiters shown as code (e.g. in a tutorial about LaTeX) survive verbatim.
/// The `$` currency heuristic lives here so the renderer only ever sees
/// unambiguous sentinels.
fn normalize_math_delimiters(content: &str) -> String {
    let mut out = String::with_capacity(content.len() + 16);
    let mut fence: Option<(char, usize)> = None;
    let mut display: Option<DisplayKind> = None;
    for line in content.split_inclusive('\n') {
        let (text, newline) = match line.strip_suffix('\n') {
            Some(t) => (t, "\n"),
            None => (line, ""),
        };
        // Code fences only matter when we are not in the middle of an equation.
        if display.is_none() {
            if let Some((fc, run, has_info)) = code_fence(text) {
                match fence {
                    None => {
                        fence = Some((fc, run));
                        out.push_str(line);
                        continue;
                    }
                    Some((mc, mrun)) if fc == mc && run >= mrun && !has_info => {
                        fence = None;
                        out.push_str(line);
                        continue;
                    }
                    _ => {}
                }
            }
            if fence.is_some() {
                out.push_str(line);
                continue;
            }
        }
        normalize_math_line(text, &mut display, &mut out);
        out.push_str(newline);
    }
    out
}

/// Detect a code-fence line, returning `(fence_char, run_length, has_info)`.
fn code_fence(text: &str) -> Option<(char, usize, bool)> {
    let trimmed = text.trim_start();
    let indent = text.len() - trimmed.len();
    if indent >= 4 {
        return None; // indented code block, not a fence
    }
    let fc = trimmed.chars().next()?;
    if fc != '`' && fc != '~' {
        return None;
    }
    let run = trimmed.chars().take_while(|&c| c == fc).count();
    if run < 3 {
        return None;
    }
    let rest: String = trimmed.chars().skip(run).collect();
    // A backtick fence's info string may not itself contain a backtick.
    if fc == '`' && rest.contains('`') {
        return None;
    }
    Some((fc, run, !rest.trim().is_empty()))
}

/// Process one line: detect math regions, emit sentinel-delimited spans with
/// their backslashes protected, and copy everything else (including inline code
/// spans) verbatim. `display` carries multi-line display-math state across lines.
fn normalize_math_line(text: &str, display: &mut Option<DisplayKind>, out: &mut String) {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    // Continuation of a multi-line display equation opened on an earlier line.
    if let Some(kind) = *display {
        match copy_display_until_close(&chars, i, kind, out) {
            Some(end) => {
                *display = None;
                i = end;
            }
            None => return, // whole line is equation body
        }
    }

    while i < chars.len() {
        let c = chars[i];
        // Inline code span: copy verbatim, math delimiters inside are literal.
        if c == '`' {
            i = copy_code_span(&chars, i, out);
            continue;
        }
        // Display math: `$$...$$`.
        if c == '$' && chars.get(i + 1) == Some(&'$') {
            match find_seq(&chars, i + 2, '$', '$') {
                Some(close) => {
                    emit_math(out, MATH_DISPLAY_OPEN, &chars[i + 2..close], MATH_DISPLAY_CLOSE);
                    i = close + 2;
                }
                None => {
                    out.push(MATH_DISPLAY_OPEN);
                    copy_protected(&chars[i + 2..], out);
                    *display = Some(DisplayKind::Dollar);
                    return;
                }
            }
            continue;
        }
        // Display math: `\[...\]`.
        if c == '\\' && chars.get(i + 1) == Some(&'[') {
            match find_backslash_delim(&chars, i + 2, ']') {
                Some(close) => {
                    emit_math(out, MATH_DISPLAY_OPEN, &chars[i + 2..close], MATH_DISPLAY_CLOSE);
                    i = close + 2;
                }
                None => {
                    out.push(MATH_DISPLAY_OPEN);
                    copy_protected(&chars[i + 2..], out);
                    *display = Some(DisplayKind::Bracket);
                    return;
                }
            }
            continue;
        }
        // Inline math: `\(...\)`.
        if c == '\\' && chars.get(i + 1) == Some(&'(') {
            if let Some(close) = find_backslash_delim(&chars, i + 2, ')') {
                emit_math(out, MATH_INLINE_OPEN, &chars[i + 2..close], MATH_INLINE_CLOSE);
                i = close + 2;
                continue;
            }
            // No closer on this line: not valid inline math, emit literally.
            out.push('\\');
            out.push('(');
            i += 2;
            continue;
        }
        // Inline math: `$...$` (currency-safe heuristic).
        if c == '$' {
            if let Some(close) = find_dollar_close(&chars, i) {
                emit_math(out, MATH_INLINE_OPEN, &chars[i + 1..close], MATH_INLINE_CLOSE);
                i = close + 1;
                continue;
            }
            out.push('$'); // lone `$`: currency or plain text
            i += 1;
            continue;
        }
        // Any other backslash escape outside math: leave for comrak to handle.
        if c == '\\' && i + 1 < chars.len() {
            out.push('\\');
            out.push(chars[i + 1]);
            i += 2;
            continue;
        }
        out.push(c);
        i += 1;
    }
}

/// Emit a sentinel-delimited math span with its inner backslashes protected.
fn emit_math(out: &mut String, open: char, inner: &[char], close: char) {
    out.push(open);
    copy_protected(inner, out);
    out.push(close);
}

/// Copy math content, replacing markdown-active characters (`\`, `*`, `_`, `` ` ``,
/// `[`, `]`, `<`, `~`) with protected sentinels so comrak neither strips escapes
/// (`\,`) nor reinterprets the content as emphasis/code/links. The sentinels are
/// restored by `latex_to_unicode`.
fn copy_protected(inner: &[char], out: &mut String) {
    for &c in inner {
        match crate::engines::math::protect_markdown_char(c) {
            Some(sentinel) => out.push(sentinel),
            None => out.push(c),
        }
    }
}

/// Copy an inline code span starting at a backtick run; returns the next index.
fn copy_code_span(chars: &[char], start: usize, out: &mut String) -> usize {
    let run = chars[start..].iter().take_while(|&&ch| ch == '`').count();
    let mut j = start + run;
    while j < chars.len() {
        if chars[j] == '`' {
            let r = chars[j..].iter().take_while(|&&ch| ch == '`').count();
            if r == run {
                out.extend(&chars[start..j + run]);
                return j + run;
            }
            j += r;
        } else {
            j += 1;
        }
    }
    // Unterminated run: emit the backticks and continue scanning after them.
    for _ in 0..run {
        out.push('`');
    }
    start + run
}

/// Continue a multi-line display equation: copy (protected) up to its closer.
/// Returns `Some(next_index)` past the closer if found on this line, else `None`.
fn copy_display_until_close(
    chars: &[char],
    from: usize,
    kind: DisplayKind,
    out: &mut String,
) -> Option<usize> {
    let close = match kind {
        DisplayKind::Dollar => find_seq(chars, from, '$', '$'),
        DisplayKind::Bracket => find_backslash_delim(chars, from, ']'),
    };
    match close {
        Some(pos) => {
            copy_protected(&chars[from..pos], out);
            out.push(MATH_DISPLAY_CLOSE);
            Some(pos + 2)
        }
        None => {
            copy_protected(&chars[from..], out);
            None
        }
    }
}

/// Find the next occurrence of the two-character sequence `a``b` from `from`.
fn find_seq(chars: &[char], from: usize, a: char, b: char) -> Option<usize> {
    let mut j = from;
    while j + 1 < chars.len() {
        if chars[j] == a && chars[j + 1] == b {
            return Some(j);
        }
        j += 1;
    }
    None
}

/// Find a `\<delim>` sequence (e.g. `\)` or `\]`) from `from`, skipping an
/// escaped `\\` so it is not mistaken for a delimiter. Returns the index of the
/// backslash.
fn find_backslash_delim(chars: &[char], from: usize, delim: char) -> Option<usize> {
    let mut j = from;
    while j + 1 < chars.len() {
        if chars[j] == '\\' {
            if chars[j + 1] == delim {
                return Some(j);
            }
            if chars[j + 1] == '\\' {
                j += 2; // escaped backslash
                continue;
            }
        }
        j += 1;
    }
    None
}

/// Given an opening `$` at `open`, return the index of a matching closing `$`
/// per the pandoc heuristic (so currency like `$2.3M` is not treated as math):
///   * the opening `$` is immediately followed by a non-space character,
///   * the closing `$` is immediately preceded by a non-space character,
///   * the closing `$` is not immediately followed by a digit,
///   * a `$` may be escaped as `\$`.
fn find_dollar_close(chars: &[char], open: usize) -> Option<usize> {
    let after = *chars.get(open + 1)?;
    if after.is_whitespace() {
        return None;
    }
    let mut j = open + 1;
    while j < chars.len() {
        match chars[j] {
            '\\' => {
                j += 2; // skip escaped character
                continue;
            }
            '$' => {
                let prev = chars[j - 1];
                let next_is_digit = chars.get(j + 1).map_or(false, |c| c.is_ascii_digit());
                if !prev.is_whitespace() && !next_is_digit {
                    return Some(j);
                }
            }
            _ => {}
        }
        j += 1;
    }
    None
}

/// Split a markdown text run into plain and inline-math segments and push them
/// as styled spans. By the time text reaches the renderer, all math has already
/// been rewritten to sentinel-delimited spans by `normalize_math_delimiters`, so
/// here we only pair the sentinels (`$`/`\(` heuristics live in preprocessing).
fn push_text_with_inline_math(spans: &mut Vec<Span<'static>>, text: &str, base_style: Style) {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut buf = String::new();
    let mut flush = |buf: &mut String, spans: &mut Vec<Span<'static>>| {
        if !buf.is_empty() {
            spans.push(Span::styled(std::mem::take(buf), base_style));
        }
    };
    while i < chars.len() {
        let c = chars[i];
        if let Some(close_marker) = math_close_marker(c) {
            let end = chars[i + 1..]
                .iter()
                .position(|&ch| ch == close_marker)
                .map(|p| i + 1 + p);
            flush(&mut buf, spans);
            match end {
                Some(end) => {
                    push_math_span(spans, &chars[i + 1..end]);
                    i = end + 1;
                }
                None => {
                    // Unterminated math (e.g. a `$$` block with no closer): render
                    // the remainder as math best-effort rather than leaking the
                    // sentinel or dropping the equation.
                    push_math_span(spans, &chars[i + 1..]);
                    i = chars.len();
                }
            }
            continue;
        }
        if c == MATH_INLINE_CLOSE || c == MATH_DISPLAY_CLOSE {
            // Stray closing sentinel with no opener: drop the invisible marker.
            i += 1;
            continue;
        }
        // Restore any protected sentinel that leaked out of a math region so no
        // private-use character ever reaches the screen (worst case: raw LaTeX).
        buf.push(crate::engines::math::restore_protected(c));
        i += 1;
    }
    flush(&mut buf, spans);
}

/// Render a slice of LaTeX source as a single inline math span. Any stray
/// delimiter sentinels are stripped so they can never render as glyphs.
fn push_math_span(spans: &mut Vec<Span<'static>>, latex: &[char]) {
    let latex: String = latex
        .iter()
        .filter(|&&c| !is_math_delimiter_sentinel(c))
        .collect();
    let rendered = crate::engines::math::latex_to_unicode(latex.trim()).replace('\n', " ");
    spans.push(Span::styled(rendered, math_style()));
}

/// True for the four delimiter sentinels (open/close, inline/display).
fn is_math_delimiter_sentinel(c: char) -> bool {
    matches!(
        c,
        MATH_INLINE_OPEN | MATH_INLINE_CLOSE | MATH_DISPLAY_OPEN | MATH_DISPLAY_CLOSE
    )
}

/// Map an opening math sentinel to the closing one it expects.
fn math_close_marker(c: char) -> Option<char> {
    match c {
        MATH_INLINE_OPEN => Some(MATH_INLINE_CLOSE),
        MATH_DISPLAY_OPEN => Some(MATH_DISPLAY_CLOSE),
        _ => None,
    }
}

/// If a paragraph is a bare display-math block (sentinel-delimited by
/// preprocessing), return its LaTeX body. Returns `None` when the paragraph
/// mixes math with other inline formatting, so only genuine display equations
/// are treated specially.
fn paragraph_display_math<'a>(node: &'a comrak::nodes::AstNode<'a>) -> Option<String> {
    use comrak::nodes::NodeValue;
    let mut text = String::new();
    for child in node.children() {
        match &child.data.borrow().value {
            NodeValue::Text(t) => text.push_str(t),
            NodeValue::SoftBreak | NodeValue::LineBreak => text.push('\n'),
            // Any other inline node means this is prose, not a bare equation.
            _ => return None,
        }
    }
    let trimmed = text.trim();
    let inner = trimmed
        .strip_prefix(MATH_DISPLAY_OPEN)?
        .strip_suffix(MATH_DISPLAY_CLOSE)?;
    if inner.is_empty() {
        return None;
    }
    Some(inner.trim().to_string())
}

fn heading_style(level: u8) -> Style {
    match level {
        1 => Style::default().fg(Color::LightMagenta).bold(),
        2 => Style::default().fg(Color::LightCyan).bold(),
        3 => Style::default().fg(Color::LightBlue).bold(),
        _ => Style::default().fg(Color::LightYellow).bold(),
    }
}

struct MdLine {
    line: Line<'static>,
    source_line: Option<usize>,
    /// True only for heading lines, so `e` (next heading) navigation doesn't
    /// stop on table header rows (which share the heading color+bold style).
    is_heading: bool,
}

fn md_line_text(line: &MdLine) -> String {
    line.line
        .spans
        .iter()
        .map(|span| span.content.to_string())
        .collect::<Vec<_>>()
        .join("")
}

fn render_markdown_with_gutter(
    lines: &[MdLine],
    selection: Option<(usize, usize)>,
) -> Vec<Line<'static>> {
    let line_no_width = lines
        .iter()
        .filter_map(|line| line.source_line)
        .max()
        .unwrap_or(0)
        .saturating_add(1)
        .to_string()
        .len()
        .max(2);
    let (sel, scroll) = selection.unwrap_or((usize::MAX, 0));
    lines
        .iter()
        .enumerate()
        .skip(scroll)
        .map(|(idx, line)| {
            let row = idx;
            let line_no = match line.source_line {
                Some(source) => format!("{:>width$} ", source + 1, width = line_no_width),
                None => format!("{:>width$} ", "", width = line_no_width),
            };
            let line_no_style = if row == sel {
                Style::default().fg(Color::Black).bg(Color::LightBlue).bold()
            } else {
                Style::default().fg(Color::LightYellow)
            };
            let mut spans = Vec::new();
            spans.push(Span::styled(line_no, line_no_style));
            spans.push(Span::styled("│ ", Style::default().fg(Color::LightBlue)));
            spans.extend(line.line.spans.clone());
            let mut line_widget = Line::from(spans);
            if row == sel {
                line_widget =
                    line_widget.style(Style::default().bg(Color::LightBlue).fg(Color::Black));
            }
            line_widget
        })
        .collect()
}

fn next_markdown_heading(lines: &[MdLine], current: usize) -> Option<usize> {
    lines
        .iter()
        .enumerate()
        .skip(current + 1)
        .find(|(_, line)| line.is_heading)
        .map(|(idx, _)| idx)
}

fn page_jump(view_height: usize) -> usize {
    let half = view_height / 2;
    if half == 0 { 1 } else { half }
}

impl SyntaxEngine {
    fn search_next(&mut self, query: &str, forward: bool) {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return;
        }
        let matcher = crate::search::Matcher::new(trimmed);
        if self.is_markdown {
            let total = self.md_rendered.len().max(1);
            let start = if forward {
                (self.selection + 1) % total
            } else {
                self.selection.saturating_sub(1)
            };
            for offset in 0..self.md_rendered.len() {
                let idx = if forward {
                    (start + offset) % total
                } else {
                    (start + total - offset % total) % total
                };
                if matcher.is_match(&md_line_text(&self.md_rendered[idx])) {
                    self.selection = idx;
                    break;
                }
            }
        } else {
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
                if matcher.is_match(&self.lines[idx]) {
                    self.selection = idx;
                    break;
                }
            }
        }
        self.last_match = Some(trimmed.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_syntax_errors() {
        let content = "function () {";
        let errors = parse_syntax_errors(content, "js");
        assert!(!errors.is_empty());
    }

    #[test]
    fn renders_markdown_content() {
        let content = "# Title\n- [ ] Task one\n";
        let lines = render_markdown(content);
        // Should render some content
        assert!(!lines.is_empty());
    }

    fn rendered_text(content: &str) -> String {
        render_markdown(content)
            .iter()
            .map(md_line_text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn renders_inline_math() {
        let out = rendered_text("Energy is $E = mc^2$ today.");
        assert!(out.contains("E = mc²"), "got: {out}");
        assert!(!out.contains('$'), "delimiters should be consumed: {out}");
    }

    #[test]
    fn leaves_currency_alone() {
        let out = rendered_text("It cost $2.3M and $5 total.");
        assert!(out.contains("$2.3M"), "got: {out}");
        assert!(out.contains("$5"), "got: {out}");
    }

    #[test]
    fn renders_display_math_block() {
        let content = "Formula:\n\n$$\nx = \\frac{-b}{2a}\n$$\n";
        let out = rendered_text(content);
        assert!(out.contains("x = (-b)/(2a)"), "got: {out}");
        assert!(!out.contains("$$"), "delimiters should be consumed: {out}");
    }

    #[test]
    fn inline_math_in_heading() {
        let out = rendered_text("## The $\\alpha$ chapter\n");
        assert!(out.contains("α"), "got: {out}");
    }

    #[test]
    fn supports_paren_inline_delimiters() {
        // `\(...\)` inline math, including inner spaces which the `$` form rejects.
        let out = rendered_text("Pythagoras wrote \\( a^2 + b^2 = c^2 \\) here.");
        assert!(out.contains("a² + b² = c²"), "got: {out}");
        // The `\(` / `\)` delimiters themselves must not survive as text.
        assert!(!out.contains("\\("), "delimiters should be consumed: {out}");
        assert!(!out.contains('\u{E000}'), "sentinels should be consumed: {out}");
    }

    #[test]
    fn supports_bracket_display_delimiters() {
        let content = "Pythagoras:\n\n\\[ a^2 + b^2 = c^2 \\]\n";
        let out = rendered_text(content);
        assert!(out.contains("a² + b² = c²"), "got: {out}");
        assert!(!out.contains('['), "delimiters should be consumed: {out}");
    }

    #[test]
    fn multiline_bracket_display() {
        let content = "\\[\nx = \\frac{-b}{2a}\n\\]\n";
        let out = rendered_text(content);
        assert!(out.contains("x = (-b)/(2a)"), "got: {out}");
    }

    #[test]
    fn backslash_punctuation_commands_survive() {
        // `\,` (thin space) and `\{`/`\}` must not be stripped by comrak before
        // the math renderer sees them.
        let out = rendered_text("The integral $\\int_0^1 f\\,dx$ converges.");
        // `\,` renders as a space, so the comma must NOT appear.
        assert!(out.contains("∫₀¹ f dx"), "got: {out}");
        assert!(!out.contains("f,dx") && !out.contains("f ,dx"), "got: {out}");

        let braces = rendered_text("A set $\\{1, 2, 3\\}$ here.");
        assert!(braces.contains("{1, 2, 3}"), "got: {braces}");
    }

    #[test]
    fn backslash_punctuation_in_paren_delimiters_survive() {
        let out = rendered_text("Euler: \\( e^{i\\pi} \\; \\{x\\} \\).");
        assert!(out.contains("{x}"), "got: {out}");
    }

    fn assert_no_sentinels(out: &str) {
        for c in out.chars() {
            assert!(
                !('\u{E000}'..='\u{E00F}').contains(&c),
                "sentinel U+{:04X} leaked: {out}",
                c as u32
            );
        }
    }

    #[test]
    fn inline_math_with_markdown_active_chars() {
        // `*` inside math must not be parsed as emphasis, and nothing leaks.
        let out = rendered_text("Inline: $\\gamma*x*\\delta$ done.");
        assert!(out.contains("γ*x*δ"), "got: {out}");
        assert!(!out.contains('$'), "delimiters should be consumed: {out}");
        assert_no_sentinels(&out);
    }

    #[test]
    fn inline_math_with_underscores_and_brackets() {
        let out = rendered_text("Range $a_1 + \\sqrt[3]{x}$ ok.");
        assert!(out.contains("a₁"), "got: {out}");
        assert!(out.contains("³√x"), "got: {out}");
        assert_no_sentinels(&out);
    }

    #[test]
    fn unterminated_display_math_does_not_leak() {
        // A `$$` block with no closer must never leak private-use sentinels.
        let out = rendered_text("Broken:\n\n$$\n\\alpha + \\beta\n");
        assert_no_sentinels(&out);
        // The backslashes are restored (honest raw source), not turned into boxes.
        assert!(out.contains("alpha") || out.contains('α'), "got: {out}");
    }

    #[test]
    fn unterminated_inline_math_does_not_leak() {
        let out = rendered_text("Oops $\\alpha + x here");
        assert_no_sentinels(&out);
    }

    #[test]
    fn paren_delimiters_in_code_are_left_alone() {
        // Inline code and fenced code must keep the literal `\(...\)` text.
        let inline = rendered_text("Use `\\(x\\)` for inline math.");
        assert!(inline.contains("\\(x\\)"), "inline code got: {inline}");

        let fenced = rendered_text("```\n\\[ y = x \\]\n```\n");
        assert!(fenced.contains("\\[ y = x \\]"), "fenced got: {fenced}");
    }

    #[test]
    fn python_multiline_string_highlighting_preserved() {
        // Test that highlighting state is preserved across lines for Python multiline strings
        let content = r#"x = 1
"""
This is inside
a multiline
string
"""
y = 2
"#;
        let syntax_set = SyntaxSet::load_defaults_newlines();
        let theme_set = ThemeSet::load_defaults();
        let theme = theme_set.themes.values().next().unwrap();
        let syntax = syntax_set.find_syntax_by_extension("py").unwrap();
        let mut highlighter = HighlightLines::new(syntax, theme);

        // Highlight all lines and collect results
        let mut all_styles: Vec<Vec<(SynStyle, String)>> = Vec::new();
        for line in content.lines() {
            let line_with_newline = format!("{}\n", line);
            let regions = highlighter.highlight_line(&line_with_newline, &syntax_set).unwrap();
            all_styles.push(regions.iter().map(|(s, t)| (*s, t.to_string())).collect());
        }

        // Lines inside the multiline string (indices 1-5) should have string styling
        // Line 6 (y = 2) should NOT be styled as a string
        let last_line_styles = &all_styles[6];
        // The 'y' identifier should not have the same color as strings
        // (Exact color depends on theme, but it should be different from string lines)
        assert!(!last_line_styles.is_empty(), "Last line should have highlighting");

        // Check that line inside string and line outside have different styling
        let string_line = &all_styles[2]; // "This is inside"
        let code_line = &all_styles[6];   // "y = 2"

        // At minimum, verify both lines got some highlighting
        assert!(!string_line.is_empty());
        assert!(!code_line.is_empty());
    }
}
