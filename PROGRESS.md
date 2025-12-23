# Progress

## Summary
- Project renamed to vat; CLI name updated; README updated to install via cargo and use vat.
- Engines implemented: TreeEngine (structured), TableEngine (csv/parquet), LogicEngine (configs incl. ssh), SyntaxEngine (code + markdown viewer), HtmlEngine (table-like), LockEngine (lockfiles), JsonlEngine (JSON Lines streaming), EnvEngine (.env files), TextEngine fallback.
- Paging behavior: auto uses plain output if fits, otherwise TUI. Plain output renders boxed header and content with ANSI colors.
- Search prompt `/` works in TUI with high-contrast footer; searches cycle with n/N in all engines.
- Navigation: j/k, gg/G, Ctrl+u/d. Tree has `e` top-level jump; Markdown has `e` next heading. HTML and Tree support Enter fold/unfold.
- Markdown viewer renders headings/lists/code/quotes/links with aligned line numbers.
- Visual tweaks: boxed header in TUI, vertical line between line numbers and content, higher contrast colors.
- SSH view: table-like output with Host, HostName, Port, User, Identity status/path. No masking.
- HTML view: table-like rows with tag/id/class/text, supports collapse.
- Lock view: shows name/version/source/checksum/dependencies for Cargo.lock, package-lock.json, pnpm-lock.yaml/yml.
- Fallback unsupported files to TextEngine.

## Key Files
- `src/app.rs`: paging, plain renderer, TUI header/footer, search prompt styling, raw piping.
- `src/analyzer.rs`: dispatch by extension and special names (Cargo.lock, ssh_config, html, jsonl).
- `src/engines/`: tree/table/logic/syntax/html/lock/jsonl/text implementations.
- `README.md`: updated usage to `vat`.

## Known Behaviors
- Search input is now high-contrast with a visible cursor marker `_`.
- `n` and `N` cycle search results starting after current selection.

## TODOs / Follow-ups
- Confirm `/` search prompt visibility in different terminal themes; adjust colors if needed.
- Consider adding a tiny indicator for search mode in the header (optional).
- Re-run `cargo test` if tests added later; currently only build verified.
- Verify sample files and README are aligned with final feature set.

## Recent Changes
- Added raw output when piping (like `bat`): detects non-TTY stdout and outputs raw file content
- Added `--plain` / `-p` flag for explicit raw output even in TTY
- Raw output uses streaming (`io::copy`) to handle large files efficiently with constant memory
- TextEngine now uses memory-mapped files (mmap) with line offset index for efficient large file handling
  - Only visible lines are read into memory during TUI rendering
  - Line offset index is ~8 bytes per line (vs full line content before)
- TableEngine remains in-memory (Polars DataFrame) since CSV doesn't support random access
  - Note: For truly massive CSV files, consider converting to Parquet first
- Broken pipe errors are now handled gracefully when piping to head/tail
- Added JSONL/NDJSON support (.jsonl, .ndjson extensions)
  - Uses mmap + line offset index for streaming
  - Each line parsed as independent JSON object on demand
  - Supports expand/collapse (Enter) to show full JSON structure
  - Perfect for log files and large data exports
- TreeEngine improvements:
  - Now uses mmap for initial file reading (reduces memory copy)
  - 50MB file size limit with helpful error message
  - Recommends JSONL for large datasets
  - Analyzer no longer reads entire file into memory upfront
- Stdin support: pipe data to vat using `-` as path
  - Auto-detection of format (JSON, JSONL, YAML, CSV, TOML, .env)
  - `-l/--language` flag for explicit format hints
  - Creates temp file for viewing, kept in memory during session
- Filter mode (`f` key): shows only matching lines/nodes
  - TextEngine and JsonlEngine filter lines in-place
  - Other engines use filter as enhanced search
  - `F` clears the active filter
- Help overlay (`?` key): shows all keyboard shortcuts
  - Centered popup with categorized keybindings
  - Press `?`, `Esc`, or `q` to close
- EnvEngine for .env files:
  - Table view with Category, Key, Value columns
  - Auto-categorization (Database, API/Network, Auth/Secret, Cloud, etc.)
  - Secret masking (tokens, passwords, keys hidden by default)
  - `s` key toggles secret visibility
  - Supports .env, .env.*, *.env files

