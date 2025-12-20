use crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::text::Line;

mod logic;
mod syntax;
mod table;
mod tree;
mod html;
mod lock;
mod text;

pub use logic::LogicEngine;
pub use syntax::SyntaxEngine;
pub use table::TableEngine;
pub use tree::TreeEngine;
pub use html::HtmlEngine;
pub use lock::LockEngine;
pub use text::TextEngine;

pub enum EngineState {
    Tree(TreeEngine),
    Table(TableEngine),
    Logic(LogicEngine),
    Syntax(SyntaxEngine),
    Html(HtmlEngine),
    Lock(LockEngine),
    Text(TextEngine),
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
            EngineState::Text(_) => "TextEngine",
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
            EngineState::Text(engine) => engine.breadcrumbs(),
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
            EngineState::Text(engine) => engine.status_line(),
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
            EngineState::Text(engine) => engine.render(frame, area),
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
            EngineState::Text(engine) => engine.handle_key(key),
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
                | EngineState::Text(_)
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
            EngineState::Text(engine) => engine.apply_search(query),
        }
    }

    pub fn selected_path(&self) -> Option<String> {
        match self {
            EngineState::Tree(engine) => engine.selected_path(),
            _ => None,
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
            EngineState::Text(engine) => engine.content_height(),
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
            EngineState::Text(engine) => engine.render_plain_lines(width),
        }
    }
}
