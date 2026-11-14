//! Панель управления: библиотека, открытые ресурсы, настройки и строка
//! «сейчас на экране» внизу.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Color32, CornerRadius, Frame, Layout, Margin, RichText, Sense, Stroke, Ui, Vec2,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;

use crate::engine::{self, Options, ScreenStatus, Wallpaper};
use crate::library::{self, Fit, Item, Kind, Library, Paths, Settings, Source};
use crate::online::{self, Download, Remote, SearchResult};
use crate::system::{self, Panel};
use crate::theme::{self, *};
use crate::thumbs::{self, Thumbnailer};

pub const TITLE: &str = "Живые обои";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Library,
    Online,
    Settings,
}

struct OnlineState {
    query: String,
    kind: Kind,
    hd: bool,
    results: Vec<Remote>,
    next: Option<u32>,
    pending: Option<Receiver<Result<SearchResult, String>>>,
    append: bool,
    error: Option<String>,
    downloads: Vec<Download>,
    thumbs_requested: HashSet<String>,
    searched: bool,
}

pub struct App {
    paths: Paths,
    lib: Library,
    settings: Settings,
    tab: Tab,
    /// Куда ставить обои: `None` — на все мониторы.
    target: Option<String>,
    filter: Option<Kind>,
    lib_search: String,
    thumbs: Thumbnailer,
    online: OnlineState,
    notice: Option<(String, Instant, bool)>,
    mpv_error: Option<String>,
    panel: Option<Panel>,
    _tray: Option<tray_icon::TrayIcon>,
    confirm_remove: Option<String>,
}

fn panel_hwnd(cc: &eframe::CreationContext<'_>) -> Option<HWND> {
    match cc.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut _)),
        _ => None,
    }
}

