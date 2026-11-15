//! Значок в трее: открыть панель, пауза, выход. Обработчики вызываются в
//! главном потоке (их доставляет скрытое окно tray-icon), поэтому движок
//! доступен прямо из них.

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::engine;
use crate::system::Panel;

/// Иконка Vibewall (assets/icon-*.png, нарисована из assets/icon.svg)
/// как RGBA: 32 точки — для трея, 256 — для окна.
pub fn icon_rgba(size: u32) -> (Vec<u8>, u32, u32) {
    let png: &[u8] = if size <= 32 {
        include_bytes!("../assets/icon-32.png")
    } else {
        include_bytes!("../assets/icon-256.png")
    };
    match image::load_from_memory_with_format(png, image::ImageFormat::Png) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            (rgba.into_raw(), w, h)
        }
        // Встроенный PNG проверен сборкой; на всякий случай — пустая иконка.
        Err(_) => (vec![0; 4], 1, 1),
    }
}

pub fn create(panel: Panel, ctx: eframe::egui::Context) -> Option<TrayIcon> {
    let menu = Menu::new();
    let open = MenuItem::new("Открыть Vibewall", true, None);
    let pause = MenuItem::new("Пауза / продолжить", true, None);
    let quit = MenuItem::new("Выход", true, None);
    let _ = menu.append_items(&[&open, &pause, &PredefinedMenuItem::separator(), &quit]);
    let (open_id, pause_id, quit_id) = (open.id().clone(), pause.id().clone(), quit.id().clone());

    let show_ctx = ctx.clone();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        if e.id == open_id {
            panel.show();
            show_ctx.request_repaint();
        } else if e.id == pause_id {
            engine::with(|en| en.set_paused(!en.is_paused()));
            show_ctx.request_repaint();
        } else if e.id == quit_id {
            engine::shutdown();
            std::process::exit(0);
        }
    }));
    TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = e
        {
            panel.show();
            ctx.request_repaint();
        }
    }));

    let (rgba, w, h) = icon_rgba(32);
    let icon = Icon::from_rgba(rgba, w, h).ok()?;
    TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("Vibewall")
        .with_icon(icon)
        .build()
        .ok()
}
