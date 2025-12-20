use std::path::Path;

use anyhow::Result;

use crate::engines::{EngineState, HtmlEngine, LockEngine, LogicEngine, SyntaxEngine, TableEngine, TreeEngine, TextEngine};

pub fn analyze(path: &Path) -> Result<EngineState> {
    let bytes = std::fs::read(path)?;
    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();

    if is_parquet(&bytes) || ext == "parquet" {
        return TableEngine::from_path(path).map(EngineState::Table);
    }

    if matches!(ext.as_str(), "csv" | "tsv") {
        return TableEngine::from_path(path).map(EngineState::Table);
    }

    if matches!(ext.as_str(), "json" | "yaml" | "yml" | "toml" | "kdl") {
        return TreeEngine::from_bytes(path, &bytes).map(EngineState::Tree);
    }

    if is_logic_file(path, file_name) {
        return LogicEngine::from_path(path).map(EngineState::Logic);
    }

    if is_lock_file(path, file_name) {
        return LockEngine::from_path(path).map(EngineState::Lock);
    }

    if matches!(ext.as_str(), "html" | "htm") {
        return HtmlEngine::from_path(path).map(EngineState::Html);
    }

    if is_code_ext(&ext) {
        return SyntaxEngine::from_path(path).map(EngineState::Syntax);
    }

    TextEngine::from_path(path).map(EngineState::Text)
}

fn is_parquet(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && &bytes[0..4] == b"PAR1"
}

fn is_logic_file(path: &Path, file_name: &str) -> bool {
    if file_name == ".tmux.conf"
        || file_name == ".bashrc"
        || file_name == "crontab"
        || file_name == "ssh_config"
    {
        return true;
    }
    if file_name == "config" {
        if let Some(parent) = path.parent() {
            if parent.ends_with(".ssh") {
                return true;
            }
        }
    }
    false
}

fn is_lock_file(_path: &Path, file_name: &str) -> bool {
    file_name == "Cargo.lock"
        || file_name == "package-lock.json"
        || file_name == "pnpm-lock.yaml"
        || file_name == "pnpm-lock.yml"
}

fn is_code_ext(ext: &str) -> bool {
    matches!(
        ext,
        "rs" | "js" | "jsx" | "ts" | "tsx" | "py" | "css" | "tcss" | "md"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn write_temp_file(name: &str, contents: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let name_path = std::path::Path::new(name);
        let stem = name_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("data");
        let ext = name_path.extension().and_then(|s| s.to_str());
        let file_name = if let Some(ext) = ext {
            format!("lens_test_{}_{}.{}", stem, suffix, ext)
        } else {
            format!("lens_test_{}_{}", stem, suffix)
        };
        path.push(file_name);
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn detects_tree_engine() {
        let path = write_temp_file("data.json", r#"{"a": 1}"#);
        let engine = analyze(&path).unwrap();
        assert!(matches!(engine, EngineState::Tree(_)));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn detects_table_engine() {
        let path = write_temp_file("data.csv", "a,b\n1,2\n");
        let engine = analyze(&path).unwrap();
        assert!(matches!(engine, EngineState::Table(_)));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn detects_logic_engine() {
        let mut dir = std::env::temp_dir();
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        dir.push(format!("lens_test_ssh_{}", suffix));
        let ssh_dir = dir.join(".ssh");
        fs::create_dir_all(&ssh_dir).unwrap();
        let path = ssh_dir.join("config");
        fs::write(&path, "Host example.com\n").unwrap();
        let engine = analyze(&path).unwrap();
        assert!(matches!(engine, EngineState::Logic(_)));
        let _ = fs::remove_file(path);
    }
}
