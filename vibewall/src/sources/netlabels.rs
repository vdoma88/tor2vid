//! Нетлейблы Internet Archive: музыка независимых лейблов под Creative
//! Commons — лоу-фай, хип-хоп биты, электросвинг, чиллвейв. Ищем только
//! записи с отметкой лицензии CC; запись — это альбом, скачиваются его mp3.
//! Ключ не нужен.

use serde_json::Value;

use super::{str_of, u64_of, Site};
use crate::library::Kind;
use crate::online::{encode_url, get_json, Fetch, Query, Remote, SearchResult, Sort, Track, PAGE};

/// Больше треков из одного альбома не берём — хватит на долгий фон.
const MAX_TRACKS: usize = 30;

/// Запрос к поиску архива: каждое слово ищется в тегах и названии.
fn archive_query(text: &str) -> String {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric() && c != '-')
        .filter(|w| w.len() > 1)
        .map(|w| w.to_lowercase())
        .collect();
    let mut q =
        String::from("collection:netlabels AND mediatype:audio AND licenseurl:*creativecommons*");
    if !words.is_empty() {
        let any = words.join(" OR ");
        q.push_str(&format!(" AND (subject:({any}) OR title:({any}))"));
    }
    q
}

pub fn search(q: &Query) -> Result<SearchResult, String> {
    let page = q.page.max(1);
    let sort = match q.sort {
        Sort::Popular => "downloads desc",
        Sort::Latest => "publicdate desc",
    };
    let params = vec![
        ("q", archive_query(&q.text)),
        ("fl[]", "identifier".to_string()),
        ("fl[]", "title".to_string()),
        ("fl[]", "creator".to_string()),
        ("fl[]", "licenseurl".to_string()),
        ("fl[]", "downloads".to_string()),
        ("sort[]", sort.to_string()),
        ("rows", PAGE.to_string()),
        ("page", page.to_string()),
        ("output", "json".to_string()),
    ];
    let json = get_json(
        "Internet Archive",
        "https://archive.org/advancedsearch.php",
        &params,
        None,
    )?;
    Ok(parse(&json, page))
}

/// http://creativecommons.org/licenses/by-nc-nd/3.0/ → «CC BY-NC-ND 3.0».
pub fn license_from_url(url: &str) -> String {
    let parts: Vec<&str> = url.trim_end_matches('/').split('/').collect();
    match parts
        .iter()
        .position(|p| *p == "licenses" || *p == "publicdomain")
    {
        Some(i) if parts.get(i + 1) == Some(&"zero") => "CC0".into(),
        Some(i) if parts[i] == "publicdomain" => "Общественное достояние".into(),
        Some(i) => {
            let code = parts.get(i + 1).unwrap_or(&"").to_uppercase();
            let ver = parts.get(i + 2).unwrap_or(&"");
            format!("CC {code} {ver}").trim().to_string()
        }
        None => "Creative Commons".into(),
    }
}

/// Поле архива бывает строкой или списком строк.
fn text_of(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    }
}

fn parse(json: &Value, page: u32) -> SearchResult {
    let resp = json.get("response").cloned().unwrap_or(Value::Null);
    let found = u64_of(&resp, "numFound");
    let next = (u64::from(page) * u64::from(PAGE) < found).then_some(page + 1);
    let items = resp
        .get("docs")
        .and_then(|d| d.as_array())
        .map(|a| a.iter().filter_map(doc).collect())
        .unwrap_or_default();
    SearchResult { items, next }
}

fn doc(d: &Value) -> Option<Remote> {
    let id = str_of(d, "identifier");
    if id.is_empty() {
        return None;
    }
    let title = text_of(d, "title");
    Some(Remote {
        site: Site::Netlabels,
        title: if title.is_empty() {
            id.to_string()
        } else {
            title
        },
        page: format!("https://archive.org/details/{id}"),
        fetch: Fetch::ArchiveAlbum(id.to_string()),
        thumb: Some(format!("https://archive.org/services/img/{id}")),
        kind: Kind::Audio,
        size: 0,
        width: 0,
        height: 0,
        duration: 0.0,
        author: text_of(d, "creator"),
        license: license_from_url(&text_of(d, "licenseurl")),
        plays: u64_of(d, "downloads"),
    })
}

