//! Snapshot tests over the `examples/` fixtures. Each renders an engine's plain
//! output to text (styles stripped) and compares it against a committed
//! snapshot, catching unintended rendering regressions across engines.
//!
//! Update snapshots with `cargo insta review` (or `INSTA_UPDATE=always`).

use crate::analyzer;
use std::path::Path;

/// Analyze a fixture and render its plain output to text (span contents joined).
fn render(path: &str) -> String {
    let mut engine = analyzer::analyze(Path::new(path)).expect("analyze fixture");
    engine
        .render_plain_lines(80)
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

macro_rules! snapshot_fixture {
    ($name:ident, $path:expr) => {
        #[test]
        fn $name() {
            insta::assert_snapshot!(render($path));
        }
    };
}

// Deterministic, text-based fixtures (no timestamps / variable metadata).
snapshot_fixture!(json, "examples/sample.json");
snapshot_fixture!(yaml, "examples/sample.yaml");
snapshot_fixture!(toml, "examples/sample.toml");
snapshot_fixture!(csv, "examples/sample.csv");
snapshot_fixture!(ini, "examples/sample.ini");
snapshot_fixture!(markdown, "examples/sample.md");
snapshot_fixture!(env, "examples/sample.env");
snapshot_fixture!(python, "examples/sample.py");
