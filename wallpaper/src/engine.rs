//! Проигрывание обоев: по окну и экземпляру mpv на каждый монитор плюс
//! отдельный mpv для фоновой музыки. Всё живёт в главном потоке —
//! окна Win32 и их сообщения принадлежат ему. Скрытое управляющее окно
//! раз в секунду проверяет, жив ли проводник, и ставит обои на паузу,
//! когда поверх рабочего стола открыта игра или фильм на весь экран.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::desktop::{self, Layer, Monitor};
use crate::library::{Fit, Kind};
use crate::mpv::Player;

#[derive(Clone, Debug, PartialEq)]
pub struct Wallpaper {
    pub id: String,
    pub path: PathBuf,
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub volume: u8,
    pub muted: bool,
    pub fit: Fit,
    pub hwdec: bool,
    pub pause_on_fullscreen: bool,
    pub music_volume: u8,
    pub music_shuffle: bool,
}

struct Screen {
    monitor: Monitor,
    host: Option<HWND>,
    player: Option<Player>,
    current: Option<Wallpaper>,
    /// Запуск не удался (mpv или окно). Такой экран не перезапускается
    /// каждую секунду — только при смене обоев, мониторов или проводника.
    failed: bool,
}

pub struct ScreenStatus {
    pub device: String,
    pub label: String,
    pub current: Option<String>,
}

pub struct Engine {
    layer: Option<Layer>,
    screens: Vec<Screen>,
    music: Option<Player>,
    music_list: Vec<PathBuf>,
    opts: Options,
    user_paused: bool,
    auto_paused: bool,
    errors: Vec<String>,
    controller: Option<HWND>,
    dirty_layout: bool,
    /// Слой не нашёлся — следующая попытка не раньше этого времени.
    layer_retry_at: Option<Instant>,
}

thread_local! {
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
}

static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
const TIMER_ID: usize = 1;
const LAYER_RETRY: Duration = Duration::from_secs(10);

/// Выполняет `f` с движком. Из обработчиков окон — только если движок
/// сейчас никем не занят (повторный вход пропускается).
pub fn with<R>(f: impl FnOnce(&mut Engine) -> R) -> Option<R> {
    ENGINE.with(|cell| {
        let mut guard = cell.try_borrow_mut().ok()?;
        guard.as_mut().map(f)
    })
}

pub fn init(opts: Options) {
    let controller = create_controller();
    let engine = Engine {
        layer: None,
        screens: desktop::monitors()
            .into_iter()
            .map(|monitor| Screen {
                monitor,
                host: None,
                player: None,
                failed: false,
                current: None,
            })
            .collect(),
        music: None,
        music_list: Vec::new(),
        opts,
        user_paused: false,
        auto_paused: false,
        errors: Vec::new(),
        controller,
        dirty_layout: false,
        layer_retry_at: None,
    };
    ENGINE.with(|cell| *cell.borrow_mut() = Some(engine));
}

pub fn shutdown() {
    let engine = ENGINE.with(|cell| cell.borrow_mut().take());
    if let Some(mut e) = engine {
        e.music = None;
        let had_any = e.screens.iter().any(|s| s.host.is_some());
        for s in &mut e.screens {
            s.player = None;
            if let Some(h) = s.host.take() {
                desktop::destroy(h);
            }
        }
        if let Some(c) = e.controller {
            desktop::destroy(c);
        }
        if had_any {
            desktop::refresh_system_wallpaper();
        }
    }
}

fn label_for(index: usize, m: &Monitor) -> String {
    let mut s = format!("Монитор {} · {}×{}", index + 1, m.width(), m.height());
    if m.primary {
        s.push_str(" · основной");
    }
    s
}

impl Engine {
    pub fn take_errors(&mut self) -> Vec<String> {
        std::mem::take(&mut self.errors)
    }

