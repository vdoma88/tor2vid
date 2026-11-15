//! Источники открытых ресурсов. У каждого — официальный API и лицензия,
//! разрешающая ставить файл на рабочий стол. Трекеры, «мастерские» чужих
//! программ и сайты с перезаливами сюда не добавляются.

mod commons;
mod nasa;
mod openverse;
mod pexels;
mod pixabay;

use crate::library::Kind;
use crate::online::{Fetch, Quality, Query, Remote, SearchResult};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Site {
    Pexels,
    Pixabay,
    Nasa,
    Openverse,
    Commons,
}

impl Site {
    pub const ALL: [Site; 5] = [
        Site::Pexels,
        Site::Pixabay,
        Site::Nasa,
        Site::Openverse,
        Site::Commons,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Site::Pexels => "Pexels",
            Site::Pixabay => "Pixabay",
            Site::Nasa => "NASA",
            Site::Openverse => "Openverse",
            Site::Commons => "Викисклад",
        }
    }

    /// Чем источник хорош — подпись под выбором.
    pub fn tagline(self) -> &'static str {
        match self {
            Site::Pexels => "Эстетичные видео в 4K: неон, города ночью, лоу-фай, природа. Лицензия Pexels — бесплатно, без указания автора.",
            Site::Pixabay => "Видео и анимации-петли: абстракции, частицы, ретровейв. Лицензия Pixabay — бесплатно.",
            Site::Nasa => "Космос от NASA: туманности, Земля с орбиты, полярные сияния. Общественное достояние.",
            Site::Openverse => "Картинки и музыка под Creative Commons: фото, арт, эмбиент и лоу-фай для фона.",
            Site::Commons => "Викисклад: видео, GIF, картинки и звук под свободными лицензиями.",
        }
    }

    pub fn kinds(self) -> &'static [Kind] {
        match self {
            Site::Pexels => &[Kind::Video, Kind::Image],
            Site::Pixabay => &[Kind::Video, Kind::Animation],
            Site::Nasa => &[Kind::Video, Kind::Image],
            Site::Openverse => &[Kind::Image, Kind::Audio],
            Site::Commons => &[Kind::Video, Kind::Animation, Kind::Image, Kind::Audio],
        }
    }

    pub fn needs_key(self) -> bool {
        matches!(self, Site::Pexels | Site::Pixabay)
    }

    /// Где взять бесплатный ключ API.
    pub fn key_url(self) -> &'static str {
        match self {
            Site::Pexels => "https://www.pexels.com/api/new/",
            Site::Pixabay => "https://pixabay.com/api/docs/",
            _ => "",
        }
    }

    /// Ключ, вшитый при сборке (секрет CI), если пользователь своего не дал.
    pub fn builtin_key(self) -> &'static str {
        match self {
            Site::Pexels => option_env!("VIBEWALL_PEXELS_KEY").unwrap_or(""),
            Site::Pixabay => option_env!("VIBEWALL_PIXABAY_KEY").unwrap_or(""),
            _ => "",
        }
    }

    /// Учитывает ли источник фильтр качества.
    pub fn has_quality(self) -> bool {
        !matches!(self, Site::Nasa)
    }
}

/// Вайбы — готовые подборки для быстрого старта. Запросы на английском:
/// так их понимают все источники.
pub const VIBES: [(&str, &str); 12] = [
    ("Неон", "neon city night"),
    ("Лоу-фай", "lofi aesthetic"),
    ("Космос", "galaxy nebula"),
    ("Киберпанк", "cyberpunk"),
    ("Ретровейв", "synthwave retro"),
    ("Токио", "tokyo night"),
    ("Дождь", "rain window"),
    ("Океан", "ocean waves"),
    ("Огонь", "fire"),
    ("Абстракция", "abstract liquid"),
    ("Лес", "forest"),
    ("Снег", "snow"),
];

pub fn search(q: &Query) -> Result<SearchResult, String> {
    match q.site {
        Site::Pexels => pexels::search(q),
        Site::Pixabay => pixabay::search(q),
        Site::Nasa => nasa::search(q),
        Site::Openverse => openverse::search(q),
        Site::Commons => commons::search(q),
    }
}

/// Ссылка на сам файл — для NASA её выбирают из списка файлов записи.
pub fn resolve(r: &Remote, quality: Quality) -> Result<String, String> {
    match &r.fetch {
        Fetch::Direct(url) => Ok(url.clone()),
        Fetch::NasaCollection(url) => nasa::resolve(url, r.kind, quality),
    }
}

fn str_of<'a>(v: &'a serde_json::Value, key: &str) -> &'a str {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("")
}

fn u64_of(v: &serde_json::Value, key: &str) -> u64 {
    v.get(key).and_then(|x| x.as_u64()).unwrap_or(0)
}

fn f64_of(v: &serde_json::Value, key: &str) -> f64 {
    v.get(key).and_then(|x| x.as_f64()).unwrap_or(0.0)
}

/// «neon, city, night» → «Neon city night».
fn title_from_tags(tags: &str) -> String {
    let words: Vec<&str> = tags
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .take(3)
        .collect();
    let t = words.join(" ");
    let mut chars = t.chars();
    match chars.next() {
        Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
        None => t,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_site_has_kinds_and_keys_where_needed() {
        for s in Site::ALL {
            assert!(!s.kinds().is_empty());
            assert_eq!(s.needs_key(), !s.key_url().is_empty());
        }
    }

    #[test]
    fn tags_become_titles() {
        assert_eq!(title_from_tags("neon, city, night, car"), "Neon city night");
        assert_eq!(title_from_tags(""), "");
    }
}

/// Живая проверка источников без ключа: `VIBEWALL_LIVE=1 cargo test live`.
#[cfg(test)]
mod live {
    use super::*;

    fn query(site: Site, kind: Kind, text: &str) -> Query {
        Query {
            site,
            kind,
            text: text.into(),
            quality: Quality::FullHd,
            short_loops: false,
            page: 1,
            key: String::new(),
        }
    }

    #[test]
    fn live_sources_without_key() {
        if std::env::var("VIBEWALL_LIVE").is_err() {
            return;
        }
        let nasa = crate::online::search(&query(Site::Nasa, Kind::Video, "aurora")).unwrap();
        assert!(!nasa.items.is_empty());
        let url = resolve(&nasa.items[0], Quality::FullHd).unwrap();
        eprintln!("NASA: {} → {url}", nasa.items[0].title);
        assert!(url.starts_with("https://") && url.ends_with(".mp4"));
        let img = crate::online::search(&query(Site::Nasa, Kind::Image, "nebula")).unwrap();
        eprintln!(
            "NASA img: {}",
            resolve(&img.items[0], Quality::FullHd).unwrap()
        );
        // Openverse за Cloudflare: с облачных адресов часто отвечает 401 —
        // это не ошибка программы, поэтому только печатаем.
        match crate::online::search(&query(Site::Openverse, Kind::Image, "neon city")) {
            Ok(ov) => eprintln!("Openverse: {} [{}]", ov.items[0].title, ov.items[0].license),
            Err(e) => eprintln!("Openverse недоступен отсюда: {e}"),
        }
    }
}
