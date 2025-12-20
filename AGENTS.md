# AGENTS.md: Technical Specification for "Vat"

## 1. Project Overview

**Vat** is a "Semantic File Viewer." It detects the structure of a file and renders it in the most intuitive way possible (Trees for data, Tables for spreadsheets, Component Maps for code, and Visual Cheat-sheets for configs).

## 2. Core Architecture

The tool uses a **Dispatcher Pattern**. Every file passed to Vat goes through the `Analyzer` to determine which `Engine` should handle the render.

### The Dispatcher Logic

1. **Sensing:** Check File Extension + Magic Bytes (to detect binary formats like Parquet).
2. **Logic Branching:**
* **Structured?** (JSON, YAML, TOML)  `TreeEngine`
* **Tabular?** (CSV, Parquet)  `TableEngine`
* **Config?** (.ssh/config, .tmux.conf)  `LogicEngine`
* **Code?** (Rust, JS, CSS)  `SyntaxEngine`



---

## 3. Engine Specifications

### A. TreeEngine (Structured Data)

* **Target:** JSON, YAML, TOML, KDL.
* **Key Features:**
* **Folding:** Ability to collapse/expand nested objects.
* **Breadcrumbs:** Persistent header showing current path (e.g., `root > metadata > tags[2]`).
* **Type Hinting:** Differentiate between strings, numbers, and booleans with distinct icons.



### B. TableEngine (Data Science & Analytics)

* **Target:** CSV, TSV, Parquet, Avro.
* **Key Features:**
* **Virtual Scrolling:** Handle 1,000,000+ rows using `ratatui`'s efficient rendering.
* **Sticky Headers:** First row stays visible during scroll.
* **Schema View:** A toggle to see column types (Int64, String, Timestamp) instead of data.



### C. LogicEngine (System Configs)

* **Target:** `.ssh/config`, `.tmux.conf`, `.bashrc`, `crontab`.
* **Key Features:**
* **SSH:** Verify `IdentityFile` paths; group hosts by domain.
* **Tmux:** Extract and display a "Cheat Sheet" of keybindings.
* **Cron:** Human-readable translation of time strings.
* **Privacy:** Automatic masking of sensitive tokens/IPs.



### D. SyntaxEngine (Source Code)

* **Target:** Rust, TSX, Python, CSS/TCSS.
* **Key Features:**
* **React:** Sidebar listing exported Components and Props.
* **CSS:** Color squares in the gutter for hex/RGB codes.
* **Lints:** Highlight TODOs and potential syntax errors.



---

## 4. Proposed Feature Table (MVP)

| Format | Library | Feature |
| --- | --- | --- |
| **JSON** | `serde_json` | Path breadcrumbs + Folding |
| **CSV** | `polars` | Auto-aligned interactive table |
| **Parquet** | `parquet` | Binary-to-Text table preview |
| **SSH** | `nom` (parser) | Identity file validation |
| **CSS** | `syntect` | Color swatches in gutter |
| **Markdown** | `comrak` | Styled headers and task lists |

---

## 5. Technical Stack

* **Language:** Rust (Edition 2021).
* **UI Framework:** `ratatui` (for the terminal interface).
* **Terminal I/O:** `crossterm`.
* **Syntax Highlighting:** `syntect` (compatible with Sublime themes).
* **Parser:** `tree-sitter` (for React and CSS structural analysis).
* **CLI Parsing:** `clap` (v4).

---

## 6. User Experience (UX) Flow

1. **Command:** `vat config.json`
2. **View:** Opens an interactive TUI.
3. **Keybinds:**
* `j / k`: Move selection.
* `Enter`: Fold/Unfold section.
* `y`: Copy current path (e.g., `user.auth.token`) to clipboard.
* `/`: Search through keys or values.
* `q`: Exit.



---

## 7. Next Steps for Implementation

1. **Phase 1:** Setup `clap` and a basic `ratatui` terminal loop.
2. **Phase 2:** Integrate `syntect` for basic `bat`-like highlighting.
3. **Phase 3:** Build the `TableEngine` using `polars` for CSV/Parquet support.
4. **Phase 4:** Build the `LogicEngine` specifically for `.ssh/config` as the first "Smart Config" example.
