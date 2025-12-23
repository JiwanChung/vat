use std::path::Path;

use anyhow::Result;

use crate::engines::{
    ArchiveEngine, DockerfileEngine, EngineState, EnvEngine, GitIgnoreEngine, HexEngine,
    HtmlEngine, ImageEngine, IniEngine, JsonlEngine, LockEngine, LogEngine, LogicEngine,
    MakefileEngine, SqliteEngine, SyntaxEngine, TableEngine, TextEngine, TreeEngine, XmlEngine,
};

pub fn analyze(path: &Path) -> Result<EngineState> {
    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();

    // Check for parquet magic bytes (need to read first 4 bytes)
    if ext == "parquet" || is_parquet_file(path) {
        return TableEngine::from_path(path).map(EngineState::Table);
    }

    if matches!(ext.as_str(), "csv" | "tsv") {
        return TableEngine::from_path(path).map(EngineState::Table);
    }

    // JSONL / NDJSON (JSON Lines) - each line is a separate JSON object
    if matches!(ext.as_str(), "jsonl" | "ndjson") {
        return JsonlEngine::from_path(path).map(EngineState::Jsonl);
    }

    // Structured data formats - uses mmap + size checking
    if matches!(ext.as_str(), "json" | "yaml" | "yml" | "toml" | "kdl") {
        return TreeEngine::from_path(path).map(EngineState::Tree);
    }

    // XML files
    if ext == "xml" {
        return XmlEngine::from_path(path).map(EngineState::Xml);
    }

    // SQLite database files
    if matches!(ext.as_str(), "db" | "sqlite" | "sqlite3") {
        return SqliteEngine::from_path(path).map(EngineState::Sqlite);
    }

    // Archive files
    if matches!(ext.as_str(), "zip" | "tar" | "tgz") || file_name.ends_with(".tar.gz") {
        return ArchiveEngine::from_path(path).map(EngineState::Archive);
    }

    // Image files
    if matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "ico") {
        return ImageEngine::from_path(path).map(EngineState::Image);
    }

    // INI/Properties config files
    if matches!(ext.as_str(), "ini" | "cfg" | "properties" | "conf") {
        return IniEngine::from_path(path).map(EngineState::Ini);
    }

    // Dockerfile
    if file_name == "Dockerfile" || file_name.starts_with("Dockerfile.") {
        return DockerfileEngine::from_path(path).map(EngineState::Dockerfile);
    }

    // Makefile
    if file_name == "Makefile" || file_name == "makefile" || file_name == "GNUmakefile" || ext == "mk" {
        return MakefileEngine::from_path(path).map(EngineState::Makefile);
    }

    // Log files
    if ext == "log" {
        return LogEngine::from_path(path).map(EngineState::Log);
    }

    // GitIgnore and similar
    if file_name == ".gitignore" || file_name == ".dockerignore" || file_name == ".npmignore" {
        return GitIgnoreEngine::from_path(path).map(EngineState::GitIgnore);
    }

    if is_logic_file(path, file_name) {
        return LogicEngine::from_path(path).map(EngineState::Logic);
    }

    if is_lock_file(path, file_name) {
        return LockEngine::from_path(path).map(EngineState::Lock);
    }

    // .env files and similar environment configs
    if is_env_file(file_name, &ext) {
        return EnvEngine::from_path(path).map(EngineState::Env);
    }

    if matches!(ext.as_str(), "html" | "htm") {
        return HtmlEngine::from_path(path).map(EngineState::Html);
    }

    if is_code_ext(&ext) {
        return SyntaxEngine::from_path(path).map(EngineState::Syntax);
    }

    // Check if binary file - fallback to hex viewer
    if is_binary_file(path) {
        return HexEngine::from_path(path).map(EngineState::Hex);
    }

    TextEngine::from_path(path).map(EngineState::Text)
}

fn is_parquet_file(path: &Path) -> bool {
    use std::fs::File;
    use std::io::Read;

    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut magic = [0u8; 4];
    if file.read_exact(&mut magic).is_err() {
        return false;
    }
    &magic == b"PAR1"
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

fn is_env_file(file_name: &str, ext: &str) -> bool {
    file_name == ".env"
        || file_name.starts_with(".env.")
        || ext == "env"
        || file_name.ends_with(".env")
}

fn is_code_ext(ext: &str) -> bool {
    matches!(
        ext,
        "rs" | "js" | "jsx" | "ts" | "tsx" | "py" | "css" | "tcss" | "md" | "sql"
    )
}

fn is_binary_file(path: &Path) -> bool {
    use std::fs::File;
    use std::io::Read;

    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };

    let mut buffer = [0u8; 8192];
    let bytes_read = match file.read(&mut buffer) {
        Ok(n) => n,
        Err(_) => return false,
    };

    // Check for null bytes or high proportion of non-printable characters
    let mut null_count = 0;
    let mut non_text_count = 0;

    for &byte in &buffer[..bytes_read] {
        if byte == 0 {
            null_count += 1;
        }
        // Non-printable and non-whitespace
        if byte < 0x09 || (byte > 0x0D && byte < 0x20) || byte == 0x7F {
            non_text_count += 1;
        }
    }

    // If there are any null bytes, likely binary
    if null_count > 0 {
        return true;
    }

    // If more than 30% non-text characters, likely binary
    if bytes_read > 0 && non_text_count * 100 / bytes_read > 30 {
        return true;
    }

    false
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
