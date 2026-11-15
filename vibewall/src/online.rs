//! Открытые ресурсы: общий поиск по источникам из `sources/` и скачивание в
//! библиотеку. Источники — только с официальным API и свободной или
//! бесплатной лицензией (Pexels, Pixabay, NASA, Викисклад, Openverse).
//! Автор и лицензия каждого файла сохраняются в библиотеке.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::Duration;

use crate::library::{safe_file_name, unique_path, Kind};
use crate::sources::{self, Site};

pub const UA: &str = concat!(
    "Vibewall/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/vdoma88/tor2vid)"
);
pub const PAGE: u32 = 24;

/// Как получить сам файл: прямая ссылка или (NASA) список файлов записи,
/// из которого ссылка выбирается при скачивании.
#[derive(Clone, Debug, PartialEq)]
pub enum Fetch {
    Direct(String),
    NasaCollection(String),
    /// Альбом нетлейбла в Internet Archive: идентификатор записи.
    ArchiveAlbum(String),
}

/// Один файл к скачиванию: ссылка, название и размер (0 — неизвестен).
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub url: String,
    pub title: String,
    pub size: u64,
}

/// Скачанное: файлы с названиями и общее превью.
pub struct Downloaded {
    pub files: Vec<(PathBuf, String)>,
    pub thumb: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct Remote {
    pub site: Site,
    pub title: String,
    pub page: String,
    pub fetch: Fetch,
    pub thumb: Option<String>,
    pub kind: Kind,
    /// Байты; 0 — источник размер не сообщает.
    pub size: u64,
    pub width: u32,
    pub height: u32,
    /// Секунды; 0 — неизвестно или картинка.
    pub duration: f64,
    pub author: String,
    pub license: String,
    /// Популярность: скачивания (у нетлейблов), 0 — неизвестно.
    pub plays: u64,
}

pub struct SearchResult {
    pub items: Vec<Remote>,
    /// Следующая страница (номер или смещение — как у источника).
    pub next: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Any,
    FullHd,
    Uhd,
}

impl Quality {
    pub const ALL: [Quality; 3] = [Quality::Any, Quality::FullHd, Quality::Uhd];
    pub fn label(self) -> &'static str {
        match self {
            Quality::Any => "Любое",
            Quality::FullHd => "Full HD",
            Quality::Uhd => "4K",
        }
    }
    /// Наименьшая ширина кадра.
    pub fn min_width(self) -> u32 {
        match self {
            Quality::Any => 0,
            Quality::FullHd => 1920,
            Quality::Uhd => 3840,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Query {
    pub site: Site,
    pub kind: Kind,
    pub text: String,
    pub quality: Quality,
    /// Только короткие ролики (до 30 с) — они лучше всего зацикливаются.
    pub short_loops: bool,
    pub page: u32,
    pub key: String,
    pub sort: Sort,
    /// Только «выбор редакции» (Pixabay).
    pub editors_choice: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    /// Самое популярное — тренды.
    Popular,
    /// Самое новое.
    Latest,
}

impl Sort {
    pub const ALL: [Sort; 2] = [Sort::Popular, Sort::Latest];
    pub fn label(self) -> &'static str {
        match self {
            Sort::Popular => "В тренде",
            Sort::Latest => "Новое",
        }
    }
}

pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .user_agent(UA)
        .build()
        .into()
}

pub fn describe(site: &str, err: ureq::Error) -> String {
    match err {
        ureq::Error::StatusCode(429) => {
            format!("{site} просит подождать: слишком много запросов. Попробуйте через минуту.")
        }
        ureq::Error::StatusCode(code @ (401 | 403)) => {
            if site == "Pexels" || site == "Pixabay" {
                format!("{site} не принял ключ API (код {code}) — проверьте его.")
            } else {
                format!("{site} закрыл доступ (код {code}), обычно на время. Попробуйте позже.")
            }
        }
        ureq::Error::StatusCode(code) => format!("{site} ответил кодом {code}"),
        other => format!("Нет связи с {site}: {other}"),
    }
}

/// GET с параметрами и необязательным заголовком, ответ — JSON.
pub fn get_json(
    site: &str,
    url: &str,
    params: &[(&str, String)],
    header: Option<(&str, &str)>,
) -> Result<serde_json::Value, String> {
    let url = encode_url(url);
    let mut req = agent().get(&url);
    for (k, v) in params {
        req = req.query(k, v);
    }
    if let Some((k, v)) = header {
        req = req.header(k, v);
    }
    let mut resp = req.call().map_err(|e| describe(site, e))?;
    resp.body_mut()
        .read_json()
        .map_err(|e| format!("Непонятный ответ {site}: {e}"))
}

pub fn search(q: &Query) -> Result<SearchResult, String> {
    if !q.site.kinds().contains(&q.kind) {
        return Err(format!(
            "{} не ищет «{}».",
            q.site.label(),
            q.kind.label().to_lowercase()
        ));
    }
    if q.site.needs_key() && q.key.trim().is_empty() {
        return Err(format!("Для {} нужен бесплатный ключ API.", q.site.label()));
    }
    let mut r = sources::search(q)?;
    if q.short_loops && matches!(q.kind, Kind::Video | Kind::Animation) {
        r.items.retain(|i| i.duration <= 0.0 || i.duration <= 30.0);
    }
    Ok(r)
}

pub fn spawn_search(
    q: Query,
    wake: impl Fn() + Send + 'static,
) -> Receiver<Result<SearchResult, String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(search(&q));
        wake();
    });
    rx
}

