<p align="center">
  <img src="https://img.shields.io/badge/rust-1.70+-orange.svg" alt="Rust 1.70+">
  <img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT License">
  <img src="https://img.shields.io/github/stars/JiwanChung/vat?style=social" alt="GitHub Stars">
</p>

<h1 align="center">vat</h1>

<p align="center">
  <strong>A semantic file viewer for the terminal</strong>
</p>

<p align="center">
  <a href="#installation">Installation</a> •
  <a href="#features">Features</a> •
  <a href="#supported-formats">Formats</a> •
  <a href="#usage">Usage</a> •
  <a href="#keybindings">Keybindings</a>
</p>

<p align="center">
  <img src="demo.gif" alt="vat demo" width="800">
</p>

---

**vat** renders files the way they're meant to be seen. Instead of dumping raw text, it understands your files and presents them semantically — JSON as collapsible trees, CSVs as aligned tables, images as ASCII art, SQLite as browsable databases, and much more.

```bash
$ vat config.json        # Interactive tree view with folding
$ vat users.csv          # Scrollable table with columns
$ vat app.db             # Browse SQLite schemas and data
$ vat backup.tar.gz      # List contents without extracting
$ curl api.com | vat -   # Pipe anything, auto-detect format
```

## Installation

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

## Features

| | |
|---|---|
| **Smart Format Detection** | Automatically picks the best viewer for 20+ file types |
| **Vim-style Navigation** | `j/k`, `gg/G`, `Ctrl+u/d` — feels like home |
| **Collapsible Trees** | Fold/unfold JSON, YAML, TOML, XML, and HTML nodes |
| **Search & Filter** | Find with `/`, filter visible lines with `f` |
| **Visual Selection** | Select ranges with `v`, yank to clipboard with `y` |
| **Clipboard Integration** | Copy lines with `yy`, selections with `vy` |
| **Stdin Support** | Pipe data directly: `curl ... \| vat -l json -` |
| **Pipe-friendly** | Outputs raw content when stdout isn't a TTY |
| **Auto-paging** | Inline for short files, pager for long ones |

## Supported Formats

### Structured Data
| Format | Extensions | Features |
|--------|------------|----------|
| JSON | `.json` | Tree view, collapse/expand, path copying |
| YAML | `.yaml`, `.yml` | Tree view, collapse/expand |
| TOML | `.toml` | Tree view, collapse/expand |
| KDL | `.kdl` | Tree view, collapse/expand |
| XML | `.xml` | Tree structure, attributes, text content |
| HTML | `.html`, `.htm` | DOM tree, element IDs and classes |

### Tabular Data
| Format | Extensions | Features |
|--------|------------|----------|
| CSV | `.csv` | Table view, column alignment, virtual scrolling |
| TSV | `.tsv` | Table view, column alignment |
| Parquet | `.parquet` | Table view, schema inspection |
| JSON Lines | `.jsonl`, `.ndjson` | Record-by-record viewing, expandable objects |

### Databases & Archives
| Format | Extensions | Features |
|--------|------------|----------|
| SQLite | `.db`, `.sqlite`, `.sqlite3` | Schema browser, table data, row navigation |
| ZIP | `.zip` | File listing, sizes, compression ratios |
| TAR | `.tar`, `.tar.gz`, `.tgz` | File listing, permissions |

### Config Files
| Format | Files/Extensions | Features |
|--------|------------------|----------|
| INI | `.ini`, `.cfg`, `.properties`, `.conf` | Sections, key-value pairs |
| Environment | `.env`, `.env.*` | Variable highlighting, secret detection |
| Dockerfile | `Dockerfile`, `Dockerfile.*` | Stage detection, instruction parsing |
| Makefile | `Makefile`, `*.mk` | Targets, dependencies, recipes |
| SSH Config | `.ssh/config` | Host blocks, smart grouping |
| Git Ignore | `.gitignore`, `.dockerignore` | Pattern categorization |

### Lock Files
| Format | Files | Features |
|--------|-------|----------|
| Cargo | `Cargo.lock` | Dependency tree, versions |
| npm | `package-lock.json` | Dependency tree, versions |
| pnpm | `pnpm-lock.yaml` | Dependency tree, versions |

### Source Code
| Languages | Features |
|-----------|----------|
| Rust, JavaScript, TypeScript, Python, CSS, SQL, Markdown | Syntax highlighting, line numbers |

