//! Библиотека обоев и настройки. Всё лежит в
//! `%LOCALAPPDATA%\zhivye-oboi`: library.json, settings.json, папки
//! media (скачанное) и thumbs (превью).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Video,
    Animation,
    Image,
    Audio,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Video, Kind::Animation, Kind::Image, Kind::Audio];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Video => "Видео",
            Kind::Animation => "Анимация",
            Kind::Image => "Картинка",
            Kind::Audio => "Звук",
        }
    }

    pub fn from_path(path: &Path) -> Option<Kind> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "mp4" | "m4v" | "mkv" | "webm" | "mov" | "avi" | "wmv" | "ogv" | "flv" | "ts"
            | "m2ts" | "mpg" | "mpeg" | "3gp" => Kind::Video,
            "gif" | "apng" => Kind::Animation,
            "jpg" | "jpeg" | "png" | "bmp" | "webp" | "tif" | "tiff" | "jxl" | "avif" => {
                Kind::Image
            }
            "mp3" | "flac" | "ogg" | "oga" | "opus" | "wav" | "m4a" | "aac" | "wma" | "mka" => {
                Kind::Audio
            }
            _ => return None,
        })
    }

    /// Расширения для окна выбора файлов.
    pub fn extensions() -> &'static [&'static str] {
        &[
            "mp4", "m4v", "mkv", "webm", "mov", "avi", "wmv", "ogv", "flv", "ts", "m2ts", "mpg",
            "mpeg", "3gp", "gif", "apng", "jpg", "jpeg", "png", "bmp", "webp", "tif", "tiff",
            "jxl", "avif", "mp3", "flac", "ogg", "oga", "opus", "wav", "m4a", "aac", "wma", "mka",
        ]
    }
}

/// Откуда взят файл из открытых источников: автор и лицензия нужны,
/// чтобы соблюдать условия CC BY / CC BY-SA.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Source {
    pub site: String,
    pub page: String,
    pub author: String,
    pub license: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub kind: Kind,
    pub file: PathBuf,
    #[serde(default)]
    pub thumb: Option<PathBuf>,
    #[serde(default)]
    pub source: Option<Source>,
    /// Файл скачан программой в её папку — при удалении из библиотеки
    /// его можно удалить с диска.
    #[serde(default)]
    pub owned: bool,
    #[serde(default)]
    pub added: u64,
}

impl Item {
    pub fn exists(&self) -> bool {
        self.file.is_file()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Fit {
    /// Заполнить экран, обрезав лишнее.
    Fill,
    /// Вписать целиком, с полосами.
    Contain,
    /// Растянуть без сохранения пропорций.
    Stretch,
}

impl Fit {
    pub const ALL: [Fit; 3] = [Fit::Fill, Fit::Contain, Fit::Stretch];
    pub fn label(self) -> &'static str {
        match self {
            Fit::Fill => "Заполнить",
            Fit::Contain => "Вписать",
            Fit::Stretch => "Растянуть",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub volume: u8,
    pub muted: bool,
    pub fit: Fit,
    pub pause_on_fullscreen: bool,
    pub hwdec: bool,
    pub autostart: bool,
    /// Монитор (`\\.\DISPLAY1`) → id элемента библиотеки.
    pub screens: HashMap<String, String>,
    pub music: Vec<String>,
    pub music_on: bool,
    pub music_volume: u8,
    pub music_shuffle: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            volume: 60,
            muted: true,
            fit: Fit::Fill,
            pause_on_fullscreen: true,
            hwdec: true,
            autostart: false,
            screens: HashMap::new(),
            music: Vec::new(),
            music_on: false,
            music_volume: 50,
            music_shuffle: false,
        }
    }
}

pub struct Paths {
    pub root: PathBuf,
    pub media: PathBuf,
    pub thumbs: PathBuf,
    pub cache: PathBuf,
}

impl Paths {
    pub fn new() -> Paths {
        let root = dirs::data_local_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("zhivye-oboi");
        let p = Paths {
            media: root.join("media"),
            thumbs: root.join("thumbs"),
            cache: root.join("cache"),
            root,
        };
        for d in [&p.root, &p.media, &p.thumbs, &p.cache] {
            let _ = fs::create_dir_all(d);
        }
        p
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn new_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}")
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Пишет через временный файл, чтобы сбой не оставил полфайла.
fn write_json<T: Serialize>(path: &Path, value: &T) {
    let Ok(bytes) = serde_json::to_vec_pretty(value) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, bytes).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

pub struct Library {
    pub items: Vec<Item>,
    file: PathBuf,
}

impl Library {
    pub fn load(paths: &Paths) -> Library {
        let file = paths.root.join("library.json");
        Library {
            items: read_json(&file),
            file,
        }
    }

