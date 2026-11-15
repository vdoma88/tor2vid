//! Openverse (WordPress): картинки и музыка под Creative Commons со всего
//! интернета. Лицензия у каждого файла своя, иногда «некоммерческая» —
//! для своего рабочего стола подходит, она видна в карточке. Ключ не нужен.

use serde_json::Value;

use super::{str_of, u64_of, Site};
use crate::library::Kind;
use crate::online::{get_json, Fetch, Quality, Query, Remote, SearchResult, PAGE};

pub fn search(q: &Query) -> Result<SearchResult, String> {
    let page = q.page.max(1);
    let text = q.text.trim();
    let audio = q.kind == Kind::Audio;
    let mut params = vec![
        (
            "q",
            if text.is_empty() {
                if audio { "ambient" } else { "aesthetic" }.to_string()
            } else {
                text.to_string()
            },
        ),
        ("page", page.to_string()),
        ("page_size", PAGE.to_string()),
        ("mature", "false".into()),
    ];
    if !audio {
        params.push(("aspect_ratio", "wide".into()));
        if q.quality != Quality::Any {
            params.push(("size", "large".into()));
        }
    }
    let url = if audio {
        "https://api.openverse.org/v1/audio/"
    } else {
        "https://api.openverse.org/v1/images/"
    };
    let json = get_json("Openverse", url, &params, None)?;
    let mut r = parse(&json, q.kind, page);
    if !audio && q.quality != Quality::Any {
        let w = q.quality.min_width();
        r.items.retain(|i| i.width == 0 || i.width >= w);
    }
    Ok(r)
}

/// by-nc-sa 2.0 → «CC BY-NC-SA 2.0», cc0 → «CC0», pdm → «Общественное достояние».
fn license(v: &Value) -> String {
    let code = str_of(v, "license").to_ascii_lowercase();
    let version = str_of(v, "license_version");
    match code.as_str() {
        "cc0" => "CC0".into(),
        "pdm" => "Общественное достояние".into(),
        "" => String::new(),
        c => format!("CC {} {version}", c.to_ascii_uppercase())
            .trim()
            .to_string(),
    }
}

fn parse(json: &Value, kind: Kind, page: u32) -> SearchResult {
    let pages = u64_of(json, "page_count");
    let next = (u64::from(page) < pages).then_some(page + 1);
    let items = json
        .get("results")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|r| result(r, kind)).collect())
        .unwrap_or_default();
    SearchResult { items, next }
}

fn result(r: &Value, kind: Kind) -> Option<Remote> {
    let url = str_of(r, "url");
    if url.is_empty() {
        return None;
    }
    let title = str_of(r, "title");
    Some(Remote {
        site: Site::Openverse,
        title: if title.is_empty() {
            "Openverse".into()
        } else {
            title.to_string()
        },
        page: str_of(r, "foreign_landing_url").to_string(),
        fetch: Fetch::Direct(url.to_string()),
        thumb: Some(str_of(r, "thumbnail").to_string()).filter(|s| !s.is_empty()),
        kind,
        size: u64_of(r, "filesize"),
        width: u64_of(r, "width") as u32,
        height: u64_of(r, "height") as u32,
        // У звука длительность в миллисекундах.
        duration: u64_of(r, "duration") as f64 / 1000.0,
        author: str_of(r, "creator").to_string(),
        license: license(r),
        plays: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_images_and_licenses() {
        let json: Value = serde_json::from_str(
            r#"{"result_count":240,"page_count":240,"page_size":1,"page":1,"results":[{"id":"471b","title":"Neon triplefin",
            "foreign_landing_url":"https://commons.wikimedia.org/w/index.php?curid=5749514",
            "url":"https://upload.wikimedia.org/x.jpg","creator":"Nick Hobgood","license":"by-sa","license_version":"3.0",
            "filesize":2747860,"filetype":"jpg","height":1500,"width":2000,
            "thumbnail":"https://api.openverse.org/v1/images/471b/thumb/"}]}"#,
        )
        .unwrap();
        let r = parse(&json, Kind::Image, 1);
        assert_eq!(r.next, Some(2));
        let i = &r.items[0];
        assert_eq!(i.license, "CC BY-SA 3.0");
        assert_eq!(i.author, "Nick Hobgood");
        assert_eq!((i.width, i.height), (2000, 1500));
        assert_eq!(
            license(&serde_json::json!({"license": "cc0", "license_version": "1.0"})),
            "CC0"
        );
        assert_eq!(
            license(&serde_json::json!({"license": "pdm"})),
            "Общественное достояние"
        );
    }

    #[test]
    fn audio_duration_is_in_ms() {
        let json: Value = serde_json::from_str(
            r#"{"page_count":1,"results":[{"title":"Night drive","url":"https://x/a.mp3","duration":185000,"license":"by","license_version":"4.0","creator":"DJ"}]}"#,
        )
        .unwrap();
        let r = parse(&json, Kind::Audio, 1);
        assert_eq!(r.next, None);
        assert_eq!(r.items[0].duration, 185.0);
        assert_eq!(r.items[0].license, "CC BY 4.0");
    }
}
