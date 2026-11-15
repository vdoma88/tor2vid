//! Pexels: видео и фото под лицензией Pexels (бесплатно, указание автора
//! необязательно, но мы его сохраняем). Нужен бесплатный ключ API.

use serde_json::Value;

use super::{f64_of, str_of, u64_of, Site};
use crate::library::Kind;
use crate::online::{get_json, Fetch, Quality, Query, Remote, SearchResult, PAGE};

const LICENSE: &str = "Лицензия Pexels";

pub fn search(q: &Query) -> Result<SearchResult, String> {
    let text = q.text.trim();
    let page = q.page.max(1);
    let mut params = vec![("per_page", PAGE.to_string()), ("page", page.to_string())];
    let url = match (q.kind, text.is_empty()) {
        (Kind::Image, true) => "https://api.pexels.com/v1/curated",
        (Kind::Image, false) => "https://api.pexels.com/v1/search",
        (_, true) => "https://api.pexels.com/videos/popular",
        (_, false) => "https://api.pexels.com/videos/search",
    };
    if !text.is_empty() {
        params.push(("query", text.to_string()));
        params.push(("orientation", "landscape".into()));
        match q.quality {
            Quality::Any => {}
            Quality::FullHd => params.push(("size", "medium".into())),
            Quality::Uhd => params.push(("size", "large".into())),
        }
    } else if q.kind != Kind::Image && q.quality != Quality::Any {
        params.push(("min_width", q.quality.min_width().to_string()));
    }
    let json = get_json(
        "Pexels",
        url,
        &params,
        Some(("Authorization", q.key.trim())),
    )?;
    Ok(parse(&json, q.kind, q.quality, page))
}

/// Ширина, к которой стремимся при выборе файла: экран Full HD или 4K.
fn target_width(quality: Quality) -> u64 {
    if quality == Quality::Uhd {
        3840
    } else {
        1920
    }
}

/// Из файлов ролика — mp4 наибольшей ширины, но не больше нужной.
fn pick_file(files: &[Value], quality: Quality) -> Option<&Value> {
    let mp4: Vec<&Value> = files
        .iter()
        .filter(|f| str_of(f, "file_type") == "video/mp4" && !str_of(f, "link").is_empty())
        .collect();
    let target = target_width(quality);
    mp4.iter()
        .filter(|f| u64_of(f, "width") <= target)
        .max_by_key(|f| u64_of(f, "width"))
        .or_else(|| mp4.iter().min_by_key(|f| u64_of(f, "width")))
        .copied()
}

fn parse(json: &Value, kind: Kind, quality: Quality, page: u32) -> SearchResult {
    let next = json
        .get("next_page")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|_| page + 1);
    let items = if kind == Kind::Image {
        json.get("photos")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|p| photo(p, quality)).collect())
            .unwrap_or_default()
    } else {
        json.get("videos")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| video(v, quality)).collect())
            .unwrap_or_default()
    };
    SearchResult { items, next }
}

/// Название из ссылки на страницу: …/video/neon-city-at-night-1234/ →
/// «Neon city at night».
fn title_from_page(url: &str) -> String {
    let slug = url.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    let words: Vec<&str> = slug
        .split('-')
        .filter(|w| !w.chars().all(|c| c.is_ascii_digit()))
        .collect();
    let t = words.join(" ");
    let mut c = t.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => "Pexels".into(),
    }
}

fn video(v: &Value, quality: Quality) -> Option<Remote> {
    let files = v.get("video_files")?.as_array()?;
    let file = pick_file(files, quality)?;
    let user = v.get("user").cloned().unwrap_or(Value::Null);
    let page = str_of(v, "url").to_string();
    Some(Remote {
        site: Site::Pexels,
        title: title_from_page(&page),
        page,
        fetch: Fetch::Direct(str_of(file, "link").to_string()),
        thumb: Some(str_of(v, "image").to_string()).filter(|s| !s.is_empty()),
        kind: Kind::Video,
        size: u64_of(file, "size"),
        width: u64_of(file, "width") as u32,
        height: u64_of(file, "height") as u32,
        duration: f64_of(v, "duration"),
        author: str_of(&user, "name").to_string(),
        license: LICENSE.into(),
        plays: 0,
    })
}

