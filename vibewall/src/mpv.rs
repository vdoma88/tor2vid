//! Тонкая обёртка над libmpv. Библиотека подгружается при запуске
//! (`libmpv-2.dll` рядом с программой), поэтому для сборки mpv не нужен,
//! а без него программа открывается и честно говорит, чего не хватает.

use std::ffi::{c_char, c_double, c_int, c_void, CStr, CString};
use std::path::PathBuf;
use std::sync::OnceLock;

use libloading::Library;

use crate::library::{Fit, Kind};

type Handle = *mut c_void;

pub const EVENT_SHUTDOWN: i32 = 1;
pub const EVENT_END_FILE: i32 = 7;

#[repr(C)]
struct RawEvent {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

struct Api {
    create: unsafe extern "C" fn() -> Handle,
    initialize: unsafe extern "C" fn(Handle) -> c_int,
    set_option_string: unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> c_int,
    set_property_string: unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> c_int,
    get_property_string: unsafe extern "C" fn(Handle, *const c_char) -> *mut c_char,
    command: unsafe extern "C" fn(Handle, *const *const c_char) -> c_int,
    wait_event: unsafe extern "C" fn(Handle, c_double) -> *mut RawEvent,
    free: unsafe extern "C" fn(*mut c_void),
    terminate_destroy: unsafe extern "C" fn(Handle),
    error_string: unsafe extern "C" fn(c_int) -> *const c_char,
    // Библиотека должна жить столько же, сколько указатели на её функции.
    _lib: Library,
}

static API: OnceLock<Result<Api, String>> = OnceLock::new();

#[cfg(windows)]
const DLL_NAMES: [&str; 3] = ["libmpv-2.dll", "mpv-2.dll", "mpv-1.dll"];
#[cfg(not(windows))]
const DLL_NAMES: [&str; 2] = ["libmpv.so.2", "libmpv.so"];

fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
    {
        for name in DLL_NAMES {
            out.push(dir.join(name));
        }
    }
    for name in DLL_NAMES {
        out.push(PathBuf::from(name));
    }
    out
}

fn load() -> Result<Api, String> {
    let mut last = String::from("файл не найден");
    for path in candidates() {
        let lib = match unsafe { Library::new(&path) } {
            Ok(lib) => lib,
            Err(e) => {
                last = e.to_string();
                continue;
            }
        };
        unsafe {
            macro_rules! sym {
                ($name:literal) => {
                    *lib.get(concat!($name, "\0").as_bytes())
                        .map_err(|e| format!("в {} нет {}: {e}", path.display(), $name))?
                };
            }
            return Ok(Api {
                create: sym!("mpv_create"),
                initialize: sym!("mpv_initialize"),
                set_option_string: sym!("mpv_set_option_string"),
                set_property_string: sym!("mpv_set_property_string"),
                get_property_string: sym!("mpv_get_property_string"),
                command: sym!("mpv_command"),
                wait_event: sym!("mpv_wait_event"),
                free: sym!("mpv_free"),
                terminate_destroy: sym!("mpv_terminate_destroy"),
                error_string: sym!("mpv_error_string"),
                _lib: lib,
            });
        }
    }
    Err(format!("не удалось загрузить libmpv-2.dll ({last})"))
}

fn api() -> Result<&'static Api, String> {
    API.get_or_init(load).as_ref().map_err(Clone::clone)
}

/// Есть ли mpv. Текст ошибки — для окна программы.
pub fn available() -> Result<(), String> {
    api().map(|_| ())
}

