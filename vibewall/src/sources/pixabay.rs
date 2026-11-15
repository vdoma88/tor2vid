//! Pixabay: видео и анимации-петли под лицензией Pixabay (бесплатно,
//! указание автора необязательно). Нужен бесплатный ключ API. Безопасный
//! поиск всегда включён.

use serde_json::Value;

use super::{f64_of, str_of, title_from_tags, u64_of, Site};
use crate::library::Kind;
use crate::online::{get_json, Fetch, Quality, Query, Remote, SearchResult, PAGE};

const LICENSE: &str = "Лицензия Pixabay";

pub fn search(q: &Query) -> Result<SearchResult, String> {
    let page = q.page.max(1);
    let video_type = if q.kind == Kind::Animation {
        "animation"
    } else {
        "film"
    };
    let mut params = vec![
        ("key", q.key.trim().to_string()),
        ("q", q.text.trim().to_string()),
        ("video_type", video_type.to_string()),
        ("safesearch", "true".into()),
        (
            "order",
            match q.sort {
                crate::online::Sort::Popular => "popular",
                crate::online::Sort::Latest => "latest",
            }
            .into(),
        ),
        ("editors_choice", q.editors_choice.to_string()),
        ("per_page", PAGE.to_string()),
        ("page", page.to_string()),
    ];
    if q.quality != Quality::Any {
        params.push(("min_width", q.quality.min_width().to_string()));
    }
    let json = get_json("Pixabay", "https://pixabay.com/api/videos/", &params, None)?;
    Ok(parse(&json, q.kind, q.quality, page))
}

fn parse(json: &Value, kind: Kind, quality: Quality, page: u32) -> SearchResult {
    let total = u64_of(json, "totalHits");
    let next = (u64::from(page) * u64::from(PAGE) < total).then_some(page + 1);
    let items = json
        .get("hits")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|h| hit(h, kind, quality)).collect())
        .unwrap_or_default();
    SearchResult { items, next }
}

fn hit(h: &Value, kind: Kind, quality: Quality) -> Option<Remote> {
    let videos = h.get("videos")?;
    // large — исходник (часто 4K), medium — Full HD, small — HD.
    let order: &[&str] = match quality {
        Quality::Uhd => &["large", "medium", "small"],
        _ => &["medium", "large", "small"],
    };
    let file = order
        .iter()
        .filter_map(|k| videos.get(*k))
        .find(|f| !str_of(f, "url").is_empty() && u64_of(f, "width") > 0)?;
    let thumb = ["medium", "large", "small"]
        .iter()
        .filter_map(|k| videos.get(*k))
        .map(|f| str_of(f, "thumbnail"))
        .find(|t| !t.is_empty())
        .map(String::from);
    let title = title_from_tags(str_of(h, "tags"));
    Some(Remote {
        site: Site::Pixabay,
        title: if title.is_empty() {
            "Pixabay".into()
        } else {
            title
        },
        page: str_of(h, "pageURL").to_string(),
        fetch: Fetch::Direct(str_of(file, "url").to_string()),
        thumb,
        kind,
        size: u64_of(file, "size"),
        width: u64_of(file, "width") as u32,
        height: u64_of(file, "height") as u32,
        duration: f64_of(h, "duration"),
        author: str_of(h, "user").to_string(),
        license: LICENSE.into(),
        plays: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HITS: &str = r#"{"total":5000,"totalHits":500,"hits":[{"id":125,"pageURL":"https://pixabay.com/videos/id-125/",
      "type":"film","tags":"neon, abstract, loop","duration":12,"user":"Coverr",
      "videos":{"large":{"url":"https://cdn.pixabay.com/large.mp4","width":3840,"height":2160,"size":40000000,"thumbnail":"https://cdn.pixabay.com/large.jpg"},
                "medium":{"url":"https://cdn.pixabay.com/medium.mp4","width":1920,"height":1080,"size":12000000,"thumbnail":"https://cdn.pixabay.com/medium.jpg"},
                "small":{"url":"https://cdn.pixabay.com/small.mp4","width":1280,"height":720,"size":6000000,"thumbnail":""},
                "tiny":{"url":"","width":0,"height":0,"size":0}}}]}"#;

    #[test]
    fn parses_hits() {
        let json: Value = serde_json::from_str(HITS).unwrap();
        let r = parse(&json, Kind::Animation, Quality::Any, 1);
        assert_eq!(r.next, Some(2));
        let v = &r.items[0];
        assert_eq!(v.title, "Neon abstract loop");
        assert_eq!(v.kind, Kind::Animation);
        assert_eq!(
            v.fetch,
            Fetch::Direct("https://cdn.pixabay.com/medium.mp4".into())
        );
        assert_eq!(
            v.thumb.as_deref(),
            Some("https://cdn.pixabay.com/medium.jpg")
        );
        let r4k = parse(&json, Kind::Video, Quality::Uhd, 21);
        assert_eq!(r4k.next, None, "21 × 24 > 500");
        assert_eq!(r4k.items[0].width, 3840);
    }
}