### Binary & Media
| Format | Extensions | Features |
|--------|------------|----------|
| Images | `.jpg`, `.png`, `.gif`, `.webp` | ASCII preview, dimensions, metadata |
| Binary | (auto-detected) | Hex viewer with ASCII column |
| Log files | `.log` | Timestamp parsing, level filtering |

## Usage

```bash
# Basic usage
vat <file>

# Read from stdin with format hint
cat data.json | vat -l json -
curl https://api.example.com/users | vat -l json -

# Paging modes (bat-compatible)
vat --paging=auto file.json     # Auto-detect (default)
vat --paging=always file.json   # Always use TUI
vat --paging=never file.json    # Print and exit

# Plain output (for piping)
vat -p file.json                # Raw output, no formatting
vat file.json | head            # Auto-detects pipe, outputs raw
```

## Keybindings

### Navigation

| Key | Action |
|-----|--------|
| `j` / `↓` | Move down |
| `k` / `↑` | Move up |
| `gg` | Jump to top |
| `G` | Jump to bottom |
| `Ctrl+d` | Half page down |
| `Ctrl+u` | Half page up |

### Search & Filter

| Key | Action |
|-----|--------|
| `/` | Search |
| `f` | Filter (show only matches) |
| `F` | Clear filter |
| `n` | Next match |
| `N` | Previous match |

### Selection & Clipboard

| Key | Action |
|-----|--------|
| `yy` | Yank (copy) current line |
| `v` | Enter visual line mode |
| `y` (visual) | Yank selection |
| `Esc` | Cancel selection |

### View Controls

| Key | Action |
|-----|--------|
| `Enter` | Expand/collapse node |
| `s` | Toggle view mode (schema/data, secrets) |
| `e` | Jump to next section/heading |
| `Tab` | Switch tables (SQLite) |

### Log Viewer

| Key | Action |
|-----|--------|
| `1` | Show DEBUG and above |
| `2` | Show INFO and above |
| `3` | Show WARN and above |
| `4` | Show ERROR only |

### General

| Key | Action |
|-----|--------|
| `?` | Show help |
| `q` | Quit |

## Examples

### Exploring JSON APIs

```bash
curl -s https://api.github.com/repos/rust-lang/rust | vat -l json -
```

Navigate the response with `j/k`, collapse objects with `Enter`, search with `/`.

### Browsing SQLite Databases

```bash
vat mydatabase.sqlite
```

Press `s` to toggle between schema view and table data. Use `Tab` to switch tables.

### Quick CSV Analysis

```bash
vat sales_data.csv
```

Scroll through thousands of rows with smooth navigation. Columns stay aligned.

### Inspecting Docker Images

```bash
vat backup.tar.gz
```

See all files with sizes and permissions without extracting anything.

### Debugging Log Files

```bash
vat application.log
```

Filter by log level with `1-4` keys. Search for errors with `/error`.

## Comparison

| Feature | vat | cat | bat | less | jq |
|---------|:---:|:---:|:---:|:----:|:--:|
| Syntax highlighting | ✓ | | ✓ | | |
| Semantic views | ✓ | | | | ~ |
| Tree navigation | ✓ | | | | |
| Table rendering | ✓ | | | | |
| SQLite browsing | ✓ | | | | |
| Image preview | ✓ | | | | |
| Interactive | ✓ | | | ✓ | |
| Vim keybindings | ✓ | | | ~ | |
| Clipboard | ✓ | | | | |

## Architecture

```
src/
├── main.rs          # CLI entrypoint, stdin handling
├── analyzer.rs      # File type detection & routing
├── app.rs           # TUI loop, input handling, clipboard
└── engines/
    ├── tree.rs      # JSON, YAML, TOML, KDL
    ├── table.rs     # CSV, TSV, Parquet
    ├── syntax.rs    # Source code highlighting
    ├── sqlite.rs    # Database browser
    ├── archive.rs   # ZIP, TAR viewer
    ├── hex.rs       # Binary viewer
    ├── html.rs      # HTML DOM viewer
    ├── xml.rs       # XML tree viewer
    ├── jsonl.rs     # JSON Lines viewer
    ├── log.rs       # Log file viewer
    ├── env.rs       # Environment files
    ├── ini.rs       # INI/Properties files
    ├── dockerfile.rs
    ├── makefile.rs
    ├── gitignore.rs
    ├── lock.rs      # Lock file dependencies
    ├── logic.rs     # Config files (.ssh, .tmux, etc.)
    ├── image.rs     # Image metadata
    └── text.rs      # Plain text fallback
```

## License

MIT

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

---

<p align="center">
  Made with Rust
</p>
