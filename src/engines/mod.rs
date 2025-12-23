use crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::text::Line;

mod archive;
mod dockerfile;
mod env;
mod gitignore;
mod hex;
mod html;
mod image;
mod ini;
mod jsonl;
mod lock;
mod log;
mod logic;
mod makefile;
mod sqlite;
mod syntax;
mod table;
mod text;
mod tree;
mod xml;

pub use archive::ArchiveEngine;
pub use dockerfile::DockerfileEngine;
pub use env::EnvEngine;
pub use gitignore::GitIgnoreEngine;
pub use hex::HexEngine;
pub use html::HtmlEngine;
pub use image::ImageEngine;
pub use ini::IniEngine;
pub use jsonl::JsonlEngine;
pub use lock::LockEngine;
pub use log::LogEngine;
pub use logic::LogicEngine;
pub use makefile::MakefileEngine;
pub use sqlite::SqliteEngine;
pub use syntax::SyntaxEngine;
pub use table::TableEngine;
pub use text::TextEngine;
pub use tree::TreeEngine;
pub use xml::XmlEngine;

pub enum EngineState {
    Tree(TreeEngine),
    Table(TableEngine),
    Logic(LogicEngine),
    Syntax(SyntaxEngine),
    Html(HtmlEngine),
    Lock(LockEngine),
    Jsonl(JsonlEngine),
    Text(TextEngine),
    Env(EnvEngine),
    Ini(IniEngine),
    Xml(XmlEngine),
    Dockerfile(DockerfileEngine),
    Makefile(MakefileEngine),
    Log(LogEngine),
    GitIgnore(GitIgnoreEngine),
    Sqlite(SqliteEngine),
    Archive(ArchiveEngine),
    Image(ImageEngine),
    Hex(HexEngine),
}