fn cache_name(url: &str) -> String {
    let mut h = DefaultHasher::new();
    url.hash(&mut h);
    format!("{:016x}.jpg", h.finish())
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> App {
        theme::apply(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);

        let paths = Paths::new();
        let lib = Library::load(&paths);
        let settings = library::load_settings(&paths);
        let mpv_error = crate::mpv::available().err();

        let panel = panel_hwnd(cc).map(Panel);
        if let Some(p) = panel {
            system::set_panel(p);
        }
        let tray = panel.and_then(|p| crate::tray::create(p, cc.egui_ctx.clone()));

        let wake_ctx = cc.egui_ctx.clone();
        let thumbs = Thumbnailer::start(paths.thumbs.clone(), move || wake_ctx.request_repaint());

        engine::init(options_of(&settings));

        let mut app = App {
            paths,
            lib,
            settings,
            tab: Tab::Library,
            target: None,
            filter: None,
            lib_search: String::new(),
            thumbs,
            online: OnlineState {
                query: String::new(),
                kind: Kind::Video,
                hd: true,
                results: Vec::new(),
                next: None,
                pending: None,
                append: false,
                error: None,
                downloads: Vec::new(),
                thumbs_requested: HashSet::new(),
                searched: false,
            },
            notice: None,
            mpv_error,
            panel,
            _tray: tray,
            confirm_remove: None,
        };
        app.restore();
        app.request_missing_thumbs();
        app
    }

    /// Обои и музыка с прошлого запуска.
    fn restore(&mut self) {
        if self.mpv_error.is_some() {
            return;
        }
        let screens: Vec<(String, String)> = self
            .settings
            .screens
            .iter()
            .map(|(d, i)| (d.clone(), i.clone()))
            .collect();
        for (device, id) in screens {
            if let Some(wp) = self.wallpaper(&id) {
                engine::with(|e| e.set(Some(&device), Some(wp)));
            }
        }
        if self.settings.music_on {
            self.restart_music();
        }
    }

    fn wallpaper(&self, id: &str) -> Option<Wallpaper> {
        let item = self.lib.get(id)?;
        item.exists().then(|| Wallpaper {
            id: item.id.clone(),
            path: item.file.clone(),
            kind: item.kind,
        })
    }

    fn request_missing_thumbs(&self) {
        for item in &self.lib.items {
            if item.kind != Kind::Audio
                && item.exists()
                && item.thumb.as_ref().is_none_or(|t| !t.exists())
            {
                self.thumbs.request(thumbs::Job {
                    id: item.id.clone(),
                    file: item.file.clone(),
                    kind: item.kind,
                });
            }
        }
    }

    fn notify(&mut self, text: impl Into<String>) {
        self.notice = Some((text.into(), Instant::now(), false));
    }

    fn notify_error(&mut self, text: impl Into<String>) {
        self.notice = Some((text.into(), Instant::now(), true));
    }

    fn save_settings(&self) {
        library::save_settings(&self.paths, &self.settings);
    }

    fn push_options(&self) {
        let opts = options_of(&self.settings);
        engine::with(|e| e.set_options(opts));
    }

    fn apply(&mut self, id: &str) {
        let Some(wp) = self.wallpaper(id) else {
            self.notify_error("Файл не найден — возможно, его переместили или удалили.");
            return;
        };
        let target = self.target.clone();
        let screens = engine::with(|e| {
            e.set(target.as_deref(), Some(wp));
            e.screens()
        })
        .unwrap_or_default();
        for s in screens {
            if target.as_ref().is_none_or(|t| *t == s.device) {
                self.settings.screens.insert(s.device, id.to_string());
            }
        }
        self.save_settings();
    }

    fn clear(&mut self, device: Option<&str>) {
        engine::with(|e| e.set(device, None));
        match device {
            Some(d) => {
                self.settings.screens.remove(d);
            }
            None => self.settings.screens.clear(),
        }
        self.save_settings();
    }

    fn restart_music(&mut self) {
        let files: Vec<PathBuf> = if self.settings.music_on {
            self.settings
                .music
                .iter()
                .filter_map(|id| self.lib.get(id))
                .filter(|i| i.exists())
                .map(|i| i.file.clone())
                .collect()
        } else {
            Vec::new()
        };
        engine::with(|e| e.set_music(&files));
    }

    fn toggle_music_item(&mut self, id: &str) {
        if let Some(pos) = self.settings.music.iter().position(|m| m == id) {
            self.settings.music.remove(pos);
        } else {
            self.settings.music.push(id.to_string());
            self.settings.music_on = true;
        }
        self.save_settings();
        self.restart_music();
    }

    fn remove_item(&mut self, id: &str) {
        let on_screen: Vec<String> = self
            .settings
            .screens
            .iter()
            .filter(|(_, v)| v.as_str() == id)
            .map(|(d, _)| d.clone())
            .collect();
        for d in on_screen {
            self.clear(Some(&d));
        }
        let in_music = self.settings.music.iter().any(|m| m == id);
        self.settings.music.retain(|m| m != id);
        self.lib.remove(id);
        self.lib.save();
        self.save_settings();
        if in_music {
            self.restart_music();
        }
    }

    fn add_files(&mut self, files: Vec<PathBuf>) {
        let mut added = 0;
        let mut skipped = 0;
        for f in files {
            let before = self.lib.items.len();
            match self.lib.add_local(&f) {
                Some(id) if self.lib.items.len() > before => {
                    added += 1;
                    if let Some(item) = self.lib.get(&id) {
                        self.thumbs.request(thumbs::Job {
                            id: id.clone(),
                            file: item.file.clone(),
                            kind: item.kind,
                        });
                    }
                }
                Some(_) => {}
                None => skipped += 1,
            }
        }
        self.lib.save();
        match (added, skipped) {
            (0, 0) => self.notify("Эти файлы уже в библиотеке."),
            (a, 0) => self.notify(format!("Добавлено в библиотеку: {a}")),
            (a, s) => self.notify_error(format!("Добавлено: {a}. Не подошло по формату: {s}")),
        }
    }

    fn poll_background(&mut self, ctx: &egui::Context) {
        while let Ok(done) = self.thumbs.rx.try_recv() {
            if let (Some(item), Some(t)) = (self.lib.get_mut(&done.id), done.thumb) {
                ctx.forget_image(&format!("file://{}", t.display()));
                item.thumb = Some(t);
                self.lib.save();
            }
        }

        if let Some(rx) = &self.online.pending {
            if let Ok(res) = rx.try_recv() {
                self.online.pending = None;
                match res {
                    Ok(r) => {
                        if !self.online.append {
                            self.online.results.clear();
                        }
                        self.online.results.extend(r.items);
                        self.online.next = r.next;
                        self.online.error = None;
                    }
                    Err(e) => self.online.error = Some(e),
                }
            }
        }

        let mut finished = Vec::new();
        self.online.downloads.retain(|d| match d.result.try_recv() {
            Ok(res) => {
                finished.push((d.remote.clone(), res));
                false
            }
            Err(_) => true,
        });
        for (remote, res) in finished {
            match res {
                Ok((file, thumb)) => {
                    let id = library::new_id();
                    self.lib.push(Item {
                        id: id.clone(),
                        title: remote.title.clone(),
                        kind: remote.kind,
                        file: file.clone(),
                        thumb: thumb.clone(),
                        source: Some(Source {
                            site: "Викисклад".into(),
                            page: remote.page.clone(),
                            author: remote.author.clone(),
                            license: remote.license.clone(),
                        }),
                        owned: true,
                        added: library::now(),
                    });
                    self.lib.save();
                    if thumb.is_none() && remote.kind != Kind::Audio {
                        self.thumbs.request(thumbs::Job {
                            id,
                            file,
                            kind: remote.kind,
                        });
                    }
                    self.notify(format!("Скачано в библиотеку: {}", remote.title));
                }
                Err(e) if e == "Отменено" => {}
                Err(e) => self.notify_error(format!("{}: {e}", remote.title)),
            }
        }

        let errors = engine::with(|e| e.take_errors()).unwrap_or_default();
        if let Some(last) = errors.into_iter().last() {
            self.notify_error(last);
        }

        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        if !dropped.is_empty() {
            self.add_files(dropped);
        }
    }

    fn start_search(&mut self, ctx: &egui::Context, append: bool) {
        let offset = if append {
            self.online.next.unwrap_or(0)
        } else {
            0
        };
        let wake = ctx.clone();
        self.online.append = append;
        self.online.searched = true;
        self.online.pending = Some(online::spawn_search(
            self.online.query.clone(),
            self.online.kind,
            self.online.hd,
            offset,
            move || wake.request_repaint(),
        ));
    }

    fn start_download(&mut self, ctx: &egui::Context, remote: Remote) {
        let wake = ctx.clone();
        let id = library::new_id();
        self.online.downloads.push(online::spawn_download(
            remote,
            self.paths.media.clone(),
            self.paths.thumbs.clone(),
            id,
            move || wake.request_repaint(),
        ));
    }
}