fn photo(p: &Value, quality: Quality) -> Option<Remote> {
    let src = p.get("src")?;
    let original = str_of(src, "original");
    if original.is_empty() {
        return None;
    }
    let w = target_width(quality);
    // Pexels отдаёт любой размер по параметрам ссылки — берём под экран.
    let url = format!("{original}?auto=compress&cs=tinysrgb&w={w}");
    let (pw, ph) = (u64_of(p, "width"), u64_of(p, "height"));
    let (width, height) = if pw > w && pw > 0 {
        (w, ph * w / pw)
    } else {
        (pw, ph)
    };
    let alt = str_of(p, "alt");
    let page = str_of(p, "url").to_string();
    Some(Remote {
        site: Site::Pexels,
        title: if alt.is_empty() {
            title_from_page(&page)
        } else {
            alt.to_string()
        },
        page,
        fetch: Fetch::Direct(url),
        thumb: Some(str_of(src, "medium").to_string()).filter(|s| !s.is_empty()),
        kind: Kind::Image,
        size: 0,
        width: width as u32,
        height: height as u32,
        duration: 0.0,
        author: str_of(p, "photographer").to_string(),
        license: LICENSE.into(),
        plays: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIDEOS: &str = r#"{"page":1,"per_page":2,"total_results":2000,"next_page":"https://api.pexels.com/videos/search/?page=2",
      "videos":[{"id":3129671,"width":3840,"height":2160,"duration":14,
        "url":"https://www.pexels.com/video/neon-lights-in-the-city-3129671/",
        "image":"https://images.pexels.com/videos/3129671/free-video-3129671.jpg",
        "user":{"id":1,"name":"Kelly","url":"https://www.pexels.com/@kelly"},
        "video_files":[
          {"id":1,"quality":"sd","file_type":"video/mp4","width":640,"height":360,"link":"https://videos.pexels.com/sd.mp4"},
          {"id":2,"quality":"hd","file_type":"video/mp4","width":1920,"height":1080,"link":"https://videos.pexels.com/hd.mp4","size":10485760},
          {"id":3,"quality":"uhd","file_type":"video/mp4","width":3840,"height":2160,"link":"https://videos.pexels.com/uhd.mp4"},
          {"id":4,"quality":null,"file_type":"video/webm","width":4096,"height":2160,"link":"https://videos.pexels.com/x.webm"}]}]}"#;

    #[test]
    fn picks_video_file_by_quality() {
        let json: Value = serde_json::from_str(VIDEOS).unwrap();
        let r = parse(&json, Kind::Video, Quality::FullHd, 1);
        assert_eq!(r.next, Some(2));
        let v = &r.items[0];
        assert_eq!(v.title, "Neon lights in the city");
        assert_eq!(
            v.fetch,
            Fetch::Direct("https://videos.pexels.com/hd.mp4".into())
        );
        assert_eq!((v.width, v.height, v.size), (1920, 1080, 10485760));
        assert_eq!(v.author, "Kelly");
        assert_eq!(v.duration, 14.0);
        let r4k = parse(&json, Kind::Video, Quality::Uhd, 1);
        assert_eq!(
            r4k.items[0].fetch,
            Fetch::Direct("https://videos.pexels.com/uhd.mp4".into())
        );
    }

    #[test]
    fn photos_are_scaled_to_screen() {
        let json: Value = serde_json::from_str(
            r#"{"photos":[{"id":1,"width":6000,"height":4000,"url":"https://www.pexels.com/photo/tokyo-street-1/",
            "photographer":"Aiko","alt":"Tokyo street at night",
            "src":{"original":"https://images.pexels.com/photos/1/p.jpeg","medium":"https://images.pexels.com/photos/1/m.jpeg"}}]}"#,
        )
        .unwrap();
        let r = parse(&json, Kind::Image, Quality::Any, 1);
        assert_eq!(r.next, None);
        let p = &r.items[0];
        assert_eq!(p.title, "Tokyo street at night");
        assert_eq!((p.width, p.height), (1920, 1280));
        assert!(matches!(&p.fetch, Fetch::Direct(u) if u.ends_with("w=1920")));
    }
}