/// Скачивает по ссылке в файл, через `.part`, с прогрессом и отменой.
fn fetch(
    url: &str,
    target: &Path,
    done: &AtomicU64,
    total: &AtomicU64,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let resp = agent()
        .get(&encode_url(url))
        .call()
        .map_err(|e| describe("сервер", e))?;
    if let Some(len) = resp.body().content_length() {
        total.store(len, Ordering::Relaxed);
    }
    let mut reader = resp.into_body().into_reader();
    let part = target.with_extension("part");
    let mut file = File::create(&part).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(file);
            let _ = fs::remove_file(&part);
            return Err("Отменено".into());
        }
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("Обрыв загрузки: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done.fetch_add(n as u64, Ordering::Relaxed);
    }
    drop(file);
    fs::rename(&part, target).map_err(|e| e.to_string())
}

pub struct Download {
    pub remote: Remote,
    pub done: Arc<AtomicU64>,
    pub total: Arc<AtomicU64>,
    pub cancel: Arc<AtomicBool>,
    pub result: Receiver<Result<Downloaded, String>>,
}

impl Download {
    pub fn progress(&self) -> f32 {
        let total = self.total.load(Ordering::Relaxed).max(self.remote.size);
        if total == 0 {
            return 0.0;
        }
        (self.done.load(Ordering::Relaxed) as f32 / total as f32).min(1.0)
    }
}

/// Имя файла: название записи плюс расширение из ссылки — у Pexels и
/// NASA в ссылках только номера.
fn file_name(title: &str, url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let last = path.rsplit('/').next().unwrap_or("");
    let ext = last
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .filter(|e| e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric()));
    let base = if title.trim().is_empty() {
        percent_decode(last)
    } else {
        title.chars().take(80).collect()
    };
    let base = safe_file_name(&base);
    match ext {
        Some(e) if !base.to_ascii_lowercase().ends_with(&format!(".{e}")) => format!("{base}.{e}"),
        _ => base,
    }
}

pub fn spawn_download(
    remote: Remote,
    quality: Quality,
    media: PathBuf,
    thumbs: PathBuf,
    id: String,
    wake: impl Fn() + Send + 'static,
) -> Download {
    let done = Arc::new(AtomicU64::new(0));
    let total = Arc::new(AtomicU64::new(remote.size));
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, result) = channel();
    let (d, t, c, r) = (done.clone(), total.clone(), cancel.clone(), remote.clone());
    std::thread::spawn(move || {
        let res = sources::resolve(&r, quality).and_then(|tracks| {
            let known: u64 = tracks.iter().map(|x| x.size).sum();
            if known > 0 {
                t.store(known, Ordering::Relaxed);
            }
            // Альбом — в свою папку, одиночный файл — прямо в media.
            let dir = if tracks.len() > 1 {
                let dir = unique_path(&media, &safe_file_name(&r.title));
                fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                dir
            } else {
                media.clone()
            };
            let mut files = Vec::new();
            for tr in &tracks {
                let target = unique_path(&dir, &file_name(&tr.title, &tr.url));
                // У одиночного файла размер берётся из ответа сервера.
                let total_probe = AtomicU64::new(0);
                let total = if tracks.len() > 1 { &total_probe } else { &*t };
                fetch(&tr.url, &target, &d, total, &c)?;
                files.push((target, tr.title.clone()));
            }
            let thumb = r.thumb.as_ref().and_then(|url| {
                let path = thumbs.join(format!("{id}.jpg"));
                let (a, b, n) = (AtomicU64::new(0), AtomicU64::new(0), AtomicBool::new(false));
                fetch(url, &path, &a, &b, &n).ok().map(|_| path)
            });
            Ok(Downloaded { files, thumb })
        });
        let _ = tx.send(res);
        wake();
    });
    Download {
        remote,
        done,
        total,
        cancel,
        result,
    }
}