fn options_of(s: &Settings) -> Options {
    Options {
        volume: s.volume,
        muted: s.muted,
        fit: s.fit,
        hwdec: s.hwdec,
        pause_on_fullscreen: s.pause_on_fullscreen,
        music_volume: s.music_volume,
        music_shuffle: s.music_shuffle,
    }
}

// ——— Мелкие компоненты ———

fn primary_button(ui: &mut Ui, text: &str) -> egui::Response {
    let hovered_id = ui.next_auto_id();
    let hovered = ui
        .ctx()
        .read_response(hovered_id)
        .is_some_and(|r| r.hovered());
    ui.add(
        egui::Button::new(RichText::new(text).color(Color32::WHITE))
            .fill(if hovered { PRIMARY_HOVER } else { PRIMARY })
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(8)),
    )
}

fn secondary_button(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(text).color(INK)).corner_radius(CornerRadius::same(8)))
}

fn chip(ui: &mut Ui, selected: bool, text: &str) -> egui::Response {
    let (fill, color, stroke) = if selected {
        (SURFACE_2, INK, Stroke::new(1.0, HAIRLINE_STRONG))
    } else {
        (CANVAS, INK_SUBTLE, Stroke::new(1.0, HAIRLINE))
    };
    ui.add(
        egui::Button::new(RichText::new(text).color(color))
            .fill(fill)
            .stroke(stroke)
            .corner_radius(CornerRadius::same(255))
            .min_size(Vec2::new(0.0, 32.0)),
    )
}

fn badge(ui: &mut Ui, text: &str, dot: Option<Color32>) {
    Frame::new()
        .fill(SURFACE_2)
        .corner_radius(CornerRadius::same(255))
        .inner_margin(Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if let Some(c) = dot {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(7.0), Sense::hover());
                    ui.painter().circle_filled(r.center(), 3.5, c);
                }
                ui.label(RichText::new(text).size(12.0).color(INK_MUTED));
            });
        });
}

fn eyebrow(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .size(12.0)
            .color(INK_SUBTLE)
            .extra_letter_spacing(0.4),
    );
}

fn card_frame() -> Frame {
    Frame::new()
        .fill(SURFACE_1)
        .stroke(Stroke::new(1.0, HAIRLINE))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(12))
}

/// Превью 16:9: картинка по центру на подложке или подпись типа.
fn preview(ui: &mut Ui, width: f32, src: Option<String>, kind: Kind) {
    let size = Vec2::new(width, (width * 9.0 / 16.0).round());
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(8), CANVAS);
    match src {
        Some(uri) => {
            let img = egui::Image::new(uri)
                .max_size(size)
                .maintain_aspect_ratio(true)
                .corner_radius(CornerRadius::same(8));
            ui.put(rect, img);
        }
        None => {
            let label = match kind {
                Kind::Audio => "♪",
                Kind::Image => "▣",
                _ => "▶",
            };
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(32.0),
                INK_TERTIARY,
            );
        }
    }
}

/// Сетка карточек: 3 и больше колонок на широком окне, меньше на узком.
fn grid<T>(ui: &mut Ui, items: &[T], mut card: impl FnMut(&mut Ui, &T, f32)) {
    let gap = 16.0;
    let avail = ui.available_width();
    let cols = (((avail + gap) / (260.0 + gap)).floor() as usize).max(1);
    let width = ((avail - gap * (cols as f32 - 1.0)) / cols as f32).floor();
    for row in items.chunks(cols) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for item in row {
                ui.allocate_ui_with_layout(
                    Vec2::new(width, 0.0),
                    Layout::top_down(Align::Min),
                    |ui| {
                        ui.set_width(width);
                        card(ui, item, width);
                    },
                );
            }
        });
        ui.add_space(gap - 8.0);
    }
}

