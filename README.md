# Vat

Vat is a semantic file viewer that detects a file's structure and renders it in the most intuitive terminal view.
It uses a dispatcher pattern to route files into specialized engines for structured data, tables, configs, or source code.

## Features

- Dispatcher: extension + magic byte detection
- TreeEngine: JSON/YAML/TOML/KDL with folding, breadcrumbs, type hints
- TableEngine: CSV/TSV/Parquet with virtual scrolling, sticky headers, schema view
- LogicEngine: smart views for .ssh/config, .tmux.conf, .bashrc, crontab with masking
- SyntaxEngine: syntax highlighting, TODO lints, React component sidebar, CSS color swatches, Markdown styling

## Requirements

- Rust 1.70+

## Install

```bash
cargo install --path .
```

## Usage

```bash
vat path/to/file.json
```

## Keybinds

- `j` / `k`: move selection
- `Enter`: fold/unfold (TreeEngine)
- `y`: copy current path (TreeEngine)
- `/`: search (TreeEngine, SyntaxEngine)
- `s`: toggle schema view (TableEngine) or sidebar (SyntaxEngine)
- `q`: quit

## Notes

- Parquet detection uses magic bytes (`PAR1`) when possible.
- SSH IdentityFile entries are expanded and validated.

## Project Structure

- `src/analyzer.rs`: dispatcher logic
- `src/app.rs`: ratatui TUI loop and input handling
- `src/engines/`: engine implementations
- `src/main.rs`: CLI entrypoint
