//! Открытые ресурсы: поиск и скачивание с Викисклада (Wikimedia Commons).
//! Там лежат только файлы под свободными лицензиями (CC0, CC BY,
//! CC BY-SA, общественное достояние), поэтому их можно ставить на рабочий
//! стол. Автор и лицензия сохраняются в библиотеке.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;

use crate::library::{safe_file_name, unique_path, Kind};

const API: &str = "https://commons.wikimedia.org/w/api.php";
const UA: &str = "zhivye-oboi/0.1 (+https://github.com/vdoma88/tor2vid)";
pub const PAGE: u32 = 24;

#[derive(Clone, Debug)]
pub struct Remote {
    pub title: String,
    pub page: String,
    pub url: String,
    pub thumb: Option<String>,
    pub kind: Kind,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub author: String,
    pub license: String,
}

pub struct SearchResult {
    pub items: Vec<Remote>,
    pub next: Option<u32>,
}

#[derive(Deserialize)]
struct ApiResponse {
    #[serde(default)]
    query: Option<ApiQuery>,
    #[serde(rename = "continue", default)]
    cont: Option<ApiContinue>,
}

#[derive(Deserialize)]
struct ApiContinue {
    gsroffset: Option<u32>,
}

#[derive(Deserialize)]
struct ApiQuery {
    #[serde(default)]
    pages: Vec<ApiPage>,
}

#[derive(Deserialize)]
struct ApiPage {
    title: String,
    #[serde(default)]
    index: u32,
    #[serde(default)]
    imageinfo: Vec<ApiInfo>,
}

#[derive(Deserialize)]
struct ApiInfo {
    url: String,
    descriptionurl: String,
    #[serde(default)]
    thumburl: Option<String>,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    mime: String,
    #[serde(default)]
    extmetadata: Option<serde_json::Value>,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .user_agent(UA)
        .build()
        .into()
}

fn describe(err: ureq::Error) -> String {
    match err {
        ureq::Error::StatusCode(429) => {
            "Викисклад просит подождать: слишком много запросов. Попробуйте через минуту.".into()
        }
        ureq::Error::StatusCode(code) => format!("Сервер ответил кодом {code}"),
        other => format!("Нет связи: {other}"),
    }
}