fn kind_meta(item: &Item) -> String {
    let mut s = item.kind.label().to_string();
    match &item.source {
        Some(src) => {
            if !src.license.is_empty() {
                s.push_str(" · ");
                s.push_str(&src.license);
            }
            if !src.author.is_empty() {
                s.push_str(" · ");
                s.push_str(&src.author);
            }
        }
        None => s.push_str(" · свой файл"),
    }
    s
}

enum CardAction {
    Apply(String),
    Music(String),
    Remove(String),
    Reveal(PathBuf),
    Open(String),
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = &ui.ctx().clone();
        self.poll_background(ctx);

        // Крестик прячет окно в трей, пока на столе что-то играет.
        if ctx.input(|i| i.viewport().close_requested()) {
            let busy = engine::with(|e| e.any_playing()).unwrap_or(false);
            if busy {
                if let Some(p) = self.panel {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    p.hide();
                }
            }
        }

        let screens = engine::with(|e| e.screens()).unwrap_or_default();

        egui::Panel::top("top")
            .frame(
                Frame::new()
                    .fill(CANVAS)
                    .inner_margin(Margin::symmetric(24, 12)),
            )
            .show_separator_line(false)
            .show(ui, |ui| self.top_bar(ui));

        egui::Panel::bottom("now")
            .frame(
                Frame::new()
                    .fill(SURFACE_1)
                    .stroke(Stroke::new(1.0, HAIRLINE))
                    .inner_margin(Margin::symmetric(24, 12)),
            )
            .show_separator_line(false)
            .show(ui, |ui| self.now_playing(ui, &screens));

        egui::CentralPanel::default()
            .frame(Frame::new().fill(CANVAS).inner_margin(Margin { left: 24, right: 24, top: 8, bottom: 0 }))
            .show(ui, |ui| {
                if let Some(err) = &self.mpv_error {
                    Frame::new()
                        .fill(SURFACE_2)
                        .stroke(Stroke::new(1.0, DANGER))
                        .corner_radius(CornerRadius::same(12))
                        .inner_margin(Margin::same(16))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(RichText::new("Не найден проигрыватель mpv").font(semibold(15.0)).color(INK));
                            ui.label(
                                RichText::new(format!(
                                    "Положите libmpv-2.dll рядом с программой — она есть в архиве релиза. ({err})"
                                ))
                                .color(INK_SUBTLE),
                            );
                        });
                    ui.add_space(12.0);
                }
                match self.tab {
                    Tab::Library => self.library_tab(ui, &screens),
                    Tab::Online => self.online_tab(ui),
                    Tab::Settings => self.settings_tab(ui),
                }
            });

        self.notice_overlay(ctx);
        self.confirm_dialog(ctx);

        if !self.online.downloads.is_empty() || self.online.pending.is_some() {
            ctx.request_repaint_after(Duration::from_millis(200));
        } else {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }
}