    pub fn save(&self) {
        write_json(&self.file, &self.items);
    }

    pub fn get(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Item> {
        self.items.iter_mut().find(|i| i.id == id)
    }

    /// Добавляет файл с диска. Файл остаётся на месте, в библиотеку
    /// попадает ссылка на него. Повторно тот же путь не добавляется.
    pub fn add_local(&mut self, path: &Path) -> Option<String> {
        let kind = Kind::from_path(path)?;
        if let Some(existing) = self.items.iter().find(|i| i.file == path) {
            return Some(existing.id.clone());
        }
        let title = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Без названия".into());
        let id = new_id();
        self.items.push(Item {
            id: id.clone(),
            title,
            kind,
            file: path.to_path_buf(),
            thumb: None,
            source: None,
            owned: false,
            added: now(),
        });
        Some(id)
    }

    pub fn push(&mut self, item: Item) {
        self.items.push(item);
    }

    /// Убирает из библиотеки; скачанные программой файлы и превью удаляет.
    pub fn remove(&mut self, id: &str) {
        if let Some(pos) = self.items.iter().position(|i| i.id == id) {
            let item = self.items.remove(pos);
            if item.owned {
                let _ = fs::remove_file(&item.file);
            }
            if let Some(t) = item.thumb {
                let _ = fs::remove_file(t);
            }
        }
    }
}

pub fn load_settings(paths: &Paths) -> Settings {
    read_json(&paths.root.join("settings.json"))
}

pub fn save_settings(paths: &Paths, s: &Settings) {
    write_json(&paths.root.join("settings.json"), s);
}

/// Имя файла без символов, которые Windows не принимает.
pub fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches('.');
    let mut out: String = trimmed.chars().take(120).collect();
    if out.is_empty() {
        out = "file".into();
    }
    out
}

/// Свободное имя в папке: «a.webm», «a (2).webm», …
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let p = Path::new(name);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = p.extension().and_then(|s| s.to_str());
    for n in 2.. {
        let file = match ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        let c = dir.join(file);
        if !c.exists() {
            return c;
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_by_extension() {
        assert_eq!(Kind::from_path(Path::new("a.MP4")), Some(Kind::Video));
        assert_eq!(Kind::from_path(Path::new("a.gif")), Some(Kind::Animation));
        assert_eq!(Kind::from_path(Path::new("a.flac")), Some(Kind::Audio));
        assert_eq!(Kind::from_path(Path::new("a.png")), Some(Kind::Image));
        assert_eq!(Kind::from_path(Path::new("a.txt")), None);
    }

    #[test]
    fn safe_names() {
        assert_eq!(safe_file_name("a:b/c?.webm"), "a_b_c_.webm");
        assert_eq!(safe_file_name("  ...  "), "file");
    }

    #[test]
    fn unique_names() {
        let dir = std::env::temp_dir().join(format!("oboi-uniq-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.webm"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "a.webm"), dir.join("a (2).webm"));
        assert_eq!(unique_path(&dir, "b.webm"), dir.join("b.webm"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_survive_missing_fields() {
        let s: Settings = serde_json::from_str(r#"{"volume": 10}"#).unwrap();
        assert_eq!(s.volume, 10);
        assert!(s.pause_on_fullscreen);
    }
}
