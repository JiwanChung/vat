use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

#[derive(Clone)]
struct XmlNode {
    depth: usize,
    tag: String,
    attributes: Vec<(String, String)>,
    text: Option<String>,
    has_children: bool,
    node_index: usize,
}

pub struct XmlEngine {
    nodes: Vec<XmlNode>,
    collapsed: HashSet<usize>,
    selection: usize,
    scroll: usize,
    file_name: String,
    last_query: Option<String>,
    pending_g: bool,
    last_view_height: usize,
    last_match: Option<String>,
    /// Visual selection range (start, end) for highlighting
    pub visual_range: Option<(usize, usize)>,
}

impl XmlEngine {
    pub fn from_path(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        let nodes = parse_xml(&content)?;

        Ok(Self {
            nodes,
            collapsed: HashSet::new(),
            selection: 0,
            scroll: 0,
            file_name,
            last_query: None,
            pending_g: false,
            last_view_height: 0,
            last_match: None,
            visual_range: None,
        })
    }

    fn visible_nodes(&self) -> Vec<usize> {
        let mut visible = Vec::new();
        let mut skip_depth: Option<usize> = None;

        for (idx, node) in self.nodes.iter().enumerate() {
            if let Some(depth) = skip_depth {
                if node.depth > depth {
                    continue;
                }
                skip_depth = None;
            }
            visible.push(idx);
            if self.collapsed.contains(&node.node_index) && node.has_children {
                skip_depth = Some(node.depth);
            }
        }
        visible
    }

