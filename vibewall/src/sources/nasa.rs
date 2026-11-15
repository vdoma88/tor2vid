//! NASA Image and Video Library: космос, Земля с орбиты, сияния. Материалы
//! NASA — общественное достояние (кроме редких чужих, это видно на
//! странице записи). Ключ не нужен.

use serde_json::Value;

use super::{str_of, u64_of, Site};
use crate::library::Kind;
use crate::online::{get_json, Fetch, Quality, Query, Remote, SearchResult, PAGE};

const LICENSE: &str = "NASA — общественное достояние";

pub fn search(q: &Query) -> Result<SearchResult, String> {
    let page = q.page.max(1);
    let text = q.text.trim();
    let params = vec![
        (
            "q",
            if text.is_empty() {
                "space".to_string()
            } else {
                text.to_string()
            },
        ),
        (
            "media_type",
            if q.kind == Kind::Image {
                "image"
            } else {
                "video"
            }
            .to_string(),
        ),
        ("page", page.to_string()),
        ("page_size", PAGE.to_string()),
    ];
    let json = get_json("NASA", "https://images-api.nasa.gov/search", &params, None)?;
    Ok(parse(&json, page))
}

fn parse(json: &Value, page: u32) -> SearchResult {
    let col = json.get("collection").cloned().unwrap_or(Value::Null);
    let next = col
        .get("links")
        .and_then(|l| l.as_array())
        .is_some_and(|l| l.iter().any(|x| str_of(x, "rel") == "next"))
        .then_some(page + 1);
    let items = col
        .get("items")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(item).collect())
        .unwrap_or_default();
    SearchResult { items, next }
}

fn item(it: &Value) -> Option<Remote> {
    let data = it.get("data")?.as_array()?.first()?;
    let kind = match str_of(data, "media_type") {
        "video" => Kind::Video,
        "image" => Kind::Image,
        _ => return None,
    };
    let collection = str_of(it, "href");
    if collection.is_empty() {
        return None;
    }
    let links = it.get("links").and_then(|l| l.as_array());
    // Превью: «~large.jpg» ширины ~800 лучше смотрится в карточке, чем thumb.
    let thumb = links.and_then(|l| {
        l.iter()
            .filter(|x| str_of(x, "render") == "image")
            .max_by_key(|x| u64_of(x, "width").min(800))
            .map(|x| str_of(x, "href").to_string())
    });
    let id = str_of(data, "nasa_id");
    let author = [
        str_of(data, "secondary_creator"),
        str_of(data, "photographer"),
        str_of(data, "center"),
    ]
    .into_iter()
    .find(|s| !s.is_empty())
    .unwrap_or("NASA");
    Some(Remote {
        site: Site::Nasa,
        title: str_of(data, "title").to_string(),
        page: format!("https://images.nasa.gov/details/{id}"),
        fetch: Fetch::NasaCollection(collection.to_string()),
        thumb,
        kind,
        size: 0,
        width: 0,
        height: 0,
        duration: 0.0,
        author: crate::online::plain(author),
        license: LICENSE.into(),
    })
}

/// Выбирает файл из collection.json записи: для видео — mp4 «~large»
/// (обычно Full HD) или исходник для 4K, для картинок — исходник jpg/png.
pub fn resolve(collection: &str, kind: Kind, quality: Quality) -> Result<String, String> {
    let json = get_json("NASA", collection, &[], None)?;
    let files: Vec<String> = json
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    pick(&files, kind, quality)
        .map(|u| u.replacen("http://", "https://", 1))
        .ok_or_else(|| "У записи NASA нет подходящего файла.".into())
}

fn pick(files: &[String], kind: Kind, quality: Quality) -> Option<String> {
    let find = |suffix: &str| files.iter().find(|f| f.ends_with(suffix)).cloned();
    let order: &[&str] = match (kind, quality) {
        (Kind::Image, _) => &["~orig.jpg", "~orig.png", "~large.jpg", "~medium.jpg"],
        (_, Quality::Uhd) => &["~orig.mp4", "~large.mp4", "~medium.mp4"],
        _ => &["~large.mp4", "~orig.mp4", "~medium.mp4", "~mobile.mp4"],
    };
    order.iter().find_map(|s| find(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Сокращённый настоящий ответ images-api.nasa.gov.
    const SEARCH: &str = r#"{"collection":{"version":"1.1","items":[{"href":"https://images-assets.nasa.gov/video/GSFC_20180314_Aurora_m12865_Steve/collection.json",
      "data":[{"center":"GSFC","media_type":"video","nasa_id":"GSFC_20180314_Aurora_m12865_Steve","secondary_creator":"Genna Duberstein, Kasha Patel","title":"The Aurora Named STEVE"}],
      "links":[{"href":"https://images-assets.nasa.gov/video/G/G~large.jpg","rel":"alternate","render":"image","width":800},
               {"href":"https://images-assets.nasa.gov/video/G/G~thumb.jpg","rel":"preview","render":"image","width":300},
               {"href":"https://images-assets.nasa.gov/video/G/G.vtt","rel":"captions"}]},
      {"href":"x","data":[{"media_type":"audio","nasa_id":"a","title":"t"}]}],
      "metadata":{"total_hits":32},
      "links":[{"rel":"next","prompt":"Next","href":"http://images-api.nasa.gov/search?q=aurora&page=2"}]}}"#;

    #[test]
    fn parses_search() {
        let json: Value = serde_json::from_str(SEARCH).unwrap();
        let r = parse(&json, 1);
        assert_eq!(r.next, Some(2));
        assert_eq!(r.items.len(), 1, "звук отбрасывается");
        let v = &r.items[0];
        assert_eq!(v.title, "The Aurora Named STEVE");
        assert_eq!(
            v.page,
            "https://images.nasa.gov/details/GSFC_20180314_Aurora_m12865_Steve"
        );
        assert_eq!(
            v.thumb.as_deref(),
            Some("https://images-assets.nasa.gov/video/G/G~large.jpg")
        );
        assert_eq!(v.author, "Genna Duberstein, Kasha Patel");
    }

    #[test]
    fn picks_files() {
        let files: Vec<String> = [
            "a~orig.mp4",
            "a.vtt",
            "a~large.mp4",
            "a~medium.mp4",
            "a~thumb.jpg",
        ]
        .iter()
        .map(|s| format!("http://images-assets.nasa.gov/video/a/{s}"))
        .collect();
        assert!(pick(&files, Kind::Video, Quality::FullHd)
            .unwrap()
            .ends_with("~large.mp4"));
        assert!(pick(&files, Kind::Video, Quality::Uhd)
            .unwrap()
            .ends_with("~orig.mp4"));
        let imgs = vec!["x~orig.tif".to_string(), "x~large.jpg".to_string()];
        assert!(pick(&imgs, Kind::Image, Quality::Any)
            .unwrap()
            .ends_with("~large.jpg"));
    }
}
