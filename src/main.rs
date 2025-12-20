use anyhow::Result;
use clap::{Parser, ValueEnum};

mod analyzer;
mod app;
mod engines;

#[derive(Parser, Debug)]
#[command(name = "vat", version, about = "Semantic file viewer")]
struct Args {
    /// Path to the file to view
    path: String,
    /// Paging mode: auto, always, never (bat-compatible)
    #[arg(long, value_enum, default_value = "auto")]
    paging: Paging,
}

#[derive(ValueEnum, Clone, Debug)]
enum Paging {
    Auto,
    Always,
    Never,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let engine = analyzer::analyze(args.path.as_ref())?;
    let mut app = app::App::new(engine, args.path, args.paging.into());
    app.run()
}

impl From<Paging> for app::Paging {
    fn from(value: Paging) -> Self {
        match value {
            Paging::Auto => app::Paging::Auto,
            Paging::Always => app::Paging::Always,
            Paging::Never => app::Paging::Never,
        }
    }
}
