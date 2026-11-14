//! Журнал в `%LOCALAPPDATA%\zhivye-oboi\log.txt`: окно программы без
//! консоли, поэтому всё, что пошло не так, должно остаться в файле.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

pub fn path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("zhivye-oboi")
        .join("log.txt")
}

pub fn write(msg: &str) {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Журнал не растёт бесконечно: больше 1 МБ — начинаем заново.
    if std::fs::metadata(&p)
        .map(|m| m.len() > 1_048_576)
        .unwrap_or(false)
    {
        let _ = std::fs::remove_file(&p);
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "[{}] {msg}", crate::library::now());
    }
}

/// Последние строки журнала — для отчёта `--diagnose`.
pub fn tail(lines: usize) -> String {
    let text = std::fs::read_to_string(path()).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}
