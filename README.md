# Vat

A semantic file viewer that detects a file's structure and renders it in the most intuitive terminal view.
Uses a dispatcher pattern to route files into specialized engines for structured data, tables, configs, archives, databases, and source code.

## Install

### From GitHub (recommended)

```bash
cargo install --git https://github.com/JiwanChung/vat
```

### From source

```bash
git clone https://github.com/JiwanChung/vat
cd vat
cargo install --path .
```

### Requirements

- Rust 1.70+

## Usage

```bash
vat path/to/file.json
vat data.csv
vat config.yaml
vat database.sqlite
vat archive.zip
cat file.json | vat -l json   # stdin with language hint
```

## Supported Formats

| Engine | Formats | Features |
|--------|---------|----------|
| **TreeEngine** | JSON, YAML, TOML, KDL | Folding, breadcrumbs, type hints, path copying |
| **TableEngine** | CSV, TSV, Parquet | Virtual scrolling, sticky headers, schema view |
| **SyntaxEngine** | RS, JS, TS, PY, CSS, MD, SQL, etc. | Syntax highlighting, TODO lints, Markdown rendering |
| **LogicEngine** | .ssh/config, .tmux.conf, .bashrc, crontab | Smart views with secret masking |
| **LockEngine** | Cargo.lock, package-lock.json, pnpm-lock.yaml | Dependency tree view |
| **HtmlEngine** | HTML | DOM tree with tag/attribute display |
| **EnvEngine** | .env files | Categorized view with secret masking |
| **IniEngine** | INI, CFG, Properties, CONF | Section/key-value parsing |
| **XmlEngine** | XML | Collapsible tree view |
| **DockerfileEngine** | Dockerfile | Stage detection, instruction parsing |
| **MakefileEngine** | Makefile | Target/recipe parsing, .PHONY markers |
| **LogEngine** | .log files | Timestamp/level parsing, level filtering (1-4 keys) |
| **GitIgnoreEngine** | .gitignore, .dockerignore | Pattern categorization |
| **SqliteEngine** | SQLite databases | Schema view, data preview, table switching |
| **ArchiveEngine** | ZIP, TAR, TAR.GZ | File listing, compression ratios |
| **ImageEngine** | JPG, PNG, GIF, WebP | Metadata viewer (dimensions, color, compression) |
| **HexEngine** | Binary files | Hex/ASCII view with lazy loading |
| **JsonlEngine** | JSONL, NDJSON | Line-by-line JSON viewing |

## Keybinds

### Navigation
- `j` / `k` or Arrow keys: Move selection
- `gg` / `G`: Jump to top/bottom
- `Ctrl+u` / `Ctrl+d`: Half-page up/down

### Actions
- `Enter`: Fold/unfold (tree views)
- `y`: Copy current path/value
- `/`: Search
- `f`: Filter mode (show only matching lines)
- `F`: Clear filter
- `n` / `N`: Next/previous search match
- `s`: Toggle view mode (schema/preview, secrets, sidebar)
- `e`: Jump to next section/error/stage
- `?`: Show help overlay
- `q`: Quit

### Log Engine Specific
- `1`: Show DEBUG and above
- `2`: Show INFO and above
- `3`: Show WARN and above
- `4`: Show ERROR only

### SQLite Engine Specific
- `Tab` / `Shift+Tab`: Switch between tables
- `s`: Toggle schema/data view

## Piping

Vat supports bat-like piping behavior:
- When stdout is a TTY: Interactive TUI mode
- When piped: Raw file output (for use with `grep`, `head`, etc.)
- `--paging=never`: Force plain output mode
- `--paging=always`: Force TUI mode
- `-p` / `--plain`: Force raw output

## Project Structure

```
src/
├── main.rs          # CLI entrypoint
├── analyzer.rs      # File type dispatcher
├── app.rs           # TUI loop and input handling
└── engines/         # Specialized viewers
    ├── tree.rs      # JSON/YAML/TOML/KDL
    ├── table.rs     # CSV/TSV/Parquet
    ├── syntax.rs    # Source code
    ├── sqlite.rs    # SQLite databases
    ├── archive.rs   # ZIP/TAR archives
    ├── hex.rs       # Binary files
    └── ...          # Other engines
```

## License

MIT