impl EngineState {
    pub fn name(&self) -> &'static str {
        match self {
            EngineState::Tree(_) => "TreeEngine",
            EngineState::Table(_) => "TableEngine",
            EngineState::Logic(_) => "LogicEngine",
            EngineState::Syntax(_) => "SyntaxEngine",
            EngineState::Html(_) => "HtmlEngine",
            EngineState::Lock(_) => "LockEngine",
            EngineState::Jsonl(_) => "JsonlEngine",
            EngineState::Text(_) => "TextEngine",
            EngineState::Env(_) => "EnvEngine",
            EngineState::Ini(_) => "IniEngine",
            EngineState::Xml(_) => "XmlEngine",
            EngineState::Dockerfile(_) => "DockerfileEngine",
            EngineState::Makefile(_) => "MakefileEngine",
            EngineState::Log(_) => "LogEngine",
            EngineState::GitIgnore(_) => "GitIgnoreEngine",
            EngineState::Sqlite(_) => "SqliteEngine",
            EngineState::Archive(_) => "ArchiveEngine",
            EngineState::Image(_) => "ImageEngine",
            EngineState::Hex(_) => "HexEngine",
        }
    }

    pub fn breadcrumbs(&self) -> String {
        match self {
            EngineState::Tree(engine) => engine.breadcrumbs(),
            EngineState::Table(engine) => engine.breadcrumbs(),
            EngineState::Logic(engine) => engine.breadcrumbs(),
            EngineState::Syntax(engine) => engine.breadcrumbs(),
            EngineState::Html(engine) => engine.breadcrumbs(),
            EngineState::Lock(engine) => engine.breadcrumbs(),
            EngineState::Jsonl(engine) => engine.breadcrumbs(),
            EngineState::Text(engine) => engine.breadcrumbs(),
            EngineState::Env(engine) => engine.breadcrumbs(),
            EngineState::Ini(engine) => engine.breadcrumbs(),
            EngineState::Xml(engine) => engine.breadcrumbs(),
            EngineState::Dockerfile(engine) => engine.breadcrumbs(),
            EngineState::Makefile(engine) => engine.breadcrumbs(),
            EngineState::Log(engine) => engine.breadcrumbs(),
            EngineState::GitIgnore(engine) => engine.breadcrumbs(),
            EngineState::Sqlite(engine) => engine.breadcrumbs(),
            EngineState::Archive(engine) => engine.breadcrumbs(),
            EngineState::Image(engine) => engine.breadcrumbs(),
            EngineState::Hex(engine) => engine.breadcrumbs(),
        }
    }

    pub fn status_line(&self) -> String {
        match self {
            EngineState::Tree(engine) => engine.status_line(),
            EngineState::Table(engine) => engine.status_line(),
            EngineState::Logic(engine) => engine.status_line(),
            EngineState::Syntax(engine) => engine.status_line(),
            EngineState::Html(engine) => engine.status_line(),
            EngineState::Lock(engine) => engine.status_line(),
            EngineState::Jsonl(engine) => engine.status_line(),
            EngineState::Text(engine) => engine.status_line(),
            EngineState::Env(engine) => engine.status_line(),
            EngineState::Ini(engine) => engine.status_line(),
            EngineState::Xml(engine) => engine.status_line(),
            EngineState::Dockerfile(engine) => engine.status_line(),
            EngineState::Makefile(engine) => engine.status_line(),
            EngineState::Log(engine) => engine.status_line(),
            EngineState::GitIgnore(engine) => engine.status_line(),
            EngineState::Sqlite(engine) => engine.status_line(),
            EngineState::Archive(engine) => engine.status_line(),
            EngineState::Image(engine) => engine.status_line(),
            EngineState::Hex(engine) => engine.status_line(),
        }
    }

    /// Set visual selection range for highlighting
    pub fn set_visual_range(&mut self, range: Option<(usize, usize)>) {
        match self {
            EngineState::Tree(engine) => engine.visual_range = range,
            EngineState::Table(engine) => engine.visual_range = range,
            EngineState::Logic(engine) => engine.visual_range = range,
            EngineState::Syntax(engine) => engine.visual_range = range,
            EngineState::Html(engine) => engine.visual_range = range,
            EngineState::Lock(engine) => engine.visual_range = range,
            EngineState::Jsonl(engine) => engine.visual_range = range,
            EngineState::Text(engine) => engine.visual_range = range,
            EngineState::Env(engine) => engine.visual_range = range,
            EngineState::Ini(engine) => engine.visual_range = range,
            EngineState::Xml(engine) => engine.visual_range = range,
            EngineState::Dockerfile(engine) => engine.visual_range = range,
            EngineState::Makefile(engine) => engine.visual_range = range,
            EngineState::Log(engine) => engine.visual_range = range,
            EngineState::GitIgnore(engine) => engine.visual_range = range,
            EngineState::Sqlite(engine) => engine.visual_range = range,
            EngineState::Archive(engine) => engine.visual_range = range,
            EngineState::Image(engine) => engine.visual_range = range,
            EngineState::Hex(engine) => engine.visual_range = range,
        }
    }

    pub fn render(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        match self {
            EngineState::Tree(engine) => engine.render(frame, area),
            EngineState::Table(engine) => engine.render(frame, area),
            EngineState::Logic(engine) => engine.render(frame, area),
            EngineState::Syntax(engine) => engine.render(frame, area),
            EngineState::Html(engine) => engine.render(frame, area),
            EngineState::Lock(engine) => engine.render(frame, area),
            EngineState::Jsonl(engine) => engine.render(frame, area),
            EngineState::Text(engine) => engine.render(frame, area),
            EngineState::Env(engine) => engine.render(frame, area),
            EngineState::Ini(engine) => engine.render(frame, area),
            EngineState::Xml(engine) => engine.render(frame, area),
            EngineState::Dockerfile(engine) => engine.render(frame, area),
            EngineState::Makefile(engine) => engine.render(frame, area),
            EngineState::Log(engine) => engine.render(frame, area),
            EngineState::GitIgnore(engine) => engine.render(frame, area),
            EngineState::Sqlite(engine) => engine.render(frame, area),
            EngineState::Archive(engine) => engine.render(frame, area),
            EngineState::Image(engine) => engine.render(frame, area),
            EngineState::Hex(engine) => engine.render(frame, area),
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match self {
            EngineState::Tree(engine) => engine.handle_key(key),
            EngineState::Table(engine) => engine.handle_key(key),
            EngineState::Logic(engine) => engine.handle_key(key),
            EngineState::Syntax(engine) => engine.handle_key(key),
            EngineState::Html(engine) => engine.handle_key(key),
            EngineState::Lock(engine) => engine.handle_key(key),
            EngineState::Jsonl(engine) => engine.handle_key(key),
            EngineState::Text(engine) => engine.handle_key(key),
            EngineState::Env(engine) => engine.handle_key(key),
            EngineState::Ini(engine) => engine.handle_key(key),
            EngineState::Xml(engine) => engine.handle_key(key),
            EngineState::Dockerfile(engine) => engine.handle_key(key),
            EngineState::Makefile(engine) => engine.handle_key(key),
            EngineState::Log(engine) => engine.handle_key(key),
            EngineState::GitIgnore(engine) => engine.handle_key(key),
            EngineState::Sqlite(engine) => engine.handle_key(key),
            EngineState::Archive(engine) => engine.handle_key(key),
            EngineState::Image(engine) => engine.handle_key(key),
            EngineState::Hex(engine) => engine.handle_key(key),
        }
    }

    pub fn supports_search(&self) -> bool {
        matches!(
            self,
            EngineState::Tree(_)
                | EngineState::Syntax(_)
                | EngineState::Logic(_)
                | EngineState::Table(_)
                | EngineState::Html(_)
                | EngineState::Lock(_)
                | EngineState::Jsonl(_)
                | EngineState::Text(_)
                | EngineState::Env(_)
                | EngineState::Ini(_)
                | EngineState::Xml(_)
                | EngineState::Dockerfile(_)
                | EngineState::Makefile(_)
                | EngineState::Log(_)
                | EngineState::GitIgnore(_)
                | EngineState::Sqlite(_)
                | EngineState::Archive(_)
                | EngineState::Image(_)
                | EngineState::Hex(_)
        )
    }

    pub fn apply_search(&mut self, query: &str) {
        match self {
            EngineState::Tree(engine) => engine.apply_search(query),
            EngineState::Syntax(engine) => engine.apply_search(query),
            EngineState::Logic(engine) => engine.apply_search(query),
            EngineState::Table(engine) => engine.apply_search(query),
            EngineState::Html(engine) => engine.apply_search(query),
            EngineState::Lock(engine) => engine.apply_search(query),
            EngineState::Jsonl(engine) => engine.apply_search(query),
            EngineState::Text(engine) => engine.apply_search(query),
            EngineState::Env(engine) => engine.apply_search(query),
            EngineState::Ini(engine) => engine.apply_search(query),
            EngineState::Xml(engine) => engine.apply_search(query),
            EngineState::Dockerfile(engine) => engine.apply_search(query),
            EngineState::Makefile(engine) => engine.apply_search(query),
            EngineState::Log(engine) => engine.apply_search(query),
            EngineState::GitIgnore(engine) => engine.apply_search(query),
            EngineState::Sqlite(engine) => engine.apply_search(query),
            EngineState::Archive(engine) => engine.apply_search(query),
            EngineState::Image(engine) => engine.apply_search(query),
            EngineState::Hex(engine) => engine.apply_search(query),
        }
    }

    pub fn apply_filter(&mut self, query: &str) {
        match self {
            EngineState::Tree(engine) => engine.apply_filter(query),
            EngineState::Syntax(engine) => engine.apply_filter(query),
            EngineState::Logic(engine) => engine.apply_filter(query),
            EngineState::Table(engine) => engine.apply_filter(query),
            EngineState::Html(engine) => engine.apply_filter(query),
            EngineState::Lock(engine) => engine.apply_filter(query),
            EngineState::Jsonl(engine) => engine.apply_filter(query),
            EngineState::Text(engine) => engine.apply_filter(query),
            EngineState::Env(engine) => engine.apply_filter(query),
            EngineState::Ini(engine) => engine.apply_filter(query),
            EngineState::Xml(engine) => engine.apply_filter(query),
            EngineState::Dockerfile(engine) => engine.apply_filter(query),
            EngineState::Makefile(engine) => engine.apply_filter(query),
            EngineState::Log(engine) => engine.apply_filter(query),
            EngineState::GitIgnore(engine) => engine.apply_filter(query),
            EngineState::Sqlite(engine) => engine.apply_filter(query),
            EngineState::Archive(engine) => engine.apply_filter(query),
            EngineState::Image(engine) => engine.apply_filter(query),
            EngineState::Hex(engine) => engine.apply_filter(query),
        }
    }

    pub fn clear_filter(&mut self) {
        match self {
            EngineState::Tree(engine) => engine.clear_filter(),
            EngineState::Syntax(engine) => engine.clear_filter(),
            EngineState::Logic(engine) => engine.clear_filter(),
            EngineState::Table(engine) => engine.clear_filter(),
            EngineState::Html(engine) => engine.clear_filter(),
            EngineState::Lock(engine) => engine.clear_filter(),
            EngineState::Jsonl(engine) => engine.clear_filter(),
            EngineState::Text(engine) => engine.clear_filter(),
            EngineState::Env(engine) => engine.clear_filter(),
            EngineState::Ini(engine) => engine.clear_filter(),
            EngineState::Xml(engine) => engine.clear_filter(),
            EngineState::Dockerfile(engine) => engine.clear_filter(),
            EngineState::Makefile(engine) => engine.clear_filter(),
            EngineState::Log(engine) => engine.clear_filter(),
            EngineState::GitIgnore(engine) => engine.clear_filter(),
            EngineState::Sqlite(engine) => engine.clear_filter(),
            EngineState::Archive(engine) => engine.clear_filter(),
            EngineState::Image(engine) => engine.clear_filter(),
            EngineState::Hex(engine) => engine.clear_filter(),
        }
    }

    #[allow(dead_code)]
    pub fn selected_path(&self) -> Option<String> {
        match self {
            EngineState::Tree(engine) => engine.selected_path(),
            EngineState::Archive(engine) => engine.selected_path(),
            EngineState::Sqlite(engine) => engine.selected_path(),
            EngineState::GitIgnore(engine) => engine.selected_path(),
            EngineState::Image(engine) => engine.selected_path(),
            EngineState::Hex(engine) => engine.selected_path(),
            _ => None,
        }
    }

    /// Get the content of the currently selected line/row
    pub fn get_selected_line(&self) -> Option<String> {
        match self {
            EngineState::Text(engine) => engine.get_selected_line(),
            EngineState::Syntax(engine) => engine.get_selected_line(),
            EngineState::Tree(engine) => engine.get_selected_line(),
            EngineState::Table(engine) => engine.get_selected_line(),
            EngineState::Logic(engine) => engine.get_selected_line(),
            EngineState::Html(engine) => engine.get_selected_line(),
            EngineState::Lock(engine) => engine.get_selected_line(),
            EngineState::Jsonl(engine) => engine.get_selected_line(),
            EngineState::Env(engine) => engine.get_selected_line(),
            EngineState::Ini(engine) => engine.get_selected_line(),
            EngineState::Xml(engine) => engine.get_selected_line(),
            EngineState::Dockerfile(engine) => engine.get_selected_line(),
            EngineState::Makefile(engine) => engine.get_selected_line(),
            EngineState::Log(engine) => engine.get_selected_line(),
            EngineState::GitIgnore(engine) => engine.get_selected_line(),
            EngineState::Sqlite(engine) => engine.get_selected_line(),
            EngineState::Archive(engine) => engine.get_selected_line(),
            EngineState::Image(engine) => engine.get_selected_line(),
            EngineState::Hex(engine) => engine.get_selected_line(),
        }
    }

    /// Get lines in a range (inclusive), joined by newlines
    pub fn get_lines_range(&self, start: usize, end: usize) -> Option<String> {
        match self {
            EngineState::Text(engine) => engine.get_lines_range(start, end),
            EngineState::Syntax(engine) => engine.get_lines_range(start, end),
            EngineState::Tree(engine) => engine.get_lines_range(start, end),
            EngineState::Table(engine) => engine.get_lines_range(start, end),
            EngineState::Logic(engine) => engine.get_lines_range(start, end),
            EngineState::Html(engine) => engine.get_lines_range(start, end),
            EngineState::Lock(engine) => engine.get_lines_range(start, end),
            EngineState::Jsonl(engine) => engine.get_lines_range(start, end),
            EngineState::Env(engine) => engine.get_lines_range(start, end),
            EngineState::Ini(engine) => engine.get_lines_range(start, end),
            EngineState::Xml(engine) => engine.get_lines_range(start, end),
            EngineState::Dockerfile(engine) => engine.get_lines_range(start, end),
            EngineState::Makefile(engine) => engine.get_lines_range(start, end),
            EngineState::Log(engine) => engine.get_lines_range(start, end),
            EngineState::GitIgnore(engine) => engine.get_lines_range(start, end),
            EngineState::Sqlite(engine) => engine.get_lines_range(start, end),
            EngineState::Archive(engine) => engine.get_lines_range(start, end),
            EngineState::Image(engine) => engine.get_lines_range(start, end),
            EngineState::Hex(engine) => engine.get_lines_range(start, end),
        }
    }

    /// Get current selection index (for visual mode)
    pub fn selection(&self) -> usize {
        match self {
            EngineState::Text(engine) => engine.selection(),
            EngineState::Syntax(engine) => engine.selection(),
            EngineState::Tree(engine) => engine.selection(),
            EngineState::Table(engine) => engine.selection(),
            EngineState::Logic(engine) => engine.selection(),
            EngineState::Html(engine) => engine.selection(),
            EngineState::Lock(engine) => engine.selection(),
            EngineState::Jsonl(engine) => engine.selection(),
            EngineState::Env(engine) => engine.selection(),
            EngineState::Ini(engine) => engine.selection(),
            EngineState::Xml(engine) => engine.selection(),
            EngineState::Dockerfile(engine) => engine.selection(),
            EngineState::Makefile(engine) => engine.selection(),
            EngineState::Log(engine) => engine.selection(),
            EngineState::GitIgnore(engine) => engine.selection(),
            EngineState::Sqlite(engine) => engine.selection(),
            EngineState::Archive(engine) => engine.selection(),
            EngineState::Image(engine) => engine.selection(),
            EngineState::Hex(engine) => engine.selection(),
        }
    }

    pub fn content_height(&mut self) -> usize {
        match self {
            EngineState::Tree(engine) => engine.content_height(),
            EngineState::Table(engine) => engine.content_height(),
            EngineState::Logic(engine) => engine.content_height(),
            EngineState::Syntax(engine) => engine.content_height(),
            EngineState::Html(engine) => engine.content_height(),
            EngineState::Lock(engine) => engine.content_height(),
            EngineState::Jsonl(engine) => engine.content_height(),
            EngineState::Text(engine) => engine.content_height(),
            EngineState::Env(engine) => engine.content_height(),
            EngineState::Ini(engine) => engine.content_height(),
            EngineState::Xml(engine) => engine.content_height(),
            EngineState::Dockerfile(engine) => engine.content_height(),
            EngineState::Makefile(engine) => engine.content_height(),
            EngineState::Log(engine) => engine.content_height(),
            EngineState::GitIgnore(engine) => engine.content_height(),
            EngineState::Sqlite(engine) => engine.content_height(),
            EngineState::Archive(engine) => engine.content_height(),
            EngineState::Image(engine) => engine.content_height(),
            EngineState::Hex(engine) => engine.content_height(),
        }
    }

    pub fn render_plain_lines(&mut self, width: u16) -> Vec<Line<'static>> {
        match self {
            EngineState::Tree(engine) => engine.render_plain_lines(),
            EngineState::Table(engine) => engine.render_plain_lines(width),
            EngineState::Logic(engine) => engine.render_plain_lines(),
            EngineState::Syntax(engine) => engine.render_plain_lines(),
            EngineState::Html(engine) => engine.render_plain_lines(width),
            EngineState::Lock(engine) => engine.render_plain_lines(width),
            EngineState::Jsonl(engine) => engine.render_plain_lines(width),
            EngineState::Text(engine) => engine.render_plain_lines(width),
            EngineState::Env(engine) => engine.render_plain_lines(width),
            EngineState::Ini(engine) => engine.render_plain_lines(width),
            EngineState::Xml(engine) => engine.render_plain_lines(width),
            EngineState::Dockerfile(engine) => engine.render_plain_lines(width),
            EngineState::Makefile(engine) => engine.render_plain_lines(width),
            EngineState::Log(engine) => engine.render_plain_lines(width),
            EngineState::GitIgnore(engine) => engine.render_plain_lines(width),
            EngineState::Sqlite(engine) => engine.render_plain_lines(width),
            EngineState::Archive(engine) => engine.render_plain_lines(width),
            EngineState::Image(engine) => engine.render_plain_lines(width),
            EngineState::Hex(engine) => engine.render_plain_lines(width),
        }
    }
}