/// HTML из метаданных Викисклада → обычный текст.
fn plain(html: &str) -> String {
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

fn meta(info: &ApiInfo, key: &str) -> String {
    info.extmetadata
        .as_ref()
        .and_then(|m| m.get(key))
        .and_then(|v| v.get("value"))
        .and_then(|v| v.as_str())
        .map(plain)
        .unwrap_or_default()
}

fn kind_of(mime: &str) -> Option<Kind> {
    if mime == "image/gif" || mime == "image/apng" {
        Some(Kind::Animation)
    } else if mime.starts_with("video/") || mime == "application/ogg" {
        Some(Kind::Video)
    } else if mime.starts_with("audio/") {
        Some(Kind::Audio)
    } else if mime.starts_with("image/") && mime != "image/svg+xml" && mime != "image/tiff" {
        Some(Kind::Image)
    } else {
        None
    }
}

pub fn search(query: &str, kind: Kind, hd: bool, offset: u32) -> Result<SearchResult, String> {
    let filter = match kind {
        Kind::Video => "filetype:video",
        Kind::Animation => "filemime:image/gif",
        Kind::Image => "filetype:bitmap",
        Kind::Audio => "filetype:audio",
    };
    let mut q = format!("{filter} {}", query.trim());
    if hd && kind != Kind::Audio {
        q.push_str(" filew:>1279");
    }
    let offset_s = offset.to_string();
    let limit_s = PAGE.to_string();
    let mut resp = agent()
        .get(API)
        .query("action", "query")
        .query("format", "json")
        .query("formatversion", "2")
        .query("generator", "search")
        .query("gsrnamespace", "6")
        .query("gsrsearch", &q)
        .query("gsrlimit", &limit_s)
        .query("gsroffset", &offset_s)
        .query("prop", "imageinfo")
        .query("iiprop", "url|size|mime|extmetadata")
        .query("iiurlwidth", "480")
        .query("iiextmetadatafilter", "LicenseShortName|Artist|ObjectName")
        .call()
        .map_err(describe)?;
    let data: ApiResponse = resp
        .body_mut()
        .read_json()
        .map_err(|e| format!("Непонятный ответ Викисклада: {e}"))?;
    Ok(parse(data))
}

fn parse(data: ApiResponse) -> SearchResult {
    let mut pages = data.query.map(|q| q.pages).unwrap_or_default();
    pages.sort_by_key(|p| p.index);
    let items = pages
        .into_iter()
        .filter_map(|p| {
            let info = p.imageinfo.into_iter().next()?;
            let kind = kind_of(&info.mime)?;
            let mut title = meta(&info, "ObjectName");
            if title.is_empty() {
                title = p.title.trim_start_matches("File:").to_string();
                if let Some(dot) = title.rfind('.') {
                    title.truncate(dot);
                }
            }
            Some(Remote {
                title,
                page: info.descriptionurl.clone(),
                url: info.url.clone(),
                thumb: info.thumburl.clone(),
                kind,
                size: info.size,
                width: info.width,
                height: info.height,
                duration: info.duration.unwrap_or(0.0),
                author: meta(&info, "Artist"),
                license: meta(&info, "LicenseShortName"),
            })
        })
        .collect();
    SearchResult {
        items,
        next: data.cont.and_then(|c| c.gsroffset),
    }
}

pub fn spawn_search(
    query: String,
    kind: Kind,
    hd: bool,
    offset: u32,
    wake: impl Fn() + Send + 'static,
) -> Receiver<Result<SearchResult, String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(search(&query, kind, hd, offset));
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
    let resp = agent().get(url).call().map_err(describe)?;
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
    pub result: Receiver<Result<(PathBuf, Option<PathBuf>), String>>,
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

pub fn spawn_download(
    remote: Remote,
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
        let name = r
            .url
            .split('?')
            .next()
            .and_then(|u| u.rsplit('/').next())
            .map(percent_decode)
            .unwrap_or_else(|| r.title.clone());
        let target = unique_path(&media, &safe_file_name(&name));
        let res = fetch(&r.url, &target, &d, &t, &c).map(|_| {
            let thumb = r.thumb.as_ref().and_then(|url| {
                let path = thumbs.join(format!("{id}.jpg"));
                let (a, b, n) = (AtomicU64::new(0), AtomicU64::new(0), AtomicBool::new(false));
                fetch(url, &path, &a, &b, &n).ok().map(|_| path)
            });
            (target, thumb)
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

fn percent_decode(s: &str) -> String {
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

    // Сокращённый настоящий ответ API Викисклада.
    const SAMPLE: &str = r#"{"batchcomplete":true,"continue":{"gsroffset":2,"continue":"gsroffset||"},
      "query":{"pages":[
        {"pageid":2,"ns":6,"title":"File:Second clip.webm","index":2,"imageinfo":[{"size":5,"width":1280,"height":720,
          "url":"https://upload.wikimedia.org/b.webm","descriptionurl":"https://commons.wikimedia.org/wiki/File:Second_clip.webm",
          "mime":"video/webm","extmetadata":{}}]},
        {"pageid":81916645,"ns":6,"title":"File:Ocean in Sommaroy.webm","index":1,"imageinfo":[{"size":2928602,"width":1920,"height":1080,"duration":15.472,
          "thumburl":"https://thumb.wikimedia.org/x/330px--Ocean_in_Sommaroy.webm.jpg","thumbwidth":320,"thumbheight":180,
          "url":"https://upload.wikimedia.org/wikipedia/commons/5/5d/Ocean_in_Sommaroy.webm?utm_source=commons.wikimedia.org",
          "descriptionurl":"https://commons.wikimedia.org/wiki/File:Ocean_in_Sommaroy.webm",
          "extmetadata":{"ObjectName":{"value":"Ocean in Sommaroy","source":"mediawiki-metadata"},
            "Artist":{"value":"<a href=\"//commons.wikimedia.org/wiki/User:Netha_Hussain\" title=\"User:Netha Hussain\">User: Netha Hussain</a>","source":"commons-desc-page"},
            "LicenseShortName":{"value":"CC0","source":"commons-desc-page","hidden":""}},
          "mime":"video/webm"}]},
        {"pageid":3,"ns":6,"title":"File:Drawing.svg","index":3,"imageinfo":[{"url":"u","descriptionurl":"d","mime":"image/svg+xml"}]}
      ]}}"#;

    #[test]
    fn parses_commons_answer() {
        let data: ApiResponse = serde_json::from_str(SAMPLE).unwrap();
        let r = parse(data);
        assert_eq!(r.next, Some(2));
        assert_eq!(r.items.len(), 2, "SVG отбрасывается");
        let first = &r.items[0];
        assert_eq!(first.title, "Ocean in Sommaroy");
        assert_eq!(first.author, "User: Netha Hussain");
        assert_eq!(first.license, "CC0");
        assert_eq!(first.kind, Kind::Video);
        assert_eq!((first.width, first.height), (1920, 1080));
        assert!(first.thumb.is_some());
        assert_eq!(
            r.items[1].title, "Second clip",
            "без ObjectName — имя файла"
        );
    }

    #[test]
    fn decodes_file_names() {
        assert_eq!(
            percent_decode("Ocean_in_Sommar%C3%B8y.webm"),
            "Ocean in Sommarøy.webm"
        );
        assert_eq!(percent_decode("bad%zz%"), "bad%zz%");
    }

    #[test]
    fn mime_kinds() {
        assert_eq!(kind_of("image/gif"), Some(Kind::Animation));
        assert_eq!(kind_of("application/ogg"), Some(Kind::Video));
        assert_eq!(kind_of("audio/x-flac"), Some(Kind::Audio));
        assert_eq!(kind_of("image/jpeg"), Some(Kind::Image));
        assert_eq!(kind_of("image/svg+xml"), None);
    }
}