    pub fn layer_name(&self) -> Option<&'static str> {
        self.layer.map(|l| l.describe())
    }

    pub fn screens(&self) -> Vec<ScreenStatus> {
        self.screens
            .iter()
            .enumerate()
            .map(|(i, s)| ScreenStatus {
                device: s.monitor.device.clone(),
                label: label_for(i, &s.monitor),
                current: s.current.as_ref().map(|w| w.id.clone()),
            })
            .collect()
    }

    pub fn is_paused(&self) -> bool {
        self.user_paused
    }

    pub fn is_auto_paused(&self) -> bool {
        self.auto_paused && !self.user_paused
    }

    pub fn any_playing(&self) -> bool {
        self.screens.iter().any(|s| s.current.is_some()) || self.music.is_some()
    }

    fn layer(&mut self) -> Option<Layer> {
        if let Some(l) = self.layer.filter(|l| l.is_alive()) {
            return Some(l);
        }
        if self.layer_retry_at.is_some_and(|t| Instant::now() < t) {
            return None;
        }
        self.layer = desktop::find_layer();
        if self.layer.is_some() {
            self.layer_retry_at = None;
        } else {
            // Сообщаем один раз за серию неудач, ищем не чаще раза в 10 с.
            if self.layer_retry_at.is_none() {
                self.errors.push(
                    "Не удалось найти слой рабочего стола. Перезапустите проводник или программу."
                        .into(),
                );
            }
            self.layer_retry_at = Some(Instant::now() + LAYER_RETRY);
        }
        self.layer
    }

    /// Ставит обои на монитор (`None` — на все) или убирает их.
    pub fn set(&mut self, device: Option<&str>, wallpaper: Option<Wallpaper>) {
        let indices: Vec<usize> = (0..self.screens.len())
            .filter(|&i| device.is_none_or(|d| self.screens[i].monitor.device == d))
            .collect();
        // Действие пользователя — ищем слой сразу, без ожидания.
        self.layer_retry_at = None;
        for i in indices {
            self.screens[i].current = wallpaper.clone();
            self.start_screen(i);
        }
        self.apply_audio();
    }

    fn stop_screen(&mut self, i: usize) {
        let s = &mut self.screens[i];
        s.player = None;
        if let Some(h) = s.host.take() {
            desktop::destroy(h);
        }
    }

    fn start_screen(&mut self, i: usize) {
        self.stop_screen(i);
        self.screens[i].failed = false;
        let Some(wp) = self.screens[i].current.clone() else {
            return;
        };
        let Some(layer) = self.layer() else { return };
        let monitor = self.screens[i].monitor.clone();
        let Some(host) = desktop::create_host(layer, &monitor) else {
            self.errors.push("Не удалось создать окно обоев.".into());
            self.screens[i].failed = true;
            return;
        };
        self.screens[i].host = Some(host);
        match self.create_player(host, &wp) {
            Ok(p) => self.screens[i].player = Some(p),
            Err(e) => {
                self.errors.push(format!("mpv: {e}"));
                self.stop_screen(i);
                self.screens[i].failed = true;
            }
        }
    }

    fn create_player(&self, host: HWND, wp: &Wallpaper) -> Result<Player, String> {
        let o = &self.opts;
        let opts = crate::mpv::wallpaper_options(
            Some((host.0 as usize).to_string()),
            wp.kind,
            o.fit,
            o.hwdec,
            o.volume,
            self.user_paused || self.auto_paused,
        );
        let refs: Vec<(&str, &str)> = opts.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let player = Player::new(&refs)?;
        player.load(&wp.path.to_string_lossy())?;
        Ok(player)
    }

    /// Звук идёт только с одного монитора — с первого по порядку
    /// (основной идёт первым), на котором что-то стоит. Иначе одно и то же
    /// видео на двух мониторах звучит эхом.
    fn apply_audio(&mut self) {
        let loud = self.screens.iter().position(|s| s.player.is_some());
        for (i, s) in self.screens.iter().enumerate() {
            if let Some(p) = &s.player {
                let mute = self.opts.muted || Some(i) != loud;
                p.set("mute", if mute { "yes" } else { "no" });
                p.set("volume", &self.opts.volume.to_string());
            }
        }
        if let Some(m) = &self.music {
            m.set("volume", &self.opts.music_volume.to_string());
        }
    }

    pub fn set_options(&mut self, opts: Options) {
        let restart = opts.fit != self.opts.fit || opts.hwdec != self.opts.hwdec;
        let reshuffle = opts.music_shuffle != self.opts.music_shuffle;
        self.opts = opts;
        if restart {
            for i in 0..self.screens.len() {
                self.start_screen(i);
            }
        }
        if !self.opts.pause_on_fullscreen && self.auto_paused {
            self.auto_paused = false;
            self.apply_pause();
        }
        if reshuffle && self.music.is_some() {
            let list = self.music_list.clone();
            self.set_music(&list);
        }
        self.apply_audio();
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.user_paused = paused;
        self.apply_pause();
    }

    fn apply_pause(&self) {
        let wall = if self.user_paused || self.auto_paused {
            "yes"
        } else {
            "no"
        };
        for s in &self.screens {
            if let Some(p) = &s.player {
                p.set("pause", wall);
            }
        }
        if let Some(m) = &self.music {
            m.set("pause", if self.user_paused { "yes" } else { "no" });
        }
    }

    /// Фоновая музыка: список файлов по кругу. Пустой список — тишина.
    pub fn set_music(&mut self, files: &[PathBuf]) {
        self.music = None;
        self.music_list = files.to_vec();
        if files.is_empty() {
            return;
        }
        let opts = crate::mpv::music_options(
            self.opts.music_shuffle,
            self.opts.music_volume,
            self.user_paused,
        );
        let opts: Vec<(&str, &str)> = opts.iter().map(|(k, v)| (*k, v.as_str())).collect();
        match Player::new(&opts) {
            Ok(p) => {
                for (n, f) in files.iter().enumerate() {
                    let mode = if n == 0 { "replace" } else { "append" };
                    if let Err(e) = p.command(&["loadfile", &f.to_string_lossy(), mode]) {
                        self.errors.push(format!("Музыка: {e}"));
                    }
                }
                self.music = Some(p);
            }
            Err(e) => self.errors.push(format!("Музыка: {e}")),
        }
    }

    pub fn music_title(&self) -> Option<String> {
        self.music.as_ref().and_then(|m| m.get("media-title"))
    }

    pub fn music_next(&self) {
        if let Some(m) = &self.music {
            let _ = m.command(&["playlist-next", "force"]);
        }
    }

    /// Мониторы поменялись: заново раскладываем окна, обои сохраняются
    /// по имени устройства.
    pub fn refresh_monitors(&mut self) {
        let old: Vec<(String, Option<Wallpaper>)> = self
            .screens
            .iter()
            .map(|s| (s.monitor.device.clone(), s.current.clone()))
            .collect();
        for i in 0..self.screens.len() {
            self.stop_screen(i);
        }
        self.screens = desktop::monitors()
            .into_iter()
            .map(|monitor| {
                let current = old
                    .iter()
                    .find(|(d, _)| *d == monitor.device)
                    .and_then(|(_, w)| w.clone());
                Screen {
                    monitor,
                    host: None,
                    player: None,
                    failed: false,
                    current,
                }
            })
            .collect();
        self.layer = None;
        for i in 0..self.screens.len() {
            self.start_screen(i);
        }
        self.apply_audio();
    }

    fn own_windows(&self) -> Vec<HWND> {
        self.screens.iter().filter_map(|s| s.host).collect()
    }

    fn tick(&mut self) {
        for s in &self.screens {
            if let Some(p) = &s.player {
                p.drain_events();
            }
        }
        if let Some(m) = &self.music {
            m.drain_events();
        }

        if self.dirty_layout {
            self.dirty_layout = false;
            self.refresh_monitors();
            return;
        }

        // Проводник перезапустился — слой и наши окна пропали.
        let layer_ok = self.layer.is_some_and(|l| l.is_alive());
        let lost: Vec<usize> = (0..self.screens.len())
            .filter(|&i| {
                let s = &self.screens[i];
                s.current.is_some()
                    && !s.failed
                    && match (s.host, self.layer) {
                        (Some(h), Some(l)) => !layer_ok || !desktop::host_alive(h, l),
                        _ => true,
                    }
            })
            .collect();
        if !lost.is_empty() {
            if !layer_ok {
                self.layer = None;
            }
            for i in lost {
                self.start_screen(i);
            }
            self.apply_audio();
        }

        if self.opts.pause_on_fullscreen && self.screens.iter().any(|s| s.player.is_some()) {
            let busy = desktop::fullscreen_app_active(&self.own_windows());
            if busy != self.auto_paused {
                self.auto_paused = busy;
                self.apply_pause();
            }
        }
    }
}

