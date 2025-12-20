# Progress

## Summary
- Project renamed to vat; CLI name updated; README updated to install via cargo and use vat.
- Engines implemented: TreeEngine (structured), TableEngine (csv/parquet), LogicEngine (configs incl. ssh), SyntaxEngine (code + markdown viewer), HtmlEngine (table-like), LockEngine (lockfiles), TextEngine fallback.
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
- `src/app.rs`: paging, plain renderer, TUI header/footer, search prompt styling.
- `src/analyzer.rs`: dispatch by extension and special names (Cargo.lock, ssh_config, html).
- `src/engines/`: tree/table/logic/syntax/html/lock/text implementations.
- `README.md`: updated usage to `vat`.

## Known Behaviors
- Search input is now high-contrast with a visible cursor marker `_`.
- `n` and `N` cycle search results starting after current selection.

## TODOs / Follow-ups
- Confirm `/` search prompt visibility in different terminal themes; adjust colors if needed.
- Consider adding a tiny indicator for search mode in the header (optional).
- Re-run `cargo test` if tests added later; currently only build verified.
- Verify sample files and README are aligned with final feature set.

