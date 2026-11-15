//! Викисклад (Wikimedia Commons): только свободные лицензии — CC0, CC BY,
//! CC BY-SA, общественное достояние.

use serde::Deserialize;

use crate::library::Kind;
use crate::online::{describe, plain, Fetch, Quality, Query, Remote, SearchResult, PAGE};
use crate::sources::Site;

const API: &str = "https://commons.wikimedia.org/w/api.php";

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

pub fn search(q: &Query) -> Result<SearchResult, String> {
    let filter = match q.kind {
        Kind::Video => "filetype:video",
        Kind::Animation => "filemime:image/gif",
        Kind::Image => "filetype:bitmap",
        Kind::Audio => "filetype:audio",
    };
    let mut text = format!("{filter} {}", q.text.trim());
    if q.kind != Kind::Audio {
        match q.quality {
            Quality::Any => {}
            Quality::FullHd => text.push_str(" filew:>1919"),
            Quality::Uhd => text.push_str(" filew:>3839"),
        }
    }
    let mut resp = crate::online::agent()
        .get(API)
        .query("action", "query")
        .query("format", "json")
        .query("formatversion", "2")
        .query("generator", "search")
        .query("gsrnamespace", "6")
        .query("gsrsearch", &text)
        .query("gsrlimit", PAGE.to_string())
        .query("gsroffset", q.page.to_string())
        .query("prop", "imageinfo")
        .query("iiprop", "url|size|mime|extmetadata")
        .query("iiurlwidth", "480")
        .query("iiextmetadatafilter", "LicenseShortName|Artist|ObjectName")
        .call()
        .map_err(|e| describe("Викисклад", e))?;
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
                site: Site::Commons,
                title,
                page: info.descriptionurl.clone(),
                fetch: Fetch::Direct(info.url.clone()),
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
    fn mime_kinds() {
        assert_eq!(kind_of("image/gif"), Some(Kind::Animation));
        assert_eq!(kind_of("application/ogg"), Some(Kind::Video));
        assert_eq!(kind_of("audio/x-flac"), Some(Kind::Audio));
        assert_eq!(kind_of("image/jpeg"), Some(Kind::Image));
        assert_eq!(kind_of("image/svg+xml"), None);
    }
}