fn cstr(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

/// Один экземпляр mpv: обои на одном мониторе, фоновая музыка или
/// создание превью.
pub struct Player {
    api: &'static Api,
    handle: Handle,
}

// Функции клиентского API mpv можно вызывать из любого потока.
unsafe impl Send for Player {}

impl Player {
    /// Создаёт mpv с заданными до `mpv_initialize` опциями.
    pub fn new(options: &[(&str, &str)]) -> Result<Self, String> {
        let api = api()?;
        let handle = unsafe { (api.create)() };
        if handle.is_null() {
            return Err("mpv_create вернул пустой указатель".into());
        }
        let player = Player { api, handle };
        for (name, value) in options {
            let (n, v) = (cstr(name), cstr(value));
            let code = unsafe { (api.set_option_string)(handle, n.as_ptr(), v.as_ptr()) };
            if code < 0 {
                return Err(format!("опция {name}={value}: {}", player.error(code)));
            }
        }
        let code = unsafe { (api.initialize)(handle) };
        if code < 0 {
            return Err(format!("mpv_initialize: {}", player.error(code)));
        }
        Ok(player)
    }

    fn error(&self, code: c_int) -> String {
        unsafe { CStr::from_ptr((self.api.error_string)(code)) }
            .to_string_lossy()
            .into_owned()
    }

    pub fn command(&self, args: &[&str]) -> Result<(), String> {
        let owned: Vec<CString> = args.iter().map(|a| cstr(a)).collect();
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        let code = unsafe { (self.api.command)(self.handle, ptrs.as_ptr()) };
        if code < 0 {
            Err(format!(
                "{}: {}",
                args.first().unwrap_or(&""),
                self.error(code)
            ))
        } else {
            Ok(())
        }
    }

    pub fn set(&self, name: &str, value: &str) {
        let (n, v) = (cstr(name), cstr(value));
        unsafe { (self.api.set_property_string)(self.handle, n.as_ptr(), v.as_ptr()) };
    }

    pub fn get(&self, name: &str) -> Option<String> {
        let n = cstr(name);
        let ptr = unsafe { (self.api.get_property_string)(self.handle, n.as_ptr()) };
        if ptr.is_null() {
            return None;
        }
        let value = unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned();
        unsafe { (self.api.free)(ptr.cast()) };
        Some(value)
    }

    /// Следующее событие (код и ошибка) или `None`, если за `timeout`
    /// секунд ничего не пришло.
    pub fn wait_event(&self, timeout: f64) -> Option<(i32, i32)> {
        let ev = unsafe { (self.api.wait_event)(self.handle, timeout) };
        if ev.is_null() {
            return None;
        }
        let ev = unsafe { &*ev };
        (ev.event_id != 0).then_some((ev.event_id, ev.error))
    }

    /// Выбирает накопившиеся события, чтобы очередь mpv не переполнялась.
    pub fn drain_events(&self) {
        for _ in 0..64 {
            if self.wait_event(0.0).is_none() {
                break;
            }
        }
    }

    pub fn load(&self, path: &str) -> Result<(), String> {
        self.command(&["loadfile", path, "replace"])
    }
}

fn yes(on: bool) -> String {
    if on { "yes" } else { "no" }.into()
}

/// Общие опции: без чужих конфигов, скриптов и youtube-dl.
const BASE: [(&str, &str); 4] = [
    ("config", "no"),
    ("load-scripts", "no"),
    ("ytdl", "no"),
    ("terminal", "no"),
];

/// Опции mpv для обоев. `wid` — окно за значками (в тестах `None`).
pub fn wallpaper_options(
    wid: Option<String>,
    kind: Kind,
    fit: Fit,
    hwdec: bool,
    volume: u8,
    paused: bool,
) -> Vec<(&'static str, String)> {
    let mut o: Vec<(&'static str, String)> =
        BASE.iter().map(|(k, v)| (*k, v.to_string())).collect();
    if let Some(wid) = wid {
        o.push(("wid", wid));
    }
    o.extend([
        ("osc", "no".into()),
        ("osd-level", "0".into()),
        ("input-default-bindings", "no".into()),
        ("input-vo-keyboard", "no".into()),
        ("input-cursor", "no".into()),
        ("cursor-autohide", "no".into()),
        ("force-window", "yes".into()),
        ("keep-open", "yes".into()),
        ("loop-file", "inf".into()),
        ("image-display-duration", "inf".into()),
        ("hwdec", if hwdec { "auto-safe" } else { "no" }.into()),
        ("volume", volume.to_string()),
        // Звук включает движок — только на одном мониторе.
        ("mute", "yes".into()),
        ("pause", yes(paused)),
        ("sub-auto", "no".into()),
        ("sid", "no".into()),
    ]);
    match fit {
        Fit::Fill => o.push(("panscan", "1.0".into())),
        Fit::Contain => o.push(("panscan", "0".into())),
        Fit::Stretch => o.push(("keepaspect", "no".into())),
    }
    if kind == Kind::Audio {
        // У звука нет картинки — рисуем спектр (фильтр showcqt из ffmpeg).
        o.push((
            "lavfi-complex",
            "[aid1]asplit[ao][v];[v]showcqt=s=1920x1080:axis=0:bar_g=2:sono_g=3:fps=30[vo]".into(),
        ));
    }
    o
}

/// Опции mpv для фоновой музыки: без картинки, список по кругу.
pub fn music_options(shuffle: bool, volume: u8, paused: bool) -> Vec<(&'static str, String)> {
    let mut o: Vec<(&'static str, String)> =
        BASE.iter().map(|(k, v)| (*k, v.to_string())).collect();
    o.extend([
        ("vid", "no".into()),
        ("force-window", "no".into()),
        ("loop-playlist", "inf".into()),
        ("shuffle", yes(shuffle)),
        ("volume", volume.to_string()),
        ("pause", yes(paused)),
    ]);
    o
}

impl Drop for Player {
    fn drop(&mut self) {
        unsafe { (self.api.terminate_destroy)(self.handle) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(
        opts: Vec<(&'static str, String)>,
        extra: &[(&str, &str)],
        file: &str,
    ) -> Result<(), String> {
        let mut refs: Vec<(&str, &str)> = opts.iter().map(|(k, v)| (*k, v.as_str())).collect();
        refs.extend_from_slice(extra);
        let p = Player::new(&refs)?;
        p.load(file)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            match p.wait_event(0.5) {
                Some((8, _)) => return Ok(()), // MPV_EVENT_FILE_LOADED
                Some((EVENT_END_FILE, err)) if err < 0 => return Err(p.error(err)),
                _ => {}
            }
        }
        Err("файл так и не загрузился".into())
    }

    /// Все опции обоев и музыки принимаются настоящим mpv, а спектр для
    /// звука собирается. Без libmpv тест пропускается.
    #[test]
    fn real_mpv_accepts_our_options() {
        if available().is_err() {
            eprintln!("libmpv нет — пропуск");
            return;
        }
        let headless = [("vo", "null"), ("ao", "null")];
        let video = "av://lavfi:testsrc=duration=3:size=640x360";
        let tone = "av://lavfi:sine=frequency=440:duration=3";
        for fit in Fit::ALL {
            run(
                wallpaper_options(None, Kind::Video, fit, true, 50, false),
                &headless,
                video,
            )
            .unwrap();
        }
        run(
            wallpaper_options(None, Kind::Audio, Fit::Fill, false, 50, true),
            &headless,
            tone,
        )
        .unwrap();
        run(music_options(true, 40, false), &headless, tone).unwrap();
        // Проверка самой проверки: сломанный граф фильтров должен падать.
        let broken = [
            ("vo", "null"),
            ("ao", "null"),
            ("lavfi-complex", "[aid1]nosuchfilter[ao]"),
        ];
        assert!(run(music_options(false, 40, false), &broken, tone).is_err());
    }
}