impl App {
    fn top_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
            ui.painter()
                .rect_filled(rect, CornerRadius::same(6), PRIMARY);
            let c = rect.center();
            ui.painter().add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(c.x - 3.5, c.y - 5.5),
                    egui::pos2(c.x + 5.5, c.y),
                    egui::pos2(c.x - 3.5, c.y + 5.5),
                ],
                Color32::WHITE,
                Stroke::NONE,
            ));
            ui.label(RichText::new(TITLE).font(semibold(16.0)).color(INK));
            ui.add_space(24.0);
            for (tab, name) in [
                (Tab::Library, "Библиотека"),
                (Tab::Online, "Открытые ресурсы"),
                (Tab::Settings, "Настройки"),
            ] {
                if chip(ui, self.tab == tab, name).clicked() {
                    self.tab = tab;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (paused, auto) =
                    engine::with(|e| (e.is_paused(), e.is_auto_paused())).unwrap_or((false, false));
                let playing = engine::with(|e| e.any_playing()).unwrap_or(false);
                if !playing {
                    badge(ui, "Ничего не играет", None);
                } else if paused {
                    badge(ui, "Пауза", Some(INK_TERTIARY));
                } else if auto {
                    badge(ui, "Пауза: окно на весь экран", Some(INK_TERTIARY));
                } else {
                    badge(ui, "Играет", Some(SUCCESS));
                }
            });
        });
    }

    fn now_playing(&mut self, ui: &mut Ui, screens: &[ScreenStatus]) {
        ui.horizontal_wrapped(|ui| {
            ui.vertical(|ui| {
                eyebrow(ui, "Сейчас на рабочем столе");
                for s in screens {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&s.label).size(12.0).color(INK_SUBTLE));
                        match s.current.as_ref().and_then(|id| self.lib.get(id)) {
                            Some(item) => {
                                ui.label(RichText::new(&item.title).color(INK));
                                if ui.small_button("Убрать").clicked() {
                                    let d = s.device.clone();
                                    self.clear(Some(&d));
                                }
                            }
                            None => {
                                ui.label(RichText::new("обычные обои Windows").color(INK_TERTIARY));
                            }
                        }
                    });
                }
            });

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let paused = engine::with(|e| e.is_paused()).unwrap_or(false);
                if primary_button(ui, if paused { "Продолжить" } else { "Пауза" }).clicked() {
                    engine::with(|e| e.set_paused(!paused));
                }
                ui.add_space(16.0);

                let mut changed = false;
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        changed |= ui
                            .add(egui::Slider::new(&mut self.settings.volume, 0..=100).show_value(false))
                            .changed();
                        let mut sound = !self.settings.muted;
                        if ui.checkbox(&mut sound, "Звук обоев").changed() {
                            self.settings.muted = !sound;
                            changed = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        changed |= ui
                            .add(egui::Slider::new(&mut self.settings.music_volume, 0..=100).show_value(false))
                            .changed();
                        let mut on = self.settings.music_on;
                        let count = self.settings.music.len();
                        if ui
                            .checkbox(&mut on, format!("Музыка ({count})"))
                            .on_hover_text("Звуковые файлы из библиотеки, отмеченные «В музыку», играют по кругу")
                            .changed()
                        {
                            self.settings.music_on = on;
                            self.save_settings();
                            self.restart_music();
                        }
                        if self.settings.music_on && count > 0 && ui.small_button("▶▶").on_hover_text("Следующий трек").clicked() {
                            engine::with(|e| e.music_next());
                        }
                    });
                });
                if changed {
                    self.save_settings();
                    self.push_options();
                }
                if let Some(title) = engine::with(|e| e.music_title()).flatten() {
                    ui.label(RichText::new(format!("♪ {title}")).size(12.0).color(INK_SUBTLE));
                }
            });
        });
    }

    fn target_picker(&mut self, ui: &mut Ui, screens: &[ScreenStatus]) {
        if screens.len() < 2 {
            self.target = None;
            return;
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Куда ставить:").color(INK_SUBTLE));
            if chip(ui, self.target.is_none(), "Все мониторы").clicked() {
                self.target = None;
            }
            for s in screens {
                let selected = self.target.as_deref() == Some(s.device.as_str());
                if chip(ui, selected, &s.label).clicked() {
                    self.target = Some(s.device.clone());
                }
            }
        });
    }

    fn library_tab(&mut self, ui: &mut Ui, screens: &[ScreenStatus]) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Библиотека")
                    .font(semibold(28.0))
                    .color(INK)
                    .extra_letter_spacing(-0.6),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if primary_button(ui, "Добавить файлы").clicked() {
                    if let Some(files) = rfd::FileDialog::new()
                        .set_title("Видео, анимация, картинки или звук")
                        .add_filter("Медиа", Kind::extensions())
                        .pick_files()
                    {
                        self.add_files(files);
                    }
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.lib_search)
                        .hint_text("Поиск по названию")
                        .desired_width(200.0)
                        .margin(Margin::symmetric(12, 8)),
                );
            });
        });
        ui.label(
            RichText::new("Свои файлы можно просто перетащить в это окно. Файлы остаются на месте — в библиотеку попадает ссылка.")
                .color(INK_SUBTLE),
        );
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if chip(ui, self.filter.is_none(), "Всё").clicked() {
                self.filter = None;
            }
            for k in Kind::ALL {
                if chip(ui, self.filter == Some(k), k.label()).clicked() {
                    self.filter = Some(k);
                }
            }
        });
        self.target_picker(ui, screens);
        ui.add_space(8.0);

        let needle = self.lib_search.to_lowercase();
        let mut items: Vec<Item> = self
            .lib
            .items
            .iter()
            .filter(|i| self.filter.is_none_or(|k| i.kind == k))
            .filter(|i| needle.is_empty() || i.title.to_lowercase().contains(&needle))
            .cloned()
            .collect();
        items.sort_by_key(|i| std::cmp::Reverse(i.added));

        if self.lib.items.is_empty() {
            card_frame().inner_margin(Margin::same(32)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Библиотека пуста").font(semibold(22.0)).color(INK));
                ui.label(
                    RichText::new("Добавьте свои видео, GIF, картинки или музыку — или скачайте свободные ролики с Викисклада.")
                        .color(INK_SUBTLE),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if primary_button(ui, "Открытые ресурсы").clicked() {
                        self.tab = Tab::Online;
                    }
                });
            });
            return;
        }

        let on_screen: HashSet<String> = screens.iter().filter_map(|s| s.current.clone()).collect();
        let music: HashSet<String> = self.settings.music.iter().cloned().collect();
        let mut action = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                grid(ui, &items, |ui, item, width| {
                    card_frame().show(ui, |ui| {
                        let inner = width - 24.0;
                        let src = item
                            .thumb
                            .as_ref()
                            .filter(|t| t.exists())
                            .map(|t| format!("file://{}", t.display()));
                        preview(ui, inner, src, item.kind);
                        ui.add_space(4.0);
                        ui.add(
                            egui::Label::new(
                                RichText::new(&item.title).font(semibold(15.0)).color(INK),
                            )
                            .truncate(),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(kind_meta(item)).size(12.0).color(INK_SUBTLE),
                            )
                            .truncate(),
                        );
                        if !item.exists() {
                            ui.label(RichText::new("Файл не найден").size(12.0).color(DANGER));
                        }
                        ui.horizontal(|ui| {
                            if item.kind == Kind::Audio {
                                let label = if music.contains(&item.id) {
                                    "Убрать из музыки"
                                } else {
                                    "В музыку"
                                };
                                if primary_button(ui, label).clicked() {
                                    action = Some(CardAction::Music(item.id.clone()));
                                }
                                if secondary_button(ui, "Спектр")
                                    .on_hover_text("Звук со спектром на рабочем столе")
                                    .clicked()
                                {
                                    action = Some(CardAction::Apply(item.id.clone()));
                                }
                            } else if primary_button(ui, "На рабочий стол").clicked() {
                                action = Some(CardAction::Apply(item.id.clone()));
                            }
                            if on_screen.contains(&item.id) {
                                badge(ui, "на экране", Some(SUCCESS));
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.menu_button("⋯", |ui| {
                                    if ui.button("Показать в папке").clicked() {
                                        action = Some(CardAction::Reveal(item.file.clone()));
                                    }
                                    if let Some(src) = &item.source {
                                        if ui.button("Страница источника").clicked()
                                        {
                                            action = Some(CardAction::Open(src.page.clone()));
                                        }
                                    }
                                    if ui.button("Удалить из библиотеки").clicked()
                                    {
                                        action = Some(CardAction::Remove(item.id.clone()));
                                    }
                                });
                            });
                        });
                    });
                });
                if items.is_empty() {
                    ui.label(RichText::new("Ничего не найдено.").color(INK_SUBTLE));
                }
            });

        match action {
            Some(CardAction::Apply(id)) => self.apply(&id),
            Some(CardAction::Music(id)) => self.toggle_music_item(&id),
            Some(CardAction::Remove(id)) => self.confirm_remove = Some(id),
            Some(CardAction::Reveal(path)) => {
                if let Some(dir) = path.parent() {
                    system::open(&dir.to_string_lossy());
                }
            }
            Some(CardAction::Open(url)) => system::open(&url),
            None => {}
        }
    }

    fn online_tab(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        ui.label(
            RichText::new("Открытые ресурсы")
                .font(semibold(28.0))
                .color(INK)
                .extra_letter_spacing(-0.6),
        );
        ui.label(
            RichText::new(
                "Поиск по Викискладу (Wikimedia Commons): только свободные лицензии — CC0, CC BY, CC BY-SA и общественное достояние. \
                 Автор и лицензия сохраняются в библиотеке; если показываете обои другим, указывайте автора.",
            )
            .color(INK_SUBTLE),
        );
        ui.add_space(8.0);

        let mut go = false;
        ui.horizontal(|ui| {
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.online.query)
                    .hint_text("Например: ocean, rain, space, fire, forest")
                    .desired_width(360.0)
                    .margin(Margin::symmetric(12, 8)),
            );
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go = true;
            }
            if primary_button(ui, "Найти").clicked() {
                go = true;
            }
            ui.checkbox(&mut self.online.hd, "Только HD")
                .on_hover_text("Ширина от 1280 точек (для звука не действует)");
        });
        ui.horizontal_wrapped(|ui| {
            for k in Kind::ALL {
                if chip(ui, self.online.kind == k, k.label()).clicked() && self.online.kind != k {
                    self.online.kind = k;
                    if self.online.searched {
                        go = true;
                    }
                }
            }
            ui.add_space(12.0);
            ui.label(RichText::new("Идеи:").color(INK_TERTIARY));
            for idea in [
                "ocean",
                "rain",
                "clouds timelapse",
                "fire",
                "aurora",
                "forest",
                "space",
                "snow",
            ] {
                if ui.link(RichText::new(idea).size(13.0)).clicked() {
                    self.online.query = idea.into();
                    go = true;
                }
            }
        });
        if go && self.online.pending.is_none() {
            self.start_search(&ctx, false);
        }
        ui.add_space(8.0);

        if let Some(err) = &self.online.error {
            ui.label(RichText::new(err).color(DANGER));
        }
        if self.online.pending.is_some() && !self.online.append {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Ищем…").color(INK_SUBTLE));
            });
        }
        if self.online.searched
            && self.online.results.is_empty()
            && self.online.pending.is_none()
            && self.online.error.is_none()
        {
            ui.label(
                RichText::new("Ничего не нашлось. Попробуйте английское слово или другой тип.")
                    .color(INK_SUBTLE),
            );
        }

        let in_library: HashSet<String> = self
            .lib
            .items
            .iter()
            .filter_map(|i| i.source.as_ref().map(|s| s.page.clone()))
            .collect();
        let results = self.online.results.clone();
        let mut download = None;
        let mut cancel = None;
        let mut open_page = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                grid(ui, &results, |ui, r, width| {
                    card_frame().show(ui, |ui| {
                        let inner = width - 24.0;
                        let src = r.thumb.as_ref().and_then(|url| {
                            let path = self.paths.cache.join(cache_name(url));
                            if path.exists() {
                                Some(format!("file://{}", path.display()))
                            } else {
                                if self.online.thumbs_requested.insert(url.clone()) {
                                    let wake = ctx.clone();
                                    online::spawn_thumb_cache(url.clone(), path, move || {
                                        wake.request_repaint()
                                    });
                                }
                                None
                            }
                        });
                        preview(ui, inner, src, r.kind);
                        ui.add_space(4.0);
                        ui.add(
                            egui::Label::new(
                                RichText::new(&r.title).font(semibold(15.0)).color(INK),
                            )
                            .truncate(),
                        );
                        let mut meta = vec![r.kind.label().to_string()];
                        if r.width > 0 && r.kind != Kind::Audio {
                            meta.push(format!("{}×{}", r.width, r.height));
                        }
                        if r.duration > 0.0 {
                            meta.push(online::human_duration(r.duration));
                        }
                        meta.push(online::human_size(r.size));
                        ui.label(RichText::new(meta.join(" · ")).size(12.0).color(INK_SUBTLE));
                        let credit = [r.license.as_str(), r.author.as_str()]
                            .iter()
                            .filter(|s| !s.is_empty())
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(" · ");
                        ui.add(
                            egui::Label::new(RichText::new(credit).size(12.0).color(INK_TERTIARY))
                                .truncate(),
                        );
                        ui.horizontal(|ui| {
                            if let Some(d) = self
                                .online
                                .downloads
                                .iter()
                                .find(|d| d.remote.page == r.page)
                            {
                                ui.add(
                                    egui::ProgressBar::new(d.progress())
                                        .desired_width(inner - 90.0)
                                        .show_percentage(),
                                );
                                if ui.small_button("Отмена").clicked() {
                                    cancel = Some(r.page.clone());
                                }
                            } else if in_library.contains(&r.page) {
                                badge(ui, "в библиотеке", Some(SUCCESS));
                            } else if primary_button(ui, "Скачать").clicked() {
                                download = Some(r.clone());
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.link("Страница").clicked() {
                                    open_page = Some(r.page.clone());
                                }
                            });
                        });
                    });
                });
                if self.online.next.is_some() && !results.is_empty() {
                    ui.vertical_centered(|ui| {
                        if self.online.pending.is_some() {
                            ui.spinner();
                        } else if secondary_button(ui, "Показать ещё").clicked() {
                            self.start_search(&ctx, true);
                        }
                    });
                    ui.add_space(16.0);
                }
            });

        if let Some(r) = download {
            if r.size > 1024 * 1024 * 1024 {
                self.notify("Файл больше гигабайта — загрузка займёт время.");
            }
            self.start_download(&ctx, r);
        }
        if let Some(page) = cancel {
            for d in &self.online.downloads {
                if d.remote.page == page {
                    d.cancel.store(true, Ordering::Relaxed);
                }
            }
        }
        if let Some(url) = open_page {
            system::open(&url);
        }
    }

    fn settings_tab(&mut self, ui: &mut Ui) {
        ui.label(
            RichText::new("Настройки")
                .font(semibold(28.0))
                .color(INK)
                .extra_letter_spacing(-0.6),
        );
        ui.add_space(8.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let mut changed = false;
            card_frame().inner_margin(Margin::same(24)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                eyebrow(ui, "Картинка");
                ui.horizontal(|ui| {
                    ui.label("Масштаб:");
                    for f in Fit::ALL {
                        if chip(ui, self.settings.fit == f, f.label()).clicked() && self.settings.fit != f {
                            self.settings.fit = f;
                            changed = true;
                        }
                    }
                });
                changed |= ui
                    .checkbox(&mut self.settings.hwdec, "Аппаратное декодирование видео (меньше нагрузка на процессор)")
                    .changed();
                changed |= ui
                    .checkbox(
                        &mut self.settings.pause_on_fullscreen,
                        "Пауза, когда игра, фильм или презентация открыты на весь экран",
                    )
                    .changed();
                changed |= ui.checkbox(&mut self.settings.music_shuffle, "Музыка в случайном порядке").changed();
            });
            ui.add_space(16.0);
            card_frame().inner_margin(Margin::same(24)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                eyebrow(ui, "Запуск");
                let mut auto = self.settings.autostart;
                if ui.checkbox(&mut auto, "Запускать вместе с Windows (сразу в трей, с прошлыми обоями)").changed() {
                    match system::set_autostart(auto) {
                        Ok(()) => {
                            self.settings.autostart = auto;
                            self.save_settings();
                        }
                        Err(e) => self.notify_error(format!("Не удалось изменить автозапуск: {e}")),
                    }
                }
                ui.label(
                    RichText::new("Крестик прячет окно в трей, пока обои или музыка играют. Выход — через значок в трее.")
                        .color(INK_SUBTLE),
                );
            });
            ui.add_space(16.0);
            card_frame().inner_margin(Margin::same(24)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                eyebrow(ui, "Файлы");
                ui.horizontal(|ui| {
                    ui.label(RichText::new(self.paths.root.display().to_string()).monospace().color(INK_MUTED));
                    if secondary_button(ui, "Открыть папку").clicked() {
                        system::open(&self.paths.root.to_string_lossy());
                    }
                });
                ui.label(
                    RichText::new("Скачанные файлы лежат в media, превью — в thumbs. Свои файлы программа не копирует и не удаляет.")
                        .color(INK_SUBTLE),
                );
            });
            ui.add_space(16.0);
            card_frame().inner_margin(Margin::same(24)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                eyebrow(ui, "О программе");
                ui.label(format!("Версия {}", env!("CARGO_PKG_VERSION")));
                let mpv = match &self.mpv_error {
                    None => "mpv найден".to_string(),
                    Some(e) => format!("mpv не найден: {e}"),
                };
                ui.label(RichText::new(mpv).color(INK_SUBTLE));
                if let Some(layer) = engine::with(|e| e.layer_name()).flatten() {
                    ui.label(RichText::new(format!("Слой рабочего стола: {layer}")).color(INK_SUBTLE));
                }
                ui.label(
                    RichText::new(
                        "Видео и звук играет mpv (libmpv, лицензия GPL; исходники — mpv.io). \
                         Открытые ресурсы — Wikimedia Commons, у каждого файла своя свободная лицензия.",
                    )
                    .color(INK_SUBTLE),
                );
            });
            if changed {
                self.save_settings();
                self.push_options();
            }
        });
    }

    fn notice_overlay(&mut self, ctx: &egui::Context) {
        let Some((text, at, error)) = self.notice.clone() else {
            return;
        };
        if at.elapsed() > Duration::from_secs(6) {
            self.notice = None;
            return;
        }
        egui::Area::new(egui::Id::unique("notice"))
            .anchor(egui::Align2::CENTER_BOTTOM, Vec2::new(0.0, -96.0))
            .show(ctx, |ui| {
                Frame::new()
                    .fill(SURFACE_3)
                    .stroke(Stroke::new(
                        1.0,
                        if error { DANGER } else { HAIRLINE_STRONG },
                    ))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.label(RichText::new(text).color(INK));
                    });
            });
        ctx.request_repaint_after(Duration::from_millis(500));
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(id) = self.confirm_remove.clone() else {
            return;
        };
        let Some(item) = self.lib.get(&id).cloned() else {
            self.confirm_remove = None;
            return;
        };
        let mut close = false;
        let mut remove = false;
        egui::Modal::new(egui::Id::unique("confirm-remove")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(
                RichText::new("Удалить из библиотеки?")
                    .font(semibold(18.0))
                    .color(INK),
            );
            ui.label(RichText::new(&item.title).color(INK_MUTED));
            let note = if item.owned {
                "Файл скачан программой — он будет удалён с диска."
            } else {
                "Сам файл останется на месте."
            };
            ui.label(RichText::new(note).color(INK_SUBTLE));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if primary_button(ui, "Удалить").clicked() {
                    remove = true;
                }
                if secondary_button(ui, "Отмена").clicked() {
                    close = true;
                }
            });
        });
        if remove {
            self.remove_item(&id);
            close = true;
        }
        if close {
            self.confirm_remove = None;
        }
    }
}