unsafe extern "system" fn controller_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let taskbar = TASKBAR_CREATED.load(Ordering::Relaxed);
    if msg == WM_TIMER && wp.0 == TIMER_ID {
        with(|e| e.tick());
        return LRESULT(0);
    }
    if msg == crate::system::WM_SHOW_PANEL {
        crate::system::show_panel();
        return LRESULT(0);
    }
    if msg == WM_DISPLAYCHANGE || (taskbar != 0 && msg == taskbar) {
        with(|e| {
            e.layer = None;
            e.layer_retry_at = None;
            e.dirty_layout = true;
        });
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

fn create_controller() -> Option<HWND> {
    unsafe {
        TASKBAR_CREATED.store(
            RegisterWindowMessageW(w!("TaskbarCreated")),
            Ordering::Relaxed,
        );
        let instance = GetModuleHandleW(None).ok()?;
        let class = crate::system::CONTROLLER_CLASS;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(controller_proc),
            hInstance: instance.into(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassW(&wc);
        // Обычное скрытое окно, а не message-only: только такие получают
        // широковещательные TaskbarCreated и WM_DISPLAYCHANGE.
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class,
            w!("Живые обои — управление"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .ok()?;
        SetTimer(Some(hwnd), TIMER_ID, 1000, None);
        Some(hwnd)
    }
}
