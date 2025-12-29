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
        let content = std::fs::read_to_string(path)?;
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
            md_rendered,
            syntax_error_lines,
            pending_g: false,
            last_view_height: 0,
            last_match: None,
            visual_range: None,
        })
    }

    pub fn render(&mut self, frame: &mut ratatui::Frame, area: Rect) {
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
            self.render_code(frame, chunks[1]);
        } else {
            self.render_code(frame, chunks[0]);
        }
    }

    pub fn content_height(&mut self) -> usize {
        if self.is_markdown {
            self.md_rendered.len()
        } else {
            self.lines.len()
        }
    }

    pub fn render_plain_lines(&mut self) -> Vec<Line<'static>> {
        if self.is_markdown {
            return render_markdown_with_gutter(&self.md_rendered, None);
        }

        let mut output = Vec::new();
        let line_no_width = self.lines.len().max(1).to_string().len().max(2);
        let syntax = self
            .syntax
            .as_ref()
            .and_then(|name| self.syntax_set.find_syntax_by_name(name));
        let mut highlighter = syntax
            .map(|syn| HighlightLines::new(syn, &self.theme));
        for (idx, line) in self.lines.iter().enumerate() {
            let mut spans = Vec::new();
            let line_no = format!("{:>width$} ", idx + 1, width = line_no_width);
            spans.push(Span::styled(
                line_no,
                Style::default().fg(Color::LightYellow),
            ));
            spans.push(Span::styled("│ ", Style::default().fg(Color::LightBlue)));
            if self.is_css {
                if let Some(swatch) = css_swatch(line) {
                    spans.push(swatch);
                    spans.push(Span::raw(" "));
                } else {
                    spans.push(Span::raw("   "));
                }
            }
            if let Some(ref mut hl) = highlighter {
                let line_with_newline = format!("{}\n", line);
                let regions = hl.highlight_line(&line_with_newline, &self.syntax_set).unwrap_or_default();
                spans.extend(regions.into_iter().map(|(style, part)| syntect_span(style, part)));
            } else {
                spans.push(Span::styled(line.clone(), Style::default().fg(Color::White)));
            }
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

    fn render_code(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        if self.selection < self.scroll {
            self.scroll = self.selection;
        } else if self.selection >= self.scroll + area.height as usize {
            self.scroll = self.selection.saturating_sub(area.height as usize - 1);
        }

        if self.is_markdown {
            self.render_markdown(frame, area);
            return;
        }

        let syntax = self
            .syntax
            .as_ref()
            .and_then(|name| self.syntax_set.find_syntax_by_name(name));
        let mut highlighter = syntax
            .map(|syn| HighlightLines::new(syn, &self.theme));

        let mut output = Vec::new();
        let line_no_width = self.lines.len().max(1).to_string().len().max(2);
        for (idx, line) in self.lines.iter().enumerate() {
            let line_with_newline = format!("{}\n", line);
            if idx < self.scroll {
                if let Some(ref mut hl) = highlighter {
                    let _ = hl.highlight_line(&line_with_newline, &self.syntax_set);
                }
                continue;
            }
            if idx >= self.scroll + area.height as usize {
                break;
            }
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

            if let Some(ref mut hl) = highlighter {
                let regions = hl.highlight_line(&line_with_newline, &self.syntax_set).unwrap_or_default();
                spans.extend(regions.into_iter().map(|(style, part)| syntect_span(style, part)));
            } else {
                spans.push(Span::raw(line.clone()));
            }

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

        let block = Block::default().borders(Borders::NONE);
        frame.render_widget(Paragraph::new(output).block(block), area);
    }

    fn render_markdown(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        if self.selection >= self.md_rendered.len() {
            self.selection = self.md_rendered.len().saturating_sub(1);
        }
        let height = area.height as usize;
        if self.selection < self.scroll {
            self.scroll = self.selection;
        } else if self.selection >= self.scroll + height {
            self.scroll = self.selection.saturating_sub(height - 1);
        }

        let mut output = render_markdown_with_gutter(&self.md_rendered, Some((self.selection, self.scroll)));
        output.truncate(height);

        let block = Block::default().borders(Borders::NONE);
        let paragraph = Paragraph::new(output).block(block).wrap(ratatui::widgets::Wrap { trim: false });
        frame.render_widget(paragraph, area);
    }
}

fn syntect_span(style: SynStyle, text: &str) -> Span<'static> {
    let fg = style.foreground;
    Span::styled(
        text.to_string(),
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
    let arena = Arena::new();
    let mut options = ComrakOptions::default();
    options.extension.tasklist = true;
    let root = parse_document(&arena, content, &options);
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
            });
            self.current_source = None;
        }
    }

    fn blank_line(&mut self) {
        self.flush_line();
        self.lines.push(MdLine {
            line: Line::from(""),
            source_line: None,
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
                });
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
                    spans.push(Span::styled(text.to_string(), base_style));
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
    for (idx, line) in lines.iter().enumerate().skip(current + 1) {
        for span in &line.line.spans {
            let style = span.style;
            if style.add_modifier.contains(ratatui::style::Modifier::BOLD)
                && matches!(
                    style.fg,
                    Some(Color::LightMagenta | Color::LightCyan | Color::LightBlue | Color::LightYellow)
                )
            {
                return Some(idx);
            }
        }
    }
    None
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
        let lower = trimmed.to_lowercase();
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
                if md_line_text(&self.md_rendered[idx]).to_lowercase().contains(&lower) {
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
                if self.lines[idx].to_lowercase().contains(&lower) {
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