/// Треки альбома из метаданных записи: mp3 по порядку номеров.
pub fn tracks(id: &str) -> Result<Vec<Track>, String> {
    let meta = get_json(
        "Internet Archive",
        &format!("https://archive.org/metadata/{id}"),
        &[],
        None,
    )?;
    let list = album_tracks(id, &meta);
    if list.is_empty() {
        Err("В альбоме нет mp3.".into())
    } else {
        Ok(list)
    }
}

fn album_tracks(id: &str, meta: &Value) -> Vec<Track> {
    let mut files: Vec<&Value> = meta
        .get("files")
        .and_then(|f| f.as_array())
        .map(|a| {
            a.iter()
                .filter(|f| {
                    let fmt = str_of(f, "format");
                    (fmt == "VBR MP3" || fmt == "MP3" || fmt == "128Kbps MP3")
                        && str_of(f, "name").to_lowercase().ends_with(".mp3")
                })
                .collect()
        })
        .unwrap_or_default();
    let track_no = |f: &Value| -> u32 {
        str_of(f, "track")
            .split('/')
            .next()
            .unwrap_or("")
            .trim()
            .parse()
            .unwrap_or(u32::MAX)
    };
    files.sort_by(|a, b| {
        track_no(a)
            .cmp(&track_no(b))
            .then(str_of(a, "name").cmp(str_of(b, "name")))
    });
    files
        .into_iter()
        .take(MAX_TRACKS)
        .map(|f| {
            let name = str_of(f, "name");
            let title = str_of(f, "title");
            Track {
                url: encode_url(&format!("https://archive.org/download/{id}/{name}")),
                title: if title.is_empty() {
                    name.trim_end_matches(".mp3").replace('_', " ")
                } else {
                    title.to_string()
                },
                size: str_of(f, "size").parse().unwrap_or(0),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_queries() {
        let q = archive_query("lofi aesthetic");
        assert!(q.contains("subject:(lofi OR aesthetic)"));
        assert!(q.starts_with("collection:netlabels"));
        assert!(!archive_query("").contains("subject"));
    }

    #[test]
    fn parses_search_and_licenses() {
        let json: Value = serde_json::from_str(
            r#"{"response":{"numFound":1702,"start":0,"docs":[{"creator":"ProleteR","downloads":992546,"identifier":"DWK123",
            "licenseurl":"http://creativecommons.org/licenses/by-nc-nd/3.0/","title":"Curses From Past Times (LP)"},
            {"identifier":"MIXG032","title":["Retrovision"],"licenseurl":"https://creativecommons.org/publicdomain/zero/1.0/"}]}}"#,
        )
        .unwrap();
        let r = parse(&json, 1);
        assert_eq!(r.next, Some(2));
        let a = &r.items[0];
        assert_eq!(a.title, "Curses From Past Times (LP)");
        assert_eq!(a.author, "ProleteR");
        assert_eq!(a.license, "CC BY-NC-ND 3.0");
        assert_eq!(a.plays, 992546);
        assert_eq!(a.fetch, Fetch::ArchiveAlbum("DWK123".into()));
        assert_eq!(r.items[1].title, "Retrovision");
        assert_eq!(r.items[1].license, "CC0");
    }

    #[test]
    fn lists_album_tracks_in_order() {
        let meta: Value = serde_json::from_str(
            r#"{"files":[
              {"name":"ProleteR_-_02_-_Downtown_Irony.mp3","format":"VBR MP3","title":"Downtown Irony","track":"02","size":"10460560"},
              {"name":"ProleteR_-_01_-_April_Showers.mp3","format":"VBR MP3","title":"April Showers","track":"01","size":"10764625"},
              {"name":"ProleteR_-_01_-_April_Showers.ogg","format":"Ogg Vorbis"},
              {"name":"cover.jpg","format":"JPEG"}]}"#,
        )
        .unwrap();
        let t = album_tracks("DWK123", &meta);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].title, "April Showers");
        assert_eq!(
            t[0].url,
            "https://archive.org/download/DWK123/ProleteR_-_01_-_April_Showers.mp3"
        );
        assert_eq!(t[0].size, 10764625);
    }
}