/// Превью из результатов поиска кладётся в кэш, чтобы egui читал его с
/// диска, а не тянул из сети на каждом кадре.
pub fn spawn_thumb_cache(url: String, path: PathBuf, wake: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        let (a, b, n) = (AtomicU64::new(0), AtomicU64::new(0), AtomicBool::new(false));
        if fetch(&url, &path, &a, &b, &n).is_ok() {
            wake();
        }
    });
}

/// Кодирует в ссылке то, что нельзя передавать как есть: у NASA в путях
/// бывают пробелы («…/First Images from…~large.jpg»). Уже закодированное
/// (`%20`) и служебные символы адреса не трогает.
pub fn encode_url(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    for b in url.bytes() {
        let keep = b.is_ascii_alphanumeric() || b"-._~:/?#[]@!$&'()*+,;=%".contains(&b);
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'_' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// HTML из метаданных → обычный текст не длиннее 80 знаков.
pub fn plain(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">");
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() > 80 {
        out.chars().take(80).collect::<String>() + "…"
    } else {
        out
    }
}

pub fn human_size(bytes: u64) -> String {
    let mb = bytes as f64 / 1_048_576.0;
    if mb >= 1024.0 {
        format!("{:.1} ГБ", mb / 1024.0)
    } else if mb >= 1.0 {
        format!("{mb:.0} МБ")
    } else {
        format!("{} КБ", (bytes / 1024).max(1))
    }
}

/// 992546 → «993 тыс.», 1500000 → «1,5 млн».
pub fn human_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1} млн", n as f64 / 1e6).replace('.', ",")
    } else if n >= 1000 {
        format!("{} тыс.", (n + 500) / 1000)
    } else {
        n.to_string()
    }
}

pub fn human_duration(sec: f64) -> String {
    let s = sec.round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_file_names() {
        assert_eq!(
            percent_decode("Ocean_in_Sommar%C3%B8y.webm"),
            "Ocean in Sommarøy.webm"
        );
        assert_eq!(percent_decode("bad%zz%"), "bad%zz%");
    }

    #[test]
    fn names_downloads_by_title() {
        assert_eq!(
            file_name(
                "Neon city",
                "https://videos.pexels.com/video-files/1/1-uhd_3840_2160_25fps.mp4"
            ),
            "Neon city.mp4"
        );
        assert_eq!(
            file_name(
                "A: b?",
                "https://images.pexels.com/photos/2/p.jpeg?auto=compress&w=1920"
            ),
            "A_ b_.jpeg"
        );
        assert_eq!(file_name("clip.webm", "https://x/clip.webm"), "clip.webm");
        assert_eq!(file_name("", "https://x/Some_file.ogv"), "Some file.ogv");
    }

    #[test]
    fn encodes_spaces_in_urls() {
        assert_eq!(
            encode_url("https://images-assets.nasa.gov/video/A B/A B~large.jpg"),
            "https://images-assets.nasa.gov/video/A%20B/A%20B~large.jpg"
        );
        assert_eq!(
            encode_url("https://x/a%20b?q=1&w=2"),
            "https://x/a%20b?q=1&w=2"
        );
        assert_eq!(encode_url("https://x/ё"), "https://x/%D1%91");
    }

    #[test]
    fn counts_for_humans() {
        assert_eq!(human_count(992_546), "993 тыс.");
        assert_eq!(human_count(1_500_000), "1,5 млн");
        assert_eq!(human_count(42), "42");
    }

    #[test]
    fn strips_html() {
        assert_eq!(plain("<a href=\"x\">Jane &amp; Co</a>"), "Jane & Co");
    }
}