    pub fn render(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let height = area.height as usize;
        self.last_view_height = height;

        let visible = self.visible_nodes();
        let total = visible.len();

        if self.selection >= total && total > 0 {
            self.selection = total - 1;
        }

        if self.selection < self.scroll {
            self.scroll = self.selection;
        } else if self.selection >= self.scroll + height {
            self.scroll = self.selection.saturating_sub(height - 1);
        }

        let line_no_width = self.nodes.len().max(1).to_string().len().max(2);

        let display: Vec<Line> = visible
            .iter()
            .skip(self.scroll)
            .take(height)
            .enumerate()
            .map(|(display_idx, &node_idx)| {
                let node = &self.nodes[node_idx];
                let row = self.scroll + display_idx;
                let selected = row == self.selection;
                let is_collapsed = self.collapsed.contains(&node.node_index);

                let mut spans = Vec::new();
                let line_no = format!("{:>width$} ", node_idx + 1, width = line_no_width);
                let line_no_style = if selected {
                    Style::default().fg(Color::Black).bg(Color::LightBlue).bold()
                } else {
                    Style::default().fg(Color::LightYellow)
                };
                spans.push(Span::styled(line_no, line_no_style));
                spans.push(Span::styled("│ ", Style::default().fg(Color::LightBlue)));

                // Indentation
                let indent = "  ".repeat(node.depth);
                spans.push(Span::raw(indent));

                // Collapse marker
                if node.has_children {
                    let marker = if is_collapsed { "[+] " } else { "[-] " };
                    spans.push(Span::styled(marker, Style::default().fg(Color::Cyan)));
                } else {
                    spans.push(Span::raw("    "));
                }

                // Tag
                let tag_style = if selected {
                    Style::default().fg(Color::Black).bg(Color::LightBlue).bold()
                } else {
                    Style::default().fg(Color::LightGreen).bold()
                };
                spans.push(Span::styled(format!("<{}", node.tag), tag_style));

                // Attributes
                for (key, value) in &node.attributes {
                    let attr_style = if selected {
                        Style::default().fg(Color::Black).bg(Color::LightBlue)
                    } else {
                        Style::default().fg(Color::LightCyan)
                    };
                    let val_style = if selected {
                        Style::default().fg(Color::Black).bg(Color::LightBlue)
                    } else {
                        Style::default().fg(Color::LightYellow)
                    };
                    spans.push(Span::styled(format!(" {}=", key), attr_style));
                    spans.push(Span::styled(format!("\"{}\"", truncate(value, 20)), val_style));
                }

                spans.push(Span::styled(">", tag_style));

                // Text content
                if let Some(text) = &node.text {
                    let text_style = if selected {
                        Style::default().fg(Color::Black).bg(Color::LightBlue)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    spans.push(Span::styled(format!(" {}", truncate(text, 40)), text_style));
                }

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

        let visible = self.visible_nodes();
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
            KeyCode::Enter => {
                if let Some(&node_idx) = visible.get(self.selection) {
                    let node = &self.nodes[node_idx];
                    if node.has_children {
                        if self.collapsed.contains(&node.node_index) {
                            self.collapsed.remove(&node.node_index);
                        } else {
                            self.collapsed.insert(node.node_index);
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
    }

    pub fn breadcrumbs(&self) -> String {
        let visible = self.visible_nodes();
        if let Some(&node_idx) = visible.get(self.selection) {
            let node = &self.nodes[node_idx];
            format!("{} <{}> depth {}", self.file_name, node.tag, node.depth)
        } else {
            self.file_name.clone()
        }
    }

    pub fn status_line(&self) -> String {
        let query = self
            .last_query
            .as_ref()
            .map(|q| format!(" | search: {}", q))
            .unwrap_or_default();
        format!(
            "j/k move | gg/G jump | Ctrl+u/d half-page | Enter fold | n/N next/prev | / search{}",
            query
        )
    }

    #[allow(dead_code)]
    pub fn selected_path(&self) -> Option<String> {
        None
    }

    /// Get the content of the currently selected line
    pub fn get_selected_line(&self) -> Option<String> {
        let visible = self.visible_nodes();
        visible.get(self.selection).map(|&node_idx| {
            let node = &self.nodes[node_idx];
            let text = node.text.as_deref().unwrap_or("");
            format!("<{}> {}", node.tag, text)
        })
    }

    /// Get lines in a range (inclusive), skipping children of selected parents
    pub fn get_lines_range(&self, start: usize, end: usize) -> Option<String> {
        let (start, end) = if start <= end { (start, end) } else { (end, start) };
        let visible = self.visible_nodes();
        let total = visible.len();
        if start >= total { return None; }
        let end = end.min(total.saturating_sub(1));

        let mut results = Vec::new();
        let mut skip_depth: Option<usize> = None;

        for idx in start..=end {
            if let Some(&node_idx) = visible.get(idx) {
                let node = &self.nodes[node_idx];

                // Skip children of already-selected parent
                if let Some(parent_depth) = skip_depth {
                    if node.depth > parent_depth {
                        continue;
                    } else {
                        skip_depth = None;
                    }
                }

                let text = node.text.as_deref().unwrap_or("");
                results.push(format!("<{}> {}", node.tag, text));

                // If this node has children, skip them
                if node.has_children {
                    skip_depth = Some(node.depth);
                }
            }
        }

        if results.is_empty() { None } else { Some(results.join("\n")) }
    }

    /// Get current selection index (for visual mode)
    pub fn selection(&self) -> usize {
        self.selection
    }

    pub fn content_height(&self) -> usize {
        self.visible_nodes().len()
    }

    pub fn render_plain_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let line_no_width = self.nodes.len().max(1).to_string().len().max(2);
        self.nodes
            .iter()
            .enumerate()
            .map(|(idx, node)| {
                let mut spans = Vec::new();
                spans.push(Span::styled(
                    format!("{:>width$} ", idx + 1, width = line_no_width),
                    Style::default().fg(Color::LightYellow),
                ));
                spans.push(Span::styled("│ ", Style::default().fg(Color::LightBlue)));
                spans.push(Span::raw("  ".repeat(node.depth)));
                spans.push(Span::styled(
                    format!("<{}", node.tag),
                    Style::default().fg(Color::LightGreen).bold(),
                ));
                for (key, value) in &node.attributes {
                    spans.push(Span::styled(format!(" {}=", key), Style::default().fg(Color::LightCyan)));
                    spans.push(Span::styled(format!("\"{}\"", value), Style::default().fg(Color::LightYellow)));
                }
                spans.push(Span::styled(">", Style::default().fg(Color::LightGreen).bold()));
                if let Some(text) = &node.text {
                    spans.push(Span::styled(format!(" {}", text), Style::default().fg(Color::White)));
                }
                Line::from(spans)
            })
            .collect()
    }

    fn search_next(&mut self, query: &str, forward: bool) {
        let lower = query.to_lowercase();
        let visible = self.visible_nodes();
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
            if let Some(&node_idx) = visible.get(idx) {
                let node = &self.nodes[node_idx];
                let searchable = format!(
                    "{} {} {}",
                    node.tag,
                    node.attributes.iter().map(|(k, v)| format!("{}={}", k, v)).collect::<Vec<_>>().join(" "),
                    node.text.as_deref().unwrap_or("")
                );
                if searchable.to_lowercase().contains(&lower) {
                    self.selection = idx;
                    break;
                }
            }
        }
        self.last_match = Some(query.to_string());
    }
}

fn parse_xml(content: &str) -> Result<Vec<XmlNode>> {
    let doc = roxmltree::Document::parse(content)?;
    let mut nodes = Vec::new();
    let mut node_index = 0;

    fn visit(node: roxmltree::Node, depth: usize, nodes: &mut Vec<XmlNode>, node_index: &mut usize) {
        if node.is_element() {
            let tag = node.tag_name().name().to_string();
            let attributes: Vec<(String, String)> = node
                .attributes()
                .map(|a| (a.name().to_string(), a.value().to_string()))
                .collect();

            let text = node
                .children()
                .find(|c| c.is_text())
                .and_then(|c| c.text())
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty());

            let has_children = node.children().any(|c| c.is_element());

            nodes.push(XmlNode {
                depth,
                tag,
                attributes,
                text,
                has_children,
                node_index: *node_index,
            });
            *node_index += 1;

            for child in node.children() {
                visit(child, depth + 1, nodes, node_index);
            }
        }
    }

    visit(doc.root_element(), 0, &mut nodes, &mut node_index);
    Ok(nodes)
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
